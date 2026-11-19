//! Spotify Soloist (`player = "soloist"`, `use_soloist`): Spotify's own headless Spotify Connect
//! player. It is started like spotifyd (`soloist --device-name … --api-key … --ws 127.0.0.1:0`)
//! and, unlike an MPRIS player, read and controlled through its local WebSocket API: the
//! playback state arrives as events (no polling, no Web API rate limits) and the player
//! commands (`play-pause`, `next`, `position 12`…, the playerctl words the rest of the program
//! uses) become Soloist commands. It runs next to spotifyd: both are Connect devices.
//!
//! Reference: developer.spotify.com/documentation/soloist (WebSocket API, command line).

use crate::AppEvent;
use crate::player::{Loop, Snapshot, Status};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The player name that selects Soloist (`--player soloist`, the settings menu).
pub const PLAYER: &str = "soloist";

// ── where it is ─────────────────────────────────────────────────────────

/// Soloist's data directory (device identity, session, and the `ws.addr` / `ws.port` files
/// it writes when the WebSocket API is on). The same rules as Soloist's own defaults.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Some(d) = std::env::var_os("STATE_DIRECTORY").filter(|d| !d.is_empty()) {
            return PathBuf::from(d);
        }
        if let Some(d) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            return PathBuf::from(d).join("soloist");
        }
        dirs::home_dir().unwrap_or_default().join(".local/share/soloist")
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().unwrap_or_default().join("Library/Application Support/soloist")
    }
    #[cfg(windows)]
    {
        dirs::data_local_dir().unwrap_or_default().join("soloist").join("data")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        dirs::data_dir().unwrap_or_default().join("soloist")
    }
}

/// `ws://ADDR:PORT` from the files a running Soloist wrote into `dir` (it removes them on exit).
pub fn endpoint(dir: &Path) -> Option<String> {
    let addr = std::fs::read_to_string(dir.join("ws.addr")).ok()?;
    let port = std::fs::read_to_string(dir.join("ws.port")).ok()?;
    let (addr, port) = (addr.trim(), port.trim());
    if addr.is_empty() || port.parse::<u16>().is_err() {
        return None;
    }
    // a wildcard bind is reached on the loopback address
    let host = match addr {
        "0.0.0.0" => "127.0.0.1",
        "::" | "[::]" => "[::1]",
        a => a,
    };
    Some(format!("ws://{host}:{port}"))
}

// ── the daemon ──────────────────────────────────────────────────────────

/// Is a Soloist running (ours or not)? Its pid file in the data directory, still alive.
pub fn running() -> bool {
    let Ok(pid) = std::fs::read_to_string(data_dir().join("soloist.pid")) else { return false };
    let pid = pid.trim();
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // no cheap check elsewhere: the WebSocket files are there only while it runs
        endpoint(&data_dir()).is_some()
    }
}

/// The command line that starts Soloist as a Connect device named `name`, with the WebSocket
/// API on a free local port (it writes the port to its data directory).
pub fn args(name: &str, key: &str) -> Vec<String> {
    ["--device-name", name, "--api-key", key, "--ws", "127.0.0.1:0"].iter().map(|s| s.to_string()).collect()
}

/// Start Soloist when `on`. `Err` says why not (no key, not installed); `Ok(None)` when one is
/// already running (it is left alone and never killed) or in tests.
pub fn start(name: &str, key_env: &str) -> Result<Option<Child>, String> {
    if crate::util::testing() || running() {
        return Ok(None);
    }
    let key = std::env::var(key_env).unwrap_or_default();
    if key.trim().is_empty() {
        return Err(format!("Soloist needs an API key: set the environment variable {key_env} (Spotify for Developers dashboard)"));
    }
    Command::new("soloist")
        .args(args(name, key.trim()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Some)
        .map_err(|e| format!("could not start soloist ({e}); is it installed and on PATH?"))
}

/// A Soloist this program started for `use_soloist`; stopped when dropped (unless
/// `no_kill_spotifyd`, which covers both daemons), like spotifyd's [`crate::spotifyd::Daemon`].
pub struct Daemon {
    child: Option<Child>,
    kill: bool,
    /// Why it was not started, for the note line.
    pub error: Option<String>,
}

impl Daemon {
    pub fn start(on: bool, name: &str, key_env: &str, kill_on_exit: bool) -> Self {
        if !on {
            return Daemon { child: None, kill: kill_on_exit, error: None };
        }
        match start(name, key_env) {
            Ok(child) => Daemon { child, kill: kill_on_exit, error: None },
            Err(e) => Daemon { child: None, kill: kill_on_exit, error: Some(e) },
        }
    }

    pub fn started(&self) -> bool {
        self.child.is_some()
    }

    /// Stop the Soloist this program started now (switched off in the settings).
    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let (true, Some(c)) = (self.kill, self.child.as_mut()) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ── the playback state ──────────────────────────────────────────────────

/// What Soloist last said, kept up to date from its events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub status: String,
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_artist: String,
    pub cover: String,
    pub duration_ms: f64,
    pub position_ms: f64,
    /// Soloist's clock (unix ms) when `position_ms` was true.
    pub timestamp_ms: f64,
    /// 1.0 while playing, 0.0 when the position stands.
    pub speed: f64,
    /// 0..=100
    pub volume: Option<f64>,
    pub shuffle: Option<bool>,
    pub repeat: Option<Loop>,
    pub logged_in: bool,
}

fn name_of(e: &Value) -> String {
    e["decorations"]["identity"]["name"].as_str().unwrap_or("").to_string()
}

/// The biggest cover: `xlarge` > `large` > `default` > `small`.
fn best_cover(e: &Value) -> String {
    let rank = |s: &str| ["small", "default", "large", "xlarge"].iter().position(|x| *x == s).unwrap_or(0);
    e["decorations"]["visual_identity"]["cover"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((rank(c["size"].as_str().unwrap_or("")), c["url"].as_str()?.to_string())))
        .max_by_key(|(r, _)| *r)
        .map(|(_, u)| u)
        .unwrap_or_default()
}

impl State {
    fn set_item(&mut self, item: &Value) {
        let d = &item["decorations"];
        self.uri = item["uri"].as_str().unwrap_or("").to_string();
        self.title = name_of(item);
        self.artists = d["creators"].as_array().into_iter().flatten().map(name_of).filter(|n| !n.is_empty()).collect();
        let album = &d["parent"]["entity"];
        self.album = name_of(album);
        self.album_artist = album["decorations"]["creators"].as_array().and_then(|c| c.first()).map(name_of).unwrap_or_default();
        self.cover = best_cover(item);
        if self.cover.is_empty() {
            self.cover = best_cover(album);
        }
        self.duration_ms = d["playback"]["duration_ms"].as_f64().unwrap_or(0.0);
    }

    fn set_position(&mut self, p: &Value) {
        if let Some(ms) = p["position_ms"].as_f64() {
            self.position_ms = ms;
            self.timestamp_ms = p["timestamp_ms"].as_f64().unwrap_or(0.0);
            self.speed = p["speed"].as_f64().unwrap_or(0.0);
        }
    }

    fn set_options(&mut self, o: &Value) {
        if let Some(s) = o["shuffle"].as_bool() {
            self.shuffle = Some(s);
        }
        self.repeat = match o["repeat"].as_str() {
            Some("off") => Some(Loop::None),
            Some("context") => Some(Loop::Playlist),
            Some("track") => Some(Loop::Track),
            _ => self.repeat,
        };
    }

    /// Take in one event. True when something the window shows changed.
    pub fn apply(&mut self, ev: &Value) -> bool {
        let before = self.clone();
        match ev["type"].as_str().unwrap_or("") {
            "playback_state" => {
                self.status = ev["status"].as_str().unwrap_or("idle").to_string();
                if ev["item"].is_object() {
                    self.set_item(&ev["item"]);
                } else {
                    *self = State { status: self.status.clone(), logged_in: self.logged_in, ..Default::default() };
                }
                self.set_position(&ev["position"]);
                self.volume = ev["volume"].as_f64();
                self.set_options(&ev["options"]);
            }
            "track_changed" => self.set_item(&ev["item"]),
            "playback_changed" => self.status = ev["status"].as_str().unwrap_or("idle").to_string(),
            "position_sync" => self.set_position(&ev["position"]),
            "volume_changed" => self.volume = ev["volume"].as_f64(),
            "options_changed" => self.set_options(&ev["options"]),
            "auth_state" => self.logged_in = ev["logged_in"].as_bool().unwrap_or(false),
            _ => {}
        }
        *self != before
    }

    /// The position now, run forward from the last sync (`now_ms`: unix ms).
    pub fn position_at(&self, now_ms: f64) -> f64 {
        let ms = if self.timestamp_ms > 0.0 { self.position_ms + (now_ms - self.timestamp_ms).max(0.0) * self.speed } else { self.position_ms };
        let ms = if self.duration_ms > 0.0 { ms.clamp(0.0, self.duration_ms) } else { ms.max(0.0) };
        ms / 1000.0
    }

    /// The window's now-playing data; `None` while nothing is loaded.
    pub fn snapshot(&self, now_ms: f64) -> Option<Snapshot> {
        if self.uri.is_empty() && self.title.is_empty() {
            return None;
        }
        Some(Snapshot {
            status: match self.status.as_str() {
                "playing" | "buffering" => Status::Playing,
                "paused" => Status::Paused,
                _ => Status::Stopped,
            },
            position: self.position_at(now_ms),
            title: self.title.clone(),
            artist: self.artists.join(", "),
            album_artist: self.album_artist.clone(),
            album: self.album.clone(),
            length: self.duration_ms / 1000.0,
            art_url: self.cover.clone(),
            track_no: None,
            disc_no: None,
            content_created: String::new(),
            genre: String::new(),
            volume: self.volume.map(|v| (v / 100.0).clamp(0.0, 1.0)),
            url: String::new(),
            // a `spotify:track:` URI: likes, the library and "Go to album" work with it
            track_id: self.uri.clone(),
            player: PLAYER.into(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        })
    }
}

// ── commands ────────────────────────────────────────────────────────────

fn cmd(name: &str) -> Value {
    json!({ "type": "command", "command": name })
}

/// A player command in playerctl words (what the rest of the program sends) as Soloist
/// commands. Repeat takes two (the order the reference gives); unknown words give none.
pub fn commands(args: &[&str], st: &State, now_ms: f64) -> Vec<Value> {
    let num = |s: &str| s.trim_end_matches(['+', '-']).parse::<f64>().ok();
    match args {
        ["play-pause"] => vec![cmd(if st.status == "playing" || st.status == "buffering" { "pause" } else { "play" })],
        ["play"] => vec![cmd("play")],
        ["pause"] | ["stop"] => vec![cmd("pause")],
        ["next"] => vec![cmd("skip_next")],
        ["previous"] => vec![cmd("skip_prev")],
        ["position", p] => {
            let Some(v) = num(p) else { return vec![] };
            let secs = if p.ends_with('+') {
                st.position_at(now_ms) + v
            } else if p.ends_with('-') {
                st.position_at(now_ms) - v
            } else {
                v
            };
            let mut c = cmd("seek");
            c["position_ms"] = json!((secs.max(0.0) * 1000.0).round() as u64);
            vec![c]
        }
        ["volume", v] => {
            let Some(x) = num(v) else { return vec![] };
            let cur = st.volume.unwrap_or(0.0) / 100.0;
            let to = if v.ends_with('+') {
                cur + x
            } else if v.ends_with('-') {
                cur - x
            } else {
                x
            };
            let mut c = cmd("set_volume");
            c["volume"] = json!((to.clamp(0.0, 1.0) * 100.0).round() as u64);
            vec![c]
        }
        ["shuffle", how] => {
            let on = match *how {
                "On" => true,
                "Off" => false,
                _ => !st.shuffle.unwrap_or(false),
            };
            let mut c = cmd("set_shuffle");
            c["enabled"] = json!(on);
            vec![c]
        }
        ["loop", how] => {
            let (track, context) = match *how {
                "Track" => (true, false),
                "Playlist" => (false, true),
                _ => (false, false),
            };
            let set = |name: &str, on: bool| {
                let mut c = cmd(name);
                c["enabled"] = json!(on);
                c
            };
            // the side being switched off goes first, so the two never fight
            if track {
                vec![set("set_repeat_context", false), set("set_repeat_track", true)]
            } else {
                vec![set("set_repeat_track", false), set("set_repeat_context", context)]
            }
        }
        // make this device the one that plays ("play here")
        ["activate"] => vec![cmd("activate")],
        _ => vec![],
    }
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

// ── the link ────────────────────────────────────────────────────────────

/// Keep a connection to Soloist while `on()` says so: every change becomes an
/// `AppEvent::Player`, every command from `cmds` goes out. Reconnects when Soloist (re)starts;
/// with no Soloist the window shows nothing playing.
pub async fn run(
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    cmds: tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    on: impl Fn() -> bool,
) {
    run_at(&data_dir(), tx, cmds, on).await
}

/// [`run`] with the Soloist data directory given (the tests point it at a fake server).
pub async fn run_at(
    dir: &Path,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    mut cmds: tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    on: impl Fn() -> bool,
) {
    use futures::{SinkExt, StreamExt};
    use std::time::Duration;
    let tick = || tokio::time::sleep(Duration::from_millis(500));
    loop {
        if !on() {
            // drop what was asked while Soloist was not the player
            while cmds.try_recv().is_ok() {}
            tick().await;
            continue;
        }
        let Some(url) = endpoint(dir) else {
            if tx.send(AppEvent::Player(None)).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let Ok((ws, _)) = tokio_tungstenite::connect_async(url.as_str()).await else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let (mut out, mut inp) = ws.split();
        let mut st = State::default();
        let mut keep = tokio::time::interval(Duration::from_secs(1));
        let text = |v: Value| tokio_tungstenite::tungstenite::Message::text(v.to_string());
        let _ = out.send(text(cmd("get_state"))).await;
        loop {
            tokio::select! {
                msg = inp.next() => {
                    let Some(Ok(msg)) = msg else { break };
                    let Ok(t) = msg.to_text() else { continue };
                    let Ok(ev) = serde_json::from_str::<Value>(t) else { continue };
                    if ev["type"] == "error" {
                        continue; // a refused command; the state events say what is true
                    }
                    if st.apply(&ev) && tx.send(AppEvent::Player(st.snapshot(now_ms()))).is_err() {
                        return;
                    }
                }
                c = cmds.recv() => {
                    let Some(c) = c else { return };
                    let args: Vec<&str> = c.iter().map(String::as_str).collect();
                    for m in commands(&args, &st, now_ms()) {
                        if out.send(text(m)).await.is_err() {
                            break;
                        }
                    }
                }
                // the position moves between syncs; and a player switch away from Soloist ends this
                _ = keep.tick() => {
                    if !on() {
                        break;
                    }
                    if tx.send(AppEvent::Player(st.snapshot(now_ms()))).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Value {
        json!({
            "uri": "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            "entity_type": "track",
            "decorations": {
                "identity": { "name": "City Lights" },
                "visual_identity": { "cover": [
                    { "url": "https://i/small", "size": "small" },
                    { "url": "https://i/large", "size": "large" },
                    { "url": "https://i/default", "size": "default" }
                ] },
                "parent": { "entity": { "uri": "spotify:album:a", "decorations": {
                    "identity": { "name": "Night Drive" },
                    "creators": [ { "decorations": { "identity": { "name": "The Example Band" } } } ]
                } } },
                "creators": [
                    { "decorations": { "identity": { "name": "The Example Band" } } },
                    { "decorations": { "identity": { "name": "Lena Park" } } }
                ],
                "playback": { "duration_ms": 241000 }
            }
        })
    }

    fn state_event() -> Value {
        json!({
            "type": "playback_state", "status": "playing", "item": track(), "context": {},
            "position": { "position_ms": 95000, "timestamp_ms": 1000000, "speed": 1.0 },
            "volume": 70, "is_active": true,
            "options": { "shuffle": false, "repeat": "context", "playback_speed": 1.0, "modes": {} }
        })
    }

    #[test]
    fn a_playback_state_becomes_the_windows_now_playing_data() {
        let mut st = State::default();
        assert!(st.apply(&state_event()));
        let s = st.snapshot(1_000_000.0).unwrap();
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.album.as_str()), ("City Lights", "The Example Band, Lena Park", "Night Drive"));
        assert_eq!(s.album_artist, "The Example Band");
        assert_eq!((s.status, s.length, s.position), (Status::Playing, 241.0, 95.0));
        assert_eq!(s.art_url, "https://i/large", "the biggest cover");
        assert_eq!(s.volume, Some(0.7));
        assert_eq!((s.shuffle, s.repeat), (Some(false), Some(Loop::Playlist)));
        assert_eq!(crate::spotify::track_uri(&s.track_id).as_deref(), Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC"));
        // the position runs on from the last sync, and stops at the end
        assert_eq!(st.snapshot(1_002_500.0).unwrap().position, 97.5);
        assert_eq!(st.snapshot(9_000_000.0).unwrap().position, 241.0);
        // nothing loaded: nothing to show
        assert!(State::default().snapshot(0.0).is_none());
    }

    #[test]
    fn the_small_events_update_what_they_name() {
        let mut st = State::default();
        st.apply(&state_event());
        assert!(st.apply(&json!({ "type": "playback_changed", "status": "paused" })));
        assert!(st.apply(&json!({ "type": "position_sync", "position": { "position_ms": 10000, "timestamp_ms": 5, "speed": 0.0 } })));
        assert!(st.apply(&json!({ "type": "volume_changed", "volume": 30 })));
        assert!(st.apply(&json!({ "type": "options_changed", "options": { "shuffle": true, "repeat": "track" } })));
        let s = st.snapshot(99_999.0).unwrap();
        assert_eq!((s.status, s.position, s.volume), (Status::Paused, 10.0, Some(0.3)), "speed 0: the position stands");
        assert_eq!((s.shuffle, s.repeat), (Some(true), Some(Loop::Track)));
        assert!(!st.apply(&json!({ "type": "volume_changed", "volume": 30 })), "no change, nothing to send");
        assert!(!st.apply(&json!({ "type": "something_new" })));
        // a state without an item: nothing is playing
        st.apply(&json!({ "type": "playback_state", "status": "idle" }));
        assert!(st.snapshot(0.0).is_none());
    }

    #[test]
    fn player_words_become_soloist_commands() {
        let mut st = State::default();
        st.apply(&state_event());
        let names = |args: &[&str]| commands(args, &st, 1_000_000.0).iter().map(|c| c["command"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&["play-pause"]), ["pause"], "playing: pause");
        assert_eq!(names(&["next"]), ["skip_next"]);
        assert_eq!(names(&["previous"]), ["skip_prev"]);
        assert_eq!(commands(&["position", "30"], &st, 1_000_000.0)[0]["position_ms"], 30000);
        assert_eq!(commands(&["position", "5+"], &st, 1_000_000.0)[0]["position_ms"], 100000, "relative to the running position");
        assert_eq!(commands(&["volume", "0.5"], &st, 0.0)[0]["volume"], 50);
        assert_eq!(commands(&["volume", "0.05+"], &st, 0.0)[0]["volume"], 75);
        assert_eq!(commands(&["shuffle", "Toggle"], &st, 0.0)[0]["enabled"], true, "off: toggled on");
        // repeat: the side switched off first
        let r = commands(&["loop", "Track"], &st, 0.0);
        assert_eq!((r[0]["command"].as_str(), r[0]["enabled"].as_bool(), r[1]["command"].as_str(), r[1]["enabled"].as_bool()), (Some("set_repeat_context"), Some(false), Some("set_repeat_track"), Some(true)));
        let r = commands(&["loop", "None"], &st, 0.0);
        assert!(r.iter().all(|c| c["enabled"] == false) && r.len() == 2);
        assert_eq!(names(&["activate"]), ["activate"]);
        assert!(commands(&["metadata"], &st, 0.0).is_empty());
        assert!(commands(&["position", "x"], &st, 0.0).is_empty());
        st.status = "paused".into();
        assert_eq!(commands(&["play-pause"], &st, 0.0)[0]["command"], "play");
    }

    #[test]
    fn the_endpoint_comes_from_the_files_soloist_writes() {
        let dir = std::env::temp_dir().join(format!("mdis-soloist-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(endpoint(&dir), None);
        std::fs::write(dir.join("ws.addr"), "127.0.0.1\n").unwrap();
        std::fs::write(dir.join("ws.port"), "40123\n").unwrap();
        assert_eq!(endpoint(&dir).as_deref(), Some("ws://127.0.0.1:40123"));
        std::fs::write(dir.join("ws.addr"), "0.0.0.0").unwrap();
        assert_eq!(endpoint(&dir).as_deref(), Some("ws://127.0.0.1:40123"), "a wildcard bind is reached locally");
        std::fs::write(dir.join("ws.port"), "no").unwrap();
        assert_eq!(endpoint(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The link against a fake Soloist: it asks for the state, shows what arrives and sends
    /// the player words as Soloist commands.
    #[tokio::test]
    async fn the_link_shows_the_state_and_sends_commands() {
        use futures::{SinkExt, StreamExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = std::env::temp_dir().join(format!("mdis-soloist-link-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("ws.addr"), "127.0.0.1").unwrap();
        std::fs::write(dir.join("ws.port"), port.to_string()).unwrap();
        let (seen_tx, mut seen) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(tokio_tungstenite::tungstenite::Message::text(state_event().to_string())).await.unwrap();
            while let Some(Ok(m)) = ws.next().await {
                if let Ok(t) = m.to_text() {
                    let v: Value = serde_json::from_str(t).unwrap();
                    let _ = seen_tx.send(v["command"].as_str().unwrap_or("").to_string());
                }
            }
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (ctx, crx) = tokio::sync::mpsc::unbounded_channel();
        let d = dir.clone();
        tokio::spawn(async move { run_at(&d, tx, crx, || true).await });
        let five = std::time::Duration::from_secs(5);
        // first it asks for the state, then shows what came
        assert_eq!(tokio::time::timeout(five, seen.recv()).await.unwrap().as_deref(), Some("get_state"));
        let snap = loop {
            match tokio::time::timeout(five, rx.recv()).await.unwrap() {
                Some(AppEvent::Player(Some(s))) => break s,
                _ => continue,
            }
        };
        assert_eq!((snap.title.as_str(), snap.player.as_str()), ("City Lights", PLAYER));
        // a player word goes out as the Soloist command
        ctx.send(vec!["next".into()]).unwrap();
        assert_eq!(tokio::time::timeout(five, seen.recv()).await.unwrap().as_deref(), Some("skip_next"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_daemon_needs_a_key_and_is_never_started_in_tests() {
        crate::util::enter_test_mode();
        assert!(!Daemon::start(true, "mdis", "MDIS_NO_SUCH_VAR", true).started());
        assert!(!Daemon::start(false, "mdis", "X", true).started());
        assert_eq!(args("Kitchen", "k")[..4], ["--device-name", "Kitchen", "--api-key", "k"]);
    }
}
