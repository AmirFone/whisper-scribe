// Canonical payload types for every Rust↔TS IPC call in this app. Keep in
// lockstep with the Rust side:
//   - `HourSlot`     → `src-tauri/src/storage.rs` (`pub struct HourSlot`)
//   - `AppStatus`    → `src-tauri/src/commands.rs` (`StatusPayload`)
//   - `AudioLevelEvent` → `src-tauri/src/commands.rs` (`AudioLevelEvent`)
//
// Fields use snake_case to match Rust's default serde output. Tauri
// auto-converts camelCase params on the way in, but response payloads
// come through verbatim.

export interface HourSlot {
  id: number;
  /// UTC-bucketed "YYYY-MM-DDTHH" string. Bucketing is in UTC so dedup is
  /// tz-invariant across runs; display converts to local via
  /// `utils/format.ts::formatHourRange`.
  hour_key: string;
  text: string;
  /// Milliseconds since the Unix epoch — capture time of the first segment
  /// in this hour. Integer so no RFC3339 lex-compare contract is needed on
  /// the Rust side. Safe as a `number` (JS precision is lost around year
  /// 287396).
  start_time: number;
  /// Milliseconds since the Unix epoch — capture time of the most recent
  /// segment in this hour. Updated on every append; orphan dedup compares
  /// numerically against this.
  last_updated: number;
  device: string;
  segment_count: number;
}

export interface AppStatus {
  is_recording: boolean;
  is_paused: boolean;
  device_name: string;
  // i64 from Rust; always ≥0 (COUNT result).
  slots_count: number;
  segment_seconds_elapsed: number;
  segment_duration_secs: number;
  // u32 from Rust; clamped 0–100.
  audio_level: number;
  is_transcribing: boolean;
  // True when the audio engine could not open a new WAV writer — captured
  // samples are being dropped until rotation recovers. UI warns in the
  // empty-state / status bar.
  audio_disk_error: boolean;
  is_screen_capture_enabled: boolean;
  is_analyzing_screen: boolean;
}

export interface AudioLevelEvent {
  level: number;
}
