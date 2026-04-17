use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const MAX_LINE_BYTES: u64 = 512 * 1024;

struct DaemonHandle {
    child: Child,
    stdin: std::process::ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
}

pub struct ScreenAnalyzer {
    daemon: Mutex<Option<DaemonHandle>>,
}

impl ScreenAnalyzer {
    pub fn new() -> Result<Self, String> {
        let daemon = spawn_daemon()?;
        Ok(Self {
            daemon: Mutex::new(Some(daemon)),
        })
    }

    pub fn analyze(&self, screenshot_paths: &[PathBuf]) -> Result<String, String> {
        let paths_str: Vec<&str> = screenshot_paths
            .iter()
            .filter_map(|p| p.to_str())
            .collect();

        let request = serde_json::json!({ "paths": paths_str });

        let mut guard = self.daemon.lock();

        if guard.is_none() {
            log::warn!("Screen analyzer daemon was dead, attempting restart...");
            match spawn_daemon() {
                Ok(d) => *guard = Some(d),
                Err(e) => return Err(format!("Daemon restart failed: {e}")),
            }
        }

        let daemon = guard.as_mut().ok_or("No daemon")?;

        if let Err(e) = writeln!(daemon.stdin, "{}", request) {
            log::error!("Screen daemon stdin write failed: {e} — killing for restart");
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

        let mut response_line = String::new();
        match read_line_bounded(&mut daemon.reader, &mut response_line, MAX_LINE_BYTES) {
            Ok(0) => {
                log::error!("Screen daemon returned empty response — likely crashed");
                if let Some(d) = guard.take() {
                    shutdown_daemon(d);
                }
                return Err("Daemon crashed (empty response)".to_string());
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("Screen daemon read failed: {e} — killing for restart");
                if let Some(d) = guard.take() {
                    shutdown_daemon(d);
                }
                return Err(format!("Daemon read failed: {e}"));
            }
        }

        let response: serde_json::Value = serde_json::from_str(&response_line)
            .map_err(|e| format!("Invalid JSON from screen daemon: {e}: {response_line}"))?;

        if let Some(err) = response.get("error").and_then(|e| e.as_str()) {
            return Err(format!("Screen analysis error: {err}"));
        }

        let text = response
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        Ok(text)
    }
}

impl Drop for ScreenAnalyzer {
    fn drop(&mut self) {
        if let Some(d) = self.daemon.lock().take() {
            shutdown_daemon(d);
        }
    }
}

fn shutdown_daemon(mut d: DaemonHandle) {
    drop(d.stdin);
    d.child.kill().ok();
    match d.child.wait() {
        Ok(status) => log::info!("Screen daemon reaped: exit={status}"),
        Err(e) => log::warn!("Screen daemon wait failed: {e}"),
    }
}

fn spawn_daemon() -> Result<DaemonHandle, String> {
    let script_path = find_screen_script()?;
    let python = find_python()?;

    log::info!(
        "Starting screen analyzer daemon: {python} {}",
        script_path.display()
    );

    let mut child = Command::new(&python)
        .arg(&script_path)
        .env("PYTHONUNBUFFERED", "1")
        .env("TQDM_DISABLE", "1")
        .env("HF_HUB_DISABLE_PROGRESS_BARS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn screen daemon: {e}"))?;

    let stdin = child.stdin.take().ok_or("No stdin")?;
    let stdout = child.stdout.take().ok_or("No stdout")?;
    let stderr = child.stderr.take().ok_or("No stderr")?;

    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            log::warn!("[mlx-screen] {line}");
        }
    });

    let mut reader = BufReader::new(stdout);

    let mut ready_line = String::new();
    read_line_bounded(&mut reader, &mut ready_line, MAX_LINE_BYTES)
        .map_err(|e| format!("Screen daemon ready failed: {e}"))?;

    let ready: serde_json::Value = serde_json::from_str(&ready_line)
        .map_err(|e| format!("Invalid ready JSON: {e}: {ready_line}"))?;

    if ready.get("status").and_then(|s| s.as_str()) != Some("ready") {
        return Err(format!("Screen daemon not ready: {ready_line}"));
    }

    log::info!("Screen analyzer daemon ready");
    Ok(DaemonHandle {
        child,
        stdin,
        reader,
    })
}

fn read_line_bounded<R: BufRead>(
    reader: &mut R,
    buf: &mut String,
    max_bytes: u64,
) -> std::io::Result<usize> {
    let mut total: u64 = 0;
    let mut line_bytes: Vec<u8> = Vec::new();

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }

        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            let take = pos + 1;
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

fn find_python() -> Result<String, String> {
    if let Ok(explicit) = std::env::var("WSCRIBE_PYTHON") {
        if python_version_ok(&explicit) {
            return Ok(explicit);
        }
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
            return Ok(p.to_string());
        }
    }
    Err("No Python >=3.10 found".to_string())
}

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

fn find_screen_script() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(Path::new("."));
        for relative in &[
            "../Resources/scripts/mlx_screen_analyze.py",
            "../Resources/mlx_screen_analyze.py",
            "../../scripts/mlx_screen_analyze.py",
            "../../../src-tauri/scripts/mlx_screen_analyze.py",
        ] {
            let p = dir.join(relative);
            if p.exists() {
                return Ok(p.canonicalize().unwrap_or(p));
            }
        }
    }
    for p in &[
        "scripts/mlx_screen_analyze.py",
        "src-tauri/scripts/mlx_screen_analyze.py",
    ] {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
    }
    Err("mlx_screen_analyze.py not found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_line_bounded_normal() {
        // #given a short JSON line
        let mut reader = Cursor::new(b"{\"text\": \"hello\"}\n".to_vec());
        let mut buf = String::new();

        // #when we read
        let n = read_line_bounded(&mut reader, &mut buf, 1024).unwrap();

        // #then content matches
        assert_eq!(n, 18);
        assert!(buf.contains("hello"));
    }

    #[test]
    fn test_read_line_bounded_eof() {
        let mut reader = Cursor::new(Vec::<u8>::new());
        let mut buf = String::new();
        let n = read_line_bounded(&mut reader, &mut buf, 1024).unwrap();
        assert_eq!(n, 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_read_line_bounded_rejects_oversize() {
        // #given a line way beyond the cap
        let data = vec![b'x'; 2048];
        let mut reader = Cursor::new(data);
        let mut buf = String::new();

        // #when bounded to 100
        let err = read_line_bounded(&mut reader, &mut buf, 100).unwrap_err();

        // #then we get InvalidData
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_read_line_bounded_exact_cap() {
        // #given a line exactly at the cap
        const CAP: u64 = 64;
        let mut data = vec![b'a'; (CAP - 1) as usize];
        data.push(b'\n');
        let mut reader = std::io::BufReader::new(Cursor::new(data));
        let mut buf = String::new();

        // #when we read
        let n = read_line_bounded(&mut reader, &mut buf, CAP).unwrap();

        // #then it's accepted
        assert_eq!(n as u64, CAP);
        assert!(buf.ends_with('\n'));
    }

    #[test]
    fn test_read_line_bounded_multiple_lines() {
        // #given two JSON lines
        let data = b"{\"status\":\"ready\"}\n{\"text\":\"result\"}\n".to_vec();
        let mut reader = std::io::BufReader::new(Cursor::new(data));

        // #when we read first line
        let mut buf1 = String::new();
        read_line_bounded(&mut reader, &mut buf1, 1024).unwrap();
        assert!(buf1.contains("ready"));

        // #then second line reads correctly
        let mut buf2 = String::new();
        read_line_bounded(&mut reader, &mut buf2, 1024).unwrap();
        assert!(buf2.contains("result"));
    }

    #[test]
    fn test_read_line_bounded_fragmented_delivery() {
        // #given a reader with small buffer simulating chunked I/O
        let data = b"{\"text\": \"screen analysis\"}\n".to_vec();
        let mut reader = std::io::BufReader::with_capacity(4, Cursor::new(data));
        let mut buf = String::new();

        // #when we read
        let n = read_line_bounded(&mut reader, &mut buf, 1024).unwrap();

        // #then reassembly works
        assert!(n > 0);
        assert!(buf.contains("screen analysis"));
    }

    #[test]
    fn test_read_line_bounded_invalid_utf8() {
        // #given a line with invalid UTF-8
        let data = vec![0xFF, 0xFE, b'\n'];
        let mut reader = Cursor::new(data);
        let mut buf = String::new();

        // #when we read
        let err = read_line_bounded(&mut reader, &mut buf, 1024).unwrap_err();

        // #then we get InvalidData
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
