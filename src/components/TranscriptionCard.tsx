import { createSignal, createEffect } from "solid-js";
import type { Transcription } from "../App";

interface TranscriptionCardProps {
  transcription: Transcription;
  onCopy: (text: string) => void;
  searchQuery?: string;
}

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch { return ""; }
}

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    const today = new Date();
    if (d.toDateString() === today.toDateString()) return "Today";
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
    return d.toLocaleDateString([], { month: "short", day: "numeric" });
  } catch { return ""; }
}

function highlightText(text: string, query: string): (string | { text: string; highlight: boolean })[] {
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

export default function TranscriptionCard(props: TranscriptionCardProps) {
  const [copied, setCopied] = createSignal(false);
  let textRef: HTMLDivElement | undefined;

  function handleCopy() {
    props.onCopy(props.transcription.text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  // Scroll to first highlight when search query changes
  createEffect(() => {
    const q = props.searchQuery;
    if (q && textRef) {
      const mark = textRef.querySelector(".search-highlight");
      if (mark) {
        mark.scrollIntoView({ behavior: "smooth", block: "nearest" });
      }
    }
  });

  const parts = () => highlightText(props.transcription.text, props.searchQuery || "");

  return (
    <div class="card" id={`card-${props.transcription.id}`}>
      <div class="card-header">
        <span class="card-time">
          {formatDate(props.transcription.start_time)}{" "}
          {formatTime(props.transcription.start_time)}
          {" \u2013 "}
          {formatTime(props.transcription.end_time)}
        </span>
        <button
          onClick={handleCopy}
          class={`card-copy-btn ${copied() ? "copied" : ""}`}
        >
          {copied() ? "Copied" : "Copy"}
        </button>
      </div>
      <div class="card-text-scroll" ref={textRef}>
        <p class="card-text">
          {parts().map((part) =>
            typeof part === "string" ? (
              part
            ) : (
              <mark class="search-highlight">{part.text}</mark>
            )
          )}
        </p>
      </div>
      <div class="card-device">{props.transcription.device}</div>
    </div>
  );
}
