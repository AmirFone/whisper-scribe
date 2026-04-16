import { createSignal, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import DragHandle from "./components/DragHandle";
import SearchBar from "./components/SearchBar";
import FilterPanel, { type FilterRange } from "./components/FilterPanel";
import Timeline from "./components/Timeline";
import StatusBar from "./components/StatusBar";
import type { HourSlot, AppStatus } from "./types";
import { TRANSCRIPTION_UPDATED } from "./events";
import { tryInvoke } from "./utils/invoke";

const STATUS_POLL_MS = 1000;
const SEARCH_DEBOUNCE_MS = 200;

const INITIAL_STATUS: AppStatus = {
  is_recording: false,
  is_paused: false,
  device_name: "None",
  slots_count: 0,
  segment_seconds_elapsed: 0,
  segment_duration_secs: 120,
  audio_level: 0,
  is_transcribing: false,
  audio_disk_error: false,
};

export default function App(): JSX.Element {
  const [slots, setSlots] = createSignal<HourSlot[]>([]);
  const [status, setStatus] = createSignal<AppStatus>(INITIAL_STATUS);
  const [searchQuery, setSearchQuery] = createSignal("");
  const [localElapsed, setLocalElapsed] = createSignal(0);
  const [filterVisible, setFilterVisible] = createSignal(false);
  const [filterActive, setFilterActive] = createSignal(false);
  // The date/hour range that backs the active filter view. We keep it so the
  // `TRANSCRIPTION_UPDATED` event can re-run the query — without this, new
  // segments arriving while a filter was active were silently dropped from
  // the visible view.
  const [filterRange, setFilterRange] = createSignal<FilterRange | null>(null);

  // Monotonic generation counter for all IPC calls that resolve into
  // `setSlots`. Every fetch captures a generation before the await; any
  // user mutation (filter apply, filter clear, new search) bumps the
  // counter, which causes every older in-flight response to be discarded
  // instead of overwriting whatever is currently on screen. One abstraction
  // covers the three otherwise-separate race windows: refreshActiveFilter
  // vs clear, debounced search vs TRANSCRIPTION_UPDATED, and filter apply
  // vs background reload.
  let fetchGen = 0;

  async function fetchIntoSlots(
    fetchFn: () => Promise<HourSlot[] | null>,
  ): Promise<void> {
    const myGen = ++fetchGen;
    const results = await fetchFn();
    if (myGen !== fetchGen) return;
    if (results) setSlots(results);
  }

  async function loadTimeline(): Promise<void> {
    if (filterActive()) return;
    if (searchQuery().trim()) return; // Don't override search results
    await fetchIntoSlots(() => tryInvoke<HourSlot[]>("get_timeline", { limit: 50, offset: 0 }));
  }

  async function refreshActiveFilter(): Promise<void> {
    const range = filterRange();
    if (!range) return;
    await fetchIntoSlots(() => tryInvoke<HourSlot[]>("get_slots_by_date_range", range));
  }

  async function searchSlots(query: string): Promise<void> {
    setFilterActive(false);
    const trimmed = query.trim();
    await fetchIntoSlots(() =>
      trimmed
        ? tryInvoke<HourSlot[]>("search_transcriptions", { query: trimmed })
        : tryInvoke<HourSlot[]>("get_timeline", { limit: 50, offset: 0 }),
    );
  }

  async function loadStatus(): Promise<void> {
    const s = await tryInvoke<AppStatus>("get_status");
    if (!s) return;
    setStatus(s);
    setLocalElapsed(s.segment_seconds_elapsed);
  }

  async function togglePause(): Promise<void> {
    await tryInvoke("toggle_pause");
    await loadStatus();
  }

  async function copyText(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch (err) {
      console.warn("[clipboard]", err);
    }
  }

  function handleFilterApply(filtered: HourSlot[], range: FilterRange): void {
    fetchGen++;
    setSlots(filtered);
    setFilterRange(range);
    setFilterActive(true);
    setFilterVisible(false);
  }

  function handleCopyAll(text: string): void {
    void copyText(text);
    setFilterVisible(false);
  }

  function handleFilterToggle(): void {
    setFilterVisible((v) => !v);
  }

  function clearFilter(): void {
    fetchGen++;
    setFilterActive(false);
    setFilterRange(null);
    void loadTimeline();
  }

  // Status polling — pauses while the window is hidden to save CPU/IPC on
  // a background app. `document.hidden` flips on Cmd+H / minimize; a single
  // `visibilitychange` listener coordinates both intervals.
  createEffect(() => {
    void loadTimeline();
    void loadStatus();

    let statusTimer: ReturnType<typeof setInterval> | null = null;
    let elapsedTimer: ReturnType<typeof setInterval> | null = null;

    const start = (): void => {
      if (!statusTimer) statusTimer = setInterval(() => { void loadStatus(); }, STATUS_POLL_MS);
      if (!elapsedTimer) {
        elapsedTimer = setInterval(() => {
          setLocalElapsed((prev) => {
            const max = status().segment_duration_secs;
            return prev < max ? prev + 1 : prev;
          });
        }, STATUS_POLL_MS);
      }
    };
    const stop = (): void => {
      if (statusTimer) { clearInterval(statusTimer); statusTimer = null; }
      if (elapsedTimer) { clearInterval(elapsedTimer); elapsedTimer = null; }
    };

    const onVisibility = (): void => {
      if (document.hidden) stop();
      else { void loadStatus(); start(); }
    };

    start();
    document.addEventListener("visibilitychange", onVisibility);

    onCleanup(() => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    });
  });

  // Transcription-updated push event is the source of truth for "new
  // segment appended"; polling was previously layered on top and fired a
  // redundant round-trip every 5 s. Removed — the push event covers it.
  //
  // If a filter is active, `loadTimeline` early-returns so as not to clobber
  // the filtered view — we re-run the filter query instead so new segments
  // that fall inside the active range become visible.
  createEffect(() => {
    const unlisten = listen(TRANSCRIPTION_UPDATED, () => {
      if (filterActive()) void refreshActiveFilter();
      else void loadTimeline();
      setLocalElapsed(0);
      void loadStatus();
    });
    onCleanup(() => { void unlisten.then((fn) => fn()); });
  });

  // Search debounce — single source of truth for what runs on input.
  // Empty input re-fetches the timeline via the same path (no parallel
  // "clear shortcut" that could race the debounce).
  createEffect(() => {
    const q = searchQuery();
    const debounce = setTimeout(() => { void searchSlots(q); }, SEARCH_DEBOUNCE_MS);
    onCleanup(() => clearTimeout(debounce));
  });

  const secondsRemaining = createMemo(() =>
    Math.max(0, status().segment_duration_secs - localElapsed()),
  );
  const progress = createMemo(() => {
    const max = status().segment_duration_secs;
    if (max <= 0) return 0;
    // Round to 3 decimals so tiny FP jitter doesn't churn the SVG attribute.
    return Math.round(Math.min(1, localElapsed() / max) * 1000) / 1000;
  });

  return (
    <div class="app-container">
      <DragHandle />
      <SearchBar
        query={searchQuery()}
        onInput={setSearchQuery}
        filterActive={filterActive()}
        onFilterToggle={handleFilterToggle}
      />
      {filterActive() && (
        <div class="filter-active-bar">
          <span>Filtered view</span>
          <button class="filter-clear-btn" onClick={clearFilter}>Clear</button>
        </div>
      )}
      <FilterPanel
        visible={filterVisible()}
        onClose={() => setFilterVisible(false)}
        onApply={handleFilterApply}
        onCopyAll={handleCopyAll}
      />
      <Timeline
        slots={slots()}
        onCopy={copyText}
        isRecording={status().is_recording}
        isPaused={status().is_paused}
        secondsRemaining={secondsRemaining()}
        progress={progress()}
        deviceName={status().device_name}
        isTranscribing={status().is_transcribing}
        searchQuery={searchQuery()}
      />
      <StatusBar status={status()} onTogglePause={togglePause} secondsRemaining={secondsRemaining()} />
    </div>
  );
}
