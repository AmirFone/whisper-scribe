import { createSignal, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import DragHandle from "./components/DragHandle";
import ModeToggle, { type ViewMode } from "./components/ModeToggle";
import SearchBar from "./components/SearchBar";
import FilterPanel, { type FilterRange } from "./components/FilterPanel";
import Timeline from "./components/Timeline";
import StatusBar from "./components/StatusBar";
import type { HourSlot, AppStatus } from "./types";
import { TRANSCRIPTION_UPDATED, SCREEN_CONTEXT_UPDATED } from "./events";
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
  is_screen_capture_enabled: true,
  is_analyzing_screen: false,
};

export default function App(): JSX.Element {
  const [slots, setSlots] = createSignal<HourSlot[]>([]);
  const [status, setStatus] = createSignal<AppStatus>(INITIAL_STATUS);
  const [searchQuery, setSearchQuery] = createSignal("");
  const [localElapsed, setLocalElapsed] = createSignal(0);
  const [filterVisible, setFilterVisible] = createSignal(false);
  const [filterActive, setFilterActive] = createSignal(false);
  const [filterRange, setFilterRange] = createSignal<FilterRange | null>(null);
  const [viewMode, setViewMode] = createSignal<ViewMode>("transcription");

  let fetchGen = 0;

  async function fetchIntoSlots(
    fetchFn: () => Promise<HourSlot[] | null>,
  ): Promise<void> {
    const myGen = ++fetchGen;
    const results = await fetchFn();
    if (myGen !== fetchGen) return;
    if (results) setSlots(results);
  }

  function timelineCmd(): string {
    return viewMode() === "screen" ? "get_screen_timeline" : "get_timeline";
  }
  function searchCmd(): string {
    return viewMode() === "screen" ? "search_screen_context" : "search_transcriptions";
  }
  function dateRangeCmd(): string {
    return viewMode() === "screen" ? "get_screen_slots_by_date_range" : "get_slots_by_date_range";
  }
  function availableDatesCmd(): string {
    return viewMode() === "screen" ? "get_screen_available_dates" : "get_available_dates";
  }

  async function loadTimeline(): Promise<void> {
    if (filterActive()) return;
    if (searchQuery().trim()) return;
    await fetchIntoSlots(() => tryInvoke<HourSlot[]>(timelineCmd(), { limit: 50, offset: 0 }));
  }

  async function refreshActiveFilter(): Promise<void> {
    const range = filterRange();
    if (!range) return;
    await fetchIntoSlots(() => tryInvoke<HourSlot[]>(dateRangeCmd(), range));
  }

  async function searchSlots(query: string): Promise<void> {
    setFilterActive(false);
    const trimmed = query.trim();
    await fetchIntoSlots(() =>
      trimmed
        ? tryInvoke<HourSlot[]>(searchCmd(), { query: trimmed })
        : tryInvoke<HourSlot[]>(timelineCmd(), { limit: 50, offset: 0 }),
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

  function handleModeChange(mode: ViewMode): void {
    fetchGen++;
    setViewMode(mode);
    setFilterActive(false);
    setFilterRange(null);
    setSearchQuery("");
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
      if (viewMode() !== "transcription") return;
      if (filterActive()) void refreshActiveFilter();
      else void loadTimeline();
      setLocalElapsed(0);
      void loadStatus();
    });
    onCleanup(() => { void unlisten.then((fn) => fn()); });
  });

  createEffect(() => {
    const unlisten = listen(SCREEN_CONTEXT_UPDATED, () => {
      if (viewMode() !== "screen") return;
      if (filterActive()) void refreshActiveFilter();
      else void loadTimeline();
      void loadStatus();
    });
    onCleanup(() => { void unlisten.then((fn) => fn()); });
  });

  // Reload timeline when viewMode changes
  createEffect(() => {
    viewMode(); // track
    void loadTimeline();
  });

  // Search debounce — single source of truth for what runs on input.
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
      <ModeToggle mode={viewMode()} onModeChange={handleModeChange} />
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
        availableDatesCmd={availableDatesCmd()}
        dateRangeCmd={dateRangeCmd()}
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
