use chrono::{TimeZone, Utc};
use std::time::Duration;
use tempfile::TempDir;
use whisper_scribe_lib::storage::Storage;

fn fresh_storage() -> (Storage, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Storage::new(&dir.path().join("screen_test.db")).unwrap();
    (storage, dir)
}

#[test]
fn test_screen_segments_have_correct_type() {
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 6, 15, 10, 5, 0).unwrap();
    storage.insert_screen_context("App: VS Code", &ts).unwrap();

    let slots = storage.get_unified_timeline(10, 0).unwrap();
    assert_eq!(slots[0].segments[0].segment_type, "screen");
    assert_eq!(slots[0].segments[0].device, "Screen");
}

#[test]
fn test_screen_and_transcription_mixed_in_same_hour() {
    let (storage, _dir) = fresh_storage();
    let ts1 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 5, 0).unwrap();

    storage.insert_transcription("spoken words", &ts1, "Mic").unwrap();
    storage.insert_screen_context("editing code", &ts2).unwrap();

    let slots = storage.get_unified_timeline(10, 0).unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].segments.len(), 2);
    assert_eq!(slots[0].total_segment_count, 2);
}

#[test]
fn test_search_finds_screen_content() {
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();
    storage.insert_screen_context("App: Google Sheets, editing Zakat Calculator", &ts).unwrap();

    let hits = storage.search_segments("Zakat").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].segments[0].text.contains("Zakat"));
}

#[test]
fn test_screenshot_cleanup_removes_old_keeps_recent() {
    use std::fs;

    let dir = TempDir::new().unwrap();
    let old = dir.path().join("screen_1_old.png");
    let new = dir.path().join("screen_1_new.png");
    let txt = dir.path().join("notes.txt");
    fs::write(&old, b"old").unwrap();
    fs::write(&new, b"new").unwrap();
    fs::write(&txt, b"not png").unwrap();

    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
    filetime::set_file_mtime(&old, filetime::FileTime::from_system_time(two_hours_ago)).unwrap();
    filetime::set_file_mtime(&txt, filetime::FileTime::from_system_time(two_hours_ago)).unwrap();

    whisper_scribe_lib::screen_capture_cleanup(dir.path(), Duration::from_secs(3600));

    assert!(!old.exists());
    assert!(new.exists());
    assert!(txt.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn test_has_screen_capture_permission_returns_bool() {
    let result = whisper_scribe_lib::screen_capture_has_permission();
    assert!(result == true || result == false);
}

#[test]
fn test_segment_count_includes_both_types() {
    let (storage, _dir) = fresh_storage();
    let ts = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();

    storage.insert_transcription("a", &ts, "Mic").unwrap();
    storage.insert_screen_context("b", &ts).unwrap();
    assert_eq!(storage.segment_count().unwrap(), 2);
}

#[test]
fn test_available_dates_from_mixed_types() {
    let (storage, _dir) = fresh_storage();
    let d1 = Utc.with_ymd_and_hms(2024, 6, 14, 10, 0, 0).unwrap();
    let d2 = Utc.with_ymd_and_hms(2024, 6, 15, 10, 0, 0).unwrap();

    storage.insert_transcription("a", &d1, "Mic").unwrap();
    storage.insert_screen_context("b", &d2).unwrap();

    let dates = storage.get_available_dates().unwrap();
    assert_eq!(dates.len(), 2);
}
