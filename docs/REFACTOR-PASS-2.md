# Refactor Pass 2 — Summary (2026-04-16)

This summarizes what was changed against the Pass-2 audit at `docs/AUDIT-2026-04-16-PASS2.md`. Use it as context when reviewing the codebase for the next refactor pass.

## Verification status (post-pass-2)

- `cargo test`: **174 passed, 0 failed, 1 ignored** (the cpal live test). Up from 163 at end of pass-1.
- `cargo check`: clean — only the same pre-existing `dead_code` warning in `transcriber.rs::TranscriptionResult` (`end_time`, `confidence` unused).
- `npm run build`: clean Vite production build, ~43.79 kB JS gzipped (no regression vs. pass-1's ~43 kB).
- `npx tsc --noEmit`: same two **pre-existing** errors as pass-1 — `Timeline.tsx` SVG `stroke-dasharray` typing, and `index.tsx` missing CSS module type. Not introduced here.

## Audit items applied

| ID | Title | Result |
|----|-------|--------|
| **A1 + C1** | Wire `PauseReason` into audio callback + pipeline | Done — `AppState::pause_flag` mirror atomic; audio callback early-returns on paused; pipeline `recv()` loop skips while paused. Two unit tests added. |
| **A5** | Extract `AppState::toggle_pause` | Done — `tray.rs` and `commands.rs::toggle_pause` both call `state.toggle_pause()`; no more copy-paste. |
| **A4 + A3** | New `audio_dir.rs` module — filename grammar + cleanup primitives + orphan scan + cleanup timer | Done — `src-tauri/src/audio_dir.rs` created with `SEGMENT_PREFIX`/`SEGMENT_TIME_FMT`/`format_segment_filename`/`parse_segment_timestamp`/`cleanup_old_audio`/`cleanup_old_segments`/`find_orphan_segments`/`spawn_cleanup_timer`. `audio_engine.rs` lost its cleanup functions. `pipeline.rs` lost its cleanup constants + duplicated filename parser. Six new tests in `audio_dir`. |
| **A2 + A8 + C8** | Move `AUDIO_LEVEL` / `SEGMENT_STARTED_AT` off module statics onto `AppState` | Done — `AppState::audio_level`/`audio_level_arc()`/`audio_level()` and `AppState::segment_started_at`/`segment_started_at_arc()`/`segment_started_at()`. `AudioEngine::new` now takes `Arc<AtomicU32>` and `Arc<Mutex<Option<DateTime<Utc>>>>` parameters. `pub static AUDIO_LEVEL` and `static SEGMENT_STARTED_AT` deleted. Tests use local atomics — no more process-global mutation. |
| **AU5** | Collapse dual segment-start (`Instant` + `DateTime<Utc>`) | Done — `RecordingState.segment_started: Instant` dropped; rotation check uses `(Utc::now() - started).num_seconds()`. New `elapsed_seconds` helper + tests. |
| **AU2 + C3** | Pre-alloc audio buffers, drop `Arc<Mutex>` on resampler | Done — `mono_buf` and `chunk` are owned by the callback closure (pre-allocated, reused across callbacks). Resampler is a plain `SincFixedIn<f32>` moved into the closure — no `Arc<Mutex>` because only the audio thread uses it. Zero allocations per callback. |
| **AU4** | Lower resampler quality from 256×256 to 64×128 | Done — ~10-20× less CPU on the real-time thread, no ASR-accuracy delta expected (Whisper mel-spectrogram discards >8 kHz anyway). |
| **T8** | Pipe Python stderr to `log::warn!` instead of `Stdio::inherit()` | Done — background thread line-reads stderr; `TQDM_DISABLE=1`, `HF_HUB_DISABLE_PROGRESS_BARS=1`, `PYTHONUNBUFFERED=1` envs defend stdout framing. |
| **T7** | Bound `read_line` to `MAX_LINE_BYTES` (256 KB) | Done — new `read_line_bounded` helper used by both the ready-handshake and the per-segment response path. Three new tests. |
| **T4** | Require Python ≥3.10 + log resolved interpreter + `WSCRIBE_PYTHON` override | Done — `python_version_ok` probes `sys.version_info`; explicit override honored; candidate list extended to 3.11/3.12/3.13/3.14. |
| **S5** | `query_map(..).filter_map(.ok())` → `collect::<rusqlite::Result<Vec<_>>>()` | Done — all four read queries (`get_hour_slots`, `search_hour_slots`, `get_slots_by_date_range`, `get_available_dates`) now surface row-decode errors instead of silently dropping rows. |
| **S6** | SQLite pragma tuning | Done — `synchronous=NORMAL`, `temp_store=MEMORY`, `mmap_size=256MB`, `cache_size=64MB`, `wal_autocheckpoint=1000`, `foreign_keys=ON`. |
| **S7** | `prepare_cached` for read queries | Done — per-query SQL is a `&'static str` const; all four reads hit the statement cache. `HOUR_SLOT_COLUMNS` const removed (column list inlined per-query for cache stability). |
| **TY1** | Extract `HourSlot` + `AppStatus` + `AudioLevelEvent` into `src/types.ts` | Done — new `src/types.ts`; `HourSlotCard`, `Timeline`, `StatusBar`, `FilterPanel` all updated to import from `../types` instead of `../App`. |
| **TY2** | `tryInvoke<T>` wrapper replacing `} catch (_) {}` | Done — new `src/utils/invoke.ts`; every empty catch in `App.tsx`, `FilterPanel.tsx`, `AudioLevelBars.tsx` replaced; failures log `console.warn("[ipc]", cmd, err)` instead of vanishing. |
| **TY4** | Extract `TRANSCRIPTION_UPDATED` event name to `src/events.ts` | Done — mirror constant; cross-referenced in the Rust-side comment. |
| **TY5** | Drop `HourSlotPayload` duplicate | Done — `storage::HourSlot` returned directly from all three commands; `From` impl + `.map(Into::into)` calls deleted. |
| **TY6** | Drop dead `copy_to_clipboard` Rust command | Done — command + handler registration removed; `App.tsx::copyText` is now a single `navigator.clipboard.writeText` with a logged catch. |
| **F1** | Async `createEffect` synchronous dependency capture in `FilterPanel` | Done — both effects read their signals synchronously, then fire `void (async () => ...)()` for the IPC; Solid's dependency tracking is no longer severed by `await`. |
| **F2** | `setTimeout` copy-badge leak | Done — `HourSlotCard`'s copy timer tracked and cleared on `onCleanup`. |
| **F4** | Drop redundant 5 s timeline poll | Done — `transcription-updated` event is the source of truth. |
| **F5** | Pause polling on `document.hidden` | Done — `visibilitychange` listener starts/stops the status + local-elapsed intervals together. |
| **F8** | Fix search-clear race | Done — `SearchBar`'s `onInput` now just `setSearchQuery`; the debounce effect handles both empty and non-empty via the same code path. No more parallel `clearFilter()` → `loadTimeline()` that could overwrite in-flight search results. |
| **F11** | Explicit return types on exported functions | Done — `: JSX.Element` on every component; `: Promise<void>` on async helpers. |
| **Bonus** | `createMemo` on `progress`/`secondsRemaining` + FP rounding | Done — rounds `progress` to 3 decimals to stop SVG `stroke-dashoffset` churn on FP jitter. |

## Audit items intentionally deferred

| ID | Reason |
|----|--------|
| **A6** (load-bearing `_engine`/`_power_monitor`) | Paired with C4 graceful-shutdown work; deferring together. |
| **A7** (move `transcriber` out of `AppState`) | Structural refactor; touches `is_transcribing` owner. Defer to a pass that can consider the full `PipelineRuntime` design. |
| **A9** (move `subscribe_audio_level` poll into pipeline) | Blocked by M7 (audio-level event push) which was deferred from pass-1. |
| **A10** (rename `events.rs` or inline) | Low impact, cosmetic. |
| **A11** (parallel audio-engine + transcriber init) | Speed-of-first-segment improvement; defer until we have a reason to care. |
| **C2** (`SEGMENT_STARTED_AT` → `AtomicI64`) | Done implicitly via AU5 collapse — the dual-source concern is gone. Left the `Arc<Mutex<Option<DateTime<Utc>>>>` shape because `get_status` reads it at most 1 Hz; contention is zero. Revisit if device hot-swap (AU10) lands. |
| **C4 + AU7** (JoinHandle plumbing + `impl Drop for AudioEngine`) | Architectural change touching every long-lived component. Needs its own pass with shutdown sequence design. |
| **C5** (bound segment channel) | Needs a drop-policy decision (log-and-drop vs. delete-oldest). Defer until channel back-pressure is observed in practice. |
| **C6** (re-entrancy doc on `transcriber` mutex) | Pure doc change; will fold into a later pass that touches the module. |
| **C7** (Release/Acquire on `is_transcribing`) | No current reader needs cross-store visibility. Flag for the next pass. |
| **C9** (`subscribe_audio_level` cancellation) | Blocked on M7/A9 push-event conversion. |
| **C10** (RT-thread audio-callback I/O) | Full rtrb ring-buffer refactor (AU1) is the strategic fix; deferred. |
| **AU1** (`rtrb` SPSC ring + writer thread) | Major concurrency rewrite. The "do disk I/O on the RT thread" problem is real but the fix is a pass of its own. AU2+C3+AU4 shaved the worst of the lock-hold time; AU1 stays in the backlog. |
| **AU3** (int16 WAV) | Halves disk I/O but changes on-disk WAV format. Needs a migration plan for in-flight orphan WAVs from prior runs (they're float32), and a manual listening-test confirmation. Deferred to avoid mid-session format churn. |
| **AU6** (merge `cleanup_old_segments` + `cleanup_old_audio` into one walk) | Trivial consolidation; defer until `audio_dir` grows more consumers. |
| **AU8** (drop dead `samples_written`) | Still referenced in the rotate-finalize log line; will fold in with AU1's writer-thread refactor where the field's semantics change anyway. |
| **AU9** (smoother rounding) | Cosmetic low-level UI fix. |
| **AU10** (device hot-swap + picker) | Medium-effort feature + UX change; defer. |
| **T1** (versioned IPC handshake) | Protocol change on both Rust and Python; would ideally ship with a migration note. Defer. |
| **T2** (widen segment channel payload to `SegmentRef`) | Would eliminate `count_samples` + TZ drift, but requires changing the channel signature and touching `process_orphans`. Defer to a pass that also does S2's segment-key migration. |
| **T3** (drop `count_samples`/`end_time`) | Blocked by T2 — `end_time` is dead but lives on the existing channel contract. Will delete together. |
| **T5** (`include_str!` the Python script) | Nice-to-have for dev/bundle parity; not a bug. Defer. |
| **T6** (capture-time `device`) | Blocked by T2 (widening the segment channel). |
| **T9** (parallel daemon init) | Blocked by A11 (same parallel-init design). |
| **T10** (WAV pre-validation + backoff + structured ready errors) | Nice hardening; defer until we see a real failure in the field. |
| **S1** (legacy `transcriptions` migration + `schema_version`) | Needs a backup/rollback envelope and migration test. Defer to a dedicated storage-schema pass. |
| **S2 + S3** (idempotency segments table + per-segment FTS5) | Schema changes. S3's O(n²) amplification is a real perf cliff for long hours, but the fix is part of S2's segment-key rework. Defer together. |
| **S4** (stricter FTS5 sanitizer) | Today's sanitizer is functionally safe; the operator leak is not exploitable because everything is quoted. Defer to an accessibility/Unicode pass. |
| **S8** (segment-id dedup) | Blocked on S2. The current TZ-correctness risk is latent (audio engine writes UTC, orphan path parses UTC — no drift today). |
| **S9 + S10** (orphan transaction batch + `hour_key` CHECK constraint) | Low impact today. |
| **F3** (bound `scrollPositions` map) | Planned for M22 app-store; defer to that pass. |
| **F6** | Subsumed by the new `createMemo` on `progress`/`secondsRemaining`. |
| **F7** | Subsumed by TY2 `tryInvoke` replacement of empty catches. |
| **F9** (`unsubscribe_audio_level`) | Needs a Rust-side cancellation signal; coupled to A9 event-push migration. Defer. |
| **F10** (filter panel a11y: Esc/focus-trap/aria) | Touches visual/interaction behavior. Defer to a dedicated a11y pass. |
| **TY3** (typed `AudioLevelEvent` import at the Channel site) | Done implicitly — `AudioLevelBars` now imports `AudioLevelEvent` from `./types`. |
| **TY7** (camelCase ↔ snake_case convention doc) | Done — comment added at top of `commands.rs`. |
| **TY8 + TY9 + TY10** (envelope payloads, `pause_state` exposure, shared error path) | Behavior/API changes. Defer. |
| **CS1** (remove Tailwind) | **Visual/styling edit** — per `~/.claude/CLAUDE.md` rule, deferred to a pass with a visual-diff workflow. Confirmed no utility classes in use; removing preflight is plausibly a win but needs a before/after screenshot comparison. |
| **CS2/CS3/CS4** (a11y CSS: reduced-motion, focus-visible, contrast) | Visual/styling. Defer. |
| **CS5** (dead `.transcribing-large` rules) | Visual/styling. Defer. |
| **CS6** (dark-mode lock) | Visual/styling. Defer. |
| **CS7/CS8/CS9/CS10/CS11** | Visual/styling or visual-adjacent. Defer. |

## New file layout

```
src-tauri/src/
├── main.rs           (unchanged)
├── lib.rs            (89 LOC; copy_to_clipboard handler dropped)
├── state.rs          (+ pause_flag mirror, audio_level Arc, segment_started_at Arc, toggle_pause helper)
├── events.rs         (unchanged)
├── tray.rs           (uses state.toggle_pause())
├── shortcuts.rs      (unchanged)
├── pipeline.rs       (shrunk — delegates cleanup/filename to audio_dir; pause-gated recv loop)
├── audio_dir.rs      (NEW — filename grammar + cleanup + orphan scan + cleanup timer)
├── audio_engine.rs   (refactored — Arc-shared level/started, callback pre-alloc, dropped resampler Mutex, lower-quality resampler, pause-aware callback)
├── device_manager.rs (unchanged)
├── power_monitor.rs  (uses state.set_pause; updated tests)
├── storage.rs        (PRAGMAs, prepare_cached, collect::<Result<_>>)
├── transcriber.rs    (piped stderr + log forwarder, bounded read_line, >=3.10 python check, WSCRIBE_PYTHON override)
└── commands.rs       (HourSlotPayload deleted; copy_to_clipboard deleted; get_status reads from state)

src/
├── App.tsx           (tryInvoke, visibility-paused polling, debounced search only, createMemo derived values)
├── index.tsx         (unchanged)
├── styles.css        (unchanged — CS1 deferred)
├── types.ts          (NEW — HourSlot, AppStatus, AudioLevelEvent)
├── events.ts         (NEW — TRANSCRIPTION_UPDATED constant)
├── utils/
│   ├── format.ts     (unchanged)
│   ├── highlight.ts  (unchanged)
│   └── invoke.ts     (NEW — tryInvoke wrapper)
└── components/       (all imports redirected to ../types)
```

## Out-of-scope but worth noting for the next pass

- `transcriber.rs::TranscriptionResult::{end_time, confidence}` still flagged `dead_code` — blocked on T2+T3 channel payload widening.
- `audio_engine.rs` still does disk I/O on the cpal real-time thread (AU1 deferred).
- `pipeline.rs`'s `_engine` / `_power_monitor` load-bearing underscore bindings are still there (A6 deferred with C4).
- Segment channel is still unbounded (C5 deferred).
- `transcriber` still lives on `AppState` as `Mutex<Option<Transcriber>>` (A7 deferred).
- `subscribe_audio_level` still polls at 33 ms from inside a command-spawned tokio task (A9 deferred).
- No SQLite migration framework (S1 deferred).
- FTS5 trigger is still O(n²) per hour (S3 deferred).
- Tailwind import still ships (CS1 deferred — visual).
- Two pre-existing TS errors remain: `Timeline.tsx` SVG type, `index.tsx` CSS module.
