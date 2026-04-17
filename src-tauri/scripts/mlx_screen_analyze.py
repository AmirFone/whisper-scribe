#!/usr/bin/env python3
"""MLX Vision screen analysis daemon — Qwen3.5 via mlx-vlm."""
import json
import re
import sys
import os
import subprocess

os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

MODEL = os.environ.get(
    "WSCRIBE_SCREEN_MODEL",
    "mlx-community/Qwen3.5-9B-MLX-4bit",
)

SYSTEM_PROMPT = (
    "You are a screen logger. Output ONLY the requested information. Do NOT explain your reasoning, do NOT think step by step, do NOT say 'let me look' or 'wait'. Just output the facts directly.\n\n"
    "Format your response exactly like this:\n\n"
    "App: [exact app name from title/menu bar]\n"
    "Activity: [what the user is doing in one sentence]\n"
    "Tabs: [list all open tab titles, mark active with *]\n"
    "URL: [if browser, the URL bar text]\n"
    "Visible text: [transcribe key text you can read — page titles, headings, commands, error messages, file names, data values, chat messages. Quote exact text. Be thorough but don't repeat yourself.]\n\n"
    "Rules:\n"
    "- Output ONLY the structured fields above. No preamble, no analysis, no reasoning.\n"
    "- If a field doesn't apply (e.g., no URL for a terminal), skip it.\n"
    "- For terminals: include the last few commands and their output.\n"
    "- For spreadsheets: include column headers and notable cell values.\n"
    "- For video: include the video title and channel name.\n"
    "- If the display shows only desktop wallpaper with no windows, output just: Idle\n"
    "- Never say 'likely', 'appears to be', 'I think', 'let me', 'wait', 'actually'. Just state facts."
)

USER_PROMPT = "Log this screen. Output the structured fields only, no reasoning."


def ensure_deps():
    try:
        import mlx_vlm as _  # noqa: F401
    except ImportError:
        req_path = os.path.join(os.path.dirname(__file__), "screen_requirements.txt")
        sys.stderr.write(f"[mlx-screen] Installing dependencies from {req_path}...\n")
        subprocess.check_call([
            sys.executable, "-m", "pip", "install",
            "-r", req_path, "--quiet",
            "--user", "--break-system-packages",
        ])


ensure_deps()

from mlx_vlm import load, generate  # noqa: E402
from mlx_vlm.prompt_utils import apply_chat_template  # noqa: E402


def load_model():
    sys.stderr.write(f"[mlx-screen] Loading model {MODEL}...\n")
    model, processor = load(MODEL)
    sys.stderr.write(f"[mlx-screen] Model loaded.\n")
    return model, processor


def _clean_output(raw):
    """Post-process model output: strip thinking, reasoning artifacts, and repetition loops."""
    text = raw
    # Strip everything before </think> (the model's hidden reasoning)
    if "</think>" in text:
        text = text.split("</think>")[-1]
    # Strip <think>...</think> blocks that didn't close
    text = re.sub(r"<think>.*", "", text, flags=re.DOTALL)
    # Strip inline reasoning lines
    reasoning_patterns = [
        r"(?m)^.*(?:The user wants|Step \d|Wait,|Actually,|Let me|Looking at|I need to|No,|Ah wait|I see|So,|Let's|Hmm).*$\n?",
        r"(?m)^\s*\*\s+(?:Tab \d|Left side|Right side|Next to|Then there|Address Bar).*$\n?",
    ]
    for pattern in reasoning_patterns:
        text = re.sub(pattern, "", text)
    # Strip bold markdown
    text = re.sub(r"\*\*([^*]+)\*\*", r"\1", text)
    # Collapse repeated lines (2+ consecutive duplicates → single)
    text = re.sub(r"(?m)(^.+$)(\n\1)+", r"\1", text)
    # Collapse repeated quoted phrases
    text = re.sub(r'("[^"]{2,}"\n?){3,}', lambda m: m.group(0).split("\n")[0] + "\n", text)
    # Clean up
    text = re.sub(r"\n{3,}", "\n\n", text)
    text = re.sub(r"^\s*\*\s*$", "", text, flags=re.MULTILINE)
    return text.strip()


def analyze_screenshots(model, processor, image_paths):
    descriptions = []
    for i, path in enumerate(image_paths):
        if not os.path.exists(path):
            descriptions.append(f"[Display {i+1}: file not found: {path}]")
            continue

        prompt = SYSTEM_PROMPT + "\n\n" + USER_PROMPT

        formatted = apply_chat_template(
            processor,
            config=model.config,
            prompt=prompt,
            num_images=1,
            enable_thinking=False,
        )

        result = generate(
            model,
            processor,
            formatted,
            image=path,
            max_tokens=768,
            verbose=False,
            repetition_penalty=1.5,
            temp=0.1,
            kv_bits=4,
            quantized_kv_start=0,
        )

        raw = result.text if hasattr(result, "text") else str(result).strip()
        text = _clean_output(raw)
        if len(image_paths) > 1:
            descriptions.append(f"[Display {i+1}] {text}")
        else:
            descriptions.append(text)

    return "\n\n".join(descriptions)


def main():
    model, processor = load_model()

    sys.stdout.write(json.dumps({
        "status": "ready",
        "model": MODEL,
    }) + "\n")
    sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            paths = req.get("paths", [])
            if not paths:
                sys.stdout.write(json.dumps({"error": "no paths provided"}) + "\n")
                sys.stdout.flush()
                continue

            text = analyze_screenshots(model, processor, paths)
            sys.stdout.write(json.dumps({"text": text}) + "\n")
            sys.stdout.flush()
        except Exception as e:
            sys.stdout.write(json.dumps({"error": str(e)}) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
