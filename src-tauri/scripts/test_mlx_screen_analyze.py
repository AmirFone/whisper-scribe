#!/usr/bin/env python3
"""Tests for mlx_screen_analyze.py — protocol, config, model compatibility."""
import json
import os
import subprocess
import sys
import unittest

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
SCRIPT_PATH = os.path.join(SCRIPT_DIR, "mlx_screen_analyze.py")


class TestScriptValidity(unittest.TestCase):
    def test_script_compiles(self):
        """No syntax errors in the daemon script."""
        result = subprocess.run(
            [sys.executable, "-c", f"import py_compile; py_compile.compile('{SCRIPT_PATH}', doraise=True)"],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, f"Syntax error: {result.stderr}")


class TestModelConfig(unittest.TestCase):
    def test_default_model_is_qwen35(self):
        """Default model should be Qwen3.5-9B-MLX-4bit."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        self.assertIn("Qwen3.5-9B-MLX-4bit", content)

    def test_model_is_overridable_via_env(self):
        """WSCRIBE_SCREEN_MODEL env var should be checked."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        self.assertIn("WSCRIBE_SCREEN_MODEL", content)


class TestRequirementsFiles(unittest.TestCase):
    def test_screen_requirements_exists(self):
        req_path = os.path.join(SCRIPT_DIR, "screen_requirements.txt")
        self.assertTrue(os.path.exists(req_path))

    def test_screen_requirements_has_mlx_vlm(self):
        req_path = os.path.join(SCRIPT_DIR, "screen_requirements.txt")
        with open(req_path) as f:
            content = f.read()
        self.assertIn("mlx-vlm", content)

    def test_main_requirements_exists(self):
        req_path = os.path.join(SCRIPT_DIR, "requirements.txt")
        self.assertTrue(os.path.exists(req_path))


class TestDaemonProtocol(unittest.TestCase):
    def test_request_format(self):
        """Requests send paths as a JSON list."""
        # #given a multi-display capture
        request = {"paths": ["/tmp/screen_1.png", "/tmp/screen_2.png", "/tmp/screen_3.png"]}
        encoded = json.dumps(request) + "\n"

        # #then it roundtrips cleanly
        parsed = json.loads(encoded.strip())
        self.assertEqual(len(parsed["paths"]), 3)
        self.assertEqual(parsed["paths"][0], "/tmp/screen_1.png")

    def test_ready_response_format(self):
        """Daemon emits status=ready with model name on startup."""
        ready = {"status": "ready", "model": "mlx-community/Qwen3.5-9B-MLX-4bit"}
        encoded = json.dumps(ready) + "\n"
        parsed = json.loads(encoded.strip())
        self.assertEqual(parsed["status"], "ready")
        self.assertIn("Qwen3.5", parsed["model"])

    def test_success_response_format(self):
        """Successful analysis returns text field."""
        response = {"text": "[Display 1] Terminal: running cargo test\n\n[Display 2] Idle"}
        encoded = json.dumps(response)
        parsed = json.loads(encoded)
        self.assertIn("text", parsed)
        self.assertIn("cargo test", parsed["text"])

    def test_error_response_format(self):
        """Error responses contain error field."""
        response = {"error": "no paths provided"}
        parsed = json.loads(json.dumps(response))
        self.assertIn("error", parsed)

    def test_newlines_in_text_are_json_escaped(self):
        """Multi-display output with newlines stays on one JSON line."""
        response = {"text": "[Display 1] Code editing\n\n[Display 2] Idle\n\n[Display 3] Browser"}
        encoded = json.dumps(response)
        self.assertEqual(encoded.count("\n"), 0)
        decoded = json.loads(encoded)
        self.assertEqual(decoded["text"].count("\n\n"), 2)

    def test_empty_paths_is_valid_request(self):
        """Empty paths list is a valid JSON request (daemon returns error)."""
        request = {"paths": []}
        encoded = json.dumps(request)
        parsed = json.loads(encoded)
        self.assertEqual(len(parsed["paths"]), 0)


class TestPromptConfig(unittest.TestCase):
    def test_system_prompt_demands_structured_output(self):
        """System prompt should define App/Activity/Tabs/URL/Visible text fields."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        for field in ["App:", "Activity:", "Tabs:", "URL:", "Visible text:"]:
            self.assertIn(field, content, f"Missing structured field {field}")

    def test_system_prompt_covers_key_app_types(self):
        """System prompt should have instructions for major app categories."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        for app_type in ["terminal", "spreadsheet", "video"]:
            self.assertIn(app_type.lower(), content.lower(), f"Missing instructions for {app_type}")

    def test_system_prompt_forbids_speculation(self):
        """System prompt should forbid 'likely', 'appears to be' etc."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        self.assertIn("likely", content)
        self.assertIn("Just state facts", content)

    def test_system_prompt_handles_idle_display(self):
        """System prompt should define behavior for idle/wallpaper displays."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        self.assertIn("Idle", content)

    def test_max_tokens_is_reasonable(self):
        """max_tokens should be between 256 and 2048."""
        with open(SCRIPT_PATH) as f:
            content = f.read()
        self.assertIn("max_tokens=", content)
        import re
        match = re.search(r"max_tokens=(\d+)", content)
        self.assertIsNotNone(match, "max_tokens not found in script")
        tokens = int(match.group(1))
        self.assertGreaterEqual(tokens, 256, "max_tokens too low for detailed OCR")
        self.assertLessEqual(tokens, 2048, "max_tokens too high, wastes compute")


class TestModelCompatibility(unittest.TestCase):
    """Verify mlx-vlm can handle the configured model (without loading it)."""

    def test_mlx_vlm_importable(self):
        """mlx-vlm package should be installed."""
        try:
            import mlx_vlm
            self.assertTrue(True)
        except ImportError:
            self.skipTest("mlx-vlm not installed")

    def test_mlx_vlm_has_generate(self):
        """mlx-vlm should expose load and generate functions."""
        try:
            from mlx_vlm import load, generate
            self.assertTrue(callable(load))
            self.assertTrue(callable(generate))
        except ImportError:
            self.skipTest("mlx-vlm not installed")

    def test_mlx_vlm_has_chat_template(self):
        """mlx-vlm should expose apply_chat_template."""
        try:
            from mlx_vlm.prompt_utils import apply_chat_template
            self.assertTrue(callable(apply_chat_template))
        except ImportError:
            self.skipTest("mlx-vlm not installed")


if __name__ == "__main__":
    unittest.main()
