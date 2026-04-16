use crate::state::{AppState, PauseReason};
use std::sync::Arc;

pub struct PowerMonitor {
    _handle: std::thread::JoinHandle<()>,
}

impl PowerMonitor {
    pub fn new(state: Arc<AppState>) -> Self {
        let handle = std::thread::spawn(move || run_power_monitor(state));
        Self { _handle: handle }
    }
}

#[cfg(target_os = "macos")]
fn run_power_monitor(state: Arc<AppState>) {
    use std::process::Command;

    log::info!("Power monitor started");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(15));

        let is_locked = Command::new("ioreg")
            .args(["-r", "-k", "CGSSessionScreenIsLocked"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).contains("CGSSessionScreenIsLocked\" = Yes"))
            .unwrap_or(false);

        let current = state.pause_reason();

        match (current, is_locked) {
            // Lock detected and not currently paused → system pause
            (PauseReason::None, true) => {
                log::info!("Screen locked — pausing");
                state.set_pause(PauseReason::System);
            }
            // Unlock detected and we set the pause → resume
            (PauseReason::System, false) => {
                log::info!("Screen unlocked — resuming (was system-paused)");
                state.set_pause(PauseReason::None);
            }
            // Manual pause — never auto-resume even on unlock; never overwrite on lock.
            // System pause persisting through lock — leave alone.
            // No pause and no lock — leave alone.
            _ => {}
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn run_power_monitor(_state: Arc<AppState>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, PauseReason};
    use crate::storage::Storage;
    use tempfile::TempDir;

    fn fake_state() -> (Arc<AppState>, TempDir) {
        let dir = TempDir::new().unwrap();
        let storage = Storage::new(&dir.path().join("t.db")).unwrap();
        (Arc::new(AppState::new(storage)), dir)
    }

    /// Mirror of the decision tree inside `run_power_monitor`, kept in sync
    /// manually — the OS-specific loop can't be exercised here directly.
    fn apply(state: &AppState, is_locked: bool) {
        match (state.pause_reason(), is_locked) {
            (PauseReason::None, true) => state.set_pause(PauseReason::System),
            (PauseReason::System, false) => state.set_pause(PauseReason::None),
            _ => {}
        }
    }

    #[test]
    fn test_manual_pause_survives_lock_unlock_cycle() {
        // #given a user-paused state
        let (state, _dir) = fake_state();
        state.set_pause(PauseReason::Manual);

        // #when the screen locks then unlocks
        apply(&state, true);
        assert_eq!(
            state.pause_reason(),
            PauseReason::Manual,
            "manual pause must persist through lock"
        );

        apply(&state, false);

        // #then manual pause still holds
        assert_eq!(
            state.pause_reason(),
            PauseReason::Manual,
            "manual pause must persist through unlock"
        );
        assert!(state.is_paused());
    }

    #[test]
    fn test_system_pause_resumes_on_unlock() {
        // #given a system-paused state
        let (state, _dir) = fake_state();
        state.set_pause(PauseReason::System);

        // #when the screen unlocks
        apply(&state, false);

        // #then pause is cleared and the fast-path flag sees it
        assert_eq!(state.pause_reason(), PauseReason::None);
        assert!(!state.is_paused());
    }
}
