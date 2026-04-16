use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Upper bound on a single JSON-line response from the daemon. Anything
/// bigger is an indicator the daemon is spewing tracebacks or progress bars
/// to stdout — kill the daemon instead of growing a line buffer until OOM.
/// 256 KB is ~2500 words of text, which is far more than any realistic
/// transcription result.
const MAX_LINE_BYTES: u64 = 256 * 1024;

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
        // Canonical segment filenames encode the capture time; anything else
        // (test fixtures, user-dropped files) is rejected here rather than
        // silently stamped with `Utc::now()` — a random timestamp breaks the
        // orphan-dedup roundtrip and leaves `end_time` in the future.
        let start_time = extract_timestamp_from_path(audio_path).ok_or_else(|| {
            format!(
                "Cannot parse capture time from path: {} (expected segment_YYYYMMDD_HHMMSS.wav)",
                audio_path.display()
            )
        })?;
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
            if let Some(d) = guard.take() {
                shutdown_daemon(d);
            }
            return Err(format!("Daemon write failed: {e}"));
        }
        if let Err(e) = daemon.stdin.flush() {
            if let Some(d) = guard.take() {
                shutdown_daemon(d);
            }
            return Err(format!("Daemon flush failed: {e}"));
        }

        // Read response with a hard byte cap so a runaway daemon can't OOM
        // the host. A well-formed response is a single JSON line terminated
        // by `\n`; anything past MAX_LINE_BYTES without a newline is killed.
        let mut response_line = String::new();
        match read_line_bounded(&mut daemon.reader, &mut response_line, MAX_LINE_BYTES) {
            Ok(0) => {
                log::error!("Daemon returned empty response — likely crashed");
                if let Some(d) = guard.take() {
                    shutdown_daemon(d);
                }
                return Err("Daemon crashed (empty response)".to_string());
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("Daemon read failed: {e} — killing daemon for restart");
                if let Some(d) = guard.take() {
                    shutdown_daemon(d);
                }
                return Err(format!("Daemon read failed: {e}"));
            }
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
        if let Some(d) = self.daemon.lock().take() {
            shutdown_daemon(d);
        }
    }
}

/// Reap a daemon child that's being discarded. SIGKILL alone leaves a zombie
/// on Unix until the parent calls `waitpid`; without the wait, every daemon
/// restart (on crash, on `Transcriber::drop`, on `transcribe` errors)
/// accumulates a zombie PID for the lifetime of the host process. Dropping
/// `stdin` also helps unblock the stderr-forwarder thread by closing the
/// pipe the child writes to.
fn shutdown_daemon(mut d: DaemonHandle) {
    drop(d.stdin);
    d.child.kill().ok();
    match d.child.wait() {
        Ok(status) => log::info!("MLX daemon reaped: exit={status}"),
        Err(e) => log::warn!("MLX daemon wait failed: {e}"),
    }
}

fn spawn_daemon() -> Result<DaemonHandle, String> {
    let script_path = find_script()?;
    let python = find_python()?;

    log::info!("Starting MLX Whisper daemon: {python} {}", script_path.display());

    // `stderr` is piped (not inherited) so it stays visible in a packaged
    // .app bundle where the host's stderr has no destination. A background
    // thread forwards each line to `log::warn!` so Python tracebacks, pip
    // progress, and VAD warnings are reachable via `log::` plumbing.
    //
    // `TQDM_DISABLE`/`HF_HUB_DISABLE_PROGRESS_BARS` defend the stdout JSON
    // framing — any library that shifts progress to stdout would otherwise
    // corrupt the line-delimited protocol.
    let mut child = Command::new(&python)
        .arg(&script_path)
        .env("PYTHONUNBUFFERED", "1")
        .env("TQDM_DISABLE", "1")
        .env("HF_HUB_DISABLE_PROGRESS_BARS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn MLX daemon: {e}"))?;

    let stdin = child.stdin.take().ok_or("No stdin")?;
    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            log::warn!("[mlx-py] {line}");
        }
    });

    let mut reader = BufReader::new(stdout);

    let mut ready_line = String::new();
    read_line_bounded(&mut reader, &mut ready_line, MAX_LINE_BYTES)
        .map_err(|e| format!("Daemon ready failed: {e}"))?;

    let ready: serde_json::Value = serde_json::from_str(&ready_line)
        .map_err(|e| format!("Invalid ready JSON: {e}: {ready_line}"))?;

    if ready.get("status").and_then(|s| s.as_str()) != Some("ready") {
        return Err(format!("Daemon not ready: {ready_line}"));
    }

    log::info!("MLX daemon ready");
    Ok(DaemonHandle { child, stdin, reader })
}

/// Read a `\n`-terminated line from `reader` into `buf` (appending), refusing
/// to grow beyond `max_bytes`. Returns the total bytes read (0 = EOF).
///
/// Implemented against `BufRead::fill_buf`/`consume` directly rather than
/// wrapping the reader in `reader.take(max_bytes).read_until(b'\n', …)`.
/// The `Take`-wrapper approach had a latent bug: `read_until` consumes bytes
/// from the underlying `BufReader` up to the limit regardless of whether a
/// newline was found, so a near-cap line that happens to be exactly
/// `max_bytes - 1` bytes (no terminator) would leave the reader positioned
/// mid-protocol for the NEXT call with no error raised. Here the cap is
/// enforced BEFORE we consume, so violations are surfaced loudly and the
/// buffer's unconsumed tail is reclaimable by the caller (usually by killing
/// the daemon — it doesn't matter what's in the pipe).
fn read_line_bounded<R: BufRead>(reader: &mut R, buf: &mut String, max_bytes: u64) -> std::io::Result<usize> {
    let mut total: u64 = 0;
    let mut line_bytes: Vec<u8> = Vec::new();

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break; // EOF
        }

        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            let take = pos + 1; // include the terminating newline
            if total + take as u64 > max_bytes {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("daemon line exceeded {max_bytes} bytes"),
                ));
            }
            line_bytes.extend_from_slice(&available[..take]);
            reader.consume(take);
            total += take as u64;
            break;
        }

        let take = available.len();
        if total + take as u64 > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("daemon line exceeded {max_bytes} bytes"),
            ));
        }
        line_bytes.extend_from_slice(available);
        reader.consume(take);
        total += take as u64;
    }

    if total == 0 {
        return Ok(0);
    }

    match std::str::from_utf8(&line_bytes) {
        Ok(s) => {
            buf.push_str(s);
            Ok(total as usize)
        }
        Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
    }
}

fn count_samples(path: &Path) -> Result<u64, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("WAV open: {e}"))?;
    Ok(reader.duration() as u64)
}

fn extract_timestamp_from_path(path: &Path) -> Option<DateTime<Utc>> {
    crate::audio_dir::parse_segment_timestamp(path)
}

/// Find a Python interpreter >=3.10 (required by `mlx-whisper`). Candidates
/// are probed in priority order; `WSCRIBE_PYTHON` overrides everything.
/// The resolved path and version are logged at INFO so support issues are
/// diagnosable without attaching a debugger.
fn find_python() -> Result<String, String> {
    if let Ok(explicit) = std::env::var("WSCRIBE_PYTHON") {
        if python_version_ok(&explicit) {
            log::info!("Python: {explicit} (from WSCRIBE_PYTHON)");
            return Ok(explicit);
        }
        log::warn!("WSCRIBE_PYTHON={explicit} did not meet >=3.10 check — falling back");
    }
    for p in &[
        "/opt/homebrew/bin/python3.14",
        "/opt/homebrew/bin/python3.13",
        "/opt/homebrew/bin/python3.12",
        "/opt/homebrew/bin/python3.11",
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
        "python3",
    ] {
        if python_version_ok(p) {
            log::info!("Python: {p}");
            return Ok(p.to_string());
        }
    }
    Err("No Python >=3.10 found (mlx-whisper requires 3.10+)".to_string())
}

/// Return true iff the binary at `p` runs and reports Python >=3.10.
fn python_version_ok(p: &str) -> bool {
    Command::new(p)
        .args([
            "-c",
            "import sys; sys.exit(0 if sys.version_info >= (3,10) else 1)",
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    use std::io::Cursor;

    #[test]
    fn test_extract_timestamp_valid() {
        // #given a canonical segment filename
        let ts = extract_timestamp_from_path(Path::new("/tmp/segment_20240115_143022.wav"))
            .expect("canonical filename must parse");

        // #then the timestamp decodes to the encoded capture time
        assert_eq!(ts.format("%Y-%m-%d %H:%M:%S").to_string(), "2024-01-15 14:30:22");
    }

    #[test]
    fn test_extract_timestamp_invalid_returns_none() {
        // #given a non-canonical filename
        // #then extraction returns None rather than inventing Utc::now()
        //       (the `Utc::now()` fallback silently re-transcribed foreign
        //       WAVs on every restart with a different "now")
        assert!(extract_timestamp_from_path(Path::new("/tmp/garbage.wav")).is_none());
        assert!(extract_timestamp_from_path(Path::new("/tmp/segment_not_a_date.wav")).is_none());
    }

    #[test]
    fn test_read_line_bounded_normal() {
        // #given a short line within the cap
        let mut reader = Cursor::new(b"hello\n".to_vec());
        let mut buf = String::new();

        // #when we read bounded
        let n = read_line_bounded(&mut reader, &mut buf, 1024).unwrap();

        // #then the content matches and byte count includes the newline
        assert_eq!(n, 6);
        assert_eq!(buf, "hello\n");
    }

    #[test]
    fn test_read_line_bounded_empty_on_eof() {
        let mut reader = Cursor::new(Vec::<u8>::new());
        let mut buf = String::new();
        let n = read_line_bounded(&mut reader, &mut buf, 1024).unwrap();
        assert_eq!(n, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_read_line_bounded_rejects_oversize() {
        // #given a line larger than the cap, no newline
        let data = vec![b'x'; 1024];
        let mut reader = Cursor::new(data);
        let mut buf = String::new();

        // #when we bound to 100
        let err = read_line_bounded(&mut reader, &mut buf, 100).unwrap_err();

        // #then we get InvalidData rather than an OOM-prone infinite read
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_read_line_bounded_accepts_cap_with_newline() {
        // #given a payload of (cap - 1) bytes + '\n' = exactly cap total
        //       — this is the boundary case the Take-wrapper rewrite defends
        const CAP: u64 = 128;
        let mut data = vec![b'a'; (CAP - 1) as usize];
        data.push(b'\n');
        let mut reader = std::io::BufReader::new(Cursor::new(data));
        let mut buf = String::new();

        // #when we read with cap=128
        let n = read_line_bounded(&mut reader, &mut buf, CAP).unwrap();

        // #then we get the full cap, newline-terminated, with no error
        assert_eq!(n as u64, CAP);
        assert_eq!(buf.len() as u64, CAP);
        assert!(buf.ends_with('\n'));
    }

    #[test]
    fn test_read_line_bounded_survives_fill_buf_fragmentation() {
        // #given a reader that serves bytes in multiple fill_buf chunks
        // (BufReader of a small capacity simulates streaming delivery of
        // split packets from the daemon)
        let data = b"hello world\n".to_vec();
        let mut reader = std::io::BufReader::with_capacity(3, Cursor::new(data));
        let mut buf = String::new();

        // #when we read the line
        let n = read_line_bounded(&mut reader, &mut buf, 1024).unwrap();

        // #then the line reassembles across fill_buf boundaries
        assert_eq!(n, 12);
        assert_eq!(buf, "hello world\n");
    }

    #[test]
    fn test_read_line_bounded_leaves_trailing_bytes_untouched() {
        // #given two lines in the reader
        let data = b"first\nsecond\n".to_vec();
        let mut reader = std::io::BufReader::new(Cursor::new(data));

        // #when we read the first line
        let mut buf1 = String::new();
        read_line_bounded(&mut reader, &mut buf1, 1024).unwrap();

        // #and then the second
        let mut buf2 = String::new();
        read_line_bounded(&mut reader, &mut buf2, 1024).unwrap();

        // #then neither line leaked into the other
        assert_eq!(buf1, "first\n");
        assert_eq!(buf2, "second\n");
    }
}
