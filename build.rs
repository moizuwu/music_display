//! Puts the version and the git state this was built from into `MD_VERSION`, shown by
//! `--version` and in the settings menu: `0.1.0 (a1b2c3d)`, or `0.1.0 (a1b2c3d, dirty)` when the
//! working tree had uncommitted changes. Nothing here edits a tracked file.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // `--always` falls back to the bare hash when there is no tag at all
    let described = git(&["describe", "--tags", "--dirty", "--always", "--abbrev=7"]);
    let dirty = described.as_deref().is_some_and(|d| d.ends_with("-dirty"));
    let full = match git(&["rev-parse", "--short=7", "HEAD"]) {
        Some(h) => format!("{version} ({h}{})", if dirty { ", dirty" } else { "" }),
        None => version, // no .git (a source archive): the number alone
    };
    println!("cargo:rustc-env=MD_VERSION={full}");
    // a new commit, checkout or staged change; and edits to the files `dirty` looks at
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        for f in ["HEAD", "logs/HEAD", "index"] {
            println!("cargo:rerun-if-changed={dir}/{f}");
        }
    }
    for p in ["src", "Cargo.toml", "Cargo.lock", "build.rs"] {
        println!("cargo:rerun-if-changed={p}");
    }
}
