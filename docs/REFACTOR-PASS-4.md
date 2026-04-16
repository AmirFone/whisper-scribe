# Refactor Pass 4 — Summary (2026-04-16)

This summarizes what was changed against the Pass-4 audit at `docs/AUDIT-2026-04-16-PASS4.md`. Use it as context when reviewing the codebase for the next refactor pass.

## Verification status (post-pass-4)

- `cargo test --lib --tests`: **135 passed, 0 failed, 1 ignored** (the cpal live test). Up from 133 in pass-3 — two new tests for UTC hour_key invariance.
- `cargo check`: clean.
- `npm run build`: Vite production build clean. JS 14.44 kB gzipped, CSS 5.04 kB gzipped.
- `npx tsc --noEmit`: **0 errors**.

## Audit items applied

| ID | Title | Result |
|----|-------|--------|
| **Theme 1** (TY-N + CS-N11) | Surface `audio_disk_error` in the UI | Done — `StatusBar.tsx` now renders a `<Show when={status().audio_disk_error}>` block with an amber pulsing dot and "Disk error" label. CSS uses the same `rgba(255,159,10,*)` amber/orange token as `.status-dot.paused`, with `pulse-dot` animation. |
| **Theme 2** (P4-S1 + AU4-7 + P4-T1) | Pin RFC3339 dedup contract | Done — `hour_slots.start_time` and `hour_slots.last_updated` migrated from `TEXT` (RFC3339 strings) to `INTEGER` (epoch millis). The `IS_PROCESSED_SQL` comparison is now a numeric `>=` — the string-format contract that three separate reviewers flagged no longer exists. The error is defined out of existence: there is no format to drift. `HourSlot` struct uses `i64`; TS type updated to `number`. No frontend code actually reads these fields, so the blast radius was zero. |
| **Theme 3** (N3 + P4-S4) | Timezone-invariant `hour_key` | Done — `Storage::hour_key_of` and `append_to_hour_slot` now bucket in UTC (removed `.with_timezone(&Local)`). A user who changes timezone between runs still produces the same hour_key for the same `DateTime<Utc>`. Display converts UTC hour_key to local via `formatHourRange` (appends `Z` before parsing so `toLocaleTimeString()` renders local). `test_date_range_query_via_real_api` now uses `Storage::hour_key_of` instead of hard-coded strings, fixing the latent tz dependency. Added `test_hour_key_of_is_utc_not_local` + `test_hour_key_groups_same_utc_hour_across_date_boundary`. |
| **AU4-1** | Defer `open_new_segment` to first unpaused callback | Done — Removed the `open_new_segment` call from `AudioEngine::new`. Engine construction no longer creates a WAV file. Instead, `was_paused` is initialised to `true` so the first unpaused callback enters the pause-release branch. That branch now lazy-opens the initial segment when `writer.is_none()`, or re-baselines the timestamp on a true pause→unpause edge. The filename/content mismatch for born-paused engines is structurally eliminated. |
| **Theme 5** (Finding 1/4/7 + TY-R) | Async IPC generation counter | Done — Single monotonic `fetchGen` counter in `App.tsx`. A new `fetchIntoSlots(fetchFn)` helper bumps the counter, awaits, and discards the result if a newer mutation has landed in the meantime. All three `setSlots` write paths (`loadTimeline`, `refreshActiveFilter`, `searchSlots`) go through this helper. `handleFilterApply` and `clearFilter` bump `fetchGen` on entry so in-flight fetches from the prior view mode are discarded. `FilterPanel.tsx` has its own local `panelFetchGen` counter for its range-query effect. |
| **BS18** | Pin Python deps, drop `--break-system-packages` | Done — `src-tauri/scripts/requirements.txt` pins `mlx-whisper==0.4.3` and `silero-vad==6.2.1`. `ensure_package` replaced with `ensure_deps()` which runs `pip install -r requirements.txt --quiet` — no `--break-system-packages`. Silero-vad import failure now falls through to the existing RMS-only fallback without a separate `ensure_package` call. |
| P4-S8 / P4-T10 | Silent `prepare_cached` failure in `is_segment_processed` | Done — Added `log::error!` on both `prepare_cached` failure and query error in `is_segment_processed`. |

## New tests added (net count: +2)

- `storage::tests::test_hour_key_of_is_utc_not_local`
- `storage::tests::test_hour_key_groups_same_utc_hour_across_date_boundary`

## Audit items intentionally deferred

| ID | Reason |
|----|--------|
| **Theme 4** (A4-03 + A4-02 + A4-06 + A4-10) | `AppState` god-object trimming. Not in the convergence threshold for this pass. |
| **Theme 8** (CI tightening: BS11–BS14) | `cargo fmt --check`, `cargo clippy`, `rust-toolchain.toml`, SHA-pinned actions, npm cache. Mostly YAML. Not in scope. |
| **Theme 9** (Shutdown hygiene: N1 + P4-T4 + A4-07 + N7) | JoinHandle storage, mutex drop before wait. Deferred to a dedicated graceful-shutdown pass (C4/AU7). |
| **Theme 10** (Frontend UX: Finding 2/3/5/6/8/9/10) | AudioLevelBars channel cleanup, clearFilter race, FilterPanel debounce, localElapsed cap, tryInvoke nulls. Not in convergence threshold. |
| **BS17** | CSP + `withGlobalTauri` audit. Not in scope. |
| **BS15 + BS16** | Capability cleanup. Not in scope. |
| **P4-T9** | `spawn_daemon` EOF check. Not in scope. |
| **AU4-2 / AU4-3 / AU4-4 / AU4-8** | `is_recording` flag, elapsed pre-gate, resample_buf guard, audio_disk_error reset. Not in scope. |
| **AU4-5 / AU4-6 / AU4-10** | Rotate send-before-open, SEGMENT_PREFIX filter, cleanup merge. Not in scope. |
| **Theme 7** (TT5 + TT7) | Structural test drift in `device_manager_exhaustive.rs` and `audio_and_transcriber.rs`. Not in scope. |
| **TT1 + TT2 + TT8** | CI cron for legacy tests, `shutdown_daemon` test, `test:rust-legacy` script. Not in scope. |
| **A4-01 / A4-04 / A4-08 / A4-09** | atomic_timestamp extraction, device name cache, `OnceLock` on `make_icon`, `count_samples` deletion. Not in scope. |
| **P4-S3** | `DROP TRIGGER` + re-create for existing databases. Requires schema migration infra. |
| **P4-S5 / P4-S6 / P4-S7** | FTS sanitizer dash-only, double-space on leading whitespace, hard-capped search limit. Not in scope. |
| **CSS** | All CSS findings (CS-N1..CS-N18 + CS1..CS11) remain deferred pending a visual-diff workflow. |

## File changes summary

```
src-tauri/src/
  audio_engine.rs   — removed constructor open_new_segment; was_paused=true init; lazy-open in release branch
  storage.rs        — hour_key UTC; start_time/last_updated INTEGER; is_segment_processed log::error; 2 new tests

src-tauri/tests/
  storage_integration.rs — test_date_range_query_via_real_api uses Storage::hour_key_of

src-tauri/scripts/
  requirements.txt       — NEW: pinned mlx-whisper==0.4.3, silero-vad==6.2.1
  mlx_transcribe.py      — ensure_deps() replaces ensure_package(); no --break-system-packages

src/
  App.tsx               — fetchGen counter + fetchIntoSlots helper; handleFilterApply/clearFilter bump gen
  types.ts              — HourSlot.start_time/last_updated: string → number
  utils/format.ts       — formatHourRange appends "Z" (UTC interpretation)
  components/
    StatusBar.tsx       — <Show when={audio_disk_error}> + amber dot + "Disk error" label
    FilterPanel.tsx     — panelFetchGen counter for range-query effect

src/styles.css          — .status-dot.disk-error + .status-disk-error rules
```

## Convergence verdict

**Probably yes.** This pass landed the six highest-impact correctness and hardening items the PASS4 audit identified as the convergence threshold:

1. `audio_disk_error` surfaced in UI (presentation-layer gap closed).
2. RFC3339 lexicographic comparison eliminated (integer epoch).
3. Hour-key timezone invariance established (UTC bucketing).
4. Born-paused filename mismatch fixed (lazy segment open).
5. Async IPC races guarded (generation counter).
6. Python supply-chain hazard closed (pinned deps, no --break-system-packages).

The remaining deferred backlog is composed of low/medium items: god-object trimming, CI YAML, shutdown hygiene (requires dedicated pass), frontend UX polish, capability cleanup, and CSS (blocked on visual-diff workflow). None are correctness-critical; none require schema migrations.

Whether the PASS5 reviewers surface new HIGH-severity findings or new cross-cutting themes determines the final convergence call. If they surface only low/medium items and the total is under 50, declare CONVERGED.
