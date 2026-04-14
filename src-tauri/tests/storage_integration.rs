use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use std::path::Path;
use tempfile::TempDir;

fn create_test_db() -> (Connection, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("integration_test.db");
    let conn = Connection::open(&db_path).unwrap();

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

        CREATE VIRTUAL TABLE IF NOT EXISTS transcriptions_fts
            USING fts5(text, content='transcriptions', content_rowid='id');

        CREATE TRIGGER IF NOT EXISTS transcriptions_ai AFTER INSERT ON transcriptions BEGIN
            INSERT INTO transcriptions_fts(rowid, text) VALUES (new.id, new.text);
        END;

        CREATE INDEX IF NOT EXISTS idx_transcriptions_start_time
            ON transcriptions(start_time);
        ",
    )
    .unwrap();

    (conn, dir)
}

#[test]
fn test_fts5_search_relevance() {
    // #given
    let (conn, _dir) = create_test_db();
    let entries = [
        "discussed quarterly budget projections for next year",
        "the weather is nice today",
        "budget meeting with the finance team about projections",
        "went to lunch at the new restaurant downtown",
        "quarterly review of the annual budget",
    ];

    let now = Utc::now().to_rfc3339();
    for text in &entries {
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
            rusqlite::params![text, now],
        )
        .unwrap();
    }

    // #when
    let mut stmt = conn
        .prepare(
            "SELECT t.text FROM transcriptions t
             JOIN transcriptions_fts fts ON t.id = fts.rowid
             WHERE transcriptions_fts MATCH ?1
             ORDER BY rank",
        )
        .unwrap();

    let results: Vec<String> = stmt
        .query_map(["budget"], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // #then
    assert_eq!(results.len(), 3);
    for text in &results {
        assert!(text.to_lowercase().contains("budget"));
    }
}

#[test]
fn test_concurrent_inserts() {
    // #given
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("concurrent.db");

    let conn1 = Connection::open(&db_path).unwrap();
    conn1.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS transcriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT NOT NULL,
            device TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 0.0
        );
        PRAGMA journal_mode=WAL;
        ",
    )
    .unwrap();

    let now = Utc::now().to_rfc3339();

    // #when — simulate rapid sequential inserts
    for i in 0..100 {
        conn1
            .execute(
                "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
                rusqlite::params![format!("entry {i}"), now],
            )
            .unwrap();
    }

    // #then
    let count: i64 = conn1
        .query_row("SELECT COUNT(*) FROM transcriptions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 100);
}

#[test]
fn test_time_range_query() {
    // #given
    let (conn, _dir) = create_test_db();

    let times = [
        ("2024-01-15T10:00:00+00:00", "morning meeting"),
        ("2024-01-15T14:00:00+00:00", "afternoon standup"),
        ("2024-01-15T18:00:00+00:00", "evening wrap up"),
        ("2024-01-16T09:00:00+00:00", "next day planning"),
    ];

    for (time, text) in &times {
        conn.execute(
            "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
            rusqlite::params![text, time],
        )
        .unwrap();
    }

    // #when — query afternoon only
    let mut stmt = conn
        .prepare(
            "SELECT text FROM transcriptions WHERE start_time >= ?1 AND start_time < ?2 ORDER BY start_time",
        )
        .unwrap();

    let results: Vec<String> = stmt
        .query_map(
            rusqlite::params!["2024-01-15T12:00:00+00:00", "2024-01-15T20:00:00+00:00"],
            |row| row.get(0),
        )
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // #then
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], "afternoon standup");
    assert_eq!(results[1], "evening wrap up");
}

#[test]
fn test_empty_search_returns_nothing() {
    // #given
    let (conn, _dir) = create_test_db();

    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES ('hello world', ?1, ?1, 'Mic', 0.9)",
        rusqlite::params![now],
    )
    .unwrap();

    // #when
    let mut stmt = conn
        .prepare(
            "SELECT t.text FROM transcriptions t
             JOIN transcriptions_fts fts ON t.id = fts.rowid
             WHERE transcriptions_fts MATCH ?1",
        )
        .unwrap();

    let results: Vec<String> = stmt
        .query_map(["zzzznonexistent"], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    // #then
    assert_eq!(results.len(), 0);
}

#[test]
fn test_unicode_text_storage() {
    // #given
    let (conn, _dir) = create_test_db();
    let now = Utc::now().to_rfc3339();

    // #when
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
        rusqlite::params!["こんにちは世界 — testing unicode & special chars 'quotes' \"double\"", now],
    )
    .unwrap();

    // #then
    let text: String = conn
        .query_row(
            "SELECT text FROM transcriptions WHERE id = last_insert_rowid()",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(text.contains("こんにちは"));
    assert!(text.contains("unicode"));
}

#[test]
fn test_large_text_storage() {
    // #given
    let (conn, _dir) = create_test_db();
    let now = Utc::now().to_rfc3339();
    let large_text = "word ".repeat(10_000); // ~50KB of text

    // #when
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, 'Mic', 0.9)",
        rusqlite::params![large_text, now],
    )
    .unwrap();

    // #then
    let stored: String = conn
        .query_row(
            "SELECT text FROM transcriptions WHERE id = last_insert_rowid()",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored.len(), large_text.len());
}
