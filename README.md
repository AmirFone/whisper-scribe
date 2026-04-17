# Whisper Scribe

A fast, lightweight macOS menu bar app that continuously records audio and transcribes it locally using Whisper Large v3 on Apple Silicon GPU. Periodically captures screenshots and analyzes them with a local vision model for screen context logging. Everything stays on-device — no cloud, no subscriptions.

<p align="center">
  <img src="docs/screenshots/app-icon.png" width="128" alt="Whisper Scribe Icon" />
</p>

<p align="center">
  <img src="docs/screenshots/app-main.png" width="400" alt="Whisper Scribe" />
  <img src="docs/screenshots/screen-context.png" width="400" alt="Screen Context" />
</p>

## What It Does

Whisper Scribe sits in your menu bar and records audio in 2-minute segments, transcribing each one with MLX-accelerated Whisper Large v3. Transcriptions are grouped into hourly slots — one card per clock hour — and appended as new segments complete. All text is stored locally in SQLite with full-text search.

Every 5 minutes, it also captures screenshots of all connected displays and analyzes them with Qwen3.5-9B (a local vision model running on Apple Silicon GPU). The Screen Context tab shows a log of what you were doing on screen — which apps were open, what tabs you had, what text was visible — searchable and filterable just like transcriptions.

## Key Features

- **Always-on recording** with smart pause on screen lock/sleep
- **MLX GPU transcription** via Whisper Large v3 (~2x faster than CPU on M-series chips)
- **Hourly time slots** — clean UI, one card per hour, text grows as you talk
- **Full-text search** with highlighted results
- **Date/time filtering** — filter by day and hour range, copy all matching text
- **Hallucination filtering** — Silero VAD pre-filter + post-processing regex strips repeated phrases and silent-segment artifacts
- **Smart device selection** — auto-prefers built-in mic over Bluetooth to avoid AirPods audio degradation
- **macOS native** — translucent vibrancy, Cmd+, toggle, draggable window
- **Screen context logging** — periodic screenshot capture + on-device OCR via Qwen3.5-9B vision model
- **Multi-monitor support** — captures all connected displays via CoreGraphics
- **Privacy-first screen capture** — no screencapture CLI, direct CoreGraphics FFI, permission checked before every cycle

## Tech Stack

Rust (Tauri v2) + SolidJS + MLX Whisper + MLX-VLM/Qwen3.5 (Python) + SQLite FTS5 + CoreGraphics

## Requirements

- macOS 14+ on Apple Silicon (M1/M2/M3/M4)
- Python 3.11+ (mlx-whisper and mlx-vlm auto-install on first run)
- ~3 GB disk for Whisper Large v3 MLX model (downloads automatically)
- ~6 GB disk for Qwen3.5-9B-MLX-4bit vision model (downloads automatically)
- Screen Recording permission (prompted on first capture)

## Install

```bash
# Build from source
npm install
cargo tauri build

# The .app and .dmg are in src-tauri/target/release/bundle/
cp -R "src-tauri/target/release/bundle/macos/Whisper Scribe.app" /Applications/
```

## Roadmap

- [x] Periodic screen capture with local vision model (Qwen3.5-9B) for OCR-based activity logging
- [x] Searchable visual history — what you saw + what you said, correlated by timestamp
- [x] Multi-monitor screenshot support
- [ ] Speaker diarization (who said what)
- [ ] Export to markdown/JSON
