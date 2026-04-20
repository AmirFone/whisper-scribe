import { createSignal, createEffect, createMemo, onCleanup, For, type JSX } from "solid-js";
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

  let scrollRef: HTMLDivElement | undefined;

  createEffect(() => {
    const q = props.searchQuery;
    if (!q?.trim() || !scrollRef) return;
    requestAnimationFrame(() => {
      const container = scrollRef!;
      const mark = container.querySelector<HTMLElement>(".search-highlight");
      if (!mark) return;
      const markTop = mark.offsetTop - container.offsetTop;
      const target = Math.max(0, markTop - container.clientHeight / 3);
      container.scrollTo({ top: target, behavior: "smooth" });
    });
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
            {copied() ? "\u2713 Copied" : "Copy"}
          </button>
        </div>
      </div>
      <div class="card-text-scroll" ref={scrollRef}>
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
  // Memoize the highlight split so we don't re-scan the text on every tick of
  // the status poll or on unrelated parent re-renders. Only recomputes when
  // the segment text or the query itself changes.
  const parts = createMemo(() => highlightText(props.segment.text, props.searchQuery || ""));

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
