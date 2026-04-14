use crate::AppState;
use std::sync::Arc;
use tauri::ipc::Channel;

#[derive(serde::Serialize, Clone)]
pub struct HourSlotPayload {
    pub id: i64,
    pub hour_key: String,
    pub text: String,
    pub start_time: String,
    pub last_updated: String,
    pub device: String,
    pub segment_count: i64,
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
) -> Result<Vec<HourSlotPayload>, String> {
    let slots = state.storage.get_hour_slots(limit, offset)?;
    Ok(slots
        .into_iter()
        .map(|s| HourSlotPayload {
            id: s.id,
            hour_key: s.hour_key,
            text: s.text,
            start_time: s.start_time,
            last_updated: s.last_updated,
            device: s.device,
            segment_count: s.segment_count,
        })
        .collect())
}

#[tauri::command]
pub async fn search_transcriptions(
    state: tauri::State<'_, Arc<AppState>>,
    query: String,
) -> Result<Vec<HourSlotPayload>, String> {
    let slots = state.storage.search_hour_slots(&query)?;
    Ok(slots
        .into_iter()
        .map(|s| HourSlotPayload {
            id: s.id,
            hour_key: s.hour_key,
            text: s.text,
            start_time: s.start_time,
            last_updated: s.last_updated,
            device: s.device,
            segment_count: s.segment_count,
        })
        .collect())
}

#[tauri::command]
pub async fn get_status(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<StatusPayload, String> {
    let is_paused = *state.is_paused.lock();
    let is_recording = !is_paused;
    let device_name = crate::device_manager::get_current_device_name();
    let slots_count = state.storage.count().unwrap_or(0);

    let recording_start = state.recording_started_at.lock();
    let elapsed = recording_start
        .map(|t| (chrono::Utc::now() - t).num_seconds().max(0) as u64)
        .unwrap_or(0);

    let audio_level = crate::audio_engine::AUDIO_LEVEL.load(std::sync::atomic::Ordering::Relaxed);
    let is_transcribing = state.is_transcribing.load(std::sync::atomic::Ordering::Relaxed);

    Ok(StatusPayload {
        is_recording,
        is_paused,
        device_name,
        slots_count,
        segment_seconds_elapsed: elapsed,
        segment_duration_secs: crate::audio_engine::segment_duration_secs(),
        audio_level,
        is_transcribing,
    })
}

#[tauri::command]
pub async fn get_slots_by_date_range(
    state: tauri::State<'_, Arc<AppState>>,
    from_key: String,
    to_key: String,
) -> Result<Vec<HourSlotPayload>, String> {
    let slots = state.storage.get_slots_by_date_range(&from_key, &to_key)?;
    Ok(slots.into_iter().map(|s| HourSlotPayload {
        id: s.id, hour_key: s.hour_key, text: s.text, start_time: s.start_time,
        last_updated: s.last_updated, device: s.device, segment_count: s.segment_count,
    }).collect())
}

#[tauri::command]
pub async fn get_available_dates(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    state.storage.get_available_dates()
}

#[tauri::command]
pub async fn subscribe_audio_level(channel: Channel<AudioLevelEvent>) {
    loop {
        let level = crate::audio_engine::AUDIO_LEVEL.load(std::sync::atomic::Ordering::Relaxed);
        if channel.send(AudioLevelEvent { level }).is_err() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(33)).await;
    }
}

#[tauri::command]
pub async fn toggle_pause(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, String> {
    let mut paused = state.is_paused.lock();
    *paused = !*paused;
    Ok(*paused)
}

#[tauri::command]
pub async fn copy_to_clipboard(text: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        let mut child = Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("pbcopy: {e}"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(text.as_bytes()).map_err(|e| format!("{e}"))?;
        }
        child.wait().map_err(|e| format!("{e}"))?;
    }
    Ok(())
}
