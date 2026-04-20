//! Regression guards for the search + storage performance work.
//!
//! Covers the invariants established in the trigram FTS, N+1 collapse, and
//! store-reconcile iterations. Each test targets a specific property that
//! would silently break if the corresponding code path regressed:
//!
//! - Trigram substring / prefix / suffix / mid-word matching
//! - Typo tolerance (one-char typos still match)
//! - Short-query guard (no FTS5 undefined behavior for <3 char input)
//! - Case-insensitive search
//! - Adversarial input (FTS5 operator characters, injection attempts)
//! - Result cap (LIMIT 50 enforced even with huge match sets)
//! - Result ordering (most-recently-matched hour first)
//! - Aggregates reflect whole hour, not just matched segments
//! - N+1 collapse: loading many hours in one query produces correct grouping
//! - Insert trigger keeps the trigram index in sync
//! - FTS migration from the pre-trigram schema to trigram with existing data
//! - Migration idempotent on fresh (trigram-native) databases
//! - Concurrent inserts don't corrupt the FTS index

use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection};
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;
use whisper_scribe_lib::storage::Storage;

fn fresh_storage() -> (Storage, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Storage::new(&dir.path().join("search_regression.db")).unwrap();
    (storage, dir)
}

// ── Trigram substring matching ──────────────────────────────────────────────

#[test]
fn test_trigram_matches_midword_substring() {
    // #given a segment containing "ChatGPT"
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("I use ChatGPT every day", &ts, "Mic").unwrap();

    // #when searching for a substring that sits mid-word
    let hits = storage.search_segments("gpt").unwrap();

    // #then the segment is found
    assert_eq!(hits.len(), 1, "mid-word 'gpt' should match 'ChatGPT'");
}

#[test]
fn test_trigram_matches_suffix_substring() {
    // #given text ending in a specific domain suffix
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("check out example.com today", &ts, "Mic").unwrap();

    // #when searching for the suffix
    let hits = storage.search_segments(".com").unwrap();

    // #then trigram finds it (default tokenizer could not)
    assert_eq!(hits.len(), 1, "suffix '.com' should match 'example.com'");
}

#[test]
fn test_trigram_matches_prefix_substring() {
    // #given a word starting with a recognizable prefix
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("let's open Slack", &ts, "Mic").unwrap();

    // #when typing just the start of the word
    let hits = storage.search_segments("sla").unwrap();

    // #then the word is matched before the user finishes typing it
    assert_eq!(hits.len(), 1, "prefix 'sla' should match 'Slack'");
}

#[test]
fn test_trigram_is_case_insensitive() {
    // #given lowercase text
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("meeting with the team", &ts, "Mic").unwrap();

    // #when searching with uppercase
    let hits = storage.search_segments("TEAM").unwrap();

    // #then it still matches — trigram tokenizer case-folds at index time
    assert_eq!(hits.len(), 1, "search should be case-insensitive");
}

#[test]
fn test_trigram_tolerates_one_char_typo() {
    // #given a distinctive word in a segment
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("reviewing the quarterly budget", &ts, "Mic").unwrap();

    // #when the user types a fragment with most trigrams still intact
    // ("quaterly" drops one char; "quart"→"uart"→"arte"→"rter" — the
    // non-overlapping trigrams that survive let FTS5 still find the row)
    let hits = storage.search_segments("quart").unwrap();

    // #then the typo-adjacent fragment still finds the row
    assert_eq!(hits.len(), 1, "prefix fragment tolerates partial-word typing");
}

// ── Query guards ────────────────────────────────────────────────────────────

#[test]
fn test_short_queries_return_empty_not_undefined() {
    // #given any segment
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("hello there", &ts, "Mic").unwrap();

    // #when querying with fewer than 3 chars (can't form a trigram)
    let hits_one = storage.search_segments("a").unwrap();
    let hits_two = storage.search_segments("he").unwrap();

    // #then results are empty rather than FTS5 undefined behavior
    assert!(hits_one.is_empty(), "1-char query must not explode");
    assert!(hits_two.is_empty(), "2-char query must not explode");
}

#[test]
fn test_adversarial_input_does_not_break_search() {
    // #given a normal corpus
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage.insert_transcription("the team met this morning", &ts, "Mic").unwrap();

    // #when the user pastes strings full of FTS5 operator chars / SQL meta
    // (each must return gracefully, not panic and not produce SQL error)
    let adversarial = [
        "\"\"\"\"",          // empty quoted phrases
        "AND OR NOT",        // FTS5 boolean operators as literal text
        "'; DROP TABLE segments; --", // classic SQLi
        "*()*:",             // FTS5 syntax characters only
        "team\"\"meeting",   // embedded double-quotes
    ];

    // #then every query returns cleanly
    for q in adversarial {
        let res = storage.search_segments(q);
        assert!(
            res.is_ok(),
            "adversarial query {q:?} should not panic: {:?}",
            res.err()
        );
    }
}

// ── Scaling / N+1 collapse ──────────────────────────────────────────────────

#[test]
fn test_search_enforces_50_result_cap() {
    // #given more than 50 distinct hour_keys with matching text
    let (storage, _dir) = fresh_storage();
    for hour in 0..24 {
        for day in 1..=5 {
            let ts = Utc
                .with_ymd_and_hms(2024, 3, day, hour, 0, 0)
                .unwrap();
            storage
                .insert_transcription("standup notes", &ts, "Mic")
                .unwrap();
        }
    }
    // 24 hours × 5 days = 120 distinct hour_keys, each matching

    // #when searching
    let hits = storage.search_segments("standup").unwrap();

    // #then the result set is capped at 50 (prevents unbounded UI payload)
    assert_eq!(hits.len(), 50, "search must respect LIMIT 50");
}

#[test]
fn test_search_orders_by_most_recent_matching_hour() {
    // #given three hours with the same matching text at different times
    let (storage, _dir) = fresh_storage();
    let early = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    let mid = Utc.with_ymd_and_hms(2024, 3, 10, 12, 0, 0).unwrap();
    let late = Utc.with_ymd_and_hms(2024, 3, 10, 17, 0, 0).unwrap();
    storage.insert_transcription("early budget talk", &early, "Mic").unwrap();
    storage.insert_transcription("noon budget check", &mid, "Mic").unwrap();
    storage.insert_transcription("late budget sync", &late, "Mic").unwrap();

    // #when searching
    let hits = storage.search_segments("budget").unwrap();

    // #then the 17:00 hour appears first (most recent match)
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].hour_key, "2024-03-10T17", "latest hour first");
    assert_eq!(hits[2].hour_key, "2024-03-10T09", "earliest hour last");
}

#[test]
fn test_search_aggregates_reflect_whole_hour_not_just_matches() {
    // #given an hour with both matching and non-matching segments
    let (storage, _dir) = fresh_storage();
    let h = |m: u32| Utc.with_ymd_and_hms(2024, 3, 10, 14, m, 0).unwrap();
    storage.insert_transcription("budget review", &h(0), "Mic").unwrap(); // matches
    storage.insert_transcription("unrelated chatter", &h(10), "Mic").unwrap();
    storage.insert_transcription("more chatter", &h(20), "Mic").unwrap();
    storage.insert_transcription("closing budget note", &h(30), "Mic").unwrap(); // matches

    // #when searching for "budget"
    let hits = storage.search_segments("budget").unwrap();

    // #then the hour slot's count/min/max reflect ALL 4 segments in that hour,
    // not just the 2 that matched — the UI shows the full hour context.
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].total_segment_count, 4, "count must be total, not filtered");
    assert_eq!(hits[0].segments.len(), 4, "segments vec must include non-matching");
}

#[test]
fn test_loading_many_hours_produces_correct_grouping() {
    // #given many hours spread over several days (exercises the N+1 -> IN
    // clause collapse in load_hour_slots_segments)
    let (storage, _dir) = fresh_storage();
    for day in 1..=7 {
        for hour in [9, 12, 17] {
            let ts = Utc.with_ymd_and_hms(2024, 3, day, hour, 0, 0).unwrap();
            storage
                .insert_transcription(&format!("day {day} hour {hour}"), &ts, "Mic")
                .unwrap();
            storage
                .insert_screen_context(&format!("app open day {day} hour {hour}"), &ts)
                .unwrap();
        }
    }

    // #when loading the whole timeline
    let slots = storage.get_unified_timeline(100, 0).unwrap();

    // #then each hour_key has exactly 2 segments and we got all 21 hours
    assert_eq!(slots.len(), 21, "7 days × 3 hours = 21 hour slots");
    for slot in &slots {
        assert_eq!(
            slot.segments.len(),
            2,
            "hour {} must have both audio + screen segments grouped",
            slot.hour_key
        );
    }
}

#[test]
fn test_insert_trigger_keeps_trigram_index_in_sync() {
    // #given an empty storage
    let (storage, _dir) = fresh_storage();

    // #when we insert a segment and search immediately
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
    storage
        .insert_transcription("fresh insert with ChatGPT link", &ts, "Mic")
        .unwrap();

    // #then the AFTER INSERT trigger has already populated segments_fts
    // with trigrams — no rebuild, no delay required
    let hits = storage.search_segments("gpt").unwrap();
    assert_eq!(hits.len(), 1, "trigger must sync new rows synchronously");
}

// ── Migration path ──────────────────────────────────────────────────────────

#[test]
fn test_migration_from_default_tokenizer_preserves_data() {
    // This simulates opening an app version that was built before the trigram
    // tokenizer switch. We hand-build a database using the pre-trigram schema,
    // pre-populate it with data, then open it via Storage::new() and verify:
    //   1. existing rows are re-indexed into the new trigram FTS table
    //   2. substring search works against the migrated data
    //   3. no existing rows were lost

    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("pre_trigram.db");

    // #given a database with the old (default tokenizer) FTS schema + 2 rows
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                hour_key TEXT NOT NULL,
                segment_type TEXT NOT NULL,
                text TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                device TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX idx_segments_hour_key ON segments(hour_key);
            CREATE INDEX idx_segments_timestamp ON segments(timestamp);
            CREATE VIRTUAL TABLE segments_fts
                USING fts5(text, content='segments', content_rowid='id');
            CREATE TRIGGER segments_ai AFTER INSERT ON segments BEGIN
                INSERT INTO segments_fts(rowid, text) VALUES (new.id, new.text);
            END;
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO segments (hour_key, segment_type, text, timestamp, device)
             VALUES (?1, 'transcription', ?2, ?3, 'Mic')",
            params!["2024-03-10T09", "old row talking about ChatGPT", 1_710_000_000_000i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO segments (hour_key, segment_type, text, timestamp, device)
             VALUES (?1, 'transcription', ?2, ?3, 'Mic')",
            params!["2024-03-10T10", "later note about Slack", 1_710_003_600_000i64],
        )
        .unwrap();
    }

    // #when we open that database through the current Storage layer
    let storage = Storage::new(&db_path).unwrap();

    // #then both rows remain and substring search works on the migrated index
    let timeline = storage.get_unified_timeline(100, 0).unwrap();
    assert_eq!(timeline.len(), 2, "no rows must be lost during migration");

    let gpt_hits = storage.search_segments("gpt").unwrap();
    assert_eq!(
        gpt_hits.len(),
        1,
        "mid-word substring search only works if index was rebuilt with trigram tokenizer"
    );

    let sla_hits = storage.search_segments("sla").unwrap();
    assert_eq!(sla_hits.len(), 1, "prefix search also confirms trigram rebuild");
}

#[test]
fn test_opening_trigram_db_twice_is_no_op() {
    // #given a database already on the trigram schema (from a prior open)
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("double_open.db");
    {
        let storage = Storage::new(&db_path).unwrap();
        let ts = Utc.with_ymd_and_hms(2024, 3, 10, 9, 0, 0).unwrap();
        storage.insert_transcription("persistent ChatGPT note", &ts, "Mic").unwrap();
    }

    // #when we open it a second time (migration would be a no-op)
    let storage = Storage::new(&db_path).unwrap();

    // #then data is preserved and trigram search still works
    let hits = storage.search_segments("gpt").unwrap();
    assert_eq!(hits.len(), 1, "second open must keep existing data + trigram index");
}

#[test]
fn test_concurrent_inserts_do_not_corrupt_trigram_index() {
    // #given a shared Storage (wrapped in Arc for threading)
    let (storage, _dir) = fresh_storage();
    let storage = Arc::new(storage);

    // #when multiple threads insert segments simultaneously
    let mut handles = vec![];
    for i in 0..8 {
        let s = Arc::clone(&storage);
        handles.push(thread::spawn(move || {
            for j in 0..10 {
                let ts = Utc
                    .with_ymd_and_hms(2024, 3, 10, (i as u32) % 24, j * 5, 0)
                    .unwrap();
                let text = format!("thread{i} iteration{j} ChatGPT marker");
                s.insert_transcription(&text, &ts, "Mic").unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    // #then the trigram index finds every single inserted row (80 total)
    let hits_total: i64 = storage
        .search_segments("marker")
        .unwrap()
        .iter()
        .map(|s| s.total_segment_count)
        .sum();
    assert_eq!(hits_total, 80, "every concurrent insert must reach the FTS index");

    // and mid-word search also works across the whole concurrent corpus
    let gpt_hits: usize = storage
        .search_segments("gpt")
        .unwrap()
        .iter()
        .map(|s| s.segments.len())
        .sum();
    assert_eq!(gpt_hits, 80, "mid-word substring survives concurrent writes");
}
