use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

pub struct TranscriptionResult {
    pub text: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub device: String,
    pub confidence: f32,
}

struct DaemonHandle {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

pub struct Transcriber {
    daemon: Mutex<Option<DaemonHandle>>,
}

impl Transcriber {
    pub fn new() -> Result<Self, String> {
        let daemon = spawn_daemon()?;
        Ok(Self {
            daemon: Mutex::new(Some(daemon)),
        })
    }

    pub fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult, String> {
        let start_time = extract_timestamp_from_path(audio_path);
        let samples = count_samples(audio_path)?;
        let duration_secs = samples as f64 / 16_000.0;
        let end_time = start_time + chrono::Duration::seconds(duration_secs as i64);

        let request = serde_json::json!({ "path": audio_path.to_str().unwrap_or("") });

        // Single lock covers both stdin write and stdout read — no interleaving possible
        let mut guard = self.daemon.lock();

        // If daemon died, try to restart it
        if guard.is_none() {
            log::warn!("MLX daemon was dead, attempting restart...");
            match spawn_daemon() {
                Ok(d) => *guard = Some(d),
                Err(e) => return Err(format!("Daemon restart failed: {e}")),
            }
        }

        let daemon = guard.as_mut().ok_or("No daemon")?;

        // Send request
        if let Err(e) = writeln!(daemon.stdin, "{}", request) {
            log::error!("Daemon stdin write failed: {e} — killing daemon for restart");
            *guard = None;
            return Err(format!("Daemon write failed: {e}"));
        }
        if let Err(e) = daemon.stdin.flush() {
            *guard = None;
            return Err(format!("Daemon flush failed: {e}"));
        }

        // Read response
        let mut response_line = String::new();
        if let Err(e) = daemon.reader.read_line(&mut response_line) {
            log::error!("Daemon read failed: {e} — killing daemon for restart");
            *guard = None;
            return Err(format!("Daemon read failed: {e}"));
        }

        if response_line.is_empty() {
            log::error!("Daemon returned empty response — likely crashed");
            *guard = None;
            return Err("Daemon crashed (empty response)".to_string());
        }

        let response: serde_json::Value = serde_json::from_str(&response_line)
            .map_err(|e| format!("Invalid JSON from daemon: {e}: {response_line}"))?;

        if let Some(err) = response.get("error").and_then(|e| e.as_str()) {
            return Err(format!("MLX error: {err}"));
        }

        // Check if segment was skipped (silence)
        if response.get("skipped").and_then(|s| s.as_bool()).unwrap_or(false) {
            log::info!("Segment skipped: {}", response.get("reason").and_then(|r| r.as_str()).unwrap_or("unknown"));
            return Ok(TranscriptionResult {
                text: String::new(),
                start_time,
                end_time,
                device: crate::device_manager::get_current_device_name(),
                confidence: 0.0,
            });
        }

        let text = response.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
        if response.get("filtered").and_then(|f| f.as_bool()).unwrap_or(false) {
            let raw = response.get("raw_text").and_then(|t| t.as_str()).unwrap_or("");
            log::info!("Hallucination filtered: '{raw}' → '{text}'");
        }
        let device = crate::device_manager::get_current_device_name();

        Ok(TranscriptionResult {
            text,
            start_time,
            end_time,
            device,
            confidence: 0.95,
        })
    }
}

impl Drop for Transcriber {
    fn drop(&mut self) {
        if let Some(mut d) = self.daemon.lock().take() {
            d.child.kill().ok();
        }
    }
}

fn spawn_daemon() -> Result<DaemonHandle, String> {
    let script_path = find_script()?;
    let python = find_python()?;

    log::info!("Starting MLX Whisper daemon: {python} {}", script_path.display());

    let mut child = Command::new(&python)
        .arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("Failed to spawn MLX daemon: {e}"))?;

    let stdin = child.stdin.take().ok_or("No stdin")?;
    let stdout = child.stdout.take().ok_or("No stdout")?;
    let mut reader = BufReader::new(stdout);

    let mut ready_line = String::new();
    reader.read_line(&mut ready_line).map_err(|e| format!("Daemon ready failed: {e}"))?;

    let ready: serde_json::Value = serde_json::from_str(&ready_line)
        .map_err(|e| format!("Invalid ready JSON: {e}: {ready_line}"))?;

    if ready.get("status").and_then(|s| s.as_str()) != Some("ready") {
        return Err(format!("Daemon not ready: {ready_line}"));
    }

    log::info!("MLX daemon ready");
    Ok(DaemonHandle { child, stdin, reader })
}

fn count_samples(path: &Path) -> Result<u64, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("WAV open: {e}"))?;
    Ok(reader.duration() as u64)
}

fn extract_timestamp_from_path(path: &Path) -> DateTime<Utc> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("segment_00000000_000000");
    let date_part = stem.strip_prefix("segment_").unwrap_or(stem);
    chrono::NaiveDateTime::parse_from_str(date_part, "%Y%m%d_%H%M%S")
        .map(|dt| dt.and_utc())
        .unwrap_or_else(|_| Utc::now())
}

fn find_python() -> Result<String, String> {
    for p in &["/opt/homebrew/bin/python3.14", "/opt/homebrew/bin/python3", "/usr/local/bin/python3", "python3"] {
        if Command::new(p).arg("--version").output().is_ok() {
            return Ok(p.to_string());
        }
    }
    Err("Python 3 not found".to_string())
}

fn find_script() -> Result<std::path::PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(Path::new("."));
        for relative in &[
            "../Resources/scripts/mlx_transcribe.py",
            "../Resources/mlx_transcribe.py",
            "../../scripts/mlx_transcribe.py",
            "../../../src-tauri/scripts/mlx_transcribe.py",
        ] {
            let p = dir.join(relative);
            if p.exists() {
                return Ok(p.canonicalize().unwrap_or(p));
            }
        }
    }
    for p in &["scripts/mlx_transcribe.py", "src-tauri/scripts/mlx_transcribe.py"] {
        let path = std::path::PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
    }
    Err("mlx_transcribe.py not found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_timestamp_valid() {
        let ts = extract_timestamp_from_path(Path::new("/tmp/segment_20240115_143022.wav"));
        assert_eq!(ts.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 14:30:22");
    }

    #[test]
    fn test_extract_timestamp_invalid() {
        let ts = extract_timestamp_from_path(Path::new("/tmp/garbage.wav"));
        assert!((Utc::now() - ts).num_seconds().abs() < 5);
    }
}
