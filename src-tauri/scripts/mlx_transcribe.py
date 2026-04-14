#!/usr/bin/env python3
"""MLX Whisper transcription daemon with VAD + hallucination filtering."""
import json
import sys
import os
import re
import subprocess
import struct
import wave

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

def ensure_package(name):
    try:
        __import__(name.replace("-", "_"))
    except ImportError:
        sys.stderr.write(f"Installing {name}...\n")
        subprocess.check_call([sys.executable, "-m", "pip", "install", name, "--quiet"])

ensure_package("mlx-whisper")

import mlx_whisper

# Try to load Silero VAD (best-in-class, ONNX-backed)
_vad_model = None
_vad_available = False
try:
    ensure_package("silero-vad")
    from silero_vad import load_silero_vad, read_audio, get_speech_timestamps
    _vad_available = True
except Exception:
    sys.stderr.write("silero-vad not available, using RMS-only silence detection\n")

MODEL = "mlx-community/whisper-large-v3-mlx"

INITIAL_PROMPT = (
    "Transcribe the following spoken English conversation. "
    "Use standard punctuation and capitalization. "
    "This is a real conversation, not a video or podcast."
)

# Hallucination patterns — only match REPEATED phrases (3+), not single occurrences
HALLUCINATION_PATTERNS = [
    r"(?i)\b(thank you[.,!]?\s*){3,}",
    r"(?i)\b(thanks for watching[.,!]?\s*){2,}",
    r"(?i)\b(please subscribe[.,!]?\s*){2,}",
    r"(?i)\b(like and subscribe[.,!]?\s*){2,}",
    r"(?i)\b(bye[.,!]?\s*){3,}",
    r"(?i)^\s*\.+\s*$",
    r"(?i)^\s*\.\.\.\s*$",
    r"(?i)^[\s.,:;!?]*$",
    r"(?i)^\s*(um|uh|hmm|ah)[.,]?\s*$",
    r"(?i)\b(\w+[.,!]?\s*)\1{4,}",  # Any single word repeated 5+ times
    r"(?i)^\s*unintelligible\s*$",
    r"(?i)^\s*\[.*\]\s*$",
    r"(?i)^\s*(music|applause|laughter)\s*$",
]
COMPILED_PATTERNS = [re.compile(p) for p in HALLUCINATION_PATTERNS]


def get_vad_model():
    global _vad_model
    if _vad_model is None and _vad_available:
        _vad_model = load_silero_vad()
    return _vad_model


def is_audio_silent(path, threshold=0.005):
    try:
        with wave.open(path, "rb") as w:
            frames = w.readframes(w.getnframes())
            if len(frames) < 4:
                return True
            if w.getsampwidth() == 4:
                samples = struct.unpack(f"<{len(frames)//4}f", frames)
            elif w.getsampwidth() == 2:
                raw = struct.unpack(f"<{len(frames)//2}h", frames)
                samples = [s / 32768.0 for s in raw]
            else:
                return False
            rms = (sum(s * s for s in samples) / len(samples)) ** 0.5
            return rms < threshold
    except Exception:
        return False


def has_speech_vad(audio_path):
    if not _vad_available:
        return True  # No VAD = assume speech
    try:
        model = get_vad_model()
        wav = read_audio(audio_path)
        timestamps = get_speech_timestamps(wav, model, return_seconds=True)
        return len(timestamps) > 0
    except Exception:
        return True  # Fail open


def clean_hallucinations(text):
    if not text or not text.strip():
        return ""

    cleaned = text.strip()

    for pattern in COMPILED_PATTERNS:
        cleaned = pattern.sub("", cleaned)

    # Repeated long phrases (10+ chars repeated 3+ times)
    cleaned = re.sub(r"(?i)(.{10,}?)[.,!?]?\s*(\1[.,!?]?\s*){2,}", r"\1.", cleaned)

    cleaned = re.sub(r"\s{2,}", " ", cleaned).strip()
    cleaned = re.sub(r"^[.,;:!?\s]+", "", cleaned)
    cleaned = re.sub(r"[.,;:!?\s]+$", "", cleaned).strip()

    if len(cleaned) < 3:
        return ""

    return cleaned


def transcribe(audio_path):
    if is_audio_silent(audio_path):
        return {"text": "", "skipped": True, "reason": "rms_silence"}

    if not has_speech_vad(audio_path):
        return {"text": "", "skipped": True, "reason": "vad_no_speech"}

    result = mlx_whisper.transcribe(
        audio_path,
        path_or_hf_repo=MODEL,
        language="en",
        verbose=False,
        condition_on_previous_text=False,
        compression_ratio_threshold=2.0,
        no_speech_threshold=0.35,
        logprob_threshold=-0.8,
        temperature=(0.0, 0.2, 0.4, 0.6, 0.8, 1.0),
        initial_prompt=INITIAL_PROMPT,
    )

    raw_text = result.get("text", "").strip()
    cleaned = clean_hallucinations(raw_text)

    return {
        "text": cleaned,
        "raw_text": raw_text,
        "language": result.get("language", "en"),
        "filtered": raw_text != cleaned,
    }


def main():
    sys.stdout.write(json.dumps({"status": "ready", "model": MODEL, "vad": _vad_available}) + "\n")
    sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            path = req.get("path", "")
            if not path or not os.path.exists(path):
                sys.stdout.write(json.dumps({"error": f"not found: {path}"}) + "\n")
                sys.stdout.flush()
                continue
            result = transcribe(path)
            sys.stdout.write(json.dumps(result) + "\n")
            sys.stdout.flush()
        except Exception as e:
            sys.stdout.write(json.dumps({"error": str(e)}) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
