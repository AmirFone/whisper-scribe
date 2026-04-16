//! Legacy-schema exhaustive SQL tests. Build/run only when
//! `cargo test --features legacy_schema_tests` is passed. Everything here
//! constructs its own `rusqlite::Connection` against the `transcriptions`
//! table that predates the `hour_slots` migration — it does not import
//! `whisper_scribe_lib::storage::Storage`. The new regression guard that
//! tests the real public API lives in `storage_integration.rs`.
#![cfg(feature = "legacy_schema_tests")]

use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection};
use tempfile::TempDir;

fn setup() -> (Connection, TempDir) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE transcriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT NOT NULL,
            device TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 0.0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE VIRTUAL TABLE transcriptions_fts USING fts5(text, content='transcriptions', content_rowid='id');
        CREATE TRIGGER transcriptions_ai AFTER INSERT ON transcriptions BEGIN
            INSERT INTO transcriptions_fts(rowid, text) VALUES (new.id, new.text);
        END;
        CREATE TRIGGER transcriptions_ad AFTER DELETE ON transcriptions BEGIN
            INSERT INTO transcriptions_fts(transcriptions_fts, rowid, text) VALUES('delete', old.id, old.text);
        END;
        CREATE INDEX idx_start ON transcriptions(start_time);
        PRAGMA journal_mode=WAL;
        ",
    )
    .unwrap();
    (conn, dir)
}

fn insert(conn: &Connection, text: &str, start: &str, device: &str, confidence: f32) -> i64 {
    conn.execute(
        "INSERT INTO transcriptions (text, start_time, end_time, device, confidence) VALUES (?1, ?2, ?2, ?3, ?4)",
        params![text, start, device, confidence],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn fts_search(conn: &Connection, query: &str) -> Vec<(i64, String)> {
    let mut stmt = conn
        .prepare("SELECT t.id, t.text FROM transcriptions t JOIN transcriptions_fts f ON t.id = f.rowid WHERE transcriptions_fts MATCH ?1 ORDER BY rank")
        .unwrap();
    stmt.query_map([query], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

// ── Basic CRUD ──────────────────────────────────────────

#[test]
fn test_insert_returns_incrementing_ids() {
    let (conn, _d) = setup();
    let id1 = insert(&conn, "first", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let id2 = insert(&conn, "second", "2024-01-01T00:00:00Z", "Mic", 0.9);
    assert_eq!(id2, id1 + 1);
}

#[test]
fn test_insert_empty_text() {
    let (conn, _d) = setup();
    let id = insert(&conn, "", "2024-01-01T00:00:00Z", "Mic", 0.5);
    assert!(id > 0);
}

#[test]
fn test_insert_very_long_text() {
    let (conn, _d) = setup();
    let text = "a".repeat(100_000);
    let id = insert(&conn, &text, "2024-01-01T00:00:00Z", "Mic", 0.9);
    let stored: String = conn.query_row("SELECT text FROM transcriptions WHERE id=?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(stored.len(), 100_000);
}

#[test]
fn test_insert_special_characters() {
    let (conn, _d) = setup();
    let text = r#"He said "hello" & she said 'goodbye' <tag> \n \t"#;
    insert(&conn, text, "2024-01-01T00:00:00Z", "Mic", 0.9);
    let stored: String = conn.query_row("SELECT text FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(stored, text);
}

#[test]
fn test_insert_newlines_preserved() {
    let (conn, _d) = setup();
    let text = "line one\nline two\nline three";
    insert(&conn, text, "2024-01-01T00:00:00Z", "Mic", 0.9);
    let stored: String = conn.query_row("SELECT text FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!(stored.contains('\n'));
    assert_eq!(stored.matches('\n').count(), 2);
}

#[test]
fn test_insert_emoji_text() {
    let (conn, _d) = setup();
    insert(&conn, "Hello 🎤🎧 World 🌍", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let stored: String = conn.query_row("SELECT text FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!(stored.contains("🎤"));
}

#[test]
fn test_insert_cjk_text() {
    // #given — FTS5 default tokenizer doesn't split CJK, so test storage only
    let (conn, _d) = setup();
    insert(&conn, "日本語テスト 中文测试 한국어", "2024-01-01T00:00:00Z", "Mic", 0.9);

    // #then — text stored correctly
    let stored: String = conn.query_row("SELECT text FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!(stored.contains("日本語"));
    assert!(stored.contains("한국어"));
}

#[test]
fn test_insert_arabic_rtl_text() {
    let (conn, _d) = setup();
    insert(&conn, "مرحبا بالعالم", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_confidence_zero() {
    let (conn, _d) = setup();
    insert(&conn, "low confidence", "2024-01-01T00:00:00Z", "Mic", 0.0);
    let c: f64 = conn.query_row("SELECT confidence FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!((c - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_confidence_one() {
    let (conn, _d) = setup();
    insert(&conn, "perfect", "2024-01-01T00:00:00Z", "Mic", 1.0);
    let c: f64 = conn.query_row("SELECT confidence FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!((c - 1.0).abs() < f64::EPSILON);
}

// ── FTS5 Search ─────────────────────────────────────────

#[test]
fn test_fts_exact_match() {
    let (conn, _d) = setup();
    insert(&conn, "discussing the project timeline", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "timeline");
    assert_eq!(results.len(), 1);
}

#[test]
fn test_fts_prefix_search() {
    let (conn, _d) = setup();
    insert(&conn, "budget meeting discussion", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "budg*");
    assert_eq!(results.len(), 1);
}

#[test]
fn test_fts_phrase_search() {
    let (conn, _d) = setup();
    insert(&conn, "the quick brown fox jumps", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "brown bag lunch", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "\"quick brown\"");
    assert_eq!(results.len(), 1);
    assert!(results[0].1.contains("quick brown"));
}

#[test]
fn test_fts_boolean_and() {
    let (conn, _d) = setup();
    insert(&conn, "meeting about budget and timeline", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "budget review only", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "timeline discussion only", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "budget AND timeline");
    assert_eq!(results.len(), 1);
}

#[test]
fn test_fts_boolean_or() {
    let (conn, _d) = setup();
    insert(&conn, "budget review", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "timeline review", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "unrelated content", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "budget OR timeline");
    assert_eq!(results.len(), 2);
}

#[test]
fn test_fts_boolean_not() {
    let (conn, _d) = setup();
    insert(&conn, "budget meeting important", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "budget lunch casual", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "budget NOT lunch");
    assert_eq!(results.len(), 1);
    assert!(results[0].1.contains("meeting"));
}

#[test]
fn test_fts_no_results() {
    let (conn, _d) = setup();
    insert(&conn, "hello world", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "zzzznonexistent");
    assert!(results.is_empty());
}

#[test]
fn test_fts_case_insensitive() {
    let (conn, _d) = setup();
    insert(&conn, "IMPORTANT MEETING", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "important");
    assert_eq!(results.len(), 1);
}

#[test]
fn test_fts_multiple_words_implicit_and() {
    let (conn, _d) = setup();
    insert(&conn, "quarterly budget review meeting", "2024-01-01T00:00:00Z", "Mic", 0.9);
    insert(&conn, "quarterly earnings call", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "quarterly budget");
    assert_eq!(results.len(), 1);
}

#[test]
fn test_fts_after_delete() {
    let (conn, _d) = setup();
    let id = insert(&conn, "deletable content here", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let results = fts_search(&conn, "deletable");
    assert_eq!(results.len(), 1);

    conn.execute("DELETE FROM transcriptions WHERE id=?1", [id]).unwrap();
    let results = fts_search(&conn, "deletable");
    assert!(results.is_empty());
}

// ── Time Queries ────────────────────────────────────────

#[test]
fn test_order_by_start_time_desc() {
    let (conn, _d) = setup();
    insert(&conn, "first", "2024-01-01T08:00:00Z", "Mic", 0.9);
    insert(&conn, "second", "2024-01-01T12:00:00Z", "Mic", 0.9);
    insert(&conn, "third", "2024-01-01T16:00:00Z", "Mic", 0.9);

    let mut stmt = conn.prepare("SELECT text FROM transcriptions ORDER BY start_time DESC").unwrap();
    let texts: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert_eq!(texts, vec!["third", "second", "first"]);
}

#[test]
fn test_time_range_boundary_inclusive() {
    let (conn, _d) = setup();
    insert(&conn, "boundary", "2024-01-15T14:00:00Z", "Mic", 0.9);
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM transcriptions WHERE start_time >= ?1 AND start_time <= ?1").unwrap();
    let count: i64 = stmt.query_row(["2024-01-15T14:00:00Z"], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_same_timestamp_multiple_entries() {
    let (conn, _d) = setup();
    let ts = "2024-01-15T14:00:00Z";
    for i in 0..10 {
        insert(&conn, &format!("entry {i}"), ts, "Mic", 0.9);
    }
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM transcriptions WHERE start_time=?1", [ts], |r| r.get(0)).unwrap();
    assert_eq!(count, 10);
}

#[test]
fn test_cross_day_boundary() {
    let (conn, _d) = setup();
    insert(&conn, "late night", "2024-01-15T23:55:00Z", "Mic", 0.9);
    insert(&conn, "early morning", "2024-01-16T00:05:00Z", "Mic", 0.9);

    let mut stmt = conn.prepare("SELECT text FROM transcriptions WHERE start_time >= '2024-01-16T00:00:00Z'").unwrap();
    let results: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], "early morning");
}

// ── Pagination ──────────────────────────────────────────

#[test]
fn test_limit_zero() {
    let (conn, _d) = setup();
    insert(&conn, "test", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let mut stmt = conn.prepare("SELECT COUNT(*) FROM (SELECT id FROM transcriptions LIMIT 0)").unwrap();
    let count: i64 = stmt.query_row([], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_offset_beyond_total() {
    let (conn, _d) = setup();
    for i in 0..5 {
        insert(&conn, &format!("entry {i}"), "2024-01-01T00:00:00Z", "Mic", 0.9);
    }
    let mut stmt = conn.prepare("SELECT text FROM transcriptions LIMIT 10 OFFSET 100").unwrap();
    let results: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    assert!(results.is_empty());
}

#[test]
fn test_pagination_consistency() {
    let (conn, _d) = setup();
    for i in 0..50 {
        insert(&conn, &format!("entry {i:03}"), &format!("2024-01-{:02}T12:00:00Z", (i % 28) + 1), "Mic", 0.9);
    }

    let mut all_ids = Vec::new();
    for page in 0..10 {
        let mut stmt = conn.prepare("SELECT id FROM transcriptions ORDER BY start_time DESC LIMIT 5 OFFSET ?1").unwrap();
        let ids: Vec<i64> = stmt.query_map([page * 5], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        all_ids.extend(ids);
    }
    // No duplicates
    let unique: std::collections::HashSet<i64> = all_ids.iter().copied().collect();
    assert_eq!(unique.len(), all_ids.len());
}

// ── Device Field ────────────────────────────────────────

#[test]
fn test_filter_by_device() {
    let (conn, _d) = setup();
    insert(&conn, "from airpods", "2024-01-01T00:00:00Z", "AirPods Pro", 0.9);
    insert(&conn, "from macbook", "2024-01-01T00:00:00Z", "MacBook Pro Microphone", 0.9);
    insert(&conn, "from airpods again", "2024-01-01T00:00:00Z", "AirPods Pro", 0.9);

    let mut stmt = conn.prepare("SELECT COUNT(*) FROM transcriptions WHERE device=?1").unwrap();
    let count: i64 = stmt.query_row(["AirPods Pro"], |r| r.get(0)).unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_empty_device_name() {
    let (conn, _d) = setup();
    insert(&conn, "no device", "2024-01-01T00:00:00Z", "", 0.9);
    let device: String = conn.query_row("SELECT device FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(device, "");
}

#[test]
fn test_long_device_name() {
    let (conn, _d) = setup();
    let device = "A".repeat(500);
    insert(&conn, "test", "2024-01-01T00:00:00Z", &device, 0.9);
    let stored: String = conn.query_row("SELECT device FROM transcriptions ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(stored.len(), 500);
}

// ── Stress Tests ────────────────────────────────────────

#[test]
fn test_bulk_insert_1000() {
    let (conn, _d) = setup();
    conn.execute_batch("BEGIN").unwrap();
    for i in 0..1000 {
        insert(&conn, &format!("bulk entry number {i}"), "2024-01-01T00:00:00Z", "Mic", 0.9);
    }
    conn.execute_batch("COMMIT").unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1000);
}

#[test]
fn test_fts_search_over_1000_entries() {
    let (conn, _d) = setup();
    conn.execute_batch("BEGIN").unwrap();
    for i in 0..1000 {
        let text = if i % 100 == 0 { format!("special entry {i}") } else { format!("regular entry {i}") };
        insert(&conn, &text, "2024-01-01T00:00:00Z", "Mic", 0.9);
    }
    conn.execute_batch("COMMIT").unwrap();

    let results = fts_search(&conn, "special");
    assert_eq!(results.len(), 10);
}

// ── Schema Integrity ────────────────────────────────────

#[test]
fn test_autoincrement_after_delete() {
    let (conn, _d) = setup();
    let id1 = insert(&conn, "first", "2024-01-01T00:00:00Z", "Mic", 0.9);
    conn.execute("DELETE FROM transcriptions WHERE id=?1", [id1]).unwrap();
    let id2 = insert(&conn, "second", "2024-01-01T00:00:00Z", "Mic", 0.9);
    assert!(id2 > id1);
}

#[test]
fn test_created_at_auto_populated() {
    let (conn, _d) = setup();
    insert(&conn, "test", "2024-01-01T00:00:00Z", "Mic", 0.9);
    let created: String = conn.query_row("SELECT created_at FROM transcriptions LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!(!created.is_empty());
}

#[test]
fn test_wal_mode_enabled() {
    let (conn, _d) = setup();
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, "wal");
}
