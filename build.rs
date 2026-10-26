//! Puts the version and the git commit this was built from into `MD_VERSION`, shown by
//! `--version` and in the settings menu.
use std::process::Command;

fn main() {
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let full = match commit {
        Some(h) => format!("{version} ({h})"),
        None => version,
    };
    println!("cargo:rustc-env=MD_VERSION={full}");
    // a new commit or checkout changes the hash
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/logs/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
}
