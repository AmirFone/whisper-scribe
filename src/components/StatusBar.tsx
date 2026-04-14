import { Show } from "solid-js";
import AudioLevelBars from "./AudioLevelBars";
import type { AppStatus } from "../App";

interface StatusBarProps {
  status: AppStatus;
  onTogglePause: () => void;
  secondsRemaining: number;
}

function formatCountdown(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function StatusBar(props: StatusBarProps) {
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
      </div>
      <button class="pause-btn" onClick={props.onTogglePause}>
        {props.status.is_paused ? "Resume" : "Pause"}
      </button>
    </div>
  );
}
