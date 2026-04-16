use chrono::{DateTime, Datelike, Timelike, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, Row};
use std::path::Path;

pub struct Storage {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HourSlot {
    pub id: i64,
    /// UTC-bucketed `YYYY-MM-DDTHH` string. Stored in UTC (not local) so that
    /// a user changing timezones between runs still dedups against the same
    /// hour slot for the same `DateTime<Utc>`. The display layer converts
    /// back to local time at render time.
    pub hour_key: String,
    pub text: String,
    /// Capture time of the FIRST segment in this hour, as milliseconds since
    /// the Unix epoch. Integer storage makes the dedup comparison in
    /// `is_segment_processed` a trivial numeric `>=` — no string-format
    /// contract to keep the rest of the stack honest.
    pub start_time: i64,
    /// Capture time of the MOST RECENT segment in this hour, milliseconds
    /// since the Unix epoch. Updated on every append. `is_segment_processed`
    /// compares against this.
    pub last_updated: i64,
    pub device: String,
    pub segment_count: i64,
}

// Pre-built SQL strings used with `prepare_cached`. Static lifetimes mean the
// cache key is stable and the query plan is reused across every invocation.
// Column list is inlined per-query so each const is a valid standalone SQL
// string the cache can key on.
const GET_HOUR_SLOTS_SQL: &str =
    "SELECT id, hour_key, text, start_time, last_updated, device, segment_count
     FROM hour_slots ORDER BY start_time DESC LIMIT ?1 OFFSET ?2";

const SEARCH_HOUR_SLOTS_SQL: &str =
    "SELECT h.id, h.hour_key, h.text, h.start_time, h.last_updated, h.device, h.segment_count
     FROM hour_slots h
     JOIN hour_slots_fts f ON h.id = f.rowid
     WHERE hour_slots_fts MATCH ?1
     ORDER BY rank
     LIMIT 50";

const GET_SLOTS_BY_DATE_RANGE_SQL: &str =
    "SELECT id, hour_key, text, start_time, last_updated, device, segment_count
     FROM hour_slots
     WHERE hour_key >= ?1 AND hour_key <= ?2
     ORDER BY hour_key ASC";

const GET_AVAILABLE_DATES_SQL: &str =
    "SELECT DISTINCT substr(hour_key, 1, 10) as date FROM hour_slots ORDER BY date DESC";

// Orphan dedup: a segment captured at time T is "already processed" if its
// hour slot exists and `last_updated` (which IS the latest capture time
// appended to that hour, not the transcription completion time) has moved
// past T. Covers non-first-segment orphans — the previous implementation
// only matched on `start_time`, which is fixed on first insert and never
// updated, so every non-first orphan was re-transcribed after a crash.
//
// Both columns are integer epoch milliseconds, so the `>=` is a numeric
// comparison — no RFC3339 lexical-order contract to enforce.
const IS_PROCESSED_SQL: &str =
    "SELECT COUNT(*) FROM hour_slots WHERE hour_key = ?1 AND last_updated >= ?2";

const COUNT_HOUR_SLOTS_SQL: &str = "SELECT COUNT(*) FROM hour_slots";

fn map_hour_slot(row: &Row) -> rusqlite::Result<HourSlot> {
    Ok(HourSlot {
        id: row.get(0)?,
        hour_key: row.get(1)?,
        text: row.get(2)?,
        start_time: row.get(3)?,
        last_updated: row.get(4)?,
        device: row.get(5)?,
        segment_count: row.get(6)?,
    })
}

impl Storage {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

        // WAL + `synchronous=NORMAL` is the standard SQLite profile for a
        // single-writer append-mostly workload. `NORMAL` risks losing the last
        // few seconds of writes on an OS crash (not power loss) — acceptable
        // here because the WAV files on disk are the source of truth for
        // orphan recovery on next launch. The mmap + cache sizes are sized
        // for a multi-year transcript history staying paged-in.
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            PRAGMA mmap_size=268435456;   -- 256 MB
            PRAGMA cache_size=-65536;     -- 64 MB page cache (negative = KB)
            PRAGMA wal_autocheckpoint=1000;
            PRAGMA foreign_keys=ON;
            ",
        )
        .map_err(|e| format!("Pragma init failed: {e}"))?;

        conn.execute_batch(
            "
            -- Legacy table (keep for migration)
            CREATE TABLE IF NOT EXISTS transcriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                start_time TEXT NOT NULL,
                end_time TEXT NOT NULL,
                device TEXT NOT NULL DEFAULT '',
                confidence REAL NOT NULL DEFAULT 0.0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS hour_slots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hour_key TEXT NOT NULL UNIQUE,
                text TEXT NOT NULL DEFAULT '',
                start_time INTEGER NOT NULL,
                last_updated INTEGER NOT NULL,
                device TEXT NOT NULL DEFAULT '',
                segment_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS hour_slots_fts
                USING fts5(text, content='hour_slots', content_rowid='id');

            CREATE TRIGGER IF NOT EXISTS hour_slots_ai AFTER INSERT ON hour_slots BEGIN
                INSERT INTO hour_slots_fts(rowid, text) VALUES (new.id, new.text);
            END;

            CREATE TRIGGER IF NOT EXISTS hour_slots_au AFTER UPDATE ON hour_slots BEGIN
                INSERT INTO hour_slots_fts(hour_slots_fts, rowid, text)
                    VALUES('delete', old.id, old.text);
                INSERT INTO hour_slots_fts(rowid, text) VALUES (new.id, new.text);
            END;

            CREATE TRIGGER IF NOT EXISTS hour_slots_ad AFTER DELETE ON hour_slots BEGIN
                INSERT INTO hour_slots_fts(hour_slots_fts, rowid, text)
                    VALUES('delete', old.id, old.text);
            END;

            CREATE INDEX IF NOT EXISTS idx_hour_slots_key ON hour_slots(hour_key);
            CREATE INDEX IF NOT EXISTS idx_hour_slots_start ON hour_slots(start_time);
            ",
        )
        .map_err(|e| format!("Schema init failed: {e}"))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Append a transcribed segment to its hour slot. Atomically inserts a new
    /// row or appends text to the existing one. The original `device` column is
    /// preserved across appends — only INSERT sets it.
    ///
    /// `capture_time` is the moment the segment was captured (the WAV stem
    /// parses into this value). Its epoch-millis value is stored in
    /// `start_time` on the initial INSERT and `last_updated` on every append.
    /// `is_segment_processed` compares `last_updated` numerically.
    pub fn append_to_hour_slot(
        &self,
        text: &str,
        capture_time: &DateTime<Utc>,
        device: &str,
    ) -> Result<(), String> {
        let hour_key = Self::hour_key_of(capture_time);
        let capture_ms = capture_time.timestamp_millis();

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO hour_slots (hour_key, text, start_time, last_updated, device, segment_count)
             VALUES (?1, ?2, ?3, ?3, ?4, 1)
             ON CONFLICT(hour_key) DO UPDATE SET
                 text = text || ' ' || excluded.text,
                 last_updated = excluded.last_updated,
                 segment_count = segment_count + 1",
            params![hour_key, text, capture_ms, device],
        )
        .map_err(|e| format!("Upsert failed: {e}"))?;
        Ok(())
    }

    /// Compute the hour_key used in `append_to_hour_slot` for a given capture
    /// time. Bucketed in UTC so that a timezone change between runs (user
    /// travel, CI runner tz, `TZ` drift between login shell and .app bundle
    /// env) cannot produce a different hour_key for the same `DateTime<Utc>`
    /// and silently miss the dedup check. Display layer converts to local.
    pub fn hour_key_of(capture_time: &DateTime<Utc>) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}",
            capture_time.year(),
            capture_time.month(),
            capture_time.day(),
            capture_time.hour()
        )
    }

    pub fn get_hour_slots(&self, limit: i64, offset: i64) -> Result<Vec<HourSlot>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(GET_HOUR_SLOTS_SQL)
            .map_err(|e| format!("Query failed: {e}"))?;

        stmt.query_map(params![limit, offset], map_hour_slot)
            .map_err(|e| format!("Map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Row decode failed: {e}"))
    }

    pub fn search_hour_slots(&self, query: &str) -> Result<Vec<HourSlot>, String> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(SEARCH_HOUR_SLOTS_SQL)
            .map_err(|e| format!("Search failed: {e}"))?;

        stmt.query_map(params![sanitized], map_hour_slot)
            .map_err(|e| format!("Search map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Row decode failed: {e}"))
    }

    pub fn get_slots_by_date_range(
        &self,
        from_key: &str,
        to_key: &str,
    ) -> Result<Vec<HourSlot>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(GET_SLOTS_BY_DATE_RANGE_SQL)
            .map_err(|e| format!("Date range query failed: {e}"))?;

        stmt.query_map(params![from_key, to_key], map_hour_slot)
            .map_err(|e| format!("Map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Row decode failed: {e}"))
    }

    pub fn get_available_dates(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(GET_AVAILABLE_DATES_SQL)
            .map_err(|e| format!("Dates query failed: {e}"))?;

        stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Row decode failed: {e}"))
    }

    /// True if a segment captured at `capture_time` has already been appended
    /// to its hour slot. Matches on `hour_key` + `last_updated >= capture_time`;
    /// since every append updates `last_updated` to the new capture time, a
    /// hour slot whose `last_updated` is at or past T means T's segment (and
    /// anything earlier in the same hour) is already stored.
    ///
    /// Used on startup to skip orphan WAVs from a crashed prior run. The
    /// previous `start_time`-based check only matched the FIRST segment of
    /// each hour and silently re-transcribed the rest; this one works for
    /// every orphan in an existing hour.
    pub fn is_segment_processed(&self, capture_time: &DateTime<Utc>) -> bool {
        let hour_key = Self::hour_key_of(capture_time);
        let capture_ms = capture_time.timestamp_millis();
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare_cached(IS_PROCESSED_SQL) {
            Ok(s) => s,
            Err(e) => {
                log::error!("is_segment_processed prepare_cached failed: {e}");
                return false;
            }
        };
        match stmt.query_row(params![hour_key, capture_ms], |row| row.get::<_, i64>(0)) {
            Ok(count) => count > 0,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            Err(e) => {
                log::error!("is_segment_processed query failed: {e}");
                false
            }
        }
    }

    pub fn count(&self) -> Result<i64, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(COUNT_HOUR_SLOTS_SQL)
            .map_err(|e| format!("Count prepare failed: {e}"))?;
        stmt.query_row([], |row| row.get(0))
            .map_err(|e| format!("Count failed: {e}"))
    }
}

fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|word| {
            let clean: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '-')
                .collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{clean}\"")
            }
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn test_storage() -> (Storage, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = Storage::new(&db_path).unwrap();
        (storage, dir)
    }

    #[test]
    fn test_append_creates_new_slot() {
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();
        storage.append_to_hour_slot("hello world", &ts, "Mic").unwrap();

        let slots = storage.get_hour_slots(10, 0).unwrap();
        assert_eq!(slots.len(), 1);
        // hour_key is UTC-bucketed "YYYY-MM-DDTHH" — stable regardless of
        // the test runner's timezone.
        assert_eq!(slots[0].hour_key, "2024-01-15T14");
        assert_eq!(slots[0].text, "hello world");
        assert_eq!(slots[0].segment_count, 1);
        assert_eq!(slots[0].device, "Mic");
        assert_eq!(slots[0].start_time, ts.timestamp_millis());
        assert_eq!(slots[0].last_updated, ts.timestamp_millis());
    }

    #[test]
    fn test_append_to_existing_slot_preserves_original_device() {
        let (storage, _dir) = test_storage();
        let ts1 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 7, 0).unwrap();
        let ts3 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 9, 0).unwrap();

        storage.append_to_hour_slot("first segment", &ts1, "BuiltIn").unwrap();
        storage.append_to_hour_slot("second segment", &ts2, "USB Mic").unwrap();
        storage.append_to_hour_slot("third segment", &ts3, "AirPods").unwrap();

        let slots = storage.get_hour_slots(10, 0).unwrap();
        assert_eq!(slots.len(), 1);
        assert!(slots[0].text.contains("first segment"));
        assert!(slots[0].text.contains("second segment"));
        assert!(slots[0].text.contains("third segment"));
        assert_eq!(slots[0].segment_count, 3);
        // Original device must be preserved across appends
        assert_eq!(slots[0].device, "BuiltIn");
    }

    #[test]
    fn test_different_hours_create_separate_slots() {
        let (storage, _dir) = test_storage();
        let ts1 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 30, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 1, 15, 15, 5, 0).unwrap();

        storage.append_to_hour_slot("hour 14", &ts1, "Mic").unwrap();
        storage.append_to_hour_slot("hour 15", &ts2, "Mic").unwrap();

        let slots = storage.get_hour_slots(10, 0).unwrap();
        assert_eq!(slots.len(), 2);
    }

    #[test]
    fn test_search_hour_slots() {
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 0, 0).unwrap();
        storage.append_to_hour_slot("discussed budget projections", &ts, "Mic").unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 1, 15, 16, 0, 0).unwrap();
        storage.append_to_hour_slot("lunch break conversation", &ts2, "Mic").unwrap();

        let results = storage.search_hour_slots("budget").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].text.contains("budget"));
    }

    #[test]
    fn test_slot_ordering_desc() {
        let (storage, _dir) = test_storage();
        let early = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        let late = Utc.with_ymd_and_hms(2024, 1, 15, 16, 0, 0).unwrap();

        storage.append_to_hour_slot("morning", &early, "Mic").unwrap();
        storage.append_to_hour_slot("afternoon", &late, "Mic").unwrap();

        let slots = storage.get_hour_slots(10, 0).unwrap();
        assert!(slots[0].text.contains("afternoon"));
        assert!(slots[1].text.contains("morning"));
    }

    #[test]
    fn test_count() {
        let (storage, _dir) = test_storage();
        assert_eq!(storage.count().unwrap(), 0);
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 0, 0).unwrap();
        storage.append_to_hour_slot("test", &ts, "Mic").unwrap();
        assert_eq!(storage.count().unwrap(), 1);
    }

    #[test]
    fn test_is_segment_processed_first_orphan() {
        // #given an empty store
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();

        // #when there is no row yet
        // #then no segment at that time is processed
        assert!(!storage.is_segment_processed(&ts));

        // #when we append that segment
        storage.append_to_hour_slot("hello", &ts, "Mic").unwrap();

        // #then the same capture time is now marked processed
        assert!(storage.is_segment_processed(&ts));
    }

    #[test]
    fn test_is_segment_processed_later_orphan_in_existing_hour() {
        // #given an hour slot that has already absorbed two segments
        let (storage, _dir) = test_storage();
        let ts_a = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();
        let ts_b = Utc.with_ymd_and_hms(2024, 1, 15, 14, 7, 0).unwrap();
        storage.append_to_hour_slot("seg a", &ts_a, "Mic").unwrap();
        storage.append_to_hour_slot("seg b", &ts_b, "Mic").unwrap();

        // #when we dedup-check segment B by its capture time
        // #then it is recognised as processed even though it was not the
        //       first segment of the hour (prior impl missed this case)
        assert!(storage.is_segment_processed(&ts_b));

        // #and a later segment in the same hour that is NOT yet stored is
        //     not flagged as processed
        let ts_c = Utc.with_ymd_and_hms(2024, 1, 15, 14, 9, 0).unwrap();
        assert!(!storage.is_segment_processed(&ts_c));
    }

    #[test]
    fn test_hour_key_of_is_utc_not_local() {
        // #given the UTC instant "2024-01-15T23:30:00Z" (late UTC evening,
        //        early-next-day in positive local zones, still-yesterday in
        //        negative ones)
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 23, 30, 0).unwrap();

        // #when we bucket it
        let key = Storage::hour_key_of(&ts);

        // #then the UTC hour is what we get — independent of the runner's TZ
        assert_eq!(key, "2024-01-15T23");
    }

    #[test]
    fn test_hour_key_groups_same_utc_hour_across_date_boundary() {
        // #given two UTC instants inside the same UTC hour but straddling
        //        the local-date boundary for negative-offset zones (e.g.
        //        US/Pacific — this UTC 01:xx on the 16th is "evening of
        //        the 15th" locally)
        let ts_a = Utc.with_ymd_and_hms(2024, 1, 16, 1, 10, 0).unwrap();
        let ts_b = Utc.with_ymd_and_hms(2024, 1, 16, 1, 45, 0).unwrap();

        // #when we bucket both
        // #then they share a hour_key, so dedup keys them into the same
        //       slot regardless of how any local zone interprets them
        assert_eq!(Storage::hour_key_of(&ts_a), Storage::hour_key_of(&ts_b));

        // #and appending the LATER one marks the EARLIER one processed
        //     (last_updated moves forward; the dedup `>=` covers everything
        //     captured at or before it)
        let (storage, _dir) = test_storage();
        storage.append_to_hour_slot("b", &ts_b, "Mic").unwrap();
        assert!(storage.is_segment_processed(&ts_a));
        assert!(storage.is_segment_processed(&ts_b));
    }

    #[test]
    fn test_is_segment_processed_different_hour_is_independent() {
        // #given an hour slot for hour 14
        let (storage, _dir) = test_storage();
        let ts_14 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();
        storage.append_to_hour_slot("afternoon", &ts_14, "Mic").unwrap();

        // #when we dedup a segment in hour 15
        let ts_15 = Utc.with_ymd_and_hms(2024, 1, 15, 15, 5, 0).unwrap();

        // #then hour 14's append does not bleed into hour 15's dedup
        assert!(!storage.is_segment_processed(&ts_15));
    }

    #[test]
    fn test_fts_sanitize() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(sanitize_fts_query("test\"broken"), "\"testbroken\"");
        assert_eq!(sanitize_fts_query("(bad)"), "\"bad\"");
        assert_eq!(sanitize_fts_query(""), "");
    }
}
