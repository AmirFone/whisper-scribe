import { createSignal, createEffect, createMemo, onMount, onCleanup, Show, For, type JSX } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import type { UnifiedHourSlot, Segment } from "../types";
import { formatHourRange, formatSegmentTime } from "../utils/format";
import { highlightText } from "../utils/highlight";

interface ExpandedCardModalProps {
  slot: UnifiedHourSlot | null;
  onClose: () => void;
  onCopy: (text: string) => void;
  searchQuery?: string;
}

const COPIED_BADGE_MS = 1500;
const EXPANDED_WIDTH = 1100;
const EXPANDED_HEIGHT = 750;
const NORMAL_WIDTH = 380;
const NORMAL_HEIGHT = 560;

function formatCopyText(segments: Segment[]): string {
  return segments
    .map((seg) => {
      const time = formatSegmentTime(seg.timestamp);
      const tag = seg.segment_type === "screen" ? "Screen Context" : "Transcription";
      return `[${tag} ${time}] ${seg.text}`;
    })
    .join("\n\n");
}

export default function ExpandedCardModal(props: ExpandedCardModalProps): JSX.Element {
  const [copied, setCopied] = createSignal(false);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  createEffect(() => {
    const win = getCurrentWindow();
    if (props.slot) {
      void (async () => {
        await win.setAlwaysOnTop(false);
        await win.setSize(new LogicalSize(EXPANDED_WIDTH, EXPANDED_HEIGHT));
      })();
    } else {
      void (async () => {
        await win.setSize(new LogicalSize(NORMAL_WIDTH, NORMAL_HEIGHT));
        await win.setAlwaysOnTop(true);
      })();
    }
  });

  function handleCopy(): void {
    if (!props.slot) return;
    props.onCopy(formatCopyText(props.slot.segments));
    setCopied(true);
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => setCopied(false), COPIED_BADGE_MS);
  }

  function handleClose(): void {
    props.onClose();
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === "Escape") handleClose();
  }

  createEffect(() => {
    if (props.slot) {
      document.addEventListener("keydown", handleKeyDown);
    } else {
      document.removeEventListener("keydown", handleKeyDown);
    }
  });

  onCleanup(() => {
    if (copyTimer) clearTimeout(copyTimer);
    document.removeEventListener("keydown", handleKeyDown);
  });

  return (
    <Show when={props.slot}>
      {(slot) => (
        <div class="expanded-overlay">
          <div class="expanded-panel">
            <div class="expanded-header" data-tauri-drag-region>
              <span class="expanded-title" data-tauri-drag-region>{formatHourRange(slot().hour_key)}</span>
              <div class="expanded-header-right">
                <span class="card-segments">
                  {slot().total_segment_count} segments
                </span>
                <button
                  onClick={handleCopy}
                  class={`card-copy-btn ${copied() ? "copied" : ""}`}
                >
                  {copied() ? "\u2713 Copied" : "Copy All"}
                </button>
                <button class="filter-close" onClick={handleClose}>
                  x
                </button>
              </div>
            </div>
            <div class="expanded-body">
              <For each={slot().segments}>
                {(seg: Segment) => {
                  const isScreen = () => seg.segment_type === "screen";
                  const parts = createMemo(() => highlightText(seg.text, props.searchQuery || ""));
                  return (
                    <div class={`segment-item segment-expanded ${isScreen() ? "segment-type-screen" : "segment-type-transcription"}`}>
                      <div class="segment-header">
                        <span class={`segment-dot ${isScreen() ? "dot-screen" : "dot-transcription"}`} />
                        <span class="segment-timestamp">
                          {formatSegmentTime(seg.timestamp)}
                        </span>
                        <span class="segment-type-label">
                          {isScreen() ? "Screen" : "Audio"}
                        </span>
                        <span class="segment-device">{seg.device}</span>
                      </div>
                      <p class="card-text">
                        {parts().map((part) =>
                          typeof part === "string"
                            ? part
                            : <mark class="search-highlight">{part.text}</mark>
                        )}
                      </p>
                    </div>
                  );
                }}
              </For>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}
