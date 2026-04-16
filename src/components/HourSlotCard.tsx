import { createSignal, createEffect, onCleanup, onMount, type JSX } from "solid-js";
import type { HourSlot } from "../types";
import { formatHourRange } from "../utils/format";
import { highlightText } from "../utils/highlight";

interface HourSlotCardProps {
  slot: HourSlot;
  onCopy: (text: string) => void;
  searchQuery?: string;
}

// Persist scroll positions across re-renders so a re-fetch doesn't jump the user
// back to the top of every visible card.
const scrollPositions = new Map<number, number>();

const COPIED_BADGE_MS = 1500;

export default function HourSlotCard(props: HourSlotCardProps): JSX.Element {
  const [copied, setCopied] = createSignal(false);
  let textRef: HTMLDivElement | undefined;
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  function handleCopy(): void {
    props.onCopy(props.slot.text);
    setCopied(true);
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => setCopied(false), COPIED_BADGE_MS);
  }

  onCleanup(() => {
    if (copyTimer) clearTimeout(copyTimer);
  });

  onMount(() => {
    if (textRef) {
      const saved = scrollPositions.get(props.slot.id);
      if (saved !== undefined) textRef.scrollTop = saved;
    }
  });

  function handleScroll() {
    if (textRef) scrollPositions.set(props.slot.id, textRef.scrollTop);
  }

  createEffect(() => {
    if (props.searchQuery && textRef) {
      const mark = textRef.querySelector(".search-highlight");
      if (mark) mark.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  });

  const parts = () => highlightText(props.slot.text, props.searchQuery || "");

  return (
    <div class="card" id={`slot-${props.slot.id}`}>
      <div class="card-header">
        <span class="card-time">{formatHourRange(props.slot.hour_key)}</span>
        <div class="card-header-right">
          <span class="card-segments">
            {props.slot.segment_count} {props.slot.segment_count === 1 ? "segment" : "segments"}
          </span>
          <button onClick={handleCopy} class={`card-copy-btn ${copied() ? "copied" : ""}`}>
            {copied() ? "Copied" : "Copy"}
          </button>
        </div>
      </div>
      <div class="card-text-scroll" ref={textRef} onScroll={handleScroll}>
        <p class="card-text">
          {parts().map((part) =>
            typeof part === "string" ? part : <mark class="search-highlight">{part.text}</mark>
          )}
        </p>
      </div>
      <div class="card-device">{props.slot.device}</div>
    </div>
  );
}
