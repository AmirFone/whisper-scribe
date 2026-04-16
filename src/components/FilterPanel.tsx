import { createSignal, createEffect, Show, For, type JSX } from "solid-js";
import type { HourSlot } from "../types";
import { tryInvoke } from "../utils/invoke";
import { formatRelativeDateWithWeekday } from "../utils/format";

export interface FilterRange extends Record<string, unknown> {
  fromKey: string;
  toKey: string;
}

interface FilterPanelProps {
  visible: boolean;
  onClose: () => void;
  onApply: (slots: HourSlot[], range: FilterRange) => void;
  onCopyAll: (text: string) => void;
}

export default function FilterPanel(props: FilterPanelProps): JSX.Element {
  const [dates, setDates] = createSignal<string[]>([]);
  const [selectedDate, setSelectedDate] = createSignal<string>("");
  const [fromHour, setFromHour] = createSignal(0);
  const [toHour, setToHour] = createSignal(23);
  const [filteredSlots, setFilteredSlots] = createSignal<HourSlot[]>([]);
  const [totalWords, setTotalWords] = createSignal(0);

  // Generation counter for the panel's own IPC calls. Prevents a slow date-
  // range fetch from overwriting the results of a faster subsequent one when
  // the user rapidly changes date or hour selectors.
  let panelFetchGen = 0;

  // Read `props.visible` synchronously so Solid's effect tracking sees it
  // (dependency reads after `await` are NOT tracked — that was a latent bug).
  createEffect(() => {
    if (!props.visible) return;
    void (async () => {
      const d = await tryInvoke<string[]>("get_available_dates");
      if (!d) return;
      setDates(d);
      if (d.length > 0 && !selectedDate()) setSelectedDate(d[0]);
    })();
  });

  function rangeKeys(): FilterRange | null {
    const date = selectedDate();
    if (!date) return null;
    return {
      fromKey: `${date}T${fromHour().toString().padStart(2, "0")}`,
      toKey: `${date}T${toHour().toString().padStart(2, "0")}`,
    };
  }

  createEffect(() => {
    // Read signals synchronously so Solid's tracking sees them (async reads
    // after `await` are NOT tracked).
    const range = rangeKeys();
    if (!range) return;

    const myGen = ++panelFetchGen;
    void (async () => {
      const slots = await tryInvoke<HourSlot[]>("get_slots_by_date_range", range);
      if (myGen !== panelFetchGen) return;
      if (!slots) return;
      setFilteredSlots(slots);
      const words = slots.reduce((sum, s) => sum + s.text.split(/\s+/).length, 0);
      setTotalWords(words);
    })();
  });

  function handleApply() {
    const range = rangeKeys();
    if (!range) return;
    props.onApply(filteredSlots(), range);
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
                    {formatRelativeDateWithWeekday(d)}
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
