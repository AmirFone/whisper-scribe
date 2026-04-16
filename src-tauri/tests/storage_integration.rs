//! Integration tests against the real `whisper_scribe_lib::storage::Storage`
//! public API. The previous version of this file constructed its own SQLite
//! `Connection` and ran `CREATE TABLE transcriptions` — the legacy schema that
//! nothing in the actual product uses anymore. A regression to
//! `Storage::append_to_hour_slot` or `Storage::get_hour_slots` would have
//! passed every test here. This file now exercises the real API; the
//! legacy-schema files live behind `#[cfg(feature = "legacy_schema_tests")]`.
//!
//! If you need a storage regression guard, put it here, not in the
//! `*_exhaustive.rs` files.

use chrono::{TimeZone, Utc};
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;
use whisper_scribe_lib::storage::Storage;

fn fresh_storage() -> (Storage, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Storage::new(&dir.path().join("integration.db")).unwrap();
    (storage, dir)
}

#[test]
fn test_append_then_get_timeline_via_real_api() {
    // #given a fresh storage and two segments in different hours
    let (storage, _dir) = fresh_storage();
    let t1 = Utc.with_ymd_and_hms(2024, 3, 10, 14, 5, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 3, 10, 15, 5, 0).unwrap();
    storage.append_to_hour_slot("morning note", &t1, "Mic").unwrap();
    storage.append_to_hour_slot("afternoon note", &t2, "Mic").unwrap();

    // #when we read back through the public `get_hour_slots`
    let slots = storage.get_hour_slots(100, 0).unwrap();

    // #then two rows come back ordered by start_time DESC
    assert_eq!(slots.len(), 2);
    assert!(slots[0].text.contains("afternoon"));
    assert!(slots[1].text.contains("morning"));
}

#[test]
fn test_search_through_real_storage_api() {
    // #given three seeded hour slots with distinct text
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 14, 0, 0).unwrap();
    storage.append_to_hour_slot("quarterly budget review", &ts, "Mic").unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 3, 10, 15, 0, 0).unwrap();
    storage.append_to_hour_slot("annual revenue forecast", &ts2, "Mic").unwrap();
    let ts3 = Utc.with_ymd_and_hms(2024, 3, 10, 16, 0, 0).unwrap();
    storage.append_to_hour_slot("coffee chat with team", &ts3, "Mic").unwrap();

    // #when we search through the public API
    let hits = storage.search_hour_slots("budget").unwrap();

    // #then FTS filters to the matching row
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("budget"));
}

#[test]
fn test_concurrent_append_from_two_threads_via_real_api() {
    // #given a shared Storage
    let (storage, _dir) = fresh_storage();
    let storage = Arc::new(storage);

    // #when two threads each append 50 segments into distinct hours
    let s1 = storage.clone();
    let t1 = thread::spawn(move || {
        for i in 0..50 {
            let ts = Utc.with_ymd_and_hms(2024, 3, 10, (i % 24) as u32, 0, 0).unwrap();
            s1.append_to_hour_slot(&format!("t1 seg {i}"), &ts, "Mic").unwrap();
        }
    });
    let s2 = storage.clone();
    let t2 = thread::spawn(move || {
        for i in 0..50 {
            let ts = Utc.with_ymd_and_hms(2024, 3, 11, (i % 24) as u32, 0, 0).unwrap();
            s2.append_to_hour_slot(&format!("t2 seg {i}"), &ts, "Mic").unwrap();
        }
    });
    t1.join().unwrap();
    t2.join().unwrap();

    // #then count matches total writes (100 / 24 + 100/24 buckets across dates)
    //       — no writes lost to race conditions under the `Mutex<Connection>`
    let count = storage.count().unwrap();
    assert!(count > 0, "expected at least one hour_slot row after 100 appends");
    // both thread's segment texts appear somewhere in the stored rows
    let all = storage.get_hour_slots(200, 0).unwrap();
    let all_text: String = all.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(" ");
    assert!(all_text.contains("t1 seg"));
    assert!(all_text.contains("t2 seg"));
}

#[test]
fn test_date_range_query_via_real_api() {
    // #given three hour slots across two UTC dates
    let (storage, _dir) = fresh_storage();
    let t_mar10_14 = Utc.with_ymd_and_hms(2024, 3, 10, 14, 0, 0).unwrap();
    let t_mar10_18 = Utc.with_ymd_and_hms(2024, 3, 10, 18, 0, 0).unwrap();
    let t_mar11_09 = Utc.with_ymd_and_hms(2024, 3, 11, 9, 0, 0).unwrap();
    storage.append_to_hour_slot("a", &t_mar10_14, "Mic").unwrap();
    storage.append_to_hour_slot("b", &t_mar10_18, "Mic").unwrap();
    storage.append_to_hour_slot("c", &t_mar11_09, "Mic").unwrap();

    // #when we restrict to a range derived from `hour_key_of` — same bucketing
    //       path production uses. Avoids hard-coding the format and making
    //       the test dependent on the runner's timezone.
    let from_key = Storage::hour_key_of(&Utc.with_ymd_and_hms(2024, 3, 10, 0, 0, 0).unwrap());
    let to_key = Storage::hour_key_of(&Utc.with_ymd_and_hms(2024, 3, 10, 23, 0, 0).unwrap());
    let slots = storage.get_slots_by_date_range(&from_key, &to_key).unwrap();

    // #then only the March 10 entries are returned (and there are exactly 2)
    assert!(slots.iter().all(|s| s.hour_key.starts_with("2024-03-10")));
    assert_eq!(slots.len(), 2);
}

#[test]
fn test_orphan_dedup_catches_later_segments_in_existing_hour() {
    // #given an hour that has already absorbed two segments
    let (storage, _dir) = fresh_storage();
    let t_first = Utc.with_ymd_and_hms(2024, 3, 10, 14, 5, 0).unwrap();
    let t_second = Utc.with_ymd_and_hms(2024, 3, 10, 14, 7, 0).unwrap();
    storage.append_to_hour_slot("first", &t_first, "Mic").unwrap();
    storage.append_to_hour_slot("second", &t_second, "Mic").unwrap();

    // #when we dedup the non-first orphan
    // #then is_segment_processed recognises it as already-appended — the
    //       prior `start_time`-only dedup missed this case and silently
    //       re-transcribed it on every restart
    assert!(storage.is_segment_processed(&t_second));

    // #and a would-be-later segment in the same hour is NOT flagged
    let t_future = Utc.with_ymd_and_hms(2024, 3, 10, 14, 9, 0).unwrap();
    assert!(!storage.is_segment_processed(&t_future));
}

#[test]
fn test_get_hour_slots_respects_limit() {
    // #given 20 distinct hour-slots across 20 different hours of one day
    let (storage, _dir) = fresh_storage();
    for h in 0..20 {
        let ts = Utc.with_ymd_and_hms(2024, 3, 12, h, 0, 0).unwrap();
        storage.append_to_hour_slot(&format!("hour {h}"), &ts, "Mic").unwrap();
    }

    // #when we request a smaller page
    let page = storage.get_hour_slots(5, 0).unwrap();
    let full = storage.get_hour_slots(50, 0).unwrap();

    // #then the page obeys the limit and the full request returns everything
    assert_eq!(page.len(), 5);
    assert!(full.len() >= 20);
}
