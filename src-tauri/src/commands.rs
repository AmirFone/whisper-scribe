use crate::audio_engine;
use crate::device_manager;
use crate::state::AppState;
use crate::storage::UnifiedHourSlot;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::ipc::Channel;
use tokio::time::{sleep, Duration};

const MAX_TIMELINE_LIMIT: i64 = 200;

fn clamp_timeline_params(limit: i64, offset: i64) -> (i64, i64) {
    (limit.clamp(1, MAX_TIMELINE_LIMIT), offset.max(0))
}

#[derive(serde::Serialize)]
pub struct StatusPayload {
    pub is_recording: bool,
    pub is_paused: bool,
    pub device_name: String,
    pub slots_count: i64,
    pub segment_seconds_elapsed: u64,
    pub segment_duration_secs: u64,
    pub audio_level: u32,
    pub is_transcribing: bool,
    pub audio_disk_error: bool,
    pub is_screen_capture_enabled: bool,
    pub is_analyzing_screen: bool,
}

#[derive(Clone, serde::Serialize)]
pub struct AudioLevelEvent {
    pub level: u32,
}

#[tauri::command]
pub async fn get_timeline(
    state: tauri::State<'_, Arc<AppState>>,
    limit: i64,
    offset: i64,
) -> Result<Vec<UnifiedHourSlot>, String> {
    let (limit, offset) = clamp_timeline_params(limit, offset);
    state.storage.get_unified_timeline(limit, offset)
}

#[tauri::command]
pub async fn search_transcriptions(
    state: tauri::State<'_, Arc<AppState>>,
    query: String,
) -> Result<Vec<UnifiedHourSlot>, String> {
    state.storage.search_segments(&query)
}

#[tauri::command]
pub async fn get_status(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<StatusPayload, String> {
    let is_paused = state.is_paused();
    let is_recording = !is_paused;
    let device_name = device_manager::get_current_device_name();
    let slots_count = state.storage.segment_count().unwrap_or(0);

    let elapsed = state
        .segment_started_at()
        .map(|t| (chrono::Utc::now() - t).num_seconds().max(0) as u64)
        .unwrap_or(0);

    let audio_level = state.audio_level();
    let is_transcribing = state.is_transcribing.load(Ordering::Relaxed);
    let audio_disk_error = state.audio_disk_error();
    let is_screen_capture_enabled = state.screen_capture_enabled();
    let is_analyzing_screen = state.is_analyzing_screen.load(Ordering::Relaxed);

    Ok(StatusPayload {
        is_recording,
        is_paused,
        device_name,
        slots_count,
        segment_seconds_elapsed: elapsed,
        segment_duration_secs: audio_engine::segment_duration_secs(),
        audio_level,
        is_transcribing,
        audio_disk_error,
        is_screen_capture_enabled,
        is_analyzing_screen,
    })
}

#[tauri::command]
pub async fn get_slots_by_date_range(
    state: tauri::State<'_, Arc<AppState>>,
    from_key: String,
    to_key: String,
) -> Result<Vec<UnifiedHourSlot>, String> {
    state.storage.get_segments_by_date_range(&from_key, &to_key)
}

#[tauri::command]
pub async fn get_available_dates(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    state.storage.get_available_dates()
}

#[tauri::command]
pub async fn subscribe_audio_level(
    state: tauri::State<'_, Arc<AppState>>,
    channel: Channel<AudioLevelEvent>,
) -> Result<(), String> {
    let level = state.audio_level_arc();
    loop {
        if channel.send(AudioLevelEvent { level: level.load(Ordering::Relaxed) }).is_err() {
            break;
        }
        sleep(Duration::from_millis(33)).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_pause(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, String> {
    Ok(state.toggle_pause())
}

#[tauri::command]
pub async fn toggle_screen_capture(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<bool, String> {
    Ok(state.toggle_screen_capture())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_timeline_params_enforces_upper_bound() {
        let (limit, offset) = clamp_timeline_params(10_000, 0);
        assert_eq!(limit, MAX_TIMELINE_LIMIT);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_clamp_timeline_params_rejects_non_positive_limit() {
        assert_eq!(clamp_timeline_params(0, 0).0, 1);
        assert_eq!(clamp_timeline_params(-5, 0).0, 1);
    }

    #[test]
    fn test_clamp_timeline_params_negative_offset_is_floored_to_zero() {
        let (_, offset) = clamp_timeline_params(50, -10);
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_clamp_timeline_params_preserves_sane_values() {
        assert_eq!(clamp_timeline_params(50, 20), (50, 20));
        assert_eq!(clamp_timeline_params(MAX_TIMELINE_LIMIT, 0), (MAX_TIMELINE_LIMIT, 0));
    }
}
