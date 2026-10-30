//! Starting and stopping spotifyd for `use_spotifyd`.

use std::process::{Child, Command, Stdio};

/// Is a spotifyd process running (ours or not)?
#[cfg(windows)]
pub fn running() -> bool {
    Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq spotifyd.exe", "/NH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("spotifyd.exe"))
}

#[cfg(not(windows))]
pub fn running() -> bool {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .any(|e| std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "spotifyd"))
}

/// Start spotifyd in the foreground with its output silenced. `None` when one is already
/// running (it is left alone, so it is never killed either) or the binary cannot be started.
pub fn start() -> Option<Child> {
    // tests flip `use_spotifyd` and must not start a real daemon
    if crate::util::testing() || running() {
        return None;
    }
    Command::new("spotifyd")
        .arg("--no-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Stop a spotifyd this program started.
pub fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A spotifyd this program started for `use_spotifyd`; stopped when dropped (unless
/// `no_kill_spotifyd`), so every way out of a front end cleans up.
pub struct Daemon {
    child: Option<Child>,
    kill: bool,
}

impl Daemon {
    /// Start spotifyd when `on` (nothing happens, and nothing is killed later, if one already runs).
    pub fn start(on: bool, kill_on_exit: bool) -> Self {
        Daemon { child: if on { start() } else { None }, kill: kill_on_exit }
    }

    /// Did this program start the daemon it holds?
    pub fn started(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let (true, Some(c)) = (self.kill, self.child.as_mut()) {
            stop(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_started_when_off_or_in_tests() {
        crate::util::enter_test_mode();
        assert!(!Daemon::start(false, true).started());
        assert!(!Daemon::start(true, true).started(), "tests never start a real daemon");
    }
}
