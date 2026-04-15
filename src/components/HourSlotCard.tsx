import { createSignal, createEffect, onMount, onCleanup } from "solid-js";
import type { HourSlot } from "../App";

interface HourSlotCardProps {
  slot: HourSlot;
  onCopy: (text: string) => void;
  searchQuery?: string;
}

// Persist scroll positions across re-renders
const scrollPositions = new Map<number, number>();

function formatHourRange(hourKey: string): string {
  try {
    const parts = hourKey.split("T");
    const date = parts[0];
    const hour = parseInt(parts[1], 10);
    const d = new Date(date + "T" + hour.toString().padStart(2, "0") + ":00:00");
    const today = new Date();

    let dayLabel = "";
    if (d.toDateString() === today.toDateString()) {
      dayLabel = "Today";
    } else {
      const yesterday = new Date(today);
      yesterday.setDate(yesterday.getDate() - 1);
      dayLabel = d.toDateString() === yesterday.toDateString()
        ? "Yesterday"
        : d.toLocaleDateString([], { month: "short", day: "numeric" });
    }

    const startHour = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    const endDate = new Date(d.getTime() + 3600000);
    const endHour = endDate.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

    return `${dayLabel} ${startHour} \u2013 ${endHour}`;
  } catch {
    return hourKey;
  }
}

function highlightText(text: string, query: string) {
  if (!query.trim()) return [text];
  const parts: (string | { text: string; highlight: boolean })[] = [];
  const lower = text.toLowerCase();
  const qLower = query.toLowerCase();
  let lastIdx = 0;
  let idx = lower.indexOf(qLower);
  while (idx !== -1) {
    if (idx > lastIdx) parts.push(text.slice(lastIdx, idx));
    parts.push({ text: text.slice(idx, idx + query.length), highlight: true });
    lastIdx = idx + query.length;
    idx = lower.indexOf(qLower, lastIdx);
  }
  if (lastIdx < text.length) parts.push(text.slice(lastIdx));
  return parts;
}

export default function HourSlotCard(props: HourSlotCardProps) {
  const [copied, setCopied] = createSignal(false);
  let textRef: HTMLDivElement | undefined;

  function handleCopy() {
    props.onCopy(props.slot.text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  // Restore scroll position on mount
  onMount(() => {
    if (textRef) {
      const saved = scrollPositions.get(props.slot.id);
      if (saved !== undefined) {
        textRef.scrollTop = saved;
      }
    }
  });

  // Save scroll position on every scroll
  function handleScroll() {
    if (textRef) {
      scrollPositions.set(props.slot.id, textRef.scrollTop);
    }
  }

  // Scroll to search highlight
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
          <span class="card-segments">{props.slot.segment_count} {props.slot.segment_count === 1 ? "segment" : "segments"}</span>
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
