use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Storage {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HourSlot {
    pub id: i64,
    pub hour_key: String, // "2026-04-14T12" format
    pub text: String,
    pub start_time: String,
    pub last_updated: String,
    pub device: String,
    pub segment_count: i64,
}

impl Storage {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;").map_err(|e| format!("WAL: {e}"))?;

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

            -- New hour-slot table
            CREATE TABLE IF NOT EXISTS hour_slots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hour_key TEXT NOT NULL UNIQUE,
                text TEXT NOT NULL DEFAULT '',
                start_time TEXT NOT NULL,
                last_updated TEXT NOT NULL,
                device TEXT NOT NULL DEFAULT '',
                segment_count INTEGER NOT NULL DEFAULT 0
            );

            -- FTS5 on hour_slots
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

    pub fn append_to_hour_slot(
        &self,
        text: &str,
        timestamp: &DateTime<Utc>,
        device: &str,
    ) -> Result<i64, String> {
        // Use LOCAL time for the hour key so UI displays correctly
        let local = timestamp.with_timezone(&Local);
        let hour_key = format!(
            "{:04}-{:02}-{:02}T{:02}",
            local.year(),
            local.month(),
            local.day(),
            local.hour()
        );
        let now_str = timestamp.to_rfc3339();

        let conn = self.conn.lock();

        // Check if hour slot exists
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM hour_slots WHERE hour_key = ?1",
                params![hour_key],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if exists {
            // Append text to existing slot
            conn.execute(
                "UPDATE hour_slots SET text = text || ' ' || ?1, last_updated = ?2, device = ?3, segment_count = segment_count + 1 WHERE hour_key = ?4",
                params![text, now_str, device, hour_key],
            )
            .map_err(|e| format!("Append failed: {e}"))?;
        } else {
            // Create new hour slot
            conn.execute(
                "INSERT INTO hour_slots (hour_key, text, start_time, last_updated, device, segment_count) VALUES (?1, ?2, ?3, ?3, ?4, 1)",
                params![hour_key, text, now_str, device],
            )
            .map_err(|e| format!("Insert slot failed: {e}"))?;
        }

        Ok(conn.last_insert_rowid())
    }

    pub fn get_hour_slots(&self, limit: i64, offset: i64) -> Result<Vec<HourSlot>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, hour_key, text, start_time, last_updated, device, segment_count
                 FROM hour_slots
                 ORDER BY start_time DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| format!("Query failed: {e}"))?;

        let rows = stmt
            .query_map(params![limit, offset], |row| {
                Ok(HourSlot {
                    id: row.get(0)?,
                    hour_key: row.get(1)?,
                    text: row.get(2)?,
                    start_time: row.get(3)?,
                    last_updated: row.get(4)?,
                    device: row.get(5)?,
                    segment_count: row.get(6)?,
                })
            })
            .map_err(|e| format!("Map failed: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    pub fn search_hour_slots(&self, query: &str) -> Result<Vec<HourSlot>, String> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT h.id, h.hour_key, h.text, h.start_time, h.last_updated, h.device, h.segment_count
                 FROM hour_slots h
                 JOIN hour_slots_fts f ON h.id = f.rowid
                 WHERE hour_slots_fts MATCH ?1
                 ORDER BY rank
                 LIMIT 50",
            )
            .map_err(|e| format!("Search failed: {e}"))?;

        let rows = stmt
            .query_map(params![sanitized], |row| {
                Ok(HourSlot {
                    id: row.get(0)?,
                    hour_key: row.get(1)?,
                    text: row.get(2)?,
                    start_time: row.get(3)?,
                    last_updated: row.get(4)?,
                    device: row.get(5)?,
                    segment_count: row.get(6)?,
                })
            })
            .map_err(|e| format!("Search map failed: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    pub fn get_slots_by_date_range(
        &self,
        from_key: &str,
        to_key: &str,
    ) -> Result<Vec<HourSlot>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, hour_key, text, start_time, last_updated, device, segment_count
                 FROM hour_slots
                 WHERE hour_key >= ?1 AND hour_key <= ?2
                 ORDER BY hour_key ASC",
            )
            .map_err(|e| format!("Date range query failed: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params![from_key, to_key], |row| {
                Ok(HourSlot {
                    id: row.get(0)?,
                    hour_key: row.get(1)?,
                    text: row.get(2)?,
                    start_time: row.get(3)?,
                    last_updated: row.get(4)?,
                    device: row.get(5)?,
                    segment_count: row.get(6)?,
                })
            })
            .map_err(|e| format!("Map failed: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    pub fn get_available_dates(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT DISTINCT substr(hour_key, 1, 10) as date FROM hour_slots ORDER BY date DESC")
            .map_err(|e| format!("Dates query failed: {e}"))?;

        let dates = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("Map failed: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(dates)
    }

    pub fn has_transcription_near(&self, start_time: &str) -> bool {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT COUNT(*) FROM transcriptions WHERE start_time = ?1",
            params![start_time],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
            > 0
    }

    pub fn count(&self) -> Result<i64, String> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM hour_slots", [], |row| row.get(0))
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
        assert_eq!(slots[0].hour_key, "2024-01-15T14");
        assert_eq!(slots[0].text, "hello world");
        assert_eq!(slots[0].segment_count, 1);
    }

    #[test]
    fn test_append_to_existing_slot() {
        let (storage, _dir) = test_storage();
        let ts1 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 5, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 7, 0).unwrap();
        let ts3 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 9, 0).unwrap();

        storage.append_to_hour_slot("first segment", &ts1, "Mic").unwrap();
        storage.append_to_hour_slot("second segment", &ts2, "Mic").unwrap();
        storage.append_to_hour_slot("third segment", &ts3, "Mic").unwrap();

        let slots = storage.get_hour_slots(10, 0).unwrap();
        assert_eq!(slots.len(), 1);
        assert!(slots[0].text.contains("first segment"));
        assert!(slots[0].text.contains("second segment"));
        assert!(slots[0].text.contains("third segment"));
        assert_eq!(slots[0].segment_count, 3);
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
        storage.append_to_hour_slot("lunch break conversation", &ts, "Mic").unwrap();

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
    fn test_fts_sanitize() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(sanitize_fts_query("test\"broken"), "\"testbroken\"");
        assert_eq!(sanitize_fts_query("(bad)"), "\"bad\"");
        assert_eq!(sanitize_fts_query(""), "");
    }
}
