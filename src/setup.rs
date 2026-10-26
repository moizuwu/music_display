//! `music_display --setup`: one-time guided setup of spotifyd and the Spotify login.
//!
//! Nothing here runs unless you ask for it. The steps, in order: check that `spotifyd` is
//! installed (else print how to get it and stop, changing nothing), run the interactive
//! `spotifyd authenticate` in this terminal, log in to Spotify in the browser (PKCE), write a
//! minimal `spotifyd.conf` and the config entries, then check that spotifyd runs and that the
//! device shows up in Spotify. Every existing file is backed up as `<name>.bak-<timestamp>`
//! before it is replaced, and is never replaced without a yes. No tokens or credentials are
//! printed or written by this module (the login keeps its token in its own private file).
//!
//! Everything that touches the outside world goes through [`Steps`], so the tests run it with
//! a fake spotifyd, a fake HTTP site and a scratch home directory.

use crate::util::now_secs;
use anyhow::{Result, bail};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Suggested device name when the config has none.
pub const DEFAULT_DEVICE: &str = "music_display";
const SPOTIFYD_URL: &str = "https://github.com/Spotifyd/spotifyd";
/// How often the device list is asked for while spotifyd registers itself with Spotify.
const READY_TRIES: u32 = 5;
const READY_WAIT_SECS: u64 = 2;

/// Where things live; filled in by [`Env::live`] and by the tests.
pub struct Env {
    pub config_path: PathBuf,
    pub spotifyd_conf: PathBuf,
    /// Contents of /etc/os-release, to pick the install hint.
    pub os_release: String,
    /// Backup suffix: seconds since the epoch.
    pub now: u64,
}

impl Env {
    pub fn live(config_path: PathBuf) -> Env {
        Env {
            config_path,
            spotifyd_conf: dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("spotifyd")
                .join("spotifyd.conf"),
            os_release: std::fs::read_to_string("/etc/os-release").unwrap_or_default(),
            now: now_secs(),
        }
    }
}

/// What the user gave us on the command line / in the config.
pub struct Inputs {
    pub client_id: Option<String>,
    pub device: String,
}

/// The outside world, replaceable in tests.
#[allow(async_fn_in_trait)]
pub trait Steps {
    /// Full path of an executable on PATH.
    fn which(&self, name: &str) -> Option<PathBuf>;
    /// Run a program attached to this terminal; `true` when it exited successfully.
    fn run_interactive(&self, prog: &Path, args: &[&str]) -> std::io::Result<bool>;
    /// Ask a question on the terminal; the answer, or `default` when it is empty / no terminal.
    fn ask(&self, question: &str, default: &str) -> String;
    fn spotifyd_running(&self) -> bool;
    /// The browser login (PKCE); saves the token itself.
    async fn login(&self, client_id: &str) -> Result<()>;
    /// `GET /me/player/devices`: (name, active).
    async fn devices(&self, client_id: &str) -> std::result::Result<Vec<(String, bool)>, String>;
    async fn wait(&self, secs: u64);
}

/// How it ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// spotifyd is not installed: the hint was printed, nothing changed.
    Missing,
    /// A step failed (or was refused) before the end; what was done is listed in the output.
    Stopped,
    /// Everything done; `ready` = spotifyd runs and its device is listed.
    Done { ready: bool },
}

/// The package hint for the detected distribution.
pub fn install_hint(os_release: &str) -> String {
    let field = |k: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim().trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let has = |n: &str| ids.split_whitespace().any(|i| i == n);
    // Artix names itself `artix` and sets no ID_LIKE, but it is Arch with pacman
    let cmd = if has("arch") || has("artix") {
        Some("pacman -S spotifyd")
    } else if has("debian") || has("ubuntu") {
        Some("apt install spotifyd")
    } else if has("fedora") || has("rhel") {
        Some("dnf install spotifyd")
    } else if has("suse") || has("opensuse") {
        Some("zypper install spotifyd")
    } else {
        None
    };
    match cmd {
        Some(c) => format!(
            "spotifyd is not installed. As root run:\n    {c}\nIf your release doesn't have the package, see {SPOTIFYD_URL}"
        ),
        None => format!("spotifyd is not installed. See {SPOTIFYD_URL} for packages and binaries."),
    }
}

/// `(name, active)` of every device in a `/me/player/devices` answer.
pub fn parse_devices(v: &Value) -> Vec<(String, bool)> {
    v["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| (d["name"].as_str().unwrap_or("").to_string(), d["is_active"].as_bool() == Some(true)))
        .collect()
}

/// A device name that is safe to write into a TOML string and to match later.
fn valid_device(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 100 && !name.chars().any(|c| c.is_control() || c == '"' || c == '\\')
}

/// The minimal spotifyd configuration. It holds no secrets: the credentials live in spotifyd's
/// own cache, written by `spotifyd authenticate`.
pub fn spotifyd_conf(device: &str) -> String {
    format!("[global]\ndevice_name = \"{device}\"\n")
}

/// Copy `path` to `<path>.bak-<timestamp>` (never over an existing backup); the backup's path.
pub fn backup(path: &Path, now: u64) -> std::io::Result<PathBuf> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut to = path.with_file_name(format!("{name}.bak-{now}"));
    let mut n = 1;
    while to.exists() {
        to = path.with_file_name(format!("{name}.bak-{now}-{n}"));
        n += 1;
    }
    std::fs::copy(path, &to)?;
    Ok(to)
}

fn yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "j" | "ja")
}

/// Replace (or create) `path` with `body` after a yes; `false` when the user kept the old file.
fn write_file(
    out: &mut dyn Write,
    steps: &impl Steps,
    env: &Env,
    path: &Path,
    what: &str,
    write: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<bool> {
    if path.exists() {
        let q = format!("{what} exists ({}). Replace it? A backup is made first [y/N]", path.display());
        if !yes(&steps.ask(&q, "n")) {
            writeln!(out, "  kept {} as it is", path.display())?;
            return Ok(false);
        }
        let b = backup(path, env.now)?;
        writeln!(out, "  backed up to {}", b.display())?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write(path)?;
    writeln!(out, "  wrote {}", path.display())?;
    Ok(true)
}

/// Run the whole setup.
pub async fn run(env: &Env, steps: &impl Steps, inputs: &Inputs, out: &mut dyn Write) -> Result<Outcome> {
    writeln!(out, "music_display setup (spotifyd + Spotify login)\n")?;

    // 1. spotifyd installed?
    let Some(spotifyd) = steps.which("spotifyd") else {
        writeln!(out, "{}", install_hint(&env.os_release))?;
        return Ok(Outcome::Missing);
    };
    writeln!(out, "[1/5] spotifyd found: {}", spotifyd.display())?;

    // Everything that is asked is asked now, before anything is started or written.
    let suggested = if valid_device(&inputs.device) { inputs.device.as_str() } else { DEFAULT_DEVICE };
    let device = steps.ask(&format!("Device name Spotify shows for this PC [{suggested}]"), suggested);
    let device = device.trim().to_string();
    if !valid_device(&device) {
        writeln!(out, "That device name can't be used (empty, too long, quotes or control characters). Nothing was changed.")?;
        return Ok(Outcome::Stopped);
    }
    let client_id = match inputs.client_id.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => c.to_string(),
        None => steps
            .ask("Client ID of your Spotify developer app (dashboard: developer.spotify.com; not a secret)", "")
            .trim()
            .to_string(),
    };
    if client_id.is_empty() || !client_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        writeln!(out, "No usable Client ID. Create an app at https://developer.spotify.com/dashboard first. Nothing was changed.")?;
        return Ok(Outcome::Stopped);
    }

    // 2. spotifyd's own login, interactive in this terminal.
    writeln!(out, "\n[2/5] spotifyd authenticate (follow its prompts)")?;
    match steps.run_interactive(&spotifyd, &["authenticate"]) {
        Ok(true) => {}
        Ok(false) => {
            writeln!(out, "`spotifyd authenticate` did not finish. Nothing was changed; run --setup again.")?;
            return Ok(Outcome::Stopped);
        }
        Err(e) => {
            writeln!(out, "Could not run spotifyd: {e}. Nothing was changed.")?;
            return Ok(Outcome::Stopped);
        }
    }

    // 3. our own login (browser, PKCE).
    writeln!(out, "\n[3/5] Spotify login in the browser")?;
    if let Err(e) = steps.login(&client_id).await {
        writeln!(out, "Login failed: {e}. spotifyd is authenticated; run --setup again to retry.")?;
        return Ok(Outcome::Stopped);
    }
    writeln!(out, "  logged in")?;

    // 4. the two files.
    writeln!(out, "\n[4/5] configuration files")?;
    let conf = spotifyd_conf(&device);
    let wrote_conf = write_file(out, steps, env, &env.spotifyd_conf, "spotifyd.conf", |p| std::fs::write(p, &conf))?;
    if !wrote_conf {
        writeln!(out, "  spotifyd needs this in it:\n{}", indent(&conf))?;
    }
    let entries: Vec<(&str, toml::Value)> = vec![
        ("spotify", true.into()),
        ("spotify_client_id", client_id.clone().into()),
        ("spotify_device", device.clone().into()),
        ("use_spotifyd", true.into()),
    ];
    let wrote_cfg = write_file(out, steps, env, &env.config_path, "config.toml", |p| crate::config::save(p, &entries))?;
    if !wrote_cfg {
        writeln!(out, "  music_display needs these in it:")?;
        // the Client ID is left out of the printout; it is yours to paste
        writeln!(out, "    spotify = true\n    spotify_device = \"{device}\"\n    use_spotifyd = true\n    spotify_client_id = \"<your Client ID>\"")?;
    }

    // 5. is it ready?
    writeln!(out, "\n[5/5] readiness")?;
    let running = steps.spotifyd_running();
    let mut listed = false;
    let mut problem = None;
    if running {
        writeln!(out, "  OK   spotifyd is running")?;
        for attempt in 0..READY_TRIES {
            match steps.devices(&client_id).await {
                Ok(d) if d.iter().any(|(n, _)| *n == device) => {
                    listed = true;
                    break;
                }
                Ok(_) => problem = None,
                Err(e) => problem = Some(e),
            }
            if attempt + 1 < READY_TRIES {
                steps.wait(READY_WAIT_SECS).await;
            }
        }
    } else {
        writeln!(out, "  MISSING   spotifyd is not running: start it (for example `spotifyd --no-daemon`, or enable its service) and run --setup again, or just start it")?;
    }
    if listed {
        writeln!(out, "  OK   \"{device}\" is listed in your Spotify devices")?;
    } else if running {
        match problem {
            Some(e) => writeln!(out, "  MISSING   could not read your devices: {e}")?,
            None => writeln!(
                out,
                "  MISSING   \"{device}\" is not in your Spotify devices yet. spotifyd must run with that device_name (spotifyd.conf); restart it and check again"
            )?,
        }
    }
    let ready = running && listed;
    writeln!(out, "\n{}", if ready { "Ready. Start music_display." } else { "Setup finished, but not everything is ready (see MISSING above)." })?;
    Ok(Outcome::Done { ready })
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
}

// ── the real world ─────────────────────────────────────────────────────────

pub struct Live;

impl Steps for Live {
    fn which(&self, name: &str) -> Option<PathBuf> {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    }

    fn run_interactive(&self, prog: &Path, args: &[&str]) -> std::io::Result<bool> {
        std::process::Command::new(prog).args(args).status().map(|s| s.success())
    }

    fn ask(&self, question: &str, default: &str) -> String {
        use std::io::{BufRead, IsTerminal};
        if !std::io::stdin().is_terminal() {
            return default.to_string();
        }
        print!("{question}: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(n) if n > 0 && !line.trim().is_empty() => line.trim().to_string(),
            _ => default.to_string(),
        }
    }

    fn spotifyd_running(&self) -> bool {
        std::fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .any(|e| std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "spotifyd"))
    }

    async fn login(&self, client_id: &str) -> Result<()> {
        crate::spotify::login(client_id).await
    }

    async fn devices(&self, client_id: &str) -> std::result::Result<Vec<(String, bool)>, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        crate::spotify::Api::new(http, client_id, "")
            .device_names()
            .await
            .map_err(|e| e.msg)
    }

    async fn wait(&self, secs: u64) {
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
    }
}

/// `--setup` from `main`: exit status 1 unless it got all the way through.
pub async fn run_live(args: &crate::cli::Args, config_path: PathBuf) -> Result<()> {
    let env = Env::live(config_path);
    let inputs = Inputs { client_id: args.spotify_client_id.clone(), device: args.spotify_device.clone() };
    let outcome = run(&env, &Live, &inputs, &mut std::io::stdout()).await?;
    match outcome {
        Outcome::Done { .. } => Ok(()),
        Outcome::Missing => bail!("spotifyd is not installed"),
        Outcome::Stopped => bail!("setup stopped"),
    }
}
