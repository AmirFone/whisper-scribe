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
fn test_insert_then_get_unified_timeline() {
    let (storage, _dir) = fresh_storage();
    let t1 = Utc.with_ymd_and_hms(2024, 3, 10, 14, 5, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 3, 10, 15, 5, 0).unwrap();
    storage.insert_transcription("morning note", &t1, "Mic").unwrap();
    storage.insert_transcription("afternoon note", &t2, "Mic").unwrap();

    let slots = storage.get_unified_timeline(100, 0).unwrap();
    assert_eq!(slots.len(), 2);
    assert!(slots[0].segments[0].text.contains("afternoon"));
    assert!(slots[1].segments[0].text.contains("morning"));
}

#[test]
fn test_search_across_types() {
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 14, 0, 0).unwrap();
    storage.insert_transcription("quarterly budget review", &ts, "Mic").unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 3, 10, 15, 0, 0).unwrap();
    storage.insert_screen_context("App: Sheets, editing budget spreadsheet", &ts2).unwrap();
    let ts3 = Utc.with_ymd_and_hms(2024, 3, 10, 16, 0, 0).unwrap();
    storage.insert_transcription("coffee chat with team", &ts3, "Mic").unwrap();

    let hits = storage.search_segments("budget").unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn test_concurrent_inserts() {
    let (storage, _dir) = fresh_storage();
    let storage = Arc::new(storage);

    let s1 = storage.clone();
    let t1 = thread::spawn(move || {
        for i in 0..50 {
            let ts = Utc.with_ymd_and_hms(2024, 3, 10, (i % 24) as u32, i as u32, 0).unwrap();
            s1.insert_transcription(&format!("t1 seg {i}"), &ts, "Mic").unwrap();
        }
    });
    let s2 = storage.clone();
    let t2 = thread::spawn(move || {
        for i in 0..50 {
            let ts = Utc.with_ymd_and_hms(2024, 3, 11, (i % 24) as u32, i as u32, 0).unwrap();
            s2.insert_screen_context(&format!("t2 cap {i}"), &ts).unwrap();
        }
    });
    t1.join().unwrap();
    t2.join().unwrap();

    let count = storage.segment_count().unwrap();
    assert_eq!(count, 100);
}

#[test]
fn test_date_range_unified() {
    let (storage, _dir) = fresh_storage();
    let t1 = Utc.with_ymd_and_hms(2024, 3, 10, 14, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 3, 10, 18, 0, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2024, 3, 11, 9, 0, 0).unwrap();
    storage.insert_transcription("a", &t1, "Mic").unwrap();
    storage.insert_screen_context("b", &t2).unwrap();
    storage.insert_transcription("c", &t3, "Mic").unwrap();

    let from = Storage::hour_key_of(&Utc.with_ymd_and_hms(2024, 3, 10, 0, 0, 0).unwrap());
    let to = Storage::hour_key_of(&Utc.with_ymd_and_hms(2024, 3, 10, 23, 0, 0).unwrap());
    let slots = storage.get_segments_by_date_range(&from, &to).unwrap();

    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|s| s.hour_key.starts_with("2024-03-10")));
}

#[test]
fn test_orphan_dedup_on_segments() {
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 3, 10, 14, 5, 0).unwrap();
    storage.insert_transcription("first", &ts, "Mic").unwrap();

    assert!(storage.is_segment_processed(&ts));

    let ts2 = Utc.with_ymd_and_hms(2024, 3, 10, 14, 7, 0).unwrap();
    assert!(!storage.is_segment_processed(&ts2));
}

#[test]
fn test_unified_timeline_respects_limit() {
    let (storage, _dir) = fresh_storage();
    for h in 0..20 {
        let ts = Utc.with_ymd_and_hms(2024, 3, 12, h, 0, 0).unwrap();
        storage.insert_transcription(&format!("hour {h}"), &ts, "Mic").unwrap();
    }

    let page = storage.get_unified_timeline(5, 0).unwrap();
    let full = storage.get_unified_timeline(50, 0).unwrap();
    assert_eq!(page.len(), 5);
    assert!(full.len() >= 20);
}

#[test]
fn test_interleaved_segments_within_hour() {
    let (storage, _dir) = fresh_storage();
    let t1 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 5, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 10, 0).unwrap();
    let t3 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 12, 0).unwrap();
    let t4 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 15, 0).unwrap();

    storage.insert_transcription("spoke about project", &t1, "Mic").unwrap();
    storage.insert_screen_context("App: VS Code, editing main.rs", &t2).unwrap();
    storage.insert_transcription("discussed next steps", &t3, "Mic").unwrap();
    storage.insert_screen_context("App: Chrome, viewing GitHub PR", &t4).unwrap();

    let slots = storage.get_unified_timeline(10, 0).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].segments.len(), 4);
    assert_eq!(slots[0].segments[0].segment_type, "transcription");
    assert_eq!(slots[0].segments[1].segment_type, "screen");
    assert_eq!(slots[0].segments[2].segment_type, "transcription");
    assert_eq!(slots[0].segments[3].segment_type, "screen");
    assert!(slots[0].segments[0].timestamp < slots[0].segments[1].timestamp);
}
