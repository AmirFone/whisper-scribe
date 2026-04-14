import { createSignal, createEffect, Show, For } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { HourSlot } from "../App";

interface FilterPanelProps {
  visible: boolean;
  onClose: () => void;
  onApply: (slots: HourSlot[]) => void;
  onCopyAll: (text: string) => void;
}

function formatDateLabel(dateStr: string): string {
  try {
    const d = new Date(dateStr + "T00:00:00");
    const today = new Date();
    if (d.toDateString() === today.toDateString()) return "Today";
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
    return d.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
  } catch {
    return dateStr;
  }
}

export default function FilterPanel(props: FilterPanelProps) {
  const [dates, setDates] = createSignal<string[]>([]);
  const [selectedDate, setSelectedDate] = createSignal<string>("");
  const [fromHour, setFromHour] = createSignal(0);
  const [toHour, setToHour] = createSignal(23);
  const [filteredSlots, setFilteredSlots] = createSignal<HourSlot[]>([]);
  const [totalWords, setTotalWords] = createSignal(0);

  createEffect(async () => {
    if (props.visible) {
      try {
        const d = await invoke<string[]>("get_available_dates");
        setDates(d);
        if (d.length > 0 && !selectedDate()) setSelectedDate(d[0]);
      } catch (_) {}
    }
  });

  createEffect(async () => {
    const date = selectedDate();
    if (!date) return;

    const fromKey = `${date}T${fromHour().toString().padStart(2, "0")}`;
    const toKey = `${date}T${toHour().toString().padStart(2, "0")}`;

    try {
      const slots = await invoke<HourSlot[]>("get_slots_by_date_range", {
        fromKey,
        toKey,
      });
      setFilteredSlots(slots);
      const words = slots.reduce((sum, s) => sum + s.text.split(/\s+/).length, 0);
      setTotalWords(words);
    } catch (_) {}
  });

  function handleApply() {
    props.onApply(filteredSlots());
  }

  function handleCopyAll() {
    const allText = filteredSlots()
      .map((s) => {
        const hour = parseInt(s.hour_key.split("T")[1], 10);
        const label = `${hour}:00 - ${hour + 1}:00`;
        return `[${label}]\n${s.text}`;
      })
      .join("\n\n");
    props.onCopyAll(allText);
  }

  const hours = Array.from({ length: 24 }, (_, i) => i);

  return (
    <Show when={props.visible}>
      <div class="filter-overlay" onClick={props.onClose}>
        <div class="filter-panel" onClick={(e) => e.stopPropagation()}>
          <div class="filter-header">
            <span class="filter-title">Filter Transcripts</span>
            <button class="filter-close" onClick={props.onClose}>x</button>
          </div>

          <div class="filter-section">
            <label class="filter-label">Date</label>
            <div class="filter-date-chips">
              <For each={dates()}>
                {(d) => (
                  <button
                    class={`filter-chip ${selectedDate() === d ? "active" : ""}`}
                    onClick={() => setSelectedDate(d)}
                  >
                    {formatDateLabel(d)}
                  </button>
                )}
              </For>
            </div>
          </div>

          <div class="filter-section">
            <label class="filter-label">Time Range</label>
            <div class="filter-time-row">
              <select class="filter-select" value={fromHour()} onChange={(e) => setFromHour(parseInt(e.currentTarget.value))}>
                <For each={hours}>{(h) => <option value={h}>{h.toString().padStart(2, "0")}:00</option>}</For>
              </select>
              <span class="filter-to">to</span>
              <select class="filter-select" value={toHour()} onChange={(e) => setToHour(parseInt(e.currentTarget.value))}>
                <For each={hours}>{(h) => <option value={h}>{h.toString().padStart(2, "0")}:00</option>}</For>
              </select>
            </div>
          </div>

          <div class="filter-summary">
            {filteredSlots().length} hour slots | ~{totalWords()} words
          </div>

          <div class="filter-actions">
            <button class="filter-btn secondary" onClick={handleApply}>
              Show in Timeline
            </button>
            <button class="filter-btn primary" onClick={handleCopyAll}>
              Copy All Text
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
