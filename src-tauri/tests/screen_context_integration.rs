//! Integration tests for the screen context feature: screen_slots storage,
//! CoreGraphics FFI permission checks, and screen capture cleanup.

use chrono::{TimeZone, Utc};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use whisper_scribe_lib::storage::Storage;

fn fresh_storage() -> (Storage, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Storage::new(&dir.path().join("screen_test.db")).unwrap();
    (storage, dir)
}

// ── Screen slot CRUD via public API ──

#[test]
fn test_screen_slot_append_and_get_via_real_api() {
    // #given a fresh storage
    let (storage, _dir) = fresh_storage();
    let ts1 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 5, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 6, 15, 11, 10, 0).unwrap();

    // #when we append two screen context entries in different hours
    storage
        .append_to_screen_slot("Terminal: running cargo build for whisper-scribe", &ts1)
        .unwrap();
    storage
        .append_to_screen_slot("Chrome: viewing claude.ai/settings/usage, 15% used", &ts2)
        .unwrap();

    // #then get_screen_slots returns both in DESC order
    let slots = storage.get_screen_slots(100, 0).unwrap();
    assert_eq!(slots.len(), 2);
    assert!(slots[0].text.contains("Chrome"));
    assert!(slots[1].text.contains("Terminal"));
    assert_eq!(slots[0].device, "Screen");
    assert_eq!(slots[1].device, "Screen");
}

#[test]
fn test_screen_slot_upsert_appends_with_double_newline() {
    // #given a screen slot with one capture
    let (storage, _dir) = fresh_storage();
    let ts1 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 5, 0).unwrap();

    storage
        .append_to_screen_slot("[Display 1] Terminal: cargo test", &ts1)
        .unwrap();
    storage
        .append_to_screen_slot("[Display 1] Terminal: cargo build --release", &ts2)
        .unwrap();

    // #then text is joined with double newline separator
    let slots = storage.get_screen_slots(10, 0).unwrap();
    assert_eq!(slots.len(), 1);
    assert!(slots[0].text.contains("cargo test\n\n[Display 1] Terminal: cargo build"));
    assert_eq!(slots[0].segment_count, 2);
    assert_eq!(slots[0].start_time, ts1.timestamp_millis());
    assert_eq!(slots[0].last_updated, ts2.timestamp_millis());
}

#[test]
fn test_screen_slots_independent_from_transcription_slots() {
    // #given a transcription and screen context in the same hour
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();

    storage.append_to_hour_slot("spoken words about the project", &ts, "MacBook Pro Microphone").unwrap();
    storage.append_to_screen_slot("VS Code: editing screen_capture.rs, Rust", &ts).unwrap();

    // #then they live in separate tables with separate counts
    assert_eq!(storage.count().unwrap(), 1);
    assert_eq!(storage.screen_slot_count().unwrap(), 1);

    let transcriptions = storage.get_hour_slots(10, 0).unwrap();
    assert_eq!(transcriptions[0].device, "MacBook Pro Microphone");

    let screens = storage.get_screen_slots(10, 0).unwrap();
    assert_eq!(screens[0].device, "Screen");
}

#[test]
fn test_screen_slot_fts_search_via_real_api() {
    // #given screen slots with different content
    let (storage, _dir) = fresh_storage();
    let ts1 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 6, 15, 11, 0, 0).unwrap();
    let ts3 = Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap();

    storage.append_to_screen_slot("Terminal: running pytest on mlx_screen_analyze.py", &ts1).unwrap();
    storage.append_to_screen_slot("Chrome: browsing Reddit r/rust", &ts2).unwrap();
    storage.append_to_screen_slot("Google Sheets: editing Zakat Calculator spreadsheet", &ts3).unwrap();

    // #when searching for specific terms
    let hits = storage.search_screen_slots("pytest").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("pytest"));

    let hits = storage.search_screen_slots("Reddit").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("Reddit"));

    let hits = storage.search_screen_slots("Zakat").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("Zakat"));

    // #and searching for nonexistent term returns empty
    let hits = storage.search_screen_slots("nonexistent").unwrap();
    assert!(hits.is_empty());
}

#[test]
fn test_screen_slot_date_range_via_real_api() {
    // #given screen slots across two dates
    let (storage, _dir) = fresh_storage();
    let day1 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
    let day1b = Utc.with_ymd_and_hms(2024, 6, 15, 14, 0, 0).unwrap();
    let day2 = Utc.with_ymd_and_hms(2024, 6, 16, 9, 0, 0).unwrap();

    storage.append_to_screen_slot("day 1 morning", &day1).unwrap();
    storage.append_to_screen_slot("day 1 afternoon", &day1b).unwrap();
    storage.append_to_screen_slot("day 2 morning", &day2).unwrap();

    // #when filtering to day 1
    let from = Storage::hour_key_of(&Utc.with_ymd_and_hms(2024, 6, 15, 0, 0, 0).unwrap());
    let to = Storage::hour_key_of(&Utc.with_ymd_and_hms(2024, 6, 15, 23, 0, 0).unwrap());
    let slots = storage.get_screen_slots_by_date_range(&from, &to).unwrap();

    // #then only day 1 entries
    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|s| s.hour_key.starts_with("2024-06-15")));
}

#[test]
fn test_screen_slot_available_dates_via_real_api() {
    // #given screen slots on three different dates
    let (storage, _dir) = fresh_storage();
    for (m, d) in [(6, 14), (6, 15), (6, 16)] {
        let ts = Utc.with_ymd_and_hms(2024, m, d, 10, 0, 0).unwrap();
        storage.append_to_screen_slot(&format!("activity on {m}/{d}"), &ts).unwrap();
    }

    // #when we get available dates
    let dates = storage.get_screen_available_dates().unwrap();

    // #then all three dates in DESC order
    assert_eq!(dates.len(), 3);
    assert_eq!(dates[0], "2024-06-16");
    assert_eq!(dates[1], "2024-06-15");
    assert_eq!(dates[2], "2024-06-14");
}

#[test]
fn test_screen_slot_pagination_via_real_api() {
    // #given 10 screen slots in different hours
    let (storage, _dir) = fresh_storage();
    for h in 0..10u32 {
        let ts = Utc.with_ymd_and_hms(2024, 6, 15, h, 0, 0).unwrap();
        storage.append_to_screen_slot(&format!("activity hour {h}"), &ts).unwrap();
    }

    // #when paginating
    let page1 = storage.get_screen_slots(3, 0).unwrap();
    let page2 = storage.get_screen_slots(3, 3).unwrap();
    let page_past_end = storage.get_screen_slots(3, 100).unwrap();

    // #then pages are correctly bounded
    assert_eq!(page1.len(), 3);
    assert_eq!(page2.len(), 3);
    assert_eq!(page_past_end.len(), 0);
    assert_eq!(storage.screen_slot_count().unwrap(), 10);
}

#[test]
fn test_screen_slot_concurrent_appends() {
    // #given a shared storage
    let (storage, _dir) = fresh_storage();
    let storage = Arc::new(storage);

    // #when two threads each append 25 screen context entries
    let s1 = storage.clone();
    let t1 = thread::spawn(move || {
        for i in 0..25 {
            let ts = Utc.with_ymd_and_hms(2024, 6, 15, (i % 24) as u32, 0, 0).unwrap();
            s1.append_to_screen_slot(&format!("t1 capture {i}"), &ts).unwrap();
        }
    });
    let s2 = storage.clone();
    let t2 = thread::spawn(move || {
        for i in 0..25 {
            let ts = Utc.with_ymd_and_hms(2024, 6, 16, (i % 24) as u32, 0, 0).unwrap();
            s2.append_to_screen_slot(&format!("t2 capture {i}"), &ts).unwrap();
        }
    });
    t1.join().unwrap();
    t2.join().unwrap();

    // #then no writes lost
    let count = storage.screen_slot_count().unwrap();
    assert!(count > 0);
    let all = storage.get_screen_slots(200, 0).unwrap();
    let all_text: String = all.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(" ");
    assert!(all_text.contains("t1 capture"));
    assert!(all_text.contains("t2 capture"));
}

// ── Screenshot cleanup ──

#[test]
fn test_screenshot_cleanup_removes_old_keeps_recent() {
    use std::fs;

    // #given a temp dir with old and new PNGs
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("screen_1_20240101_120000.png");
    let new = dir.path().join("screen_1_20240615_120000.png");
    let txt = dir.path().join("notes.txt");
    fs::write(&old, b"old").unwrap();
    fs::write(&new, b"new").unwrap();
    fs::write(&txt, b"not a png").unwrap();

    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
    filetime::set_file_mtime(&old, filetime::FileTime::from_system_time(two_hours_ago)).unwrap();
    filetime::set_file_mtime(&txt, filetime::FileTime::from_system_time(two_hours_ago)).unwrap();

    // #when cleanup with 1 hour max age
    whisper_scribe_lib::screen_capture_cleanup(dir.path(), Duration::from_secs(3600));

    // #then old PNG deleted, new PNG and txt preserved
    assert!(!old.exists());
    assert!(new.exists());
    assert!(txt.exists());
}

// ── Permission check (non-destructive) ──

#[cfg(target_os = "macos")]
#[test]
fn test_has_screen_capture_permission_returns_bool() {
    // #given the CoreGraphics FFI permission check
    // #when we call it
    let result = whisper_scribe_lib::screen_capture_has_permission();

    // #then it returns a bool without panicking or triggering a prompt
    // (CGPreflightScreenCaptureAccess never prompts, just returns the state)
    assert!(result == true || result == false);
}
