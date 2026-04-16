# Refactor Pass 1 — Summary (2026-04-16)

This summarizes what was changed against the audit at `docs/AUDIT-2026-04-16.md`. Use it as context when reviewing the codebase for the next refactor pass.

## Verification status (post-pass-1)

- `cargo test`: **163 passed, 0 failed, 1 ignored** (the cpal live test).
- `cargo check`: clean — only one pre-existing `dead_code` warning in `transcriber.rs::TranscriptionResult` (`end_time`, `confidence` unused).
- `cargo clippy`: only `useless_vec` lints in integration tests (pre-existing, unrelated to refactor).
- `npm run build`: clean Vite production build, ~43 kB JS gzipped.
- `npx tsc --noEmit`: two **pre-existing** errors remain — `Timeline.tsx` SVG `stroke-dasharray` typing, and `index.tsx` missing CSS module type. Not introduced by the refactor.

## Audit items applied

| ID | Title | Result |
|----|-------|--------|
| C2 | Orphan dedup queries `hour_slots` (not legacy `transcriptions`) | Done — `storage.rs::has_transcription_near` rewritten |
| H1 + M23 | Bluetooth filter consolidated into `device_manager` | Done — `select_best_input` is canonical; `BLUETOOTH_KEYWORDS`/`BUILTIN_KEYWORDS` consts added; old `select_non_bluetooth_input` deleted; `classify_device`/`AudioDevice` enum/struct removed (was unused); `get_current_device_name` now agrees with `AudioEngine` |
| H2 | Extract `transcribe_and_store` helper | Done — single helper in `pipeline.rs` shared by main loop and orphan path |
| H3 | Split `lib.rs` into modules | Done — `state.rs`, `tray.rs`, `shortcuts.rs`, `pipeline.rs`, `events.rs` created; `lib.rs` is 89 LOC |
| H4 | Replace startup `.expect()` with graceful errors | Done — `setup` closure returns `Err` from `String` errors instead of panicking |
| H11 + L1 + L2 | Audio-callback cleanup | Done — `eprintln!` removed; debug logging gated behind `cfg(debug_assertions)`; `segment_duration_secs` cached via `OnceLock`; smoother constants named (`LEVEL_*`); `update_audio_level` extracted |
| M1 | `PauseReason` enum replaces two-boolean pair | Done — `state::PauseReason::{None,Manual,System}`; `power_monitor` only touches `System ↔ None`; manual-pause survival has a unit test |
| M2 + M4 | Row mapper + `ON CONFLICT` upsert | Done — `HOUR_SLOT_COLUMNS` const + `map_hour_slot` row mapper; `append_to_hour_slot` is one atomic statement; return type simplified to `Result<(), String>` since callers ignored the rowid |
| M3 | Emit unit payload instead of `&true` | Done — `events::TRANSCRIPTION_UPDATED` constant; emit `&()` |
| M5 | Preserve original device on append | Done — UPDATE branch no longer overwrites `device`; new test `test_append_to_existing_slot_preserves_original_device` |
| M6 | Clamp `get_timeline` limit | Done — `MAX_TIMELINE_LIMIT = 200`; `limit.clamp(1, MAX)`, `offset.max(0)` |
| M8 | Cleanup on 30-minute timer thread | Done — `pipeline::spawn_cleanup_timer`; per-segment cleanup removed from hot path; startup pass retained |
| M9 | Single source of truth for `recording_started_at` | Done — moved into `audio_engine::SEGMENT_STARTED_AT` static updated only inside `open_new_segment`; exposed via `current_segment_started_at()`; field removed from `AppState` |
| M14 + M15 + M16 | Frontend utils extraction | Done — `src/utils/format.ts` (`formatCountdown`, `formatRelativeDate`, `formatRelativeDateWithWeekday`, `formatHourRange`, `formatTime`) and `src/utils/highlight.ts` (`highlightText`); duplicates in `Timeline`, `StatusBar`, `HourSlotCard`, `FilterPanel` removed; dead `TranscriptionCard.tsx` deleted (it imported a non-existent `Transcription` type) |
| L3 | `write_sample` errors logged once per segment | Done — added `write_error_logged` flag on `RecordingState` |
| L4 | WAV finalize failure discards segment | Done — `rotate_segment` removes the file and skips `segment_tx.send` on finalize error |
| L9 | `PowerMonitor::new` no longer wraps in `Result` | Done — returns `Self` directly |
| Bonus | Removed `unsafe impl Send + Sync for AppState` | Done — derived from field types, no manual unsafe needed |

## Audit items intentionally deferred

| ID | Reason |
|----|--------|
| **C1** (`edition = "2021"`) | The audit's premise is wrong — Rust 1.85 stabilized edition 2024 in Feb 2025; today is 2026-04-16. Pinning to 2021 would be a regression. |
| **H5** (tauri-specta) | Adds a new build-time codegen dependency. Out-of-scope for a one-session refactor; needs design discussion. |
| **H6** (`src/api.ts` typed wrappers) | Audit recommends building this on top of H5; doing it manually first creates throwaway work. |
| **H7** (`AppError` enum) | Audit notes this should land *after* H5. Not done. |
| **H8** (hallucination regex) | Behavior change — needs a captured corpus before/after for A/B comparison. |
| **H9** (transcriber timeout) | Risky correctness change — needs care to not misclassify model warm-up. |
| **H10** (drop `unsafe impl`) | Partially addressed by removing the explicit `unsafe impl` block (the compiler now derives `Send`/`Sync` automatically through field types). The audit's stronger fix (r2d2 pool) was not adopted — would add 2 deps and a structural change for a benefit the compiler now provides for free. |
| **M7 / L5** (audio-level event push) | Coupled to M3; deferred. |
| **M10** (VAD diagnostic) | Python-side change requiring daemon protocol revision. |
| **M11** (per-app venv) | Changes installation flow. Needs careful testing. |
| **M12 / M13 / L19 / L20 / L21** (CSS) | Visual risk; needs screenshot diff workflow. |
| **M17 / M18 / M19 / M22** (App.tsx restructure) | Audit recommends doing these *after* H6; deferred. |
| **M20** (CSP) | Needs iterative testing to find a CSP that doesn't break the UI. |
| **L8** (graceful shutdown) | Architectural change touching every long-lived component. |
| **L15 / L16** (more tests) | Pure addition; no refactor risk; defer. |
| **L17 / L18** (formatters, CI) | Configuration; not a code refactor. |

## New file layout

```
src-tauri/src/
├── main.rs           (unchanged thin entry)
├── lib.rs            (89 LOC — Tauri builder + module declarations)
├── state.rs          (NEW — AppState + PauseReason)
├── events.rs         (NEW — shared event-name constants)
├── tray.rs           (NEW — setup_tray + make_icon)
├── shortcuts.rs      (NEW — global shortcut registration)
├── pipeline.rs       (NEW — start, transcribe_and_store, process_orphans, cleanup timer)
├── audio_engine.rs   (refactored — SEGMENT_STARTED_AT static, named constants, debug-gated logs)
├── device_manager.rs (refactored — canonical select_best_input + Bluetooth/builtin classification)
├── power_monitor.rs  (refactored — PauseReason aware, no Result wrapper)
├── storage.rs        (refactored — row mapper, ON CONFLICT upsert, hour_slots dedup)
├── transcriber.rs    (unchanged in this pass)
└── commands.rs       (refactored — limit clamp, From impl for HourSlotPayload, PauseReason)

src/
├── App.tsx           (unchanged in this pass)
├── index.tsx         (unchanged)
├── styles.css        (unchanged)
├── utils/            (NEW)
│   ├── format.ts     (countdown, relative date, hour range, time)
│   └── highlight.ts  (highlightText with HighlightPart type)
└── components/       (TranscriptionCard.tsx removed)
```

## Out-of-scope but worth noting for the next pass

- `src/App.tsx` still has the audit-flagged 5-overlapping-effects pattern (M17), `searchQuery`/`filterActive` coupling (M18), and per-component `invoke()` calls in `FilterPanel` (M19).
- The legacy `transcriptions` SQL table is never written to but is still created on startup. Migration / drop deferred.
- `transcriber.rs` still has the indefinite `read_line()` (H9), the Python `--break-system-packages` install path (M11), and hardcoded `confidence = 0.95` (L7).
- No CSP set (M20), no design tokens (M12), no shared button abstraction (M13).
- Pre-existing TS errors in `Timeline.tsx` (SVG type) and `index.tsx` (CSS module type) — not refactor regressions, but should be cleaned up.
- `TranscriptionResult.end_time` and `confidence` are dead fields (`dead_code` warning).
