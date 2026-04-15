import { For, Show, onMount } from "solid-js";
import HourSlotCard from "./HourSlotCard";
import AudioLevelBars from "./AudioLevelBars";
import type { HourSlot } from "../App";

let savedTimelineScroll = 0;

interface TimelineProps {
  slots: HourSlot[];
  onCopy: (text: string) => void;
  isRecording: boolean;
  isPaused: boolean;
  secondsRemaining: number;
  progress: number;
  deviceName: string;
  isTranscribing: boolean;
  searchQuery: string;
}

function formatCountdown(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function Timeline(props: TimelineProps) {
  const circumference = 2 * Math.PI * 30;
  let timelineRef: HTMLDivElement | undefined;

  onMount(() => {
    if (timelineRef) timelineRef.scrollTop = savedTimelineScroll;
  });

  function handleTimelineScroll() {
    if (timelineRef) savedTimelineScroll = timelineRef.scrollTop;
  }

  return (
    <div class="timeline" ref={timelineRef} onScroll={handleTimelineScroll}>
      <Show when={props.isTranscribing}>
        <div class="transcribing-banner">
          <div class="transcribing-spinner" />
          <span>Transcribing audio segment...</span>
        </div>
      </Show>

      <Show when={props.slots.length > 0}>
        <For each={props.slots}>
          {(slot) => <HourSlotCard slot={slot} onCopy={props.onCopy} searchQuery={props.searchQuery} />}
        </For>
      </Show>

      <Show when={props.slots.length === 0 && !props.isTranscribing}>
        <div class="empty-state">
          <Show when={props.isPaused}>
            <div class="empty-mic-icon paused-icon">
              <svg fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M15.75 5.25v13.5m-7.5-13.5v13.5" />
              </svg>
            </div>
            <span class="empty-title">Paused</span>
            <span class="empty-subtitle">Recording is paused. Hit Resume to continue.</span>
          </Show>

          <Show when={props.isRecording && !props.isPaused}>
            <AudioLevelBars />
            <div class="device-badge">
              <svg class="device-badge-icon" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z" />
              </svg>
              <span>{props.deviceName}</span>
            </div>
            <div class="countdown-container">
              <div class="countdown-ring">
                <svg viewBox="0 0 72 72">
                  <circle class="countdown-ring-bg" cx="36" cy="36" r="30" />
                  <circle class="countdown-ring-progress" cx="36" cy="36" r="30"
                    stroke-dasharray={circumference}
                    stroke-dashoffset={circumference * (1 - props.progress)} />
                </svg>
                <span class="countdown-time">{formatCountdown(props.secondsRemaining)}</span>
              </div>
              <span class="countdown-label">until first transcription</span>
            </div>
          </Show>

          <Show when={!props.isRecording && !props.isPaused}>
            <div class="empty-mic-icon">
              <svg fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z" />
              </svg>
            </div>
            <span class="empty-title">Starting up...</span>
            <span class="empty-subtitle">Initializing Whisper model and audio engine.</span>
          </Show>
        </div>
      </Show>
    </div>
  );
}
