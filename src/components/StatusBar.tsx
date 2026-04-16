import { Show, type JSX } from "solid-js";
import AudioLevelBars from "./AudioLevelBars";
import type { AppStatus } from "../types";
import { formatCountdown } from "../utils/format";

interface StatusBarProps {
  status: AppStatus;
  onTogglePause: () => void;
  secondsRemaining: number;
}

export default function StatusBar(props: StatusBarProps): JSX.Element {
  const stateLabel = () => {
    if (props.status.is_transcribing) return "Transcribing...";
    if (props.status.is_paused) return "Paused";
    if (props.status.is_recording) return "Recording";
    return "Idle";
  };

  const dotClass = () => {
    if (props.status.is_transcribing) return "status-dot transcribing";
    if (props.status.is_paused) return "status-dot paused";
    if (props.status.is_recording) return "status-dot recording";
    return "status-dot idle";
  };

  return (
    <div class="status-bar">
      <div class="status-left">
        <span class={dotClass()} />
        <span class="status-label">{stateLabel()}</span>
        <span class="status-divider" />
        <Show when={props.status.is_recording && !props.status.is_paused && !props.status.is_transcribing}>
          <span class="status-timer">{formatCountdown(props.secondsRemaining)}</span>
          <span class="status-divider" />
        </Show>
        <AudioLevelBars />
        <span class="status-divider" />
        <span class="status-device">{props.status.device_name}</span>
        <Show when={props.status.audio_disk_error}>
          <span class="status-divider" />
          <span
            class="status-disk-error"
            title="Failed to open a new audio segment file — captured samples are being dropped until the next rotation succeeds."
          >
            <span class="status-dot disk-error" />
            Disk error
          </span>
        </Show>
      </div>
      <button class="pause-btn" onClick={props.onTogglePause}>
        {props.status.is_paused ? "Resume" : "Pause"}
      </button>
    </div>
  );
}
