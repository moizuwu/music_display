use crate::AppEvent;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, mpsc::UnboundedSender};
#[cfg(unix)]
use std::process::Stdio;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, BufReader};
#[cfg(unix)]
use tokio::process::Command;

// The playerctl text format and its parsers are only used by the Linux backend; Windows
// reads the system media session instead (see smtc.rs). They stay compiled so the
// parsing tests run everywhere.

/// Every field we need in one playerctl call, separated by ASCII unit separator (0x1f).
#[cfg_attr(windows, allow(dead_code))]
const FORMAT: &str = "{{status}}\x1f{{position}}\x1f{{title}}\x1f{{artist}}\x1f{{album}}\x1f{{mpris:length}}\x1f{{mpris:artUrl}}\x1f{{xesam:trackNumber}}\x1f{{xesam:discNumber}}\x1f{{xesam:contentCreated}}\x1f{{volume}}\x1f{{xesam:url}}\x1f{{mpris:trackid}}\x1f{{playerName}}\x1f{{xesam:genre}}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

/// `playerctl loop` state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    None,
    Track,
    Playlist,
}

impl Loop {
    /// The next state when the repeat button is pressed: off -> playlist -> track -> off.
    pub fn next(self) -> Loop {
        match self {
            Loop::None => Loop::Playlist,
            Loop::Playlist => Loop::Track,
            Loop::Track => Loop::None,
        }
    }

    /// The argument `playerctl loop` takes.
    pub fn arg(self) -> &'static str {
        match self {
            Loop::None => "None",
            Loop::Track => "Track",
            Loop::Playlist => "Playlist",
        }
    }
}

/// `playerctl shuffle` prints `On` / `Off`; anything else means the player has none.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_shuffle(out: &str) -> Option<bool> {
    match out.trim() {
        "On" => Some(true),
        "Off" => Some(false),
        _ => None,
    }
}

/// `playerctl loop` prints `None` / `Track` / `Playlist`.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_loop(out: &str) -> Option<Loop> {
    match out.trim() {
        "None" => Some(Loop::None),
        "Track" => Some(Loop::Track),
        "Playlist" => Some(Loop::Playlist),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    pub position: f64, // seconds
    pub title: String,
    pub artist: String,
    pub album: String,
    pub length: f64, // seconds
    pub art_url: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub content_created: String,
    /// `xesam:genre` as the player reports it (may be empty; see `genre::resolve`).
    pub genre: String,
    /// Player volume 0.0..=1.0 (`None` if the player doesn't report one).
    pub volume: Option<f64>,
    /// `xesam:url` — a `file://` path for locally played audio.
    pub url: String,
    /// `mpris:trackid` — the player's own id for the item (Spotify track URI…).
    pub track_id: String,
    /// The MPRIS instance this came from (`spotify`, `vlc`…).
    pub player: String,
    /// Shuffle / repeat state (`None` if the player has neither); filled in by the
    /// poller from the last `playerctl shuffle` / `loop` read, not by [`parse`].
    pub shuffle: Option<bool>,
    pub repeat: Option<Loop>,
}

impl Snapshot {
    /// Identity of the track; changes ⇒ refetch year/lyrics. First artist and the bare
    /// song title only: case, spacing, edition / remaster / live suffixes, guest credits
    /// and the album are ignored, so every release of a song is the same track, except one
    /// marked as another language or instrumental (see `version_marker`).
    pub fn key(&self) -> u64 {
        let first_artist = self.artist.split(", ").next().unwrap_or(&self.artist);
        crate::util::fnv1a(&format!(
            "{}\0{}\0{}",
            crate::util::normalize_track(first_artist),
            crate::util::normalize_track(&crate::year::base_title(&self.title)),
            // an English / Japanese / instrumental release is not the original song
            crate::util::version_marker(&self.title, &self.album).unwrap_or_default(),
        ))
    }
}

#[cfg_attr(windows, allow(dead_code))]
pub fn parse(line: &str) -> Option<Snapshot> {
    let f: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\x1f').collect();
    let url = f.get(11).map(|u| u.trim()).unwrap_or("");
    // a local file without any tags still has a name to show
    if f.len() < 10 || (f[2].is_empty() && f[3].is_empty() && url.is_empty()) {
        return None;
    }
    let title = match f[2].trim() {
        "" => crate::tags::title_from_url(url),
        t => t.to_string(),
    };
    let us = |s: &str| {
        s.trim()
            .parse::<f64>()
            .map(|v| v / 1_000_000.0)
            .unwrap_or(0.0)
    };
    let num = |s: &str| s.trim().parse::<u32>().ok().filter(|n| *n > 0);
    Some(Snapshot {
        status: match f[0].trim() {
            "Playing" => Status::Playing,
            "Paused" => Status::Paused,
            _ => Status::Stopped,
        },
        position: us(f[1]),
        title,
        artist: f[3].trim().to_string(),
        album: f[4].trim().to_string(),
        length: us(f[5]),
        art_url: f[6].trim().to_string(),
        track_no: num(f[7]),
        disc_no: num(f[8]),
        content_created: f[9].trim().to_string(),
        genre: f.get(14).map_or(String::new(), |g| g.trim().trim_matches(|c| c == '[' || c == ']').to_string()),
        volume: f
            .get(10)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0)),
        url: url.to_string(),
        track_id: f.get(12).map_or("", |v| v.trim()).to_string(),
        player: f.get(13).map_or("", |v| v.trim()).to_string(),
        shuffle: None,
        repeat: None,
    })
}


/// Handle used by the UI to send commands to the player.
#[derive(Clone)]
pub struct Ctl {
    /// `-p a,b,c` (playerctl tries them in order), or empty for "any". Shared by every
    /// clone so the settings menu can switch the player while the tasks run.
    #[cfg(unix)]
    base: Arc<std::sync::RwLock<Vec<String>>>,
    /// Windows: the player name to follow (part of its app id), empty = the current session.
    #[cfg(windows)]
    name: Arc<std::sync::RwLock<String>>,
    pub repoll: Arc<Notify>,
    /// The follow task restarts its `playerctl --follow` when this fires.
    #[cfg(unix)]
    switched: Arc<Notify>,
    /// Last shuffle / repeat read, stamped onto every snapshot (the follow task and the
    /// poller both send them).
    #[cfg(unix)]
    modes: Arc<std::sync::RwLock<(Option<bool>, Option<Loop>)>>,
}

#[cfg(unix)]
fn player_args(player: &str) -> Vec<String> {
    if player == "any" || player.is_empty() {
        vec![]
    } else {
        vec!["-p".to_string(), player.to_string()]
    }
}

/// Players the settings menu cycles through; `local` is the usual desktop players.
#[cfg(unix)]
pub const PRESETS: &[&str] = &[
    "spotify",
    "vlc",
    "mpv",
    "rhythmbox",
    "local",
    "any",
];

/// Windows: the names are matched against the app id of the media session.
#[cfg(windows)]
pub const PRESETS: &[&str] = &["spotify", "vlc", "chrome", "firefox", "msedge", "any"];

/// What a menu name means to playerctl (`local` = any of the common local players).
#[cfg(unix)]
pub fn expand(player: &str) -> String {
    match player {
        "local" => "vlc,mpv,rhythmbox,audacious,strawberry,deadbeef,elisa,lollypop,mpd".into(),
        p => p.into(),
    }
}

#[cfg(unix)]
impl Ctl {
    pub fn new(player: &str) -> Self {
        Ctl {
            base: Arc::new(std::sync::RwLock::new(player_args(&expand(player)))),
            repoll: Arc::new(Notify::new()),
            switched: Arc::new(Notify::new()),
            modes: Arc::new(std::sync::RwLock::new((None, None))),
        }
    }

    /// Switch to another player (or priority list) without restarting the program.
    pub fn set_player(&self, player: &str) {
        if let Ok(mut b) = self.base.write() {
            *b = player_args(&expand(player));
        }
        self.switched.notify_one();
        self.repoll.notify_one();
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new("playerctl");
        c.args(self.base.read().map(|b| b.clone()).unwrap_or_default())
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        c
    }

    /// Fire-and-forget a playerctl command, then ask the poller for a fresh snapshot.
    pub fn send(&self, args: &[&str]) {
        let mut c = self.cmd();
        c.args(args).stdout(Stdio::null());
        let notify = self.repoll.clone();
        tokio::spawn(async move {
            let _ = c.status().await;
            notify.notify_one();
        });
    }

    pub fn seek_to(&self, secs: f64) {
        self.send(&["position", &format!("{:.3}", secs.max(0.0))]);
    }

    async fn poll_once(&self) -> Option<Snapshot> {
        let out = self
            .cmd()
            .args(["metadata", "--format", FORMAT])
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse(&String::from_utf8_lossy(&out.stdout))
    }

    /// One `playerctl <cmd>` read; `None` if it failed (no player, or unsupported).
    async fn read_mode(&self, cmd: &str) -> Option<String> {
        let out = self.cmd().arg(cmd).output().await.ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    async fn refresh_modes(&self) {
        let shuffle = self.read_mode("shuffle").await.and_then(|o| parse_shuffle(&o));
        let repeat = self.read_mode("loop").await.and_then(|o| parse_loop(&o));
        if let Ok(mut m) = self.modes.write() {
            *m = (shuffle, repeat);
        }
    }

    fn with_modes(&self, mut s: Snapshot) -> Snapshot {
        if let Ok(m) = self.modes.read() {
            (s.shuffle, s.repeat) = *m;
        }
        s
    }

    /// Periodic poll (keeps position accurate) — wakes early when `repoll` is notified.
    /// Shuffle / repeat are re-read about every 2 s, and right after a command.
    pub async fn poll_loop(self, tx: UnboundedSender<AppEvent>) {
        let mut modes_at: Option<tokio::time::Instant> = None;
        loop {
            if modes_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
                self.refresh_modes().await;
                modes_at = Some(tokio::time::Instant::now());
            }
            let snap = self.poll_once().await.map(|s| self.with_modes(s));
            if tx.send(AppEvent::Player(snap)).is_err() {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(400)) => {}
                _ = self.repoll.notified() => modes_at = None,
            }
        }
    }

    /// `playerctl --follow` pushes a line the moment metadata/status changes, giving
    /// instant track switches without hammering the poller.
    pub async fn follow_loop(self, tx: UnboundedSender<AppEvent>) {
        loop {
            let mut restart = false;
            let child = self
                .cmd()
                .args(["--follow", "metadata", "--format", FORMAT])
                .stdout(Stdio::piped())
                .kill_on_drop(true)
                .spawn();
            if let Ok(mut child) = child {
                if let Some(out) = child.stdout.take() {
                    let mut lines = BufReader::new(out).lines();
                    loop {
                        tokio::select! {
                            line = lines.next_line() => match line {
                                Ok(Some(line)) => {
                                    if tx.send(AppEvent::Player(parse(&line).map(|s| self.with_modes(s)))).is_err() {
                                        return;
                                    }
                                }
                                _ => break,
                            },
                            // player changed: stop this child and start one for the new player
                            _ = self.switched.notified() => {
                                let _ = child.kill().await;
                                restart = true;
                                break;
                            }
                        }
                    }
                }
                let _ = child.wait().await;
                if restart {
                    continue;
                }
            }
            // playerctl missing or exited: retry later, polling covers us meanwhile.
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
}

/// Windows: the system media session (SMTC) stands in for playerctl. Commands keep the
/// playerctl spelling (`play-pause`, `next`, `position 12+`…) so the callers don't care.
#[cfg(windows)]
impl Ctl {
    pub fn new(player: &str) -> Self {
        Ctl {
            name: Arc::new(std::sync::RwLock::new(player_name(player))),
            repoll: Arc::new(Notify::new()),
        }
    }

    pub fn set_player(&self, player: &str) {
        if let Ok(mut n) = self.name.write() {
            *n = player_name(player);
        }
        self.repoll.notify_one();
    }

    fn wanted(&self) -> String {
        self.name.read().map(|n| n.clone()).unwrap_or_default()
    }

    pub fn send(&self, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let (name, notify) = (self.wanted(), self.repoll.clone());
        tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || crate::smtc::command(&name, &args)).await;
            notify.notify_one();
        });
    }

    pub fn seek_to(&self, secs: f64) {
        self.send(&["position", &format!("{:.3}", secs.max(0.0))]);
    }

    pub async fn poll_loop(self, tx: UnboundedSender<AppEvent>) {
        loop {
            let name = self.wanted();
            let snap = tokio::task::spawn_blocking(move || crate::smtc::snapshot(&name))
                .await
                .ok()
                .flatten();
            if tx.send(AppEvent::Player(snap)).is_err() {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(400)) => {}
                _ = self.repoll.notified() => {}
            }
        }
    }

    /// Nothing pushes updates here; the poll above is quick enough.
    pub async fn follow_loop(self, _tx: UnboundedSender<AppEvent>) {
        std::future::pending::<()>().await;
    }
}

/// The menu names that mean "whatever is playing" on Windows.
#[cfg(windows)]
fn player_name(player: &str) -> String {
    match player {
        "any" | "local" => String::new(),
        p => p.to_string(),
    }
}
