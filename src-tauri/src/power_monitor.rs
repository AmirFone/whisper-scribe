use std::sync::Arc;
use std::sync::atomic::Ordering;

pub struct PowerMonitor {
    _handle: std::thread::JoinHandle<()>,
}

impl PowerMonitor {
    pub fn new(state: Arc<crate::AppState>) -> Result<Self, String> {
        let handle = std::thread::spawn(move || run_power_monitor(state));
        Ok(Self { _handle: handle })
    }
}

#[cfg(target_os = "macos")]
fn run_power_monitor(state: Arc<crate::AppState>) {
    use std::process::Command;

    log::info!("Power monitor started");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(15));

        let is_locked = Command::new("ioreg")
            .args(["-r", "-k", "CGSSessionScreenIsLocked"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).contains("CGSSessionScreenIsLocked\" = Yes"))
            .unwrap_or(false);

        if is_locked && !*state.is_paused.lock() {
            log::info!("Screen locked — pausing");
            *state.is_paused.lock() = true;
            state.paused_by_system.store(true, Ordering::Relaxed);
        } else if !is_locked && state.paused_by_system.load(Ordering::Relaxed) {
            // Only auto-resume if WE paused it, not the user
            log::info!("Screen unlocked — resuming (system-paused)");
            *state.is_paused.lock() = false;
            state.paused_by_system.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn run_power_monitor(_state: Arc<crate::AppState>) {}

#[cfg(test)]
mod tests {
    #[test]
    fn test_power_monitor_compiles() { assert!(true); }
}
