import { createSignal, createEffect, onCleanup } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import DragHandle from "./components/DragHandle";
import SearchBar from "./components/SearchBar";
import FilterPanel from "./components/FilterPanel";
import Timeline from "./components/Timeline";
import StatusBar from "./components/StatusBar";

export interface HourSlot {
  id: number;
  hour_key: string;
  text: string;
  start_time: string;
  last_updated: string;
  device: string;
  segment_count: number;
}

export interface AppStatus {
  is_recording: boolean;
  is_paused: boolean;
  device_name: string;
  slots_count: number;
  segment_seconds_elapsed: number;
  segment_duration_secs: number;
  audio_level: number;
  is_transcribing: boolean;
}

export default function App() {
  const [slots, setSlots] = createSignal<HourSlot[]>([]);
  const [status, setStatus] = createSignal<AppStatus>({
    is_recording: false, is_paused: false, device_name: "None", slots_count: 0,
    segment_seconds_elapsed: 0, segment_duration_secs: 120, audio_level: 0, is_transcribing: false,
  });
  const [searchQuery, setSearchQuery] = createSignal("");
  const [localElapsed, setLocalElapsed] = createSignal(0);
  const [filterVisible, setFilterVisible] = createSignal(false);
  const [filterActive, setFilterActive] = createSignal(false);
  const [copiedAll, setCopiedAll] = createSignal(false);

  async function loadTimeline() {
    if (filterActive()) return;
    if (searchQuery().trim()) return; // Don't override search results
    try {
      const results = await invoke<HourSlot[]>("get_timeline", { limit: 50, offset: 0 });
      setSlots(results);
    } catch (_) {}
  }

  async function searchSlots(query: string) {
    setFilterActive(false);
    if (!query.trim()) {
      try {
        const results = await invoke<HourSlot[]>("get_timeline", { limit: 50, offset: 0 });
        setSlots(results);
      } catch (_) {}
      return;
    }
    try {
      const results = await invoke<HourSlot[]>("search_transcriptions", { query });
      setSlots(results);
    } catch (_) {}
  }

  async function loadStatus() {
    try {
      const s = await invoke<AppStatus>("get_status");
      setStatus(s);
      setLocalElapsed(s.segment_seconds_elapsed);
    } catch (_) {}
  }

  async function togglePause() {
    try { await invoke("toggle_pause"); loadStatus(); } catch (_) {}
  }

  async function copyText(text: string) {
    try { await navigator.clipboard.writeText(text); } catch { await invoke("copy_to_clipboard", { text }); }
  }

  function handleFilterApply(filtered: HourSlot[]) {
    setSlots(filtered);
    setFilterActive(true);
    setFilterVisible(false);
  }

  function handleCopyAll(text: string) {
    copyText(text);
    setCopiedAll(true);
    setTimeout(() => setCopiedAll(false), 2000);
    setFilterVisible(false);
  }

  function handleFilterToggle() {
    if (filterVisible()) {
      setFilterVisible(false);
    } else {
      setFilterVisible(true);
    }
  }

  function clearFilter() {
    setFilterActive(false);
    loadTimeline();
  }

  createEffect(() => {
    loadTimeline();
    loadStatus();
    const statusInterval = setInterval(loadStatus, 1000);
    onCleanup(() => clearInterval(statusInterval));
  });

  createEffect(() => {
    const tick = setInterval(() => {
      setLocalElapsed((prev) => {
        const max = status().segment_duration_secs;
        return prev < max ? prev + 1 : prev;
      });
    }, 1000);
    onCleanup(() => clearInterval(tick));
  });

  createEffect(() => {
    const unlisten = listen("transcription-updated", () => {
      loadTimeline();
      setLocalElapsed(0);
      loadStatus();
    });
    onCleanup(() => { unlisten.then((fn) => fn()); });
  });

  createEffect(() => {
    const reload = setInterval(loadTimeline, 5000);
    onCleanup(() => clearInterval(reload));
  });

  createEffect(() => {
    const q = searchQuery();
    const debounce = setTimeout(() => searchSlots(q), 200);
    onCleanup(() => clearTimeout(debounce));
  });

  const secondsRemaining = () => Math.max(0, status().segment_duration_secs - localElapsed());
  const progress = () => Math.min(1, localElapsed() / status().segment_duration_secs);

  return (
    <div class="app-container">
      <DragHandle />
      <SearchBar
        query={searchQuery()}
        onInput={(v) => { setSearchQuery(v); if (!v) clearFilter(); }}
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
