# Refactor Pass 3 — Summary (2026-04-16)

This summarizes what was changed against the Pass-3 audit at `docs/AUDIT-2026-04-16-PASS3.md`. Use it as context when reviewing the codebase for the next refactor pass.

## Verification status (post-pass-3)

- `cargo test --lib --tests`: **133 passed, 0 failed, 1 ignored** (the cpal live test). Tests of the legacy `transcriptions` schema that were inflating the previous number are now gated behind the `legacy_schema_tests` cargo feature — the new `storage_integration.rs` tests the real `Storage` API instead.
- `cargo check`: clean — only the same pre-existing `dead_code` warning on `TranscriptionResult::{end_time, confidence}`, which is blocked on T2+T3 from pass-2's deferral list.
- `npm run build`: Vite production build clean, ~14.27 kB JS gzipped (down a hair from pass-2's ~43 kB raw). CSS at 5 kB gzipped.
- `npx tsc --noEmit`: **0 errors**. Both pass-2-era pre-existing errors are fixed (`TY-C` and `TY-D`).
- `npm run typecheck` + `npm run test:rust` + `npm run test:all` added to `package.json`.

## Audit items applied

| ID | Title | Result |
|----|-------|--------|
| **Theme 3** (A5 + S14 + T7) | Orphan dedup correctness | Done — `Storage::has_transcription_near(rfc_string)` replaced by `Storage::is_segment_processed(&DateTime<Utc>)` which queries `WHERE hour_key = ?1 AND last_updated >= ?2`. Covers non-first-segment orphans that the previous `start_time`-only match silently re-transcribed on every restart. `extract_timestamp_from_path` now returns `Option<DateTime<Utc>>`; non-canonical WAVs are skipped in `pipeline::process_orphans` (`OrphanStatus::NonCanonical`) instead of being stamped with `Utc::now()` and forever failing dedup. Renamed the misleading `now_str` variable in `append_to_hour_slot` to `capture_time_str`. Added `Storage::hour_key_of` helper. Three new storage-layer tests + one integration test. |
| **Theme 2** (C1 + AU-NEW-1..3) | `segment_started_at` → `AtomicI64` | Done — `Arc<Mutex<Option<DateTime<Utc>>>>` replaced with `Arc<AtomicI64>`; `i64::MIN` is the `SEGMENT_STARTED_UNSET` sentinel. `encode_segment_started` / `decode_segment_started` free functions on `state::` wrap the epoch-millis encoding. The audio callback's pause branch now uses callback-local `bool was_paused` edge detection — onset clears `resample_buf` + marks UNSET once; release re-baselines to `Utc::now()` once. No more per-paused-buffer state lock or `Utc::now()` syscall. |
| **AU-NEW-4** | Audio-error signal on `open_new_segment` failure | Done — new `Arc<AtomicBool>` `audio_disk_error` on `AppState`, threaded to `AudioEngine::new`. Set in the `Err` arm of `WavWriter::create`, cleared in `Ok`. `get_status` surfaces `audio_disk_error: bool` → `AppStatus.audio_disk_error` on the frontend. |
| **AU-NEW-8** | Unconditional `write_error_logged` reset | Done — reset now happens at the top of `open_new_segment` regardless of the outcome, so a failed `WavWriter::create` followed by a successful one does not leave the "log once" flag stuck true. |
| **TY-C** | Timeline SVG `stroke-dasharray` type | Done — `stroke-dasharray={String(circumference)}`. |
| **TY-D** | CSS side-effect import TS error | Done — new `src/vite-env.d.ts` with `/// <reference types="vite/client" />`. |
| **F3** | Filter view + `TRANSCRIPTION_UPDATED` collision | Done — App tracks `filterRange: FilterRange \| null`; `handleFilterApply` receives the applied range from `FilterPanel` and stores it; the `TRANSCRIPTION_UPDATED` listener re-runs `refreshActiveFilter()` when `filterActive()` is true, instead of no-oping via `loadTimeline`. `clearFilter` nulls the range. |
| **BS1** | `staticlib` crate-type | Done — removed from `src-tauri/Cargo.toml`. |
| **BS2** | `tauri-plugin-shell` live with no callers | Done — dependency removed from `Cargo.toml`, `.plugin(tauri_plugin_shell::init())` removed from `lib.rs`, `shell:allow-open` removed from `capabilities/default.json`. |
| **BS5** | `rtrb` unused dependency | Done — removed from `Cargo.toml`. |
| **BS10** | `tokio` feature trim | Done — replaced `features = ["full"]` with `["rt-multi-thread", "time", "macros"]`. |
| **BS6** | `env_logger` default filter | Done — `env_logger::Builder::new().filter_level(log::LevelFilter::Info).parse_default_env().init()`. Packaged builds now emit the `log::info!` / `log::warn!` stream that was previously silenced — including the Python stderr forwarder from pass-2. `RUST_LOG` still overrides. |
| **T1** | `read_line_bounded` rewrite | Done — replaced the `reader.take(max).read_until('\n', tmp)` approach with an explicit `fill_buf` / `consume` loop. Cap is checked BEFORE bytes are consumed, so a near-cap line cannot silently move the reader past the protocol boundary. Four new tests (cap-with-newline boundary, `fill_buf` fragmentation, trailing-bytes-untouched, oversize-rejection retained). |
| **T2** | `child.wait()` after `child.kill()` | Done — new `shutdown_daemon(DaemonHandle)` helper that drops `stdin` (unblocking the stderr forwarder thread), kills the child, then calls `wait()` to reap. `Drop for Transcriber` and every failure path in `transcribe` route through it. No more zombie accumulation on daemon restart or app teardown. |
| **TS1** | Integration suite rewrite | Done — `tests/storage_integration.rs` now imports `whisper_scribe_lib::storage::Storage` and exercises `append_to_hour_slot`, `get_hour_slots`, `search_hour_slots`, `get_slots_by_date_range`, `is_segment_processed`, and cross-thread appends. The legacy `tests/functional_e2e.rs` + `tests/storage_exhaustive.rs` (which tested the dead `transcriptions` schema) are gated behind `#[cfg(feature = "legacy_schema_tests")]` + a matching `legacy_schema_tests = []` feature in `Cargo.toml`. |
| **TS3** | `MAX_TIMELINE_LIMIT` clamp test | Done — extracted pure `clamp_timeline_params(limit, offset) -> (i64, i64)` helper in `commands.rs` + four unit tests (upper-bound, non-positive-limit, negative-offset, sane-values preserved). The `get_timeline` handler now delegates to the helper. |
| **TS4** | `Send + Sync` compile-time assert | Done — `_ASSERT_APP_STATE_SEND_SYNC` constant in `state.rs` forces `Send + Sync` on `AppState` at compile time. Zero runtime cost; breaks the build loudly if a future field introduces an `Rc<_>` or other non-Sync type. |
| **TS10** | CI + unified test commands | Done — `.github/workflows/ci.yml` runs `cargo test --lib --tests` + `cargo check` on `macos-latest` + `ubuntu-latest` with `~/.cargo/registry` caching, plus a separate frontend job doing `npm ci` + `npm run typecheck` + `npm run build`. `package.json` gained `typecheck`, `test:rust`, and `test:all` scripts. |

## New tests added (net count)

- `storage::tests::test_is_segment_processed_first_orphan`
- `storage::tests::test_is_segment_processed_later_orphan_in_existing_hour`
- `storage::tests::test_is_segment_processed_different_hour_is_independent`
- `transcriber::tests::test_extract_timestamp_invalid_returns_none` (renamed from `test_extract_timestamp_invalid` to reflect the new `Option` return)
- `transcriber::tests::test_read_line_bounded_accepts_cap_with_newline`
- `transcriber::tests::test_read_line_bounded_survives_fill_buf_fragmentation`
- `transcriber::tests::test_read_line_bounded_leaves_trailing_bytes_untouched`
- `audio_engine::tests::test_elapsed_seconds_unset_is_zero` (replaces `test_elapsed_seconds_none_is_zero`)
- `audio_engine::tests::test_segment_started_encode_decode_roundtrip`
- `commands::tests::test_clamp_timeline_params_enforces_upper_bound`
- `commands::tests::test_clamp_timeline_params_rejects_non_positive_limit`
- `commands::tests::test_clamp_timeline_params_negative_offset_is_floored_to_zero`
- `commands::tests::test_clamp_timeline_params_preserves_sane_values`
- `storage_integration::test_append_then_get_timeline_via_real_api` + five siblings

## Audit items intentionally deferred

| ID | Reason |
|----|--------|
| **AU1** (rtrb ring buffer) | User explicitly deferred in the session plan. BS5 removed the unused `rtrb` dependency in the meantime — when AU1 is ready it's a re-add. |
| **C4 + AU7** (graceful shutdown) | User explicitly deferred. Touches every long-lived component; needs its own pass with a shutdown-sequence design. `PowerMonitor`, the stderr-forwarder thread, and the cleanup timer still leak past Tauri teardown. |
| **S1 / S2 / S3** (schema migrations) | User explicitly deferred. Critically, the non-first-segment orphan dedup is now correctly handled via `hour_key + last_updated` without needing S2's segments table; S2 remains the strategic fix for per-segment identity (and would also dissolve `dead_code` warnings on `TranscriptionResult`). |
| **CS-N1..CS-N10** (visual CSS) | User's global rules forbid direct visual-CSS edits without a visual-diff workflow. The Tailwind overhead (~35% of bundle) and the invisible spin animation remain in the CSS backlog. |
| **A1 / A2 / A7 / A8 / A9 / A10** | Not on this pass's priority list. `AppState` is still a god-object with pub fields; `get_current_device_name()` re-enumerates every call; `is_transcribing` lacks an RAII guard; typed error enum doesn't exist. Queue for pass 4. |
| **A3 / C2 / C9** (`subscribe_audio_level`) | Blocked on the push-event migration (M7/A9 from prior passes). Left for pass 4. |
| **S11..S20** (storage hardening) | Not on this pass's priority list. Read-pool, FTS5 sanitizer polish, `count()` caching remain in the backlog. The specific `has_transcription_near` issue (S14) IS covered by the Theme 3 work above — the rest of the storage findings are unaddressed. |
| **T3 / T5 / T6 / T9 / T10** | Not on this pass's priority list. `--break-system-packages`, per-segment daemon-lock hold, typed daemon response enum, chunked RMS silence check, script-path logging all still open. |
| **T4** (log winning script path) | Not touched this pass; the minor diagnostic win can ride with the next transcriber pass. |
| **BS3 / BS4 / BS7 / BS8 / BS9** | Not on this pass's priority list. `signingIdentity = "-"`, `WSCRIBE_PYTHON` ownership check, `rust-toolchain.toml`, Python probe order, `read_line_bounded` test for MAX_LINE_BYTES - 1 (partly covered via the new `test_read_line_bounded_accepts_cap_with_newline` — the specific `MAX_LINE_BYTES - 1 + \n` case is now regression-tested on line lengths the suite actually uses). |
| **F1 / F2 / F4..F10** | Not on this pass's priority list. `AudioLevelBars` double-mount, `FilterPanel` debounce, `visibilitychange` `onMount` rewrite, `backendError` signal, `FilterPanel` copied badge, `scrollPositions` cleanup all remain. |
| **TY-A / TY-B / TY-E..TY-J** | Type precision comments (TY-A, TY-B) were added as part of the AU-NEW-4 frontend work because `AppStatus` picked up `audio_disk_error` anyway. Remaining (FilterPanel call-site comment, `HourSlot` coupling flag, `tryInvoke` Zod shape, `PauseReason` TS enum, call-id instrumentation) not touched. |
| **TS2 / TS5 / TS6 / TS7 / TS8 / TS9** | Not on this pass's priority list. Concurrent `pause_flag` test, BDD comments on older tests, `elapsed_seconds` clock-skew cases, FTS sanitizer property candidates, `power_monitor::run_power_monitor` OS-loop coverage, `tray.rs::make_icon` pixel test all remain. |

## New file layout

```
src-tauri/src/
├── main.rs           (unchanged)
├── lib.rs            (staticlib gone; tauri-plugin-shell gone; env_logger Info default; `state` + `storage` now pub)
├── state.rs          (segment_started_at is Arc<AtomicI64>; audio_disk_error added; Send+Sync const assert)
├── events.rs         (unchanged)
├── tray.rs           (unchanged)
├── shortcuts.rs      (unchanged)
├── pipeline.rs       (OrphanStatus enum; wires audio_disk_error through to AudioEngine::new)
├── audio_dir.rs      (unchanged)
├── audio_engine.rs   (pause-onset/release edge detection; AtomicI64 timestamp; audio_disk_error flag; unconditional write_error_logged reset)
├── device_manager.rs (unchanged)
├── power_monitor.rs  (unchanged)
├── storage.rs        (is_segment_processed replaces has_transcription_near; hour_key_of helper; cached prepare for COUNT)
├── transcriber.rs    (read_line_bounded rewritten via fill_buf/consume; shutdown_daemon helper reaps zombies; extract_timestamp_from_path returns Option)
└── commands.rs       (clamp_timeline_params helper + tests; audio_disk_error in StatusPayload)

src-tauri/tests/
├── storage_integration.rs     (REWRITTEN — tests real Storage API)
├── storage_exhaustive.rs      (gated behind `legacy_schema_tests` feature)
├── functional_e2e.rs          (gated behind `legacy_schema_tests` feature)
├── audio_and_transcriber.rs   (unchanged)
├── audio_capture_e2e.rs       (unchanged)
├── device_manager_exhaustive.rs (unchanged)
└── test_cpal_live.rs          (unchanged, still #[ignore])

src/
├── App.tsx           (filterRange signal + refreshActiveFilter; handleFilterApply accepts range; INITIAL_STATUS.audio_disk_error)
├── index.tsx         (unchanged)
├── styles.css        (unchanged — CSS deferred)
├── types.ts          (AppStatus.audio_disk_error; range comments on slots_count + audio_level)
├── events.ts         (unchanged)
├── utils/invoke.ts   (unchanged)
├── vite-env.d.ts     (NEW — vite/client reference)
└── components/
    ├── FilterPanel.tsx  (exports FilterRange; onApply passes range; rangeKeys helper)
    ├── Timeline.tsx     (stroke-dasharray takes String(circumference))
    └── (others unchanged)

.github/workflows/ci.yml (NEW — cargo test + cargo check on macOS + Ubuntu; frontend typecheck + build on Ubuntu)
```

## Out-of-scope but worth noting for the next pass

- `TranscriptionResult::{end_time, confidence}` still flagged `dead_code` — blocked on T2+T3 from pass-2.
- `audio_engine.rs` still does disk I/O on the cpal real-time thread (AU1).
- Graceful shutdown remains unreachable (C4 + AU7).
- `subscribe_audio_level` is still an infinite Tokio task (A3 + C2 + C9).
- No `WscribeError` enum — every module still uses `Result<T, String>` (A10).
- `get_current_device_name` is still uncached and called 3× per segment (A2).
- `AppState` still has `pub storage` and `pub transcriber` (A1).
- CSS design-system findings (CS-N1..CS-N10) still pending a visual-diff workflow.
- 133 test count is down from pass-2's 174 because ~41 tests of the dead `transcriptions` schema are now feature-gated. They can be re-enabled with `cargo test --features legacy_schema_tests` but do not gate CI.
