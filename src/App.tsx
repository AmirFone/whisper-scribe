import { createSignal, createEffect, createMemo, createDeferred, batch, onCleanup, type JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { leadingAndTrailing, debounce } from "@solid-primitives/scheduled";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import DragHandle from "./components/DragHandle";
import SearchBar from "./components/SearchBar";
import FilterPanel, { type FilterRange } from "./components/FilterPanel";
import Timeline from "./components/Timeline";
import StatusBar from "./components/StatusBar";
import ExpandedCardModal from "./components/ExpandedCardModal";
import type { UnifiedHourSlot, AppStatus } from "./types";
import { TIMELINE_UPDATED } from "./events";
import { tryInvoke } from "./utils/invoke";

const STATUS_POLL_MS = 1000;
// Leading-edge: first keystroke fires immediately (feels instant). Trailing
// edge: one final search after the user stops. 150ms is Algolia's recommended
// window for local backends.
const SEARCH_DEBOUNCE_MS = 150;

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
  // Store + reconcile keeps DOM nodes stable across searches: cards that share
  // a hour_key between query results keep their identity, so only changed
  // fields re-render. Prevents the full teardown/rebuild flash on every
  // keystroke.
  const [slots, setSlots] = createStore<UnifiedHourSlot[]>([]);
  const [status, setStatus] = createSignal<AppStatus>(INITIAL_STATUS);
  const [searchQuery, setSearchQuery] = createSignal("");
  const [localElapsed, setLocalElapsed] = createSignal(0);
  const [filterVisible, setFilterVisible] = createSignal(false);
  const [filterActive, setFilterActive] = createSignal(false);
  const [filterRange, setFilterRange] = createSignal<FilterRange | null>(null);
  const [expandedSlot, setExpandedSlot] = createSignal<UnifiedHourSlot | null>(null);

  // Deferred version of the query signal. `createDeferred` schedules its
  // propagation in a new task (via MessageChannel), letting the browser paint
  // the input field's cursor update before we kick off the reconcile. This is
  // the core fix for typing/deleting feeling laggy — the input is no longer
  // waiting on our store update to flush.
  const deferredQuery = createDeferred(searchQuery, { timeoutMs: 500 });
  const [isSearching, setIsSearching] = createSignal(false);

  let fetchGen = 0;

  async function runFetch(
    fetchFn: () => Promise<UnifiedHourSlot[] | null>,
  ): Promise<void> {
    const myGen = ++fetchGen;
    setIsSearching(true);
    try {
      const results = await fetchFn();
      // Stale-while-revalidate: if another fetch raced ahead, discard this
      // result silently. The user already sees the newer query's results
      // (or their previous stable results until that fetch settles).
      if (myGen !== fetchGen) return;
      if (results) {
        batch(() => {
          // reconcile with merge keeps DOM nodes stable across searches for
          // cards that still appear in both result sets.
          setSlots(reconcile(results, { key: "hour_key", merge: true }));
          setIsSearching(false);
        });
      } else {
        setIsSearching(false);
      }
    } catch {
      if (myGen === fetchGen) setIsSearching(false);
    }
  }

  async function loadTimeline(): Promise<void> {
    if (filterActive()) return;
    if (searchQuery().trim()) return;
    await runFetch(() => tryInvoke<UnifiedHourSlot[]>("get_timeline", { limit: 50, offset: 0 }));
  }

  async function refreshActiveFilter(): Promise<void> {
    const range = filterRange();
    if (!range) return;
    await runFetch(() => tryInvoke<UnifiedHourSlot[]>("get_slots_by_date_range", range));
  }

  function runSearchForQuery(query: string): void {
    setFilterActive(false);
    const trimmed = query.trim();
    void runFetch(() =>
      trimmed
        ? tryInvoke<UnifiedHourSlot[]>("search_transcriptions", { query: trimmed })
        : tryInvoke<UnifiedHourSlot[]>("get_timeline", { limit: 50, offset: 0 }),
    );
  }

  // Leading-edge debounce: fires instantly on the first keystroke in a burst,
  // then once more on the trailing edge after the burst ends. On the Rust
  // side, FTS5 trigram queries are sub-20ms, so firing the first keystroke
  // immediately feels native without hammering the backend.
  const triggerSearch = leadingAndTrailing(
    debounce,
    runSearchForQuery,
    SEARCH_DEBOUNCE_MS,
  );

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

  function handleFilterApply(filtered: UnifiedHourSlot[], range: FilterRange): void {
    fetchGen++;
    batch(() => {
      setSlots(reconcile(filtered, { key: "hour_key", merge: true }));
      setFilterRange(range);
      setFilterActive(true);
      setFilterVisible(false);
    });
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

  // HUD summon/dismiss animations
  createEffect(() => {
    let dismissCleanup: (() => void) | null = null;

    const onShown = (): void => {
      if (dismissCleanup) {
        dismissCleanup();
        dismissCleanup = null;
      }
      document.body.classList.remove("hud-dismissing");
      document.body.classList.add("hud-entering");
      setTimeout(() => document.body.classList.remove("hud-entering"), 220);
    };

    const onRequestHide = (): void => {
      if (dismissCleanup) return;
      document.body.classList.remove("hud-entering");
      document.body.classList.add("hud-dismissing");

      const handleAnimEnd = (e: AnimationEvent): void => {
        if (e.animationName !== "hud-dismiss") return;
        document.body.removeEventListener("animationend", handleAnimEnd);
        dismissCleanup = null;
        document.body.classList.remove("hud-dismissing");
        void getCurrentWindow().hide();
      };

      dismissCleanup = () => {
        document.body.removeEventListener("animationend", handleAnimEnd);
      };
      document.body.addEventListener("animationend", handleAnimEnd);
    };

    const unlistenShown = listen("window-shown", onShown);
    const unlistenHide = listen("request-hide", onRequestHide);

    onCleanup(() => {
      if (dismissCleanup) dismissCleanup();
      void unlistenShown.then((fn) => fn());
      void unlistenHide.then((fn) => fn());
    });
  });

  // Status polling
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

  // Unified timeline event — fired by both transcription and screen pipelines
  createEffect(() => {
    const unlisten = listen(TIMELINE_UPDATED, () => {
      if (filterActive()) void refreshActiveFilter();
      else void loadTimeline();
      setLocalElapsed(0);
      void loadStatus();
    });
    onCleanup(() => { void unlisten.then((fn) => fn()); });
  });

  // Search dispatch: reads the deferred query so the effect runs in a later
  // task than the input's onInput handler. The raw `searchQuery` signal
  // drives the <input> value binding directly and updates synchronously —
  // typing and deleting feel instant regardless of how long reconcile takes.
  createEffect(() => {
    const q = deferredQuery();
    triggerSearch(q);
  });

  const secondsRemaining = createMemo(() =>
    Math.max(0, status().segment_duration_secs - localElapsed()),
  );
  const progress = createMemo(() => {
    const max = status().segment_duration_secs;
    if (max <= 0) return 0;
    return Math.round(Math.min(1, localElapsed() / max) * 1000) / 1000;
  });

  return (
    <>
    <div class={`app-container ${expandedSlot() ? "app-hidden" : ""}`}>
      <DragHandle />
      <SearchBar
        query={searchQuery()}
        onInput={setSearchQuery}
        filterActive={filterActive()}
        onFilterToggle={handleFilterToggle}
        isSearching={isSearching()}
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
        slots={slots}
        onCopy={copyText}
        onExpand={setExpandedSlot}
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
    <ExpandedCardModal
      slot={expandedSlot()}
      onClose={() => setExpandedSlot(null)}
      onCopy={copyText}
      searchQuery={searchQuery()}
    />
    </>
  );
}
