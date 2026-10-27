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
