use chrono::{DateTime, Datelike, Timelike, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Storage {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Segment {
    pub id: i64,
    pub hour_key: String,
    pub segment_type: String,
    pub text: String,
    pub timestamp: i64,
    pub device: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UnifiedHourSlot {
    pub hour_key: String,
    pub segments: Vec<Segment>,
    pub earliest_timestamp: i64,
    pub latest_timestamp: i64,
    pub total_segment_count: i64,
}

// Keep HourSlot for backward compat during orphan dedup (pipeline.rs reads it)
#[derive(Debug, Clone, serde::Serialize)]
pub struct HourSlot {
    pub id: i64,
    pub hour_key: String,
    pub text: String,
    pub start_time: i64,
    pub last_updated: i64,
    pub device: String,
    pub segment_count: i64,
}

impl Storage {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA temp_store=MEMORY;
            PRAGMA mmap_size=268435456;
            PRAGMA cache_size=-65536;
            PRAGMA wal_autocheckpoint=1000;
            PRAGMA foreign_keys=ON;
            ",
        )
        .map_err(|e| format!("Pragma init failed: {e}"))?;

        // Legacy tables — kept for migration source only
        conn.execute_batch(
            "
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
                INSERT INTO hour_slots_fts(hour_slots_fts, rowid, text) VALUES('delete', old.id, old.text);
                INSERT INTO hour_slots_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS hour_slots_ad AFTER DELETE ON hour_slots BEGIN
                INSERT INTO hour_slots_fts(hour_slots_fts, rowid, text) VALUES('delete', old.id, old.text);
            END;

            CREATE INDEX IF NOT EXISTS idx_hour_slots_key ON hour_slots(hour_key);
            CREATE INDEX IF NOT EXISTS idx_hour_slots_start ON hour_slots(start_time);

            CREATE TABLE IF NOT EXISTS screen_slots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hour_key TEXT NOT NULL UNIQUE,
                text TEXT NOT NULL DEFAULT '',
                start_time INTEGER NOT NULL,
                last_updated INTEGER NOT NULL,
                device TEXT NOT NULL DEFAULT '',
                segment_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS screen_slots_fts
                USING fts5(text, content='screen_slots', content_rowid='id');

            CREATE TRIGGER IF NOT EXISTS screen_slots_ai AFTER INSERT ON screen_slots BEGIN
                INSERT INTO screen_slots_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS screen_slots_au AFTER UPDATE ON screen_slots BEGIN
                INSERT INTO screen_slots_fts(screen_slots_fts, rowid, text) VALUES('delete', old.id, old.text);
                INSERT INTO screen_slots_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS screen_slots_ad AFTER DELETE ON screen_slots BEGIN
                INSERT INTO screen_slots_fts(screen_slots_fts, rowid, text) VALUES('delete', old.id, old.text);
            END;

            CREATE INDEX IF NOT EXISTS idx_screen_slots_key ON screen_slots(hour_key);
            CREATE INDEX IF NOT EXISTS idx_screen_slots_start ON screen_slots(start_time);

            -- Unified segments table (single source of truth going forward)
            CREATE TABLE IF NOT EXISTS segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hour_key TEXT NOT NULL,
                segment_type TEXT NOT NULL,
                text TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                device TEXT NOT NULL DEFAULT ''
            );

            CREATE INDEX IF NOT EXISTS idx_segments_hour_key ON segments(hour_key);
            CREATE INDEX IF NOT EXISTS idx_segments_timestamp ON segments(timestamp);
            CREATE INDEX IF NOT EXISTS idx_segments_type ON segments(segment_type);
            -- Compound index for ordered within-hour reads (load_hour_slots_segments).
            CREATE INDEX IF NOT EXISTS idx_segments_hour_ts
                ON segments(hour_key, timestamp);

            -- Trigram tokenizer enables substring, suffix, and typo-tolerant search.
            -- Existing databases (created before this tokenizer switch) are migrated
            -- by `migrate_fts_to_trigram` below.
            CREATE VIRTUAL TABLE IF NOT EXISTS segments_fts
                USING fts5(text, content='segments', content_rowid='id', tokenize='trigram');

            CREATE TRIGGER IF NOT EXISTS segments_ai AFTER INSERT ON segments BEGIN
                INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS segments_au AFTER UPDATE ON segments BEGIN
                INSERT INTO segments_fts(segments_fts, rowid, text) VALUES('delete', old.id, old.text);
                INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS segments_ad AFTER DELETE ON segments BEGIN
                INSERT INTO segments_fts(segments_fts, rowid, text) VALUES('delete', old.id, old.text);
            END;
            ",
        )
        .map_err(|e| format!("Schema init failed: {e}"))?;

        migrate_text_timestamps_to_epoch_ms(&conn)?;
        migrate_slots_to_segments(&conn)?;
        migrate_fts_to_trigram(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn hour_key_of(capture_time: &DateTime<Utc>) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}",
            capture_time.year(),
            capture_time.month(),
            capture_time.day(),
            capture_time.hour()
        )
    }

    // ── Write methods (segments table only) ──

    pub fn insert_segment(
        &self,
        text: &str,
        capture_time: &DateTime<Utc>,
        segment_type: &str,
        device: &str,
    ) -> Result<(), String> {
        let hour_key = Self::hour_key_of(capture_time);
        let capture_ms = capture_time.timestamp_millis();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO segments (hour_key, segment_type, text, timestamp, device)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![hour_key, segment_type, text, capture_ms, device],
        )
        .map_err(|e| format!("Segment insert failed: {e}"))?;
        Ok(())
    }

    pub fn insert_transcription(
        &self,
        text: &str,
        capture_time: &DateTime<Utc>,
        device: &str,
    ) -> Result<(), String> {
        self.insert_segment(text, capture_time, "transcription", device)
    }

    pub fn insert_screen_context(
        &self,
        text: &str,
        capture_time: &DateTime<Utc>,
    ) -> Result<(), String> {
        self.insert_segment(text, capture_time, "screen", "Screen")
    }

    // ── Unified timeline queries ──

    pub fn get_unified_timeline(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UnifiedHourSlot>, String> {
        let conn = self.conn.lock();
        let mut hour_stmt = conn
            .prepare_cached(
                "SELECT hour_key, MIN(timestamp), MAX(timestamp), COUNT(*)
                 FROM segments
                 GROUP BY hour_key
                 ORDER BY MAX(timestamp) DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| format!("Hour query failed: {e}"))?;

        let hours: Vec<(String, i64, i64, i64)> = hour_stmt
            .query_map(params![limit, offset], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| format!("Hour map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Hour decode failed: {e}"))?;

        self.load_hour_slots_segments(&conn, hours)
    }

    pub fn search_segments(&self, query: &str) -> Result<Vec<UnifiedHourSlot>, String> {
        let sanitized = fts_substring_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock();

        // Single query: find matching hours + their aggregates in one shot. The
        // MAX(timestamp) alias is reused for both HAVING filtering (only keep
        // hours that actually had a matching segment) and ORDER BY.
        let mut stmt = conn
            .prepare_cached(
                "SELECT s.hour_key,
                        MIN(s.timestamp) AS earliest,
                        MAX(s.timestamp) AS latest,
                        COUNT(*) AS total,
                        MAX(CASE WHEN f.rowid IS NOT NULL THEN s.timestamp END) AS last_match
                 FROM segments s
                 LEFT JOIN segments_fts f
                        ON s.id = f.rowid
                       AND segments_fts MATCH ?1
                 GROUP BY s.hour_key
                 HAVING last_match IS NOT NULL
                 ORDER BY last_match DESC
                 LIMIT 50",
            )
            .map_err(|e| format!("Search failed: {e}"))?;

        let hours: Vec<(String, i64, i64, i64)> = stmt
            .query_map(params![sanitized], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| format!("Search map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Search decode failed: {e}"))?;

        self.load_hour_slots_segments(&conn, hours)
    }

    pub fn get_segments_by_date_range(
        &self,
        from_key: &str,
        to_key: &str,
    ) -> Result<Vec<UnifiedHourSlot>, String> {
        let conn = self.conn.lock();
        let mut hour_stmt = conn
            .prepare_cached(
                "SELECT hour_key, MIN(timestamp), MAX(timestamp), COUNT(*)
                 FROM segments
                 WHERE hour_key >= ?1 AND hour_key <= ?2
                 GROUP BY hour_key
                 ORDER BY hour_key ASC",
            )
            .map_err(|e| format!("Date range query failed: {e}"))?;

        let hours: Vec<(String, i64, i64, i64)> = hour_stmt
            .query_map(params![from_key, to_key], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| format!("Date range map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Date range decode failed: {e}"))?;

        self.load_hour_slots_segments(&conn, hours)
    }

    pub fn get_available_dates(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached(
                "SELECT DISTINCT substr(hour_key, 1, 10) as date
                 FROM segments ORDER BY date DESC",
            )
            .map_err(|e| format!("Dates query failed: {e}"))?;

        stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Row decode failed: {e}"))
    }

    pub fn segment_count(&self) -> Result<i64, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare_cached("SELECT COUNT(*) FROM segments")
            .map_err(|e| format!("Count prepare failed: {e}"))?;
        stmt.query_row([], |row| row.get(0))
            .map_err(|e| format!("Count failed: {e}"))
    }

    pub fn is_segment_processed(&self, capture_time: &DateTime<Utc>) -> bool {
        let hour_key = Self::hour_key_of(capture_time);
        let capture_ms = capture_time.timestamp_millis();
        let conn = self.conn.lock();
        let mut stmt = match conn.prepare_cached(
            "SELECT COUNT(*) FROM segments
             WHERE hour_key = ?1 AND timestamp = ?2 AND segment_type = 'transcription'",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::error!("is_segment_processed failed: {e}");
                return false;
            }
        };
        match stmt.query_row(params![hour_key, capture_ms], |row| row.get::<_, i64>(0)) {
            Ok(count) => count > 0,
            _ => false,
        }
    }

    // ── Internal helpers ──

    fn load_hour_slots_segments(
        &self,
        conn: &Connection,
        hours: Vec<(String, i64, i64, i64)>,
    ) -> Result<Vec<UnifiedHourSlot>, String> {
        if hours.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch every matching hour's segments in one round-trip. Collapses the
        // prior N+1 (one query per hour) into a single IN-clause read, which
        // matters as the timeline grows.
        let placeholders = vec!["?"; hours.len()].join(",");
        let sql = format!(
            "SELECT id, hour_key, segment_type, text, timestamp, device
             FROM segments
             WHERE hour_key IN ({placeholders})
             ORDER BY hour_key ASC, timestamp ASC",
        );

        let hour_keys: Vec<&str> = hours.iter().map(|(hk, _, _, _)| hk.as_str()).collect();
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("Segment query failed: {e}"))?;

        let all_segments: Vec<Segment> = stmt
            .query_map(rusqlite::params_from_iter(hour_keys.iter()), |row| {
                Ok(Segment {
                    id: row.get(0)?,
                    hour_key: row.get(1)?,
                    segment_type: row.get(2)?,
                    text: row.get(3)?,
                    timestamp: row.get(4)?,
                    device: row.get(5)?,
                })
            })
            .map_err(|e| format!("Segment map failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Segment decode failed: {e}"))?;

        // Bucket segments by hour_key. The caller's `hours` vec defines output
        // order, so we build a temporary map keyed by hour_key and drain it.
        let mut by_hour: std::collections::HashMap<String, Vec<Segment>> =
            std::collections::HashMap::with_capacity(hours.len());
        for seg in all_segments {
            by_hour.entry(seg.hour_key.clone()).or_default().push(seg);
        }

        let mut result = Vec::with_capacity(hours.len());
        for (hour_key, earliest, latest, count) in hours {
            let segments = by_hour.remove(&hour_key).unwrap_or_default();
            result.push(UnifiedHourSlot {
                hour_key,
                segments,
                earliest_timestamp: earliest,
                latest_timestamp: latest,
                total_segment_count: count,
            });
        }
        Ok(result)
    }
}

// ── Migrations ──

fn migrate_text_timestamps_to_epoch_ms(conn: &Connection) -> Result<(), String> {
    let col_type: String = conn
        .query_row(
            "SELECT type FROM pragma_table_info('hour_slots') WHERE name = 'start_time'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if col_type.to_uppercase() == "INTEGER" {
        return Ok(());
    }

    log::info!("Migrating hour_slots timestamps from TEXT to INTEGER...");

    conn.execute_batch("BEGIN TRANSACTION;")
        .map_err(|e| format!("Migration begin failed: {e}"))?;

    let result = (|| -> Result<(), String> {
        conn.execute_batch(
            "ALTER TABLE hour_slots RENAME TO hour_slots_old;
             CREATE TABLE hour_slots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 hour_key TEXT NOT NULL UNIQUE,
                 text TEXT NOT NULL DEFAULT '',
                 start_time INTEGER NOT NULL,
                 last_updated INTEGER NOT NULL,
                 device TEXT NOT NULL DEFAULT '',
                 segment_count INTEGER NOT NULL DEFAULT 0
             );",
        )
        .map_err(|e| format!("Rename/create failed: {e}"))?;

        let mut read_stmt = conn
            .prepare("SELECT id, hour_key, text, start_time, last_updated, device, segment_count FROM hour_slots_old")
            .map_err(|e| format!("Read failed: {e}"))?;

        let rows: Vec<(i64, String, String, String, String, String, i64)> = read_stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?,
                    row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?,
                ))
            })
            .map_err(|e| format!("Query failed: {e}"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("Decode failed: {e}"))?;

        let mut ins = conn
            .prepare("INSERT INTO hour_slots (id, hour_key, text, start_time, last_updated, device, segment_count) VALUES (?1,?2,?3,?4,?5,?6,?7)")
            .map_err(|e| format!("Insert prepare failed: {e}"))?;

        for (id, hk, text, st, up, dev, cnt) in &rows {
            ins.execute(params![id, hk, text, parse_timestamp_value(st), parse_timestamp_value(up), dev, cnt])
                .map_err(|e| format!("Insert failed for id {id}: {e}"))?;
        }

        conn.execute_batch(
            "DROP TABLE hour_slots_old;
             INSERT INTO hour_slots_fts(hour_slots_fts) VALUES('rebuild');",
        )
        .map_err(|e| format!("Drop/rebuild failed: {e}"))?;

        log::info!("Migrated {} hour_slots rows", rows.len());
        Ok(())
    })();

    match result {
        Ok(()) => conn.execute_batch("COMMIT;").map_err(|e| format!("Commit failed: {e}")),
        Err(e) => { conn.execute_batch("ROLLBACK;").ok(); Err(format!("Rolled back: {e}")) }
    }
}

fn migrate_slots_to_segments(conn: &Connection) -> Result<(), String> {
    let seg_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM segments", [], |row| row.get(0))
        .unwrap_or(0);
    let hour_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM hour_slots", [], |row| row.get(0))
        .unwrap_or(0);
    let screen_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM screen_slots", [], |row| row.get(0))
        .unwrap_or(0);

    if seg_count > 0 || (hour_count == 0 && screen_count == 0) {
        return Ok(());
    }

    log::info!("Migrating {hour_count} hour_slots + {screen_count} screen_slots to segments...");

    conn.execute_batch("BEGIN TRANSACTION;")
        .map_err(|e| format!("Migration begin failed: {e}"))?;

    let result = (|| -> Result<(), String> {
        conn.execute(
            "INSERT INTO segments (hour_key, segment_type, text, timestamp, device)
             SELECT hour_key, 'transcription', text, start_time, device FROM hour_slots",
            [],
        )
        .map_err(|e| format!("hour_slots migration failed: {e}"))?;

        conn.execute(
            "INSERT INTO segments (hour_key, segment_type, text, timestamp, device)
             SELECT hour_key, 'screen', text, start_time, device FROM screen_slots",
            [],
        )
        .map_err(|e| format!("screen_slots migration failed: {e}"))?;

        conn.execute_batch("INSERT INTO segments_fts(segments_fts) VALUES('rebuild');")
            .map_err(|e| format!("FTS rebuild failed: {e}"))?;

        log::info!("Migrated to segments table");
        Ok(())
    })();

    match result {
        Ok(()) => conn.execute_batch("COMMIT;").map_err(|e| format!("Commit failed: {e}")),
        Err(e) => { conn.execute_batch("ROLLBACK;").ok(); Err(format!("Rolled back: {e}")) }
    }
}

fn parse_timestamp_value(s: &str) -> i64 {
    if let Ok(n) = s.parse::<i64>() {
        return n;
    }
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
        .unwrap_or(0)
}

/// Build an FTS5 MATCH query suited to the trigram tokenizer.
///
/// The trigram tokenizer turns MATCH into substring search, so each token we
/// pass is looked up as a contiguous substring anywhere in the indexed text.
/// Tokens shorter than 3 characters can't produce a trigram and would silently
/// return nothing, so we drop them. Double quotes inside user input are
/// doubled so they stay literal inside the FTS5 phrase syntax.
fn fts_substring_query(query: &str) -> String {
    const MIN_TRIGRAM_LEN: usize = 3;
    query
        .split_whitespace()
        .filter(|w| w.chars().count() >= MIN_TRIGRAM_LEN)
        .map(|word| {
            let escaped = word.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Migrate an existing `segments_fts` table to the trigram tokenizer.
///
/// Idempotent: if the existing table already uses the trigram tokenizer, or
/// the table doesn't yet exist (fresh install), this is a no-op. Otherwise it
/// drops the old inverted index + triggers, recreates them with the trigram
/// tokenizer, and rebuilds the index from the authoritative `segments` table.
/// The rebuild cost scales linearly with corpus size — measured in seconds
/// for typical datasets (~10s of thousands of rows).
fn migrate_fts_to_trigram(conn: &Connection) -> Result<(), String> {
    let existing_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='segments_fts'",
            [],
            |row| row.get(0),
        )
        .ok();

    let needs_migration = match &existing_sql {
        Some(sql) => !sql.to_lowercase().contains("trigram"),
        None => false, // Table will be created fresh with trigram by the caller.
    };

    if !needs_migration {
        return Ok(());
    }

    log::info!("Migrating segments_fts to trigram tokenizer — this may take a few seconds");

    conn.execute_batch(
        "BEGIN;
         DROP TRIGGER IF EXISTS segments_ai;
         DROP TRIGGER IF EXISTS segments_au;
         DROP TRIGGER IF EXISTS segments_ad;
         DROP TABLE IF EXISTS segments_fts;
         CREATE VIRTUAL TABLE segments_fts
             USING fts5(text, content='segments', content_rowid='id', tokenize='trigram');
         CREATE TRIGGER segments_ai AFTER INSERT ON segments BEGIN
             INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
         END;
         CREATE TRIGGER segments_au AFTER UPDATE ON segments BEGIN
             INSERT INTO segments_fts(segments_fts, rowid, text) VALUES('delete', old.id, old.text);
             INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
         END;
         CREATE TRIGGER segments_ad AFTER DELETE ON segments BEGIN
             INSERT INTO segments_fts(segments_fts, rowid, text) VALUES('delete', old.id, old.text);
         END;
         INSERT INTO segments_fts(segments_fts) VALUES('rebuild');
         COMMIT;",
    )
    .map_err(|e| {
        let _ = conn.execute_batch("ROLLBACK;");
        format!("FTS trigram migration failed: {e}")
    })?;

    log::info!("FTS trigram migration complete");
    Ok(())
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
    fn test_insert_transcription_segment() {
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();
        storage.insert_transcription("hello world", &ts, "Mic").unwrap();

        let slots = storage.get_unified_timeline(10, 0).unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].hour_key, "2024-01-15T14");
        assert_eq!(slots[0].segments.len(), 1);
        assert_eq!(slots[0].segments[0].segment_type, "transcription");
        assert_eq!(slots[0].segments[0].text, "hello world");
        assert_eq!(slots[0].segments[0].device, "Mic");
        assert_eq!(slots[0].segments[0].timestamp, ts.timestamp_millis());
    }

    #[test]
    fn test_insert_screen_context_segment() {
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 15, 0).unwrap();
        storage.insert_screen_context("App: VS Code", &ts).unwrap();

        let slots = storage.get_unified_timeline(10, 0).unwrap();
        assert_eq!(slots[0].segments[0].segment_type, "screen");
        assert_eq!(slots[0].segments[0].device, "Screen");
    }

    #[test]
    fn test_unified_timeline_interleaves_chronologically() {
        // #given transcription and screen segments in the same hour
        let (storage, _dir) = test_storage();
        let t1 = Utc.with_ymd_and_hms(2024, 3, 10, 9, 5, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 3, 10, 9, 10, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2024, 3, 10, 9, 12, 0).unwrap();

        storage.insert_transcription("spoke first", &t1, "Mic").unwrap();
        storage.insert_screen_context("App: VS Code", &t2).unwrap();
        storage.insert_transcription("spoke again", &t3, "Mic").unwrap();

        // #when we get the timeline
        let slots = storage.get_unified_timeline(10, 0).unwrap();

        // #then one hour slot with 3 segments in chronological order
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].segments.len(), 3);
        assert_eq!(slots[0].segments[0].segment_type, "transcription");
        assert_eq!(slots[0].segments[0].text, "spoke first");
        assert_eq!(slots[0].segments[1].segment_type, "screen");
        assert_eq!(slots[0].segments[1].text, "App: VS Code");
        assert_eq!(slots[0].segments[2].segment_type, "transcription");
        assert_eq!(slots[0].segments[2].text, "spoke again");
        assert_eq!(slots[0].total_segment_count, 3);
    }

    #[test]
    fn test_different_hours_create_separate_slots() {
        let (storage, _dir) = test_storage();
        let t1 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 1, 15, 15, 0, 0).unwrap();

        storage.insert_transcription("hour 14", &t1, "Mic").unwrap();
        storage.insert_transcription("hour 15", &t2, "Mic").unwrap();

        let slots = storage.get_unified_timeline(10, 0).unwrap();
        assert_eq!(slots.len(), 2);
    }

    #[test]
    fn test_timeline_ordered_desc_by_latest() {
        let (storage, _dir) = test_storage();
        let early = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        let late = Utc.with_ymd_and_hms(2024, 1, 15, 16, 0, 0).unwrap();

        storage.insert_transcription("morning", &early, "Mic").unwrap();
        storage.insert_transcription("afternoon", &late, "Mic").unwrap();

        let slots = storage.get_unified_timeline(10, 0).unwrap();
        assert!(slots[0].segments[0].text.contains("afternoon"));
        assert!(slots[1].segments[0].text.contains("morning"));
    }

    #[test]
    fn test_search_across_types() {
        let (storage, _dir) = test_storage();
        let ts1 = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 3, 10, 10, 0, 0).unwrap();

        storage.insert_transcription("budget projections discussed", &ts1, "Mic").unwrap();
        storage.insert_screen_context("App: Google Sheets, editing budget spreadsheet", &ts2).unwrap();

        let hits = storage.search_segments("budget").unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn test_search_empty_returns_empty() {
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        storage.insert_transcription("test", &ts, "Mic").unwrap();

        assert!(storage.search_segments("").unwrap().is_empty());
    }

    #[test]
    fn test_search_trigram_substring() {
        // Trigram tokenizer matches any substring, so mid-word hits work.
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        storage.insert_transcription("I use ChatGPT every day", &ts, "Mic").unwrap();

        // Mid-word match: "gpt" → finds "ChatGPT"
        assert_eq!(storage.search_segments("gpt").unwrap().len(), 1);
        // Prefix match: "Cha" → finds "ChatGPT"
        assert_eq!(storage.search_segments("Cha").unwrap().len(), 1);
        // Suffix match: "GPT" → still finds it
        assert_eq!(storage.search_segments("GPT").unwrap().len(), 1);
    }

    #[test]
    fn test_search_rejects_short_queries() {
        // Queries shorter than a trigram can't form valid grams, so they're
        // filtered out before hitting FTS5 to avoid undefined behavior.
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        storage.insert_transcription("something meaningful", &ts, "Mic").unwrap();

        assert!(storage.search_segments("a").unwrap().is_empty());
        assert!(storage.search_segments("so").unwrap().is_empty());
        // 3 chars is the threshold
        assert_eq!(storage.search_segments("som").unwrap().len(), 1);
    }

    #[test]
    fn test_migrate_fts_to_trigram_idempotent() {
        // Running the migration twice must not break the index or duplicate rows.
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        storage.insert_transcription("before migration", &ts, "Mic").unwrap();

        // Second call is a no-op because the table already uses trigram.
        let conn = storage.conn.lock();
        migrate_fts_to_trigram(&conn).unwrap();
        drop(conn);

        assert_eq!(storage.search_segments("migration").unwrap().len(), 1);
    }

    #[test]
    fn test_date_range() {
        let (storage, _dir) = test_storage();
        let d1 = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2024, 3, 11, 10, 0, 0).unwrap();

        storage.insert_transcription("mar 10", &d1, "Mic").unwrap();
        storage.insert_transcription("mar 11", &d2, "Mic").unwrap();

        let slots = storage.get_segments_by_date_range("2024-03-10T00", "2024-03-10T23").unwrap();
        assert_eq!(slots.len(), 1);
        assert!(slots[0].segments[0].text.contains("mar 10"));
    }

    #[test]
    fn test_available_dates() {
        let (storage, _dir) = test_storage();
        let d1 = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2024, 3, 11, 10, 0, 0).unwrap();

        storage.insert_transcription("a", &d1, "Mic").unwrap();
        storage.insert_screen_context("b", &d2).unwrap();

        let dates = storage.get_available_dates().unwrap();
        assert_eq!(dates.len(), 2);
        assert_eq!(dates[0], "2024-03-11");
        assert_eq!(dates[1], "2024-03-10");
    }

    #[test]
    fn test_segment_count() {
        let (storage, _dir) = test_storage();
        assert_eq!(storage.segment_count().unwrap(), 0);

        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 0, 0).unwrap();
        storage.insert_transcription("a", &ts, "Mic").unwrap();
        storage.insert_screen_context("b", &ts).unwrap();
        assert_eq!(storage.segment_count().unwrap(), 2);
    }

    #[test]
    fn test_is_segment_processed() {
        let (storage, _dir) = test_storage();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();

        assert!(!storage.is_segment_processed(&ts));
        storage.insert_transcription("test", &ts, "Mic").unwrap();
        assert!(storage.is_segment_processed(&ts));

        let ts2 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 7, 0).unwrap();
        assert!(!storage.is_segment_processed(&ts2));
    }

    #[test]
    fn test_pagination() {
        let (storage, _dir) = test_storage();
        for h in 0..10u32 {
            let ts = Utc.with_ymd_and_hms(2024, 3, 10, h, 0, 0).unwrap();
            storage.insert_transcription(&format!("hour {h}"), &ts, "Mic").unwrap();
        }

        assert_eq!(storage.get_unified_timeline(3, 0).unwrap().len(), 3);
        assert_eq!(storage.get_unified_timeline(3, 3).unwrap().len(), 3);
        assert_eq!(storage.get_unified_timeline(3, 100).unwrap().len(), 0);
    }

    #[test]
    fn test_hour_key_of_is_utc() {
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 23, 30, 0).unwrap();
        assert_eq!(Storage::hour_key_of(&ts), "2024-01-15T23");
    }

    #[test]
    fn test_fts_substring_query() {
        // Normal multi-token query → each word becomes a quoted phrase, implicitly AND-ed.
        assert_eq!(fts_substring_query("hello world"), "\"hello\" \"world\"");
        // Internal double-quotes are doubled to stay literal inside FTS5 phrase syntax.
        assert_eq!(fts_substring_query("say \"hi\""), "\"say\" \"\"\"hi\"\"\"");
        // Tokens shorter than 3 chars can't form a trigram and are dropped.
        assert_eq!(fts_substring_query("of a team"), "\"team\"");
        // Empty or all-short → empty query (caller short-circuits to no results).
        assert_eq!(fts_substring_query(""), "");
        assert_eq!(fts_substring_query("a b"), "");
    }

    #[test]
    fn test_parse_timestamp_value_rfc3339() {
        let ms = parse_timestamp_value("2024-01-15T14:05:00+00:00");
        let expected = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap().timestamp_millis();
        assert_eq!(ms, expected);
    }

    #[test]
    fn test_parse_timestamp_value_numeric() {
        assert_eq!(parse_timestamp_value("1776201056000"), 1776201056000);
    }

    #[test]
    fn test_migration_from_old_tables() {
        // #given a database with old-schema data
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("migrate.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute_batch(
            "CREATE TABLE hour_slots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 hour_key TEXT NOT NULL UNIQUE,
                 text TEXT NOT NULL DEFAULT '',
                 start_time INTEGER NOT NULL,
                 last_updated INTEGER NOT NULL,
                 device TEXT NOT NULL DEFAULT '',
                 segment_count INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE screen_slots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 hour_key TEXT NOT NULL UNIQUE,
                 text TEXT NOT NULL DEFAULT '',
                 start_time INTEGER NOT NULL,
                 last_updated INTEGER NOT NULL,
                 device TEXT NOT NULL DEFAULT '',
                 segment_count INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO hour_slots VALUES (1, '2024-01-15T14', 'spoken words', 1705327200000, 1705327500000, 'Mic', 2);
             INSERT INTO screen_slots VALUES (1, '2024-01-15T14', 'App: VS Code', 1705327300000, 1705327300000, 'Screen', 1);",
        )
        .unwrap();
        drop(conn);

        // #when we open via Storage::new (which runs migration)
        let storage = Storage::new(&db_path).unwrap();

        // #then both are in the segments table
        let slots = storage.get_unified_timeline(10, 0).unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].segments.len(), 2);

        let types: Vec<&str> = slots[0].segments.iter().map(|s| s.segment_type.as_str()).collect();
        assert!(types.contains(&"transcription"));
        assert!(types.contains(&"screen"));
    }
}
