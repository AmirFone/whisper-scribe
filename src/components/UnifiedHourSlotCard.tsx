import { createSignal, onCleanup, For, type JSX } from "solid-js";
import type { UnifiedHourSlot, Segment } from "../types";
import { formatHourRange, formatSegmentTime } from "../utils/format";
import { highlightText } from "../utils/highlight";

interface UnifiedHourSlotCardProps {
  slot: UnifiedHourSlot;
  onCopy: (text: string) => void;
  onExpand: (slot: UnifiedHourSlot) => void;
  searchQuery?: string;
}

const COPIED_BADGE_MS = 1500;

function formatCopyText(segments: Segment[]): string {
  return segments
    .map((seg) => {
      const time = formatSegmentTime(seg.timestamp);
      const tag = seg.segment_type === "screen" ? "Screen Context" : "Transcription";
      return `[${tag} ${time}] ${seg.text}`;
    })
    .join("\n\n");
}

export default function UnifiedHourSlotCard(props: UnifiedHourSlotCardProps): JSX.Element {
  const [copied, setCopied] = createSignal(false);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  function handleCopy(e: MouseEvent): void {
    e.stopPropagation();
    props.onCopy(formatCopyText(props.slot.segments));
    setCopied(true);
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => setCopied(false), COPIED_BADGE_MS);
  }

  function handleDblClick(): void {
    props.onExpand(props.slot);
  }

  onCleanup(() => {
    if (copyTimer) clearTimeout(copyTimer);
  });

  return (
    <div class="card" onDblClick={handleDblClick}>
      <div class="card-header">
        <span class="card-time">{formatHourRange(props.slot.hour_key)}</span>
        <div class="card-header-right">
          <span class="card-segments">
            {props.slot.total_segment_count}{" "}
            {props.slot.total_segment_count === 1 ? "segment" : "segments"}
          </span>
          <button onClick={handleCopy} class={`card-copy-btn ${copied() ? "copied" : ""}`}>
            {copied() ? "Copied" : "Copy"}
          </button>
        </div>
      </div>
      <div class="card-text-scroll">
        <For each={props.slot.segments}>
          {(seg: Segment) => (
            <SegmentItem segment={seg} searchQuery={props.searchQuery} />
          )}
        </For>
      </div>
    </div>
  );
}

function SegmentItem(props: { segment: Segment; searchQuery?: string }): JSX.Element {
  const isScreen = () => props.segment.segment_type === "screen";
  const parts = () => highlightText(props.segment.text, props.searchQuery || "");

  return (
    <div class={`segment-item ${isScreen() ? "segment-type-screen" : "segment-type-transcription"}`}>
      <div class="segment-header">
        <span class={`segment-dot ${isScreen() ? "dot-screen" : "dot-transcription"}`} />
        <span class="segment-timestamp">{formatSegmentTime(props.segment.timestamp)}</span>
        <span class="segment-type-label">{isScreen() ? "Screen" : "Audio"}</span>
      </div>
      <p class="card-text">
        {parts().map((part) =>
          typeof part === "string" ? part : <mark class="search-highlight">{part.text}</mark>
        )}
      </p>
    </div>
  );
}
