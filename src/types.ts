export interface Segment {
  id: number;
  hour_key: string;
  segment_type: "transcription" | "screen";
  text: string;
  timestamp: number;
  device: string;
}

export interface UnifiedHourSlot {
  hour_key: string;
  segments: Segment[];
  earliest_timestamp: number;
  latest_timestamp: number;
  total_segment_count: number;
}

export interface AppStatus {
  is_recording: boolean;
  is_paused: boolean;
  device_name: string;
  slots_count: number;
  segment_seconds_elapsed: number;
  segment_duration_secs: number;
  audio_level: number;
  is_transcribing: boolean;
  audio_disk_error: boolean;
  is_screen_capture_enabled: boolean;
  is_analyzing_screen: boolean;
}

export interface AudioLevelEvent {
  level: number;
}
