//! music_display — a zero-flicker terminal "now playing" display for MPRIS players.
//!
//! Rust port of `music_display_2.fish`:
//!   * theme auto-detection from Noctalia / Matugen colour files (+ CLI overrides); with
//!     `--terminal-colors`, or when no file is usable, it inherits the terminal's own colours
//!   * playerctl polling + `--follow` stream for instant track switches
//!   * release-year fallback (xesam:contentCreated → MusicBrainz → iTunes), cached
//!   * synced lyrics from lrclib.net (exact → search fallback), cached, live offset nudging;
//!     spaced lines, faded context, optional big block-font for the active line
//!   * cover art via Kitty / Sixel / iTerm2 graphics (halfblock fallback) or coloured ASCII,
//!     duotone-tinted; optional UI accent taken from the cover (`--cover-theme`)
//!   * optional mono cava visualiser as a third column or under the lyrics
//!   * clickable progress bar + transport buttons, runtime panel toggles
//!
//! Architecture: one `tokio` select loop owns all UI state (`App`). Every slow thing
//! (subprocesses, HTTP, image decode, cava) runs in its own task and reports back over an
//! unbounded mpsc channel as an `AppEvent`. Results are tagged with the track key they were
//! requested for, so stale results from a skipped track are dropped. The UI is only redrawn
//! when something changed, and ratatui's diffing buffer means only changed cells hit the tty.

use std::io::stdout;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, EventStream};
use crossterm::execute;
use futures::StreamExt;
use tokio::sync::mpsc;

// ════════════════════════════════════════════════════════════════════════════
// CLI
// ════════════════════════════════════════════════════════════════════════════
mod cli {
    use clap::Parser;
    use std::path::PathBuf;

    #[derive(Parser, Debug, Clone)]
    #[command(
        name = "music_display",
        version,
        about = "Terminal now-playing display with synced lyrics, cover art and cava"
    )]
    pub struct Args {
        /// Use this colour file instead of auto-detecting Noctalia/Matugen
        #[arg(short = 'c', long = "colors", value_name = "FILE")]
        pub colors: Option<PathBuf>,

        /// Force the surface (background/shadow) colour, e.g. '#1e1e2e'
        #[arg(long, value_name = "HEX")]
        pub surface: Option<String>,

        /// Force the primary (accent/highlight) colour, e.g. '#cba6f7'
        #[arg(long, value_name = "HEX")]
        pub primary: Option<String>,

        /// Start with the lyrics panel hidden (toggle with `l`)
        #[arg(long)]
        pub nolyrics: bool,

        /// Start with the cava visualiser shown (toggle with `c`)
        #[arg(long)]
        pub cava: bool,

        /// Put cava in its own third column: Metadata | Lyrics | Cava
        #[arg(long, conflicts_with = "underlyrics")]
        pub thirdpanel: bool,

        /// Put cava underneath the lyrics panel (default placement)
        #[arg(long)]
        pub underlyrics: bool,

        /// MPRIS player name passed to `playerctl -p` ("any" = playerctl's default choice)
        #[arg(long, default_value = "spotify")]
        pub player: String,

        /// Cover tint strength: 0 = original colours, 1 = full surface→primary duotone
        #[arg(long, default_value_t = 1.0)]
        pub tint: f32,

        /// Default lyric offset in seconds, applied on every new track (+ = lyrics earlier)
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        pub offset: f64,

        /// Number of bars cava renders (resampled to the panel width)
        #[arg(long, default_value_t = 64)]
        pub cava_bars: u16,

        /// Don't paint the surface colour as background (keep terminal transparency)
        #[arg(long)]
        pub transparent: bool,

        /// Inherit the terminal's own colours: default background/foreground and the ANSI
        /// accent. Skips every colour file. --surface/--primary still override their slot.
        #[arg(long)]
        pub terminal_colors: bool,

        /// Draw the cover as coloured ASCII characters instead of a terminal graphics image
        #[arg(long)]
        pub ascii_cover: bool,

        /// Take the UI accent colour from the current album cover (updates every track)
        #[arg(long)]
        pub cover_theme: bool,

        /// Render the active lyric line in a big block font (toggle with `b`)
        #[arg(long)]
        pub big_lyrics: bool,

        /// Blank rows between lyric lines (0-3)
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(0..=3))]
        pub lyric_spacing: u16,
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Small shared helpers
// ════════════════════════════════════════════════════════════════════════════
mod util {
    use std::path::{Path, PathBuf};

    /// Stable 64-bit FNV-1a — used for cache file names (std's hasher isn't stable across releases).
    pub fn fnv1a(s: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// `~/.cache/music_display`, created on demand.
    pub fn cache_dir() -> PathBuf {
        let d = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("music_display");
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// `/tmp/music_display` for downloaded cover art.
    pub fn tmp_dir() -> PathBuf {
        let d = std::env::temp_dir().join("music_display");
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// Replace a leading $HOME with `~`.
    pub fn shorten_home(p: &Path) -> String {
        if let Some(home) = dirs::home_dir() {
            if let Ok(rest) = p.strip_prefix(&home) {
                return format!("~/{}", rest.display());
            }
        }
        p.display().to_string()
    }

    pub fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Decode %XX escapes in `file://` URLs.
    pub fn percent_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// `m:ss` (or `h:mm:ss`).
    pub fn fmt_time(secs: f64) -> String {
        let s = secs.max(0.0) as u64;
        if s >= 3600 {
            format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
        } else {
            format!("{}:{:02}", s / 60, s % 60)
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Theme detection
// ════════════════════════════════════════════════════════════════════════════
mod theme {
    use crate::cli::Args;
    use crate::util::shorten_home;
    use ratatui::style::Color;
    use serde_json::Value;
    use std::path::PathBuf;

    pub type Rgb = (u8, u8, u8);

    /// Palette used by the whole UI.
    ///
    /// Every slot is optional. `None` means "inherit from the terminal": the default
    /// background/foreground, and the ANSI palette for the accent. The app never invents a
    /// hex colour, so with `--terminal-colors` (or a missing/empty/invalid colour file) it
    /// matches whatever theme the terminal emulator is running.
    #[derive(Clone, Debug)]
    pub struct Theme {
        pub surface: Option<Rgb>,
        pub primary: Option<Rgb>,
        pub on_surface: Option<Rgb>,
        /// Footer text describing where the colours came from.
        pub source: String,
    }

    impl Theme {
        pub fn c(rgb: Rgb) -> Color {
            Color::Rgb(rgb.0, rgb.1, rgb.2)
        }
        /// Background. `Color::Reset` = the terminal's own (possibly transparent) background.
        pub fn surface(&self) -> Color {
            self.surface.map_or(Color::Reset, Self::c)
        }
        /// Accent. Falls back to the terminal palette's cyan.
        pub fn primary(&self) -> Color {
            self.primary.map_or(Color::Cyan, Self::c)
        }
        /// Foreground. `Color::Reset` = the terminal's default foreground.
        pub fn text(&self) -> Color {
            self.on_surface.map_or(Color::Reset, Self::c)
        }
        /// Text blended `t` of the way toward the background (0 = full text, 1 = invisible).
        /// Without known RGB values this steps through the terminal's own grays.
        pub fn fade(&self, t: f32) -> Color {
            match (self.on_surface, self.surface) {
                (Some(o), Some(s)) => Self::c(mix(o, s, t)),
                _ if t < 0.3 => Color::Reset,
                _ if t < 0.6 => Color::Gray,
                _ => Color::DarkGray,
            }
        }
        /// Accent blended toward the background.
        pub fn accent_fade(&self, t: f32) -> Color {
            match (self.primary, self.surface) {
                (Some(p), Some(s)) => Self::c(mix(p, s, t)),
                // unknown background: blend toward mid-gray, which reads on dark and light
                (Some(p), None) => Self::c(mix(p, (128, 128, 128), t * 0.8)),
                (None, _) if t >= 0.5 => Color::DarkGray,
                (None, _) if t >= 0.25 => Color::Cyan,
                (None, _) => Color::LightCyan,
            }
        }
    }

    pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
    }

    pub fn luminance(c: Rgb) -> f32 {
        (0.2126 * c.0 as f32 + 0.7152 * c.1 as f32 + 0.0722 * c.2 as f32) / 255.0
    }

    /// Nudge an accent colour until it is readable: against `surface` when known, otherwise
    /// into a mid-brightness band that works on both dark and light terminals.
    pub fn readable(mut c: Rgb, surface: Option<Rgb>) -> Rgb {
        const WHITE: Rgb = (255, 255, 255);
        const BLACK: Rgb = (0, 0, 0);
        match surface {
            Some(s) => {
                let toward = if luminance(s) < 0.5 { WHITE } else { BLACK };
                for _ in 0..10 {
                    if (luminance(c) - luminance(s)).abs() >= 0.4 {
                        break;
                    }
                    c = mix(c, toward, 0.15);
                }
            }
            None => {
                for _ in 0..10 {
                    if luminance(c) >= 0.4 {
                        break;
                    }
                    c = mix(c, WHITE, 0.12);
                }
                for _ in 0..10 {
                    if luminance(c) <= 0.8 {
                        break;
                    }
                    c = mix(c, BLACK, 0.12);
                }
            }
        }
        c
    }

    /// Parse `#rrggbb`, `rrggbb`, `#rgb`, or `#rrggbbaa` (alpha dropped).
    pub fn parse_hex(s: &str) -> Option<Rgb> {
        let h = s.trim().trim_start_matches('#');
        let h = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 => h.to_string(),
            8 => h[..6].to_string(),
            _ => return None,
        };
        let v = u32::from_str_radix(&h, 16).ok()?;
        Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Candidate files, in priority order.
    fn candidates() -> Vec<PathBuf> {
        let home = dirs::home_dir().unwrap_or_default();
        [
            ".config/noctalia/colors.json",
            ".cache/noctalia/colors.json",
            ".config/matugen/colors.json",
            ".cache/matugen/colors.json",
            ".config/noctalia/theme.json",
        ]
        .iter()
        .map(|p| home.join(p))
        .collect()
    }

    /// A colour value can be a plain string or a nested object
    /// (`{"dark": "#.."}`, `{"dark": {"color": "#.."}}`, `{"default": {"hex": ..}}` …).
    fn color_of(v: &Value) -> Option<Rgb> {
        match v {
            Value::String(s) => parse_hex(s),
            Value::Object(m) => ["dark", "default", "color", "hex", "value"]
                .iter()
                .filter_map(|k| m.get(*k))
                .find_map(color_of),
            _ => None,
        }
    }

    fn at<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
        path.iter().try_fold(root, |v, k| v.get(*k))
    }

    /// Depth-first search for the first key named `name` that holds a colour.
    fn deep_find(v: &Value, name: &str) -> Option<Rgb> {
        match v {
            Value::Object(m) => {
                if let Some(c) = m.get(name).and_then(color_of) {
                    return Some(c);
                }
                // prefer "dark" subtrees over "light" ones
                let mut keys: Vec<_> = m.keys().collect();
                keys.sort_by_key(|k| (k.contains("light"), !k.contains("dark")));
                keys.into_iter().find_map(|k| deep_find(&m[k], name))
            }
            Value::Array(a) => a.iter().find_map(|x| deep_find(x, name)),
            _ => None,
        }
    }

    /// The `jq`-equivalent: try the known schemas explicitly, then fall back to a deep search.
    fn extract(root: &Value, names: &[&str]) -> Option<Rgb> {
        for name in names {
            let paths: [&[&str]; 6] = [
                &[name],                   // Noctalia flat: .mSurface
                &["colors", "dark", name], // matugen: .colors.dark.surface
                &["colors", name],         // matugen: .colors.surface(.dark)
                &["dark", name],           // noctalia theme.json: .dark.mSurface
                &["colors", "default", name],
                &["theme", name],
            ];
            for p in paths {
                if let Some(c) = at(root, p).and_then(color_of) {
                    return Some(c);
                }
            }
        }
        names.iter().find_map(|n| deep_find(root, n))
    }

    const SURFACE: &[&str] = &["mSurface", "surface", "background", "mBackground", "base"];
    const PRIMARY: &[&str] = &["mPrimary", "primary", "accent", "mAccent"];
    const ON_SURFACE: &[&str] = &[
        "mOnSurface",
        "on_surface",
        "onSurface",
        "foreground",
        "text",
    ];

    type FileColors = (Option<Rgb>, Option<Rgb>, Option<Rgb>);

    /// `None` for a missing, unreadable, invalid, empty (`{}`) or colour-less file.
    fn from_file(path: &PathBuf) -> Option<FileColors> {
        let txt = std::fs::read_to_string(path).ok()?;
        let v: Value = serde_json::from_str(&txt).ok()?;
        let s = extract(&v, SURFACE);
        let p = extract(&v, PRIMARY);
        if s.is_none() && p.is_none() {
            return None;
        }
        Some((s, p, extract(&v, ON_SURFACE)))
    }

    pub fn load(args: &Args) -> Theme {
        let mut surface = None;
        let mut primary = None;
        let mut on_surface = None;

        // `from_file` = where the colours came from, if a file supplied any.
        let mut from_file: Option<String> = None;
        let mut fallback = "terminal colors".to_string();
        if !args.terminal_colors {
            let files: Vec<PathBuf> = match &args.colors {
                Some(p) => vec![p.clone()],
                None => candidates(),
            };
            match files
                .iter()
                .find_map(|f| self::from_file(f).map(|c| (f, c)))
            {
                Some((f, (s, p, o))) => {
                    surface = s;
                    primary = p;
                    on_surface = o;
                    from_file = Some(shorten_home(f));
                }
                // Nothing usable (missing, unreadable, invalid or `{}`): inherit the
                // terminal's colours instead of inventing any.
                None => {
                    fallback = match &args.colors {
                        Some(p) => format!("terminal colors ({} unusable)", shorten_home(p)),
                        None => "terminal colors (no usable colour file)".to_string(),
                    }
                }
            }
        }

        let mut overridden = false;
        for (flag, slot) in [(&args.surface, &mut surface), (&args.primary, &mut primary)] {
            if let Some(hex) = flag {
                match parse_hex(hex) {
                    Some(c) => {
                        *slot = Some(c);
                        overridden = true;
                    }
                    None => eprintln!("music_display: ignoring invalid colour {hex:?}"),
                }
            }
        }
        let source = match (from_file, overridden) {
            (Some(f), false) => f,
            (Some(f), true) => format!("{f} + CLI overrides"),
            (None, false) => fallback,
            (None, true) if surface.is_some() && primary.is_some() => "CLI overrides".to_string(),
            (None, true) if args.terminal_colors => "terminal colors + CLI overrides".to_string(),
            (None, true) => "CLI overrides + terminal colors".to_string(),
        };

        // Text colour is only meaningful against a known surface; otherwise it stays the
        // terminal's default foreground.
        let on_surface = surface.map(|s| {
            let derived = if luminance(s) < 0.5 {
                mix(s, (255, 255, 255), 0.88)
            } else {
                mix(s, (0, 0, 0), 0.85)
            };
            match on_surface {
                Some(o) if (luminance(o) - luminance(s)).abs() > 0.35 => o,
                _ => derived,
            }
        });

        Theme {
            surface,
            primary,
            on_surface,
            source,
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Player (playerctl / MPRIS)
// ════════════════════════════════════════════════════════════════════════════
mod player {
    use crate::AppEvent;
    use std::process::Stdio;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;
    use tokio::sync::{Notify, mpsc::UnboundedSender};

    /// Every field we need in one playerctl call, separated by ASCII unit separator (0x1f).
    const FORMAT: &str = "{{status}}\x1f{{position}}\x1f{{title}}\x1f{{artist}}\x1f{{album}}\x1f{{mpris:length}}\x1f{{mpris:artUrl}}\x1f{{xesam:trackNumber}}\x1f{{xesam:discNumber}}\x1f{{xesam:contentCreated}}";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Status {
        Playing,
        Paused,
        Stopped,
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
    }

    impl Snapshot {
        /// Identity of the track; changes ⇒ refetch year/lyrics.
        pub fn key(&self) -> u64 {
            let artist = self.artist.trim().to_lowercase();
            let title = crate::year::clean(&self.title).trim().to_lowercase();
            let album = crate::year::clean(&self.album).trim().to_lowercase();
            crate::util::fnv1a(&format!("{}\0{}\0{}", artist, title, album))
        }
    }

    fn parse(line: &str) -> Option<Snapshot> {
        let f: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\x1f').collect();
        if f.len() < 10 || (f[2].is_empty() && f[3].is_empty()) {
            return None;
        }
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
            title: f[2].trim().to_string(),
            artist: f[3].trim().to_string(),
            album: f[4].trim().to_string(),
            length: us(f[5]),
            art_url: f[6].trim().to_string(),
            track_no: num(f[7]),
            disc_no: num(f[8]),
            content_created: f[9].trim().to_string(),
        })
    }

    /// Handle used by the UI to send commands to the player.
    #[derive(Clone)]
    pub struct Ctl {
        base: Vec<String>,
        pub repoll: Arc<Notify>,
    }

    impl Ctl {
        pub fn new(player: &str) -> Self {
            let base = if player == "any" || player.is_empty() {
                vec![]
            } else {
                vec!["-p".to_string(), player.to_string()]
            };
            Ctl {
                base,
                repoll: Arc::new(Notify::new()),
            }
        }

        fn cmd(&self) -> Command {
            let mut c = Command::new("playerctl");
            c.args(&self.base)
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

        /// Periodic poll (keeps position accurate) — wakes early when `repoll` is notified.
        pub async fn poll_loop(self, tx: UnboundedSender<AppEvent>) {
            loop {
                let snap = self.poll_once().await;
                if tx.send(AppEvent::Player(snap)).is_err() {
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(400)) => {}
                    _ = self.repoll.notified() => {}
                }
            }
        }

        /// `playerctl --follow` pushes a line the moment metadata/status changes, giving
        /// instant track switches without hammering the poller.
        pub async fn follow_loop(self, tx: UnboundedSender<AppEvent>) {
            loop {
                let child = self
                    .cmd()
                    .args(["--follow", "metadata", "--format", FORMAT])
                    .stdout(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn();
                if let Ok(mut child) = child {
                    if let Some(out) = child.stdout.take() {
                        let mut lines = BufReader::new(out).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if tx.send(AppEvent::Player(parse(&line))).is_err() {
                                return;
                            }
                        }
                    }
                    let _ = child.wait().await;
                }
                // playerctl missing or exited: retry later, polling covers us meanwhile.
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Release-year resolution: contentCreated → cache → MusicBrainz → iTunes
// ════════════════════════════════════════════════════════════════════════════
mod year {
    use crate::util::{cache_dir, fnv1a};
    use regex::Regex;
    use serde_json::Value;
    use std::sync::LazyLock;
    use std::time::Duration;

    const MB: &str = "https://musicbrainz.org/ws/2";
    /// Negative results are re-checked after this many seconds.
    const NEG_TTL: u64 = 7 * 24 * 3600;

    static PAREN_TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s*[\(\[][^\)\]]*\b(remaster(ed)?|live|version|edit|mono|stereo|deluxe|feat\.?|ft\.?|anniversary|bonus|demo|acoustic|explicit|radio|expanded|edition)\b[^\)\]]*[\)\]]").unwrap()
    });
    static DASH_TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s+-\s+.*\b(remaster(ed)?|live|version|edit|mono|stereo|single|deluxe|anniversary|demo|acoustic|bonus|radio|edition)\b.*$").unwrap()
    });

    /// Strip "- Remastered 2011", "(Live at …)", "[Deluxe Edition]" etc.
    pub fn clean(s: &str) -> String {
        let s = DASH_TAG.replace(s, "");
        let s = PAREN_TAG.replace_all(&s, "");
        let t = s.trim();
        if t.is_empty() {
            s.to_string()
        } else {
            t.to_string()
        }
    }

    /// `"2011-09-26T…"` / `"2011"` → `Some("2011")`.
    pub fn from_content_created(s: &str) -> Option<String> {
        let y: String = s.trim().chars().take(4).collect();
        let n: i32 = y.parse().ok()?;
        (1900..=2100).contains(&n).then_some(y)
    }

    fn year_of(date: &str) -> Option<i32> {
        date.get(..4)?
            .parse()
            .ok()
            .filter(|y| (1900..=2100).contains(y))
    }

    fn lucene(s: &str) -> String {
        s.replace(['"', '\\'], " ")
    }

    async fn get_json(c: &reqwest::Client, url: &str, q: &[(&str, &str)]) -> Result<Value, ()> {
        let r = c.get(url).query(q).send().await.map_err(|_| ())?;
        if !r.status().is_success() {
            return Err(());
        }
        r.json::<Value>().await.map_err(|_| ())
    }

    /// Earliest first-release-date among high-confidence matches in a MusicBrainz result list.
    fn earliest(v: &Value, list: &str) -> Option<i32> {
        v.get(list)?
            .as_array()?
            .iter()
            .filter(|r| r.get("score").and_then(Value::as_i64).unwrap_or(0) >= 90)
            .filter_map(|r| {
                r.get("first-release-date")
                    .and_then(Value::as_str)
                    .and_then(year_of)
            })
            .min()
    }

    async fn mb_recording(
        c: &reqwest::Client,
        title: &str,
        artist: &str,
    ) -> Result<Option<i32>, ()> {
        let q = format!(
            "recording:\"{}\" AND artist:\"{}\"",
            lucene(title),
            lucene(artist)
        );
        let v = get_json(
            c,
            &format!("{MB}/recording/"),
            &[("query", &q), ("fmt", "json"), ("limit", "25")],
        )
        .await?;
        Ok(earliest(&v, "recordings"))
    }

    async fn mb_release_group(
        c: &reqwest::Client,
        album: &str,
        artist: &str,
    ) -> Result<Option<i32>, ()> {
        let q = format!(
            "releasegroup:\"{}\" AND artist:\"{}\"",
            lucene(album),
            lucene(artist)
        );
        let v = get_json(
            c,
            &format!("{MB}/release-group/"),
            &[("query", &q), ("fmt", "json"), ("limit", "10")],
        )
        .await?;
        Ok(earliest(&v, "release-groups"))
    }

    async fn itunes(c: &reqwest::Client, title: &str, artist: &str) -> Result<Option<i32>, ()> {
        let term = format!("{artist} {title}");
        let v = get_json(
            c,
            "https://itunes.apple.com/search",
            &[("term", &term), ("entity", "song"), ("limit", "15")],
        )
        .await?;
        let first_artist = artist.split(", ").next().unwrap_or(artist).to_lowercase();
        let tl = title.to_lowercase();
        Ok(v.get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|r| {
                let a = r
                    .get("artistName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase();
                let t = r
                    .get("trackName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase();
                a.contains(&first_artist) && (t.contains(&tl) || tl.contains(&t))
            })
            .filter_map(|r| {
                r.get("releaseDate")
                    .and_then(Value::as_str)
                    .and_then(year_of)
            })
            .min())
    }

    /// Returns `Some(year)` or `None` (= "N/A"). Never blocks the UI: runs in its own task.
    pub async fn resolve(
        c: reqwest::Client,
        artist: String,
        title: String,
        album: String,
    ) -> Option<String> {
        let path = cache_dir().join(format!(
            "year-{:016x}",
            fnv1a(&format!("{artist}\0{title}\0{album}"))
        ));
        if let Ok(txt) = tokio::fs::read_to_string(&path).await {
            let mut it = txt.split_whitespace();
            match (it.next(), it.next()) {
                (Some("N/A"), Some(ts))
                    if crate::util::now_secs().saturating_sub(ts.parse().unwrap_or(0))
                        < NEG_TTL =>
                {
                    return None;
                }
                (Some(y), _) if y != "N/A" => return Some(y.to_string()),
                _ => {}
            }
        }

        let title_c = clean(&title);
        let album_c = clean(&album);
        let first_artist = artist.split(", ").next().unwrap_or(&artist).to_string();
        let mut had_error = false;
        let mut found: Option<i32> = None;

        // MusicBrainz asks for ≤1 req/s — space the calls out.
        let gap = Duration::from_millis(1100);
        let mut artists = vec![artist.clone()];
        if first_artist != artist {
            artists.push(first_artist.clone());
        }
        for (i, a) in artists.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(gap).await;
            }
            match mb_recording(&c, &title_c, a).await {
                Ok(Some(y)) => {
                    found = Some(y);
                    break;
                }
                Ok(None) => {}
                Err(_) => had_error = true,
            }
        }
        if found.is_none() && !album_c.is_empty() {
            tokio::time::sleep(gap).await;
            match mb_release_group(&c, &album_c, &first_artist).await {
                Ok(y) => found = y,
                Err(_) => had_error = true,
            }
        }
        if found.is_none() {
            match itunes(&c, &title_c, &artist).await {
                Ok(y) => found = y,
                Err(_) => had_error = true,
            }
        }

        match found {
            Some(y) => {
                let _ = tokio::fs::write(&path, y.to_string()).await;
                Some(y.to_string())
            }
            None => {
                // Only remember a miss if every source actually answered.
                if !had_error {
                    let _ =
                        tokio::fs::write(&path, format!("N/A {}", crate::util::now_secs())).await;
                }
                None
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Synced lyrics (lrclib.net)
// ════════════════════════════════════════════════════════════════════════════
mod lyrics {
    use crate::util::{cache_dir, fnv1a, now_secs};
    use serde::{Deserialize, Serialize};

    const NEG_TTL: u64 = 3 * 24 * 3600;

    #[derive(Clone, Debug)]
    pub enum Lyrics {
        /// (time in seconds, text), sorted by time.
        Synced(Vec<(f64, String)>),
        Plain(Vec<String>),
        Instrumental,
        NotFound,
        Error(String),
    }

    #[derive(Deserialize, Debug)]
    struct Rec {
        #[serde(rename = "syncedLyrics")]
        synced: Option<String>,
        #[serde(rename = "plainLyrics")]
        plain: Option<String>,
        instrumental: Option<bool>,
        duration: Option<f64>,
    }

    #[derive(Serialize, Deserialize)]
    struct CacheEntry {
        kind: String, // synced | plain | instrumental | none
        text: String,
        saved: u64,
    }

    /// Parse LRC: multiple stamps per line, `[mm:ss.xx]`, `[mm:ss]`, `[mm:ss:xx]`, and `[offset:±ms]`.
    pub fn parse_lrc(src: &str) -> Vec<(f64, String)> {
        let mut out = Vec::new();
        let mut file_offset = 0.0;
        for raw in src.lines() {
            let mut rest = raw.trim();
            let mut stamps = Vec::new();
            while rest.starts_with('[') {
                let Some(end) = rest.find(']') else { break };
                let tag = &rest[1..end];
                if let Some(t) = parse_stamp(tag) {
                    stamps.push(t);
                } else if let Some(ms) = tag.strip_prefix("offset:") {
                    // Positive LRC offset = lyrics appear earlier.
                    file_offset = ms.trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
                }
                rest = &rest[end + 1..];
            }
            let text = rest.trim();
            for t in stamps {
                out.push((
                    t,
                    if text.is_empty() {
                        "♪".to_string()
                    } else {
                        text.to_string()
                    },
                ));
            }
        }
        for l in &mut out {
            l.0 = (l.0 - file_offset).max(0.0);
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn parse_stamp(tag: &str) -> Option<f64> {
        let (m, s) = tag.split_once(':')?;
        let m: f64 = m.trim().parse().ok()?;
        let s: f64 = s.trim().replacen(':', ".", 1).parse().ok()?;
        Some(m * 60.0 + s)
    }

    fn to_lyrics(kind: &str, text: &str) -> Lyrics {
        match kind {
            "synced" => {
                let l = parse_lrc(text);
                if l.is_empty() {
                    Lyrics::NotFound
                } else {
                    Lyrics::Synced(l)
                }
            }
            "plain" => Lyrics::Plain(text.lines().map(|s| s.trim().to_string()).collect()),
            "instrumental" => Lyrics::Instrumental,
            _ => Lyrics::NotFound,
        }
    }

    fn classify(r: &Rec) -> Option<(&'static str, String)> {
        if let Some(s) = r.synced.as_ref().filter(|s| !s.trim().is_empty()) {
            return Some(("synced", s.clone()));
        }
        if r.instrumental == Some(true) {
            return Some(("instrumental", String::new()));
        }
        r.plain
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|p| ("plain", p.clone()))
    }

    /// Best search hit: synced with matching duration > synced > plain/instrumental.
    fn pick(recs: Vec<Rec>, length: f64) -> Option<(&'static str, String)> {
        let close = |r: &Rec| length <= 0.0 || r.duration.is_none_or(|d| (d - length).abs() <= 4.0);
        let rank = |r: &Rec| match (classify(r).map(|c| c.0), close(r)) {
            (Some("synced"), true) => 0,
            (Some("synced"), false) => 1,
            (Some(_), true) => 2,
            (Some(_), false) => 3,
            (None, _) => 9,
        };
        recs.iter()
            .filter(|r| rank(r) < 9)
            .min_by_key(|r| rank(r))
            .and_then(classify)
    }

    async fn get(
        c: &reqwest::Client,
        artist: &str,
        title: &str,
        album: &str,
        length: f64,
    ) -> Result<Option<Rec>, ()> {
        let dur = format!("{}", length.round() as i64);
        let mut q = vec![("artist_name", artist), ("track_name", title)];
        if !album.is_empty() {
            q.push(("album_name", album));
        }
        if length > 0.0 {
            q.push(("duration", &dur));
        }
        let r = c
            .get("https://lrclib.net/api/get")
            .query(&q)
            .send()
            .await
            .map_err(|_| ())?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !r.status().is_success() {
            return Err(());
        }
        r.json::<Rec>().await.map(Some).map_err(|_| ())
    }

    async fn search(c: &reqwest::Client, artist: &str, title: &str) -> Result<Vec<Rec>, ()> {
        let r = c
            .get("https://lrclib.net/api/search")
            .query(&[("artist_name", artist), ("track_name", title)])
            .send()
            .await
            .map_err(|_| ())?;
        if !r.status().is_success() {
            return Err(());
        }
        r.json::<Vec<Rec>>().await.map_err(|_| ())
    }

    pub async fn fetch(
        c: reqwest::Client,
        artist: String,
        title: String,
        album: String,
        length: f64,
    ) -> Lyrics {
        let key = fnv1a(&format!(
            "{artist}\0{title}\0{album}\0{}",
            length.round() as i64
        ));
        let path = cache_dir().join(format!("lyrics-{key:016x}.json"));
        if let Ok(txt) = tokio::fs::read_to_string(&path).await {
            if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
                if e.kind != "none" || now_secs().saturating_sub(e.saved) < NEG_TTL {
                    return to_lyrics(&e.kind, &e.text);
                }
            }
        }

        let mut errored = false;
        let mut result: Option<(&'static str, String)> = None;

        // 1. exact signature match
        match get(&c, &artist, &title, &album, length).await {
            Ok(Some(r)) => result = classify(&r),
            Ok(None) => {}
            Err(_) => errored = true,
        }
        // 2. search (raw title, then cleaned title, then first artist only)
        if result.as_ref().is_none_or(|r| r.0 != "synced") {
            let cleaned = crate::year::clean(&title);
            let first = artist.split(", ").next().unwrap_or(&artist).to_string();
            let mut tries = vec![(artist.clone(), title.clone())];
            if cleaned != title {
                tries.push((artist.clone(), cleaned.clone()));
            }
            if first != artist {
                tries.push((first, cleaned));
            }
            for (a, t) in tries {
                match search(&c, &a, &t).await {
                    Ok(recs) => {
                        if let Some(hit) = pick(recs, length) {
                            let better = result.as_ref().is_none_or(|r| r.0 != "synced");
                            if better {
                                result = Some(hit);
                            }
                            if result.as_ref().is_some_and(|r| r.0 == "synced") {
                                break;
                            }
                        }
                    }
                    Err(_) => errored = true,
                }
            }
        }

        match result {
            Some((kind, text)) => {
                let e = CacheEntry {
                    kind: kind.into(),
                    text: text.clone(),
                    saved: now_secs(),
                };
                if let Ok(s) = serde_json::to_string(&e) {
                    let _ = tokio::fs::write(&path, s).await;
                }
                to_lyrics(kind, &text)
            }
            None if errored => Lyrics::Error("lyrics unavailable (network error)".into()),
            None => {
                let e = CacheEntry {
                    kind: "none".into(),
                    text: String::new(),
                    saved: now_secs(),
                };
                if let Ok(s) = serde_json::to_string(&e) {
                    let _ = tokio::fs::write(&path, s).await;
                }
                Lyrics::NotFound
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Cover art: fetch (file:// or http), cache in /tmp, downscale + duotone tint
// ════════════════════════════════════════════════════════════════════════════
mod cover {
    use crate::theme::{Rgb, mix, readable};
    use crate::util::{fnv1a, percent_decode, tmp_dir};
    use image::DynamicImage;
    use image::imageops::FilterType;

    async fn bytes(c: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
        if let Some(p) = url.strip_prefix("file://") {
            return tokio::fs::read(percent_decode(p)).await.ok();
        }
        if url.starts_with("http") {
            let path = tmp_dir().join(format!("cover-{:016x}", fnv1a(url)));
            if let Ok(b) = tokio::fs::read(&path).await {
                return Some(b);
            }
            let r = c.get(url).send().await.ok()?;
            if !r.status().is_success() {
                return None;
            }
            let b = r.bytes().await.ok()?.to_vec();
            let _ = tokio::fs::write(&path, &b).await;
            return Some(b);
        }
        // bare path
        tokio::fs::read(url).await.ok()
    }

    /// Luma → lerp(surface, primary), blended with the original by `strength`.
    fn tint(img: DynamicImage, surface: Rgb, primary: Rgb, strength: f32) -> DynamicImage {
        let mut rgb = img.to_rgb8();
        for p in rgb.pixels_mut() {
            let [r, g, b] = p.0;
            let l = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
            let duo = mix(surface, primary, l);
            let out = mix((r, g, b), duo, strength);
            p.0 = [out.0, out.1, out.2];
        }
        DynamicImage::ImageRgb8(rgb)
    }

    /// (hue 0..360, saturation 0..1, value 0..1)
    fn hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (h, if max == 0.0 { 0.0 } else { d / max }, max)
    }

    /// The cover's dominant *vivid* colour: pixels vote into 12 hue buckets weighted by
    /// saturation × brightness, so a big dark/gray background doesn't win over the artwork's
    /// real accent. Grayscale covers fall back to the average of their lit pixels.
    pub fn dominant_accent(img: &DynamicImage) -> Option<Rgb> {
        let small = img.thumbnail(48, 48).to_rgb8();
        let n = (small.width() * small.height()) as f32;
        // per bucket: weight, Σr·w, Σg·w, Σb·w
        let mut buckets = [[0f32; 4]; 12];
        let mut lit = [0f32; 4]; // count, Σr, Σg, Σb of non-black pixels
        for p in small.pixels() {
            let [r, g, b] = p.0;
            let (h, s, v) = hsv(r, g, b);
            if v >= 0.2 {
                lit[0] += 1.0;
                lit[1] += r as f32;
                lit[2] += g as f32;
                lit[3] += b as f32;
            }
            if v < 0.2 || s < 0.2 {
                continue;
            }
            let w = s * v;
            let bucket = &mut buckets[((h / 30.0) as usize) % 12];
            bucket[0] += w;
            bucket[1] += r as f32 * w;
            bucket[2] += g as f32 * w;
            bucket[3] += b as f32 * w;
        }
        let best = buckets.iter().max_by(|a, b| a[0].total_cmp(&b[0]))?;
        let (w, r, g, b) = if best[0] >= 0.02 * n {
            (best[0], best[1], best[2], best[3])
        } else if lit[0] > 0.0 {
            (lit[0], lit[1], lit[2], lit[3])
        } else {
            return None;
        };
        Some((
            (r / w).round() as u8,
            (g / w).round() as u8,
            (b / w).round() as u8,
        ))
    }

    /// How the loader should process the decoded cover.
    #[derive(Clone, Copy)]
    pub struct Look {
        pub surface: Option<Rgb>,
        pub primary: Option<Rgb>,
        /// 0 = leave the artwork untouched.
        pub strength: f32,
        /// Extract an accent from the artwork (also becomes the duotone's highlight).
        pub cover_theme: bool,
    }

    pub struct Loaded {
        pub image: DynamicImage,
        /// Readable accent derived from the artwork (only with `--cover-theme`).
        pub accent: Option<Rgb>,
    }

    pub async fn load(c: reqwest::Client, url: String, look: Look) -> Option<Loaded> {
        let data = bytes(&c, &url).await?;
        // Decoding/resizing is CPU work — keep it off the async workers.
        tokio::task::spawn_blocking(move || {
            let img = image::load_from_memory(&data).ok()?;
            let img = if img.width() > 640 || img.height() > 640 {
                img.thumbnail(640, 640)
            } else {
                img
            };
            // Always measured on the untouched artwork, never on the tinted copy.
            let accent = look
                .cover_theme
                .then(|| dominant_accent(&img))
                .flatten()
                .map(|a| readable(a, look.surface));
            // A duotone needs both ends; with terminal colours (unknown RGB) it is skipped.
            let image = match (look.surface, accent.or(look.primary)) {
                (Some(s), Some(p)) if look.strength > 0.0 => {
                    tint(img, s, p, look.strength.clamp(0.0, 1.0))
                }
                _ => img,
            };
            Some(Loaded { image, accent })
        })
        .await
        .ok()
        .flatten()
    }

    // ── ASCII rendering ───────────────────────────────────────────────────────
    /// Dark → light density ramp.
    const RAMP: &[u8] = b" .:-=+*#%@";

    /// One coloured character per terminal cell.
    pub struct AsciiArt {
        pub cols: u16,
        pub rows: u16,
        pub cells: Vec<(char, Rgb)>,
    }

    /// Keep dark pixels' glyphs visible: scale their colour up to at least ~110 brightness.
    fn lift(c: Rgb) -> Rgb {
        let m = c.0.max(c.1).max(c.2);
        match m {
            0 => (110, 110, 110),
            110..=255 => c,
            _ => {
                let k = 110.0 / m as f32;
                let f = |x: u8| (x as f32 * k).min(255.0) as u8;
                (f(c.0), f(c.1), f(c.2))
            }
        }
    }

    /// Resample to exactly `cols`×`rows` cells (the caller picks dimensions that already
    /// account for the ~1:2 cell aspect) and map brightness to a character, colour to its fg.
    pub fn ascii_art(img: &DynamicImage, cols: u16, rows: u16) -> AsciiArt {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let small = img
            .resize_exact(cols as u32, rows as u32, FilterType::Triangle)
            .to_rgb8();
        let lum = |p: &image::Rgb<u8>| {
            (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
        };
        let (mut lo, mut hi) = (1.0f32, 0.0f32);
        for p in small.pixels() {
            let l = lum(p);
            lo = lo.min(l);
            hi = hi.max(l);
        }
        let span = (hi - lo).max(0.05); // stretch contrast so flat covers still show detail
        let cells = small
            .pixels()
            .map(|p| {
                let l = ((lum(p) - lo) / span).clamp(0.0, 1.0);
                let ch = RAMP[(l * (RAMP.len() - 1) as f32).round() as usize] as char;
                (ch, lift((p[0], p[1], p[2])))
            })
            .collect();
        AsciiArt { cols, rows, cells }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Cava: headless raw ASCII output
// ════════════════════════════════════════════════════════════════════════════
mod cava {
    use crate::AppEvent;
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;
    use tokio::sync::mpsc::UnboundedSender;

    pub const MAX: u16 = 1000;

    /// Headless config: raw ASCII frames on stdout. `channels = mono` makes cava average
    /// left+right into one set of bars (stereo output is mirrored: left reversed, then right).
    pub fn config(bars: u16) -> String {
        format!(
            "[general]\nbars = {bars}\nframerate = 60\n\n\
             [output]\nmethod = raw\nchannels = mono\nmono_option = average\n\
             raw_target = /dev/stdout\ndata_format = ascii\nascii_max_range = {MAX}\n\
             bar_delimiter = 59\nframe_delimiter = 10\n"
        )
    }

    pub async fn run(bars: u16, tx: UnboundedSender<AppEvent>) {
        let fail = |msg: String| {
            let _ = tx.send(AppEvent::CavaError(msg));
        };

        let path = crate::util::tmp_dir().join(format!("cava-{bars}.conf"));
        if tokio::fs::write(&path, config(bars)).await.is_err() {
            return fail("couldn't write the cava config in /tmp".into());
        }
        let child = Command::new("cava")
            .arg("-p")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true) // aborting this task kills cava
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return fail("cava not found in PATH (pacman -S cava)".into());
            }
            Err(e) => return fail(format!("couldn't start cava: {e}")),
        };
        let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
            return fail("couldn't attach to cava's output".into());
        };

        // Remember cava's last complaint so a failure can say *why*.
        let last_err = Arc::new(Mutex::new(String::new()));
        let le = last_err.clone();
        let err_task = tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if !l.trim().is_empty() {
                    *le.lock().unwrap() = l.trim().to_string();
                }
            }
        });

        let mut lines = BufReader::new(out).lines();
        let mut got_data = false;
        loop {
            // If nothing arrives at first, say so instead of showing "starting…" forever.
            let next = if got_data {
                lines.next_line().await
            } else {
                match tokio::time::timeout(Duration::from_secs(4), lines.next_line()).await {
                    Ok(r) => r,
                    Err(_) => {
                        let _ = tx.send(AppEvent::CavaNote(
                            "cava is running but sent no data — check [input] method in your cava config"
                                .into(),
                        ));
                        got_data = true; // stop timing out; keep waiting normally
                        continue;
                    }
                }
            };
            match next {
                Ok(Some(line)) => {
                    let bars: Vec<u16> = line
                        .split(';')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();
                    if !bars.is_empty() {
                        got_data = true;
                        if tx.send(AppEvent::Cava(bars)).is_err() {
                            return;
                        }
                    }
                }
                _ => break,
            }
        }

        let status = child.wait().await.ok();
        let _ = tokio::time::timeout(Duration::from_millis(300), err_task).await;
        let why = last_err.lock().unwrap().clone();
        fail(match (why.is_empty(), status) {
            (false, _) => format!("cava failed: {why}"),
            (true, Some(s)) if !s.success() => format!("cava exited ({s})"),
            _ => "cava exited".into(),
        });
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Application state + event handling
// ════════════════════════════════════════════════════════════════════════════
pub enum AppEvent {
    Player(Option<player::Snapshot>),
    Year {
        key: u64,
        year: Option<String>,
    },
    Lyrics {
        key: u64,
        lyrics: lyrics::Lyrics,
    },
    Cover {
        url: String,
        loaded: Option<cover::Loaded>,
    },
    Cava(Vec<u16>),
    /// Informational status while cava has produced nothing yet.
    CavaNote(String),
    /// cava failed or exited; the task is gone and `c` twice restarts it.
    CavaError(String),
}

mod app {
    use super::*;
    use crate::lyrics::Lyrics;
    use crate::player::{Snapshot, Status};
    use crate::theme::Rgb;
    use crate::theme::Theme;
    use crossterm::event::{
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    };
    use ratatui::layout::{Position, Rect};
    use ratatui_image::picker::Picker;
    use ratatui_image::protocol::Protocol;
    use tokio::sync::mpsc::UnboundedSender;
    use tokio::task::JoinHandle;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum CavaPlacement {
        ThirdPanel,
        UnderLyrics,
    }

    pub enum YearState {
        Loading,
        Known(String),
        NotFound,
    }
    pub enum LyricsState {
        Fetching,
        Ready(Lyrics),
    }
    pub enum CoverState {
        None,
        Loading,
        /// `rendered` is built lazily by the UI for the cell box it actually has, and rebuilt
        /// whenever that box changes.
        Ready {
            src: image::DynamicImage,
            rendered: Option<Rendered>,
        },
        Failed,
    }

    /// The cover prepared for one exact cell box (`cells` = width × height in cells).
    pub enum Rendered {
        Graphics {
            cells: (u16, u16),
            proto: Box<Protocol>,
        },
        Ascii {
            cells: (u16, u16),
            art: cover::AsciiArt,
        },
    }

    impl Rendered {
        pub fn cells(&self) -> (u16, u16) {
            match self {
                Rendered::Graphics { cells, .. } | Rendered::Ascii { cells, .. } => *cells,
            }
        }
    }

    /// Click targets recorded during the last render.
    #[derive(Default)]
    pub struct Hits {
        pub progress: Rect,
        pub prev: Rect,
        pub play: Rect,
        pub next: Rect,
    }

    pub struct App {
        pub args: crate::cli::Args,
        pub theme: Theme,
        /// The accent from config/CLI, restored whenever there is no cover to take one from.
        base_primary: Option<Rgb>,
        pub big_lyrics: bool,
        pub ctl: player::Ctl,
        pub client: reqwest::Client,
        pub tx: UnboundedSender<AppEvent>,
        pub picker: Picker,

        pub snap: Option<Snapshot>,
        /// Position anchor for smooth interpolation between polls.
        anchor: (f64, Instant),
        track_key: u64,
        art_url: String,

        pub year: YearState,
        pub lyrics: LyricsState,
        pub cover: CoverState,
        pub offset: f64,

        pub show_lyrics: bool,
        pub show_cava: bool,
        pub placement: CavaPlacement,
        pub cava_bars: Vec<u16>,
        pub cava_status: Option<String>,
        cava_task: Option<JoinHandle<()>>,
        tasks: Vec<JoinHandle<()>>,

        pub hits: Hits,
        /// Smoothly animated lyric scroll position (in wrapped rows).
        pub lyric_scroll: f64,
        pub lyric_scroll_target: f64,
        pub snap_scroll: bool,
        pub last_anim: Instant,
        pub dirty: bool,
    }

    impl App {
        pub fn new(
            args: crate::cli::Args,
            theme: Theme,
            picker: Picker,
            tx: UnboundedSender<AppEvent>,
        ) -> Self {
            let client = reqwest::Client::builder()
                .user_agent(concat!(
                    "music_display/",
                    env!("CARGO_PKG_VERSION"),
                    " (terminal lyrics display; https://lrclib.net)"
                ))
                .timeout(Duration::from_secs(15))
                .build()
                .expect("http client");
            let placement = if args.thirdpanel {
                CavaPlacement::ThirdPanel
            } else {
                CavaPlacement::UnderLyrics
            };
            let mut app = App {
                ctl: player::Ctl::new(&args.player),
                show_lyrics: !args.nolyrics,
                show_cava: false,
                offset: args.offset,
                placement,
                big_lyrics: args.big_lyrics,
                base_primary: theme.primary,
                args,
                theme,
                client,
                tx,
                picker,
                snap: None,
                anchor: (0.0, Instant::now()),
                track_key: 0,
                art_url: String::new(),
                year: YearState::Loading,
                lyrics: LyricsState::Fetching,
                cover: CoverState::None,
                cava_bars: Vec::new(),
                cava_status: None,
                cava_task: None,
                tasks: Vec::new(),
                hits: Hits::default(),
                lyric_scroll: 0.0,
                lyric_scroll_target: 0.0,
                snap_scroll: true,
                last_anim: Instant::now(),
                dirty: true,
            };
            if app.args.cava || app.args.thirdpanel || app.args.underlyrics {
                app.set_cava(true);
            }
            app
        }

        // ── position ───────────────────────────────────────────────────────
        pub fn playing(&self) -> bool {
            self.snap
                .as_ref()
                .is_some_and(|s| s.status == Status::Playing)
        }

        /// Interpolated playback position in seconds.
        pub fn position(&self) -> f64 {
            let Some(s) = &self.snap else { return 0.0 };
            let mut p = self.anchor.0;
            if s.status == Status::Playing {
                p += self.anchor.1.elapsed().as_secs_f64();
            }
            if s.length > 0.0 { p.min(s.length) } else { p }
        }

        fn set_position(&mut self, p: f64) {
            self.anchor = (p.max(0.0), Instant::now());
        }

        // ── background events ──────────────────────────────────────────────
        pub fn apply(&mut self, ev: AppEvent) {
            match ev {
                AppEvent::Player(None) => {
                    if self.snap.is_some() {
                        self.snap = None;
                        self.track_key = 0;
                        self.dirty = true;
                    }
                }
                AppEvent::Player(Some(s)) => self.on_snapshot(s),
                AppEvent::Year { key, year } if key == self.track_key => {
                    self.year = year.map(YearState::Known).unwrap_or(YearState::NotFound);
                    self.dirty = true;
                }
                AppEvent::Lyrics { key, lyrics } if key == self.track_key => {
                    self.lyrics = LyricsState::Ready(lyrics);
                    self.snap_scroll = true;
                    self.dirty = true;
                }
                AppEvent::Cover { url, loaded } if url == self.art_url => {
                    self.cover = match loaded {
                        Some(l) => {
                            if self.args.cover_theme {
                                self.theme.primary = l.accent.or(self.base_primary);
                            }
                            CoverState::Ready {
                                src: l.image,
                                rendered: None,
                            }
                        }
                        None => {
                            self.reset_accent();
                            CoverState::Failed
                        }
                    };
                    self.dirty = true;
                }
                AppEvent::Cava(bars) => {
                    if self.show_cava {
                        self.cava_bars = bars;
                        self.cava_status = None;
                        self.dirty = true;
                    }
                }
                AppEvent::CavaNote(n) => {
                    if self.show_cava && self.cava_bars.is_empty() {
                        self.cava_status = Some(n);
                        self.dirty = true;
                    }
                }
                AppEvent::CavaError(e) => {
                    self.cava_status = Some(e);
                    self.cava_bars.clear();
                    self.cava_task = None;
                    self.dirty = true;
                }
                _ => {} // stale result for a previous track
            }
        }

        fn on_snapshot(&mut self, s: Snapshot) {
            let key = s.key();
            let new_track = key != self.track_key;
            let new_art = s.art_url != self.art_url;

            // Only re-anchor if the reported position meaningfully disagrees with our
            // interpolation (avoids jitter from polling latency).
            let drift = (self.position() - s.position).abs();
            let status_changed = self.snap.as_ref().is_none_or(|o| o.status != s.status);
            if new_track || status_changed || drift > 0.35 {
                self.set_position(s.position);
            }

            if new_track {
                self.track_key = key;
                for t in self.tasks.drain(..) {
                    t.abort(); // cancel lookups for the track we skipped
                }
                self.offset = self.args.offset;
                self.snap_scroll = true;
                self.lyric_scroll = 0.0;

                // Year: metadata first, network fallback in background.
                self.year = match year::from_content_created(&s.content_created) {
                    Some(y) => YearState::Known(y),
                    None => {
                        let (c, tx) = (self.client.clone(), self.tx.clone());
                        let (a, t, al) = (s.artist.clone(), s.title.clone(), s.album.clone());
                        self.tasks.push(tokio::spawn(async move {
                            let year = year::resolve(c, a, t, al).await;
                            let _ = tx.send(AppEvent::Year { key, year });
                        }));
                        YearState::Loading
                    }
                };

                // Lyrics
                // (fetched even while the panel is hidden, so `l` shows them instantly)
                self.lyrics = LyricsState::Fetching;
                let (c, tx) = (self.client.clone(), self.tx.clone());
                let (a, t, al, len) =
                    (s.artist.clone(), s.title.clone(), s.album.clone(), s.length);
                self.tasks.push(tokio::spawn(async move {
                    let lyrics = lyrics::fetch(c, a, t, al, len).await;
                    let _ = tx.send(AppEvent::Lyrics { key, lyrics });
                }));
            }

            if new_art {
                self.art_url = s.art_url.clone();
                if s.art_url.is_empty() {
                    self.cover = CoverState::None;
                    self.reset_accent();
                } else {
                    self.cover = CoverState::Loading;
                    let (c, tx, url) = (self.client.clone(), self.tx.clone(), s.art_url.clone());
                    let look = cover::Look {
                        surface: self.theme.surface,
                        primary: self.base_primary,
                        strength: self.args.tint,
                        cover_theme: self.args.cover_theme,
                    };
                    tokio::spawn(async move {
                        let loaded = cover::load(c, url.clone(), look).await;
                        let _ = tx.send(AppEvent::Cover { url, loaded });
                    });
                }
            }

            let changed = new_track
                || new_art
                || self
                    .snap
                    .as_ref()
                    .is_none_or(|o| o.status != s.status || o.length != s.length);
            self.snap = Some(s);
            if changed {
                self.dirty = true;
            }
        }

        /// Back to the configured accent (no cover to take one from).
        fn reset_accent(&mut self) {
            if self.args.cover_theme {
                self.theme.primary = self.base_primary;
            }
        }

        // ── cava lifecycle ─────────────────────────────────────────────────
        fn set_cava(&mut self, on: bool) {
            self.show_cava = on;
            if on {
                if self.cava_task.is_none() {
                    self.cava_status = Some("starting cava…".into());
                    let tx = self.tx.clone();
                    let bars = self.args.cava_bars.max(4);
                    self.cava_task = Some(tokio::spawn(cava::run(bars, tx)));
                }
            } else if let Some(t) = self.cava_task.take() {
                t.abort(); // drops the Child → kill_on_drop stops cava (no CPU when hidden)
                self.cava_bars.clear();
            }
        }

        // ── input ──────────────────────────────────────────────────────────
        /// Returns `true` to quit.
        pub fn handle(&mut self, ev: Event) -> bool {
            self.dirty = true;
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return true,
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        return true;
                    }
                    KeyCode::Char(' ') => self.play_pause(),
                    KeyCode::Char('n') => self.ctl.send(&["next"]),
                    KeyCode::Char('p') => self.ctl.send(&["previous"]),
                    KeyCode::Left => self.seek_rel(-5.0),
                    KeyCode::Right => self.seek_rel(5.0),
                    KeyCode::Char(',') => self.nudge(-0.1),
                    KeyCode::Char('.') => self.nudge(0.1),
                    KeyCode::Char('l') => {
                        self.show_lyrics = !self.show_lyrics;
                        self.snap_scroll = true;
                    }
                    KeyCode::Char('c') => {
                        let on = !self.show_cava;
                        self.set_cava(on);
                    }
                    KeyCode::Char('b') => {
                        self.big_lyrics = !self.big_lyrics;
                        self.snap_scroll = true;
                    }
                    _ => {}
                },
                Event::Mouse(m) if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) => {
                    let pos = Position::new(m.column, m.row);
                    if self.hits.progress.contains(pos) {
                        if let Some(len) = self.snap.as_ref().map(|s| s.length).filter(|l| *l > 0.0)
                        {
                            let r = self.hits.progress;
                            let frac =
                                (m.column - r.x) as f64 / (r.width.saturating_sub(1).max(1)) as f64;
                            let target = frac.clamp(0.0, 1.0) * len;
                            self.set_position(target);
                            self.ctl.seek_to(target);
                        }
                    } else if self.hits.prev.contains(pos) {
                        self.ctl.send(&["previous"]);
                    } else if self.hits.play.contains(pos) {
                        self.play_pause();
                    } else if self.hits.next.contains(pos) {
                        self.ctl.send(&["next"]);
                    }
                }
                _ => {}
            }
            false
        }

        fn play_pause(&mut self) {
            // Optimistic update so the button flips immediately.
            let p = self.position();
            if let Some(s) = &mut self.snap {
                s.status = if s.status == Status::Playing {
                    Status::Paused
                } else {
                    Status::Playing
                };
            }
            self.set_position(p);
            self.ctl.send(&["play-pause"]);
        }

        fn seek_rel(&mut self, d: f64) {
            let p = self.position() + d;
            self.set_position(p);
            let arg = if d < 0.0 {
                format!("{}-", -d)
            } else {
                format!("{d}+")
            };
            self.ctl.send(&["position", &arg]);
        }

        fn nudge(&mut self, d: f64) {
            self.offset = ((self.offset + d) * 10.0).round() / 10.0;
        }

        /// Whether the next frame would differ even without new events.
        pub fn animating(&self) -> bool {
            (self.lyric_scroll - self.lyric_scroll_target).abs() > 0.02
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Big block font for the active lyric line (5×7 pixels, drawn with half blocks)
// ════════════════════════════════════════════════════════════════════════════
mod bigfont {
    /// 5×7 bitmap, one `u8` per row, bit 4 = leftmost pixel.
    fn glyph(c: char) -> Option<[u8; 7]> {
        Some(match c {
            ' ' => [0; 7],
            'A' => [
                0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
            ],
            'B' => [
                0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
            ],
            'C' => [
                0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
            ],
            'D' => [
                0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
            ],
            'E' => [
                0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
            ],
            'F' => [
                0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
            ],
            'G' => [
                0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
            ],
            'H' => [
                0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
            ],
            'I' => [
                0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
            ],
            'J' => [
                0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
            ],
            'K' => [
                0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
            ],
            'L' => [
                0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
            ],
            'M' => [
                0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
            ],
            'N' => [
                0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
            ],
            'O' => [
                0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
            ],
            'P' => [
                0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
            ],
            'Q' => [
                0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
            ],
            'R' => [
                0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
            ],
            'S' => [
                0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
            ],
            'T' => [
                0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
            ],
            'U' => [
                0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
            ],
            'V' => [
                0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
            ],
            'W' => [
                0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
            ],
            'X' => [
                0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
            ],
            'Y' => [
                0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
            ],
            'Z' => [
                0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
            ],
            '0' => [
                0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
            ],
            '1' => [
                0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
            ],
            '2' => [
                0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
            ],
            '3' => [
                0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
            ],
            '4' => [
                0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
            ],
            '5' => [
                0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
            ],
            '6' => [
                0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
            ],
            '7' => [
                0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
            ],
            '8' => [
                0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
            ],
            '9' => [
                0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
            ],
            '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
            ',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
            '!' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0, 0b00100],
            '?' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0, 0b00100],
            '\'' => [0b01100, 0b00100, 0b01000, 0, 0, 0, 0],
            '"' => [0b01010, 0b01010, 0, 0, 0, 0, 0],
            '-' => [0, 0, 0, 0b11111, 0, 0, 0],
            ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
            ';' => [0, 0b01100, 0b01100, 0, 0b01100, 0b00100, 0b01000],
            '(' => [
                0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
            ],
            ')' => [
                0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
            ],
            '&' => [
                0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
            ],
            '/' => [
                0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
            ],
            '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
            '=' => [0, 0, 0b11111, 0, 0b11111, 0, 0],
            '*' => [0, 0b00100, 0b10101, 0b01110, 0b10101, 0b00100, 0],
            '#' => [
                0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
            ],
            '%' => [
                0b11000, 0b11001, 0b00010, 0b00100, 0b01000, 0b10011, 0b00011,
            ],
            '_' => [0, 0, 0, 0, 0, 0, 0b11111],
            '♪' => [
                0b00010, 0b00011, 0b00010, 0b00010, 0b01110, 0b11110, 0b01100,
            ],
            _ => return None,
        })
    }

    /// Upper-case and strip common diacritics; `None` if the font can't draw the character.
    fn fold(c: char) -> Option<char> {
        let c = match c {
            'À'..='Å' | 'à'..='å' => 'A',
            'Ç' | 'ç' => 'C',
            'È'..='Ë' | 'è'..='ë' => 'E',
            'Ì'..='Ï' | 'ì'..='ï' => 'I',
            'Ñ' | 'ñ' => 'N',
            'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' => 'O',
            'Ù'..='Ü' | 'ù'..='ü' => 'U',
            'Ý' | 'ý' | 'ÿ' => 'Y',
            '’' | '‘' | '`' | '´' => '\'',
            '“' | '”' => '"',
            '–' | '—' => '-',
            c => c.to_ascii_uppercase(),
        };
        glyph(c).map(|_| c)
    }

    /// Columns used by one glyph incl. the 1-column gap after it; a space is 3 columns.
    fn advance(c: char) -> usize {
        if c == ' ' { 3 } else { 6 }
    }

    /// Render `text` as rows of half-block characters, at most `max_cols` wide and
    /// `max_chunks` wrapped pieces tall (each piece = 4 text rows, pieces separated by one
    /// blank row). `None` when it can't be drawn — unsupported characters, a word wider than
    /// the panel, or too many pieces — so the caller can fall back to normal text.
    pub fn render(text: &str, max_cols: usize, max_chunks: usize) -> Option<Vec<String>> {
        let text = text.replace('…', "...");
        let mut words: Vec<Vec<char>> = Vec::new();
        for w in text.split_whitespace() {
            words.push(w.chars().map(fold).collect::<Option<Vec<_>>>()?);
        }
        if words.is_empty() {
            return None;
        }

        // Greedy wrap by rendered width.
        let mut chunks: Vec<Vec<char>> = Vec::new();
        let mut cur: Vec<char> = Vec::new();
        let mut cur_w = 0usize; // columns incl. the trailing gap
        for w in words {
            let ww: usize = w.iter().map(|&c| advance(c)).sum();
            if ww - 1 > max_cols {
                return None;
            }
            if !cur.is_empty() && cur_w + advance(' ') + ww - 1 > max_cols {
                chunks.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += advance(' ');
            }
            cur_w += ww;
            cur.extend(w);
        }
        chunks.push(cur);
        if chunks.len() > max_chunks {
            return None;
        }

        let mut rows = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            if i > 0 {
                rows.push(String::new());
            }
            // pixel grid: 8 rows (7 + 1 blank so they pair up into 4 half-block rows)
            let mut px: Vec<Vec<bool>> = vec![Vec::new(); 8];
            for &c in chunk {
                let g = glyph(c).unwrap_or([0; 7]);
                if c == ' ' {
                    for row in px.iter_mut() {
                        row.extend([false; 3]);
                    }
                    continue;
                }
                for (y, row) in px.iter_mut().enumerate() {
                    for x in 0..5 {
                        row.push(y < 7 && g[y] & (1 << (4 - x)) != 0);
                    }
                    row.push(false);
                }
            }
            for row in px.iter_mut() {
                row.pop(); // trailing gap
            }
            for pair in 0..4 {
                let (top, bot) = (&px[pair * 2], &px[pair * 2 + 1]);
                rows.push(
                    top.iter()
                        .zip(bot)
                        .map(|(&t, &b)| match (t, b) {
                            (true, true) => '█',
                            (true, false) => '▀',
                            (false, true) => '▄',
                            (false, false) => ' ',
                        })
                        .collect(),
                );
            }
        }
        Some(rows)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Rendering
// ════════════════════════════════════════════════════════════════════════════
mod ui {
    use crate::app::{App, CavaPlacement, CoverState, LyricsState, Rendered, YearState};
    use crate::lyrics::Lyrics;
    use crate::player::Status;
    use crate::theme::Theme;
    use crate::util::fmt_time;
    use crate::{bigfont, cover};
    use image::imageops::FilterType;
    use ratatui::Frame;
    use ratatui::buffer::Buffer;
    use ratatui::layout::{Alignment, Constraint, Layout, Rect, Size};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, BorderType, Paragraph};
    use ratatui_image::{Image, Resize};

    /// Metadata | Lyrics | Cava. The left and right columns get exactly the same width, so
    /// the cava panel mirrors the album/metadata column; lyrics take whatever is left.
    pub fn three_panel(main: Rect) -> [Rect; 3] {
        let side = ((main.width as u32 * 28 / 100) as u16)
            .max(20)
            .min(main.width.saturating_sub(20) / 2);
        Layout::horizontal([
            Constraint::Length(side),
            Constraint::Min(1),
            Constraint::Length(side),
        ])
        .areas(main)
    }

    pub fn draw(f: &mut Frame, app: &mut App) {
        let area = f.area();
        let th = app.theme.clone();
        // Only paint a background when we actually know the surface colour; otherwise the
        // terminal's own (possibly transparent) background is left alone.
        if !app.args.transparent && th.surface.is_some() {
            f.render_widget(
                Block::new().style(Style::new().bg(th.surface()).fg(th.text())),
                area,
            );
        }

        let [main, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

        // ── column layout ──
        let (left, lyr, cav) = match (app.show_lyrics, app.show_cava, app.placement) {
            (false, false, _) => (main, None, None),
            (true, false, _) => {
                let [l, r] =
                    Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                        .areas(main);
                (l, Some(r), None)
            }
            (false, true, CavaPlacement::ThirdPanel) => {
                // Lyrics hidden: keep the two remaining columns the same width.
                let [l, r] =
                    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .areas(main);
                (l, None, Some(r))
            }
            (false, true, CavaPlacement::UnderLyrics) => {
                let [l, r] =
                    Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                        .areas(main);
                (l, None, Some(r))
            }
            (true, true, CavaPlacement::ThirdPanel) => {
                let [l, m, r] = three_panel(main);
                (l, Some(m), Some(r))
            }
            (true, true, CavaPlacement::UnderLyrics) => {
                let [l, r] =
                    Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                        .areas(main);
                let [t, b] =
                    Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                        .areas(r);
                (l, Some(t), Some(b))
            }
        };

        draw_left(f, app, left);
        if let Some(r) = lyr {
            draw_lyrics(f, app, r);
        }
        if let Some(r) = cav {
            draw_cava(f, app, r);
        }
        draw_footer(f, app, footer);
    }

    fn panel<'a>(app: &App, title: &'a str) -> Block<'a> {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(app.theme.accent_fade(0.6)))
            .title(Span::styled(
                format!(" {title} "),
                Style::new()
                    .fg(app.theme.primary())
                    .add_modifier(Modifier::BOLD),
            ))
    }

    // ── left column ─────────────────────────────────────────────────────────
    fn draw_left(f: &mut Frame, app: &mut App, area: Rect) {
        let block = panel(app, "now playing");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let th = app.theme.clone();

        let Some(snap) = app.snap.clone() else {
            app.hits = Default::default();
            let msg = format!("waiting for {}…", app.args.player);
            let y = inner.y + inner.height / 2;
            f.render_widget(
                Paragraph::new(Line::styled(msg, Style::new().fg(th.fade(0.4))))
                    .alignment(Alignment::Center),
                Rect {
                    y,
                    height: 1,
                    ..inner
                },
            );
            return;
        };

        let [cover_a, _, meta_a, _, prog_a, _, btn_a] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner.inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 0,
        }));

        // Cover
        if cover_a.height >= 3 {
            draw_cover(f, app, cover_a);
        }

        // Metadata
        let label = |s: &str| {
            Span::styled(
                format!("{s:<10}"),
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            )
        };
        let val = |s: String| Span::styled(s, Style::new().fg(th.text()));
        let dash = |s: &str| {
            if s.is_empty() {
                "—".to_string()
            } else {
                s.to_string()
            }
        };
        let track_no = format!(
            "{:02}-{}",
            snap.disc_no.unwrap_or(1),
            snap.track_no
                .map(|t| format!("{t:02}"))
                .unwrap_or_else(|| "--".into())
        );
        let year = match &app.year {
            YearState::Loading => "...".to_string(),
            YearState::Known(y) => y.clone(),
            YearState::NotFound => "N/A".to_string(),
        };
        let pos = app.position();
        let lines = vec![
            Line::from(vec![label("Track"), val(dash(&snap.title))]),
            Line::from(vec![label("Artist"), val(dash(&snap.artist))]),
            Line::from(vec![label("Album"), val(dash(&snap.album))]),
            Line::from(vec![label("Track No"), val(track_no)]),
            Line::from(vec![label("Released"), val(year)]),
            Line::from(vec![
                label("Time"),
                val(format!("{} / {}", fmt_time(pos), fmt_time(snap.length))),
            ]),
        ];
        f.render_widget(Paragraph::new(lines), meta_a);

        // Progress bar
        app.hits.progress = prog_a;
        let w = prog_a.width as usize;
        if w > 0 {
            let frac = if snap.length > 0.0 {
                (pos / snap.length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let filled = ((frac * w as f64).round() as usize).min(w);
            let knob = filled.saturating_sub(1).min(w - 1);
            let buf = f.buffer_mut();
            for i in 0..w {
                let (sym, fg) = if i == knob && filled > 0 {
                    ("●", th.primary())
                } else if i < filled {
                    ("━", th.primary())
                } else {
                    ("─", th.fade(0.7))
                };
                if let Some(c) = buf.cell_mut((prog_a.x + i as u16, prog_a.y)) {
                    c.set_symbol(sym).set_fg(fg);
                }
            }
        }

        // Buttons
        let play_lbl = if snap.status == Status::Playing {
            "[ pause ]"
        } else {
            "[ play ]"
        };
        let labels = ["[ prev ]", play_lbl, "[ next ]"];
        let gap = 2u16;
        let total: u16 = labels.iter().map(|l| l.chars().count() as u16).sum::<u16>() + gap * 2;
        let mut x = btn_a.x + btn_a.width.saturating_sub(total) / 2;
        let mut rects = [Rect::default(); 3];
        for (i, l) in labels.iter().enumerate() {
            let w = l.chars().count() as u16;
            let r = Rect {
                x,
                y: btn_a.y,
                width: w.min(btn_a.right().saturating_sub(x)),
                height: 1,
            };
            let style = if i == 1 {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(th.text())
            };
            f.render_widget(Paragraph::new(Span::styled(*l, style)), r);
            rects[i] = r;
            x = x.saturating_add(w + gap);
        }
        app.hits.prev = rects[0];
        app.hits.play = rects[1];
        app.hits.next = rects[2];
    }

    fn draw_cover(f: &mut Frame, app: &mut App, area: Rect) {
        let th = app.theme.clone();
        let fs = app.picker.font_size();
        let (fw, fh) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let ascii = app.args.ascii_cover;
        let picker = &app.picker;
        match &mut app.cover {
            CoverState::Ready { src, rendered } => {
                // Largest whole-cell box with the artwork's aspect ratio, centred.
                let (w, h) = (src.width() as f64, src.height() as f64);
                let scale =
                    ((area.width as f64 * fw as f64) / w).min((area.height as f64 * fh as f64) / h);
                let cw = (((w * scale) / fw as f64).floor() as u16).clamp(1, area.width);
                let ch = (((h * scale) / fh as f64).floor() as u16).clamp(1, area.height);
                let r = Rect {
                    x: area.x + (area.width - cw) / 2,
                    y: area.y + (area.height - ch) / 2,
                    width: cw,
                    height: ch,
                };

                if rendered.as_ref().map(Rendered::cells) != Some((cw, ch)) {
                    let to_ascii = |src: &image::DynamicImage| Rendered::Ascii {
                        cells: (cw, ch),
                        art: cover::ascii_art(src, cw, ch),
                    };
                    *rendered = Some(if ascii {
                        to_ascii(src)
                    } else {
                        // Pixel-exact bitmap: scaled and centre-cropped to *precisely* cw×ch
                        // cells, so the graphics protocol has nothing to pad. (Letting it fit
                        // a slightly-off aspect padded the remainder with transparent black,
                        // which showed up as a dark sliver on the right/bottom edge.)
                        let exact = src.resize_to_fill(
                            cw as u32 * fw,
                            ch as u32 * fh,
                            FilterType::Triangle,
                        );
                        match picker.new_protocol(exact, Size::new(cw, ch), Resize::Fit(None)) {
                            Ok(p) => Rendered::Graphics {
                                cells: (cw, ch),
                                proto: Box::new(p),
                            },
                            Err(_) => to_ascii(src),
                        }
                    });
                }
                match rendered {
                    Some(Rendered::Graphics { proto, .. }) => {
                        f.render_widget(Image::new(proto.as_ref()), r)
                    }
                    Some(Rendered::Ascii { art, .. }) => draw_ascii(f.buffer_mut(), r, art),
                    None => {}
                }
            }
            other => {
                let msg = match other {
                    CoverState::Loading => "loading cover…",
                    _ => "♪",
                };
                let y = area.y + area.height / 2;
                f.render_widget(
                    Paragraph::new(Line::styled(msg, Style::new().fg(th.fade(0.5))))
                        .alignment(Alignment::Center),
                    Rect {
                        y,
                        height: 1,
                        ..area
                    },
                );
            }
        }
    }

    fn draw_ascii(buf: &mut Buffer, r: Rect, art: &cover::AsciiArt) {
        for y in 0..art.rows.min(r.height) {
            for x in 0..art.cols.min(r.width) {
                let (ch, (cr, cg, cb)) = art.cells[y as usize * art.cols as usize + x as usize];
                if let Some(cell) = buf.cell_mut((r.x + x, r.y + y)) {
                    let mut utf8 = [0u8; 4];
                    cell.set_symbol(ch.encode_utf8(&mut utf8))
                        .set_fg(Color::Rgb(cr, cg, cb));
                }
            }
        }
    }

    // ── lyrics ──────────────────────────────────────────────────────────────
    /// Greedy word-wrap by display width.
    fn wrap(text: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![];
        }
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut cur_w = 0;
        for word in text.split_whitespace() {
            let ww = Span::raw(word).width();
            if cur_w > 0 && cur_w + 1 + ww > width {
                out.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if ww > width {
                // hard-break very long tokens
                for ch in word.chars() {
                    let cw = Span::raw(ch.to_string()).width();
                    if cur_w + cw > width {
                        out.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                    cur.push(ch);
                    cur_w += cw;
                }
                continue;
            }
            if cur_w > 0 {
                cur.push(' ');
                cur_w += 1;
            }
            cur.push_str(word);
            cur_w += ww;
        }
        if !cur.is_empty() || out.is_empty() {
            out.push(cur);
        }
        out
    }

    fn centered_msg(f: &mut Frame, area: Rect, msgs: &[(&str, Style)]) {
        let start = area.y + area.height.saturating_sub(msgs.len() as u16) / 2;
        for (i, (m, s)) in msgs.iter().enumerate() {
            let y = start + i as u16;
            if y < area.bottom() {
                f.render_widget(
                    Paragraph::new(Line::styled(*m, *s)).alignment(Alignment::Center),
                    Rect {
                        y,
                        height: 1,
                        ..area
                    },
                );
            }
        }
    }

    /// Style for a non-active lyric line `dist` lines away from the active one:
    /// fades with distance, and past lines fade a little faster than upcoming ones.
    fn fade_style(th: &Theme, dist: usize, past: bool) -> Style {
        let t = 0.40 + 0.08 * dist as f32 + if past { 0.10 } else { 0.0 };
        Style::new().fg(th.fade(t.min(0.78)))
    }

    /// "Focus" layout used by the big-font mode: the active line sits on a fixed stage in the
    /// middle of the panel (big font if it fits, otherwise bold wrapped text), with the lines
    /// before and after stacked outward from it.
    fn draw_focus(f: &mut Frame, inner: Rect, th: &Theme, lines: &[String], c: usize, gap: usize) {
        let width = inner.width as usize;
        let big = bigfont::render(&lines[c], width, 3)
            .filter(|rows| rows.len() as u16 + 4 <= inner.height);
        let (stage, stage_style): (Vec<String>, Style) = match big {
            Some(rows) => (rows, Style::new().fg(th.primary())),
            None => (
                wrap(&lines[c], width),
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            ),
        };
        let sh = (stage.len() as u16).min(inner.height);
        let top = inner.y + (inner.height - sh) / 2;

        let put = |f: &mut Frame, y: i32, text: &str, style: Style| {
            if y >= inner.y as i32 && y < inner.bottom() as i32 && !text.is_empty() {
                f.render_widget(
                    Paragraph::new(Line::styled(text.to_string(), style))
                        .alignment(Alignment::Center),
                    Rect {
                        y: y as u16,
                        height: 1,
                        ..inner
                    },
                );
            }
        };

        for (i, row) in stage.iter().take(sh as usize).enumerate() {
            put(f, top as i32 + i as i32, row, stage_style);
        }

        // Earlier lines, nearest first, growing upward.
        let mut y = top as i32 - 2;
        for k in 1..=c {
            let style = fade_style(th, k, true);
            for row in wrap(&lines[c - k], width).iter().rev() {
                put(f, y, row, style);
                y -= 1;
            }
            y -= gap as i32;
            if y < inner.y as i32 {
                break;
            }
        }
        // Upcoming lines, growing downward.
        let mut y = top as i32 + sh as i32 + 1;
        for (k, line) in lines.iter().enumerate().skip(c + 1) {
            let style = fade_style(th, k - c, false);
            for row in wrap(line, width) {
                put(f, y, &row, style);
                y += 1;
            }
            y += gap as i32;
            if y >= inner.bottom() as i32 {
                break;
            }
        }
    }

    fn draw_lyrics(f: &mut Frame, app: &mut App, area: Rect) {
        let block = panel(app, "lyrics");
        let inner = block.inner(area).inner(ratatui::layout::Margin {
            horizontal: 2,
            vertical: 1,
        });
        f.render_widget(block, area);
        let th = app.theme.clone();
        let dim = Style::new().fg(th.fade(0.5));

        if app.snap.is_none() {
            centered_msg(f, inner, &[("♪", dim)]);
            return;
        }
        let pos = app.position();
        let length = app.snap.as_ref().map(|s| s.length).unwrap_or(0.0);
        let (lines, current): (Vec<String>, Option<usize>) = match &app.lyrics {
            LyricsState::Fetching => return centered_msg(f, inner, &[("fetching lyrics…", dim)]),
            LyricsState::Ready(Lyrics::NotFound) => {
                return centered_msg(
                    f,
                    inner,
                    &[
                        ("♪", Style::new().fg(th.primary())),
                        ("", dim),
                        ("no synced lyrics found", dim),
                    ],
                );
            }
            LyricsState::Ready(Lyrics::Instrumental) => {
                return centered_msg(
                    f,
                    inner,
                    &[("♪  instrumental  ♪", Style::new().fg(th.primary()))],
                );
            }
            LyricsState::Ready(Lyrics::Error(e)) => {
                return centered_msg(f, inner, &[("♪", dim), (e.as_str(), dim)]);
            }
            LyricsState::Ready(Lyrics::Synced(l)) => {
                let t = pos + app.offset;
                let cur = l.partition_point(|(ts, _)| *ts <= t).checked_sub(1);
                (l.iter().map(|(_, s)| s.clone()).collect(), cur)
            }
            LyricsState::Ready(Lyrics::Plain(l)) => (l.clone(), None),
        };
        let plain = matches!(app.lyrics, LyricsState::Ready(Lyrics::Plain(_)));

        // Blank rows between lines — dropped on short panels where they'd waste the space.
        let gap = if inner.height >= 10 {
            app.args.lyric_spacing as usize
        } else {
            0
        };

        // Big-font mode uses the fixed "focus" layout instead of the scrolling one.
        if app.big_lyrics && !plain {
            if let Some(c) = current {
                app.lyric_scroll_target = app.lyric_scroll; // nothing left to animate
                draw_focus(f, inner, &th, &lines, c, gap);
                return;
            }
        }

        // Wrap into rows, remembering which source line each row belongs to. Spacing rows
        // belong to the line above them but hold no text.
        let mut rows: Vec<(usize, String)> = Vec::new();
        let mut starts = Vec::with_capacity(lines.len());
        let mut ends = Vec::with_capacity(lines.len());
        for (i, l) in lines.iter().enumerate() {
            starts.push(rows.len());
            for r in wrap(l, inner.width as usize) {
                rows.push((i, r));
            }
            ends.push(rows.len());
            for _ in 0..gap {
                rows.push((i, String::new()));
            }
        }
        if rows.is_empty() {
            return centered_msg(f, inner, &[("♪", dim)]);
        }

        // Scroll target: centre of the active line's text (synced) or proportional (plain).
        let target = if plain {
            if length > 0.0 {
                (pos / length).clamp(0.0, 1.0) * rows.len() as f64
            } else {
                0.0
            }
        } else {
            match current {
                Some(c) => (starts[c] + ends[c]) as f64 / 2.0 - 0.5,
                None => -1.0,
            }
        };

        // Critically-damped-ish easing toward the target; snap on big jumps.
        let now = std::time::Instant::now();
        let dt = now.duration_since(app.last_anim).as_secs_f64().min(0.1);
        app.last_anim = now;
        app.lyric_scroll_target = target;
        if app.snap_scroll || (target - app.lyric_scroll).abs() > inner.height as f64 {
            app.lyric_scroll = target;
            app.snap_scroll = false;
        } else {
            let k = 1.0 - (-dt * 10.0).exp();
            app.lyric_scroll += (target - app.lyric_scroll) * k;
            if (target - app.lyric_scroll).abs() < 0.02 {
                app.lyric_scroll = target;
            }
        }

        let half = inner.height as i64 / 2;
        let top = app.lyric_scroll.round() as i64 - half;
        if plain {
            f.render_widget(
                Paragraph::new(Line::styled(
                    "no synced lyrics — plain text",
                    Style::new()
                        .fg(th.accent_fade(0.4))
                        .add_modifier(Modifier::ITALIC),
                ))
                .alignment(Alignment::Center),
                Rect {
                    y: inner.y.saturating_sub(1),
                    height: 1,
                    ..inner
                },
            );
        }
        for y in 0..inner.height {
            let idx = top + y as i64;
            if idx < 0 || idx as usize >= rows.len() {
                continue;
            }
            let (li, text) = &rows[idx as usize];
            if text.is_empty() {
                continue;
            }
            let style = match current {
                Some(c) if *li == c => Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
                Some(c) => fade_style(&th, li.abs_diff(c), *li < c),
                None if plain => Style::new().fg(th.fade(0.35)),
                None => Style::new().fg(th.fade(0.55)),
            };
            f.render_widget(
                Paragraph::new(Line::styled(text.as_str(), style)).alignment(Alignment::Center),
                Rect {
                    y: inner.y + y,
                    height: 1,
                    ..inner
                },
            );
        }
    }

    // ── cava ────────────────────────────────────────────────────────────────
    fn draw_cava(f: &mut Frame, app: &mut App, area: Rect) {
        let block = panel(app, "cava");
        let inner = block.inner(area).inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 0,
        });
        f.render_widget(block, area);
        let th = app.theme.clone();

        if app.cava_bars.is_empty() {
            let msg = app.cava_status.clone().unwrap_or_else(|| "…".into());
            // Long error messages wrap instead of being cut off at the panel edge.
            let style = Style::new().fg(th.fade(0.5));
            let rows = wrap(&msg, inner.width as usize);
            let msgs: Vec<(&str, Style)> = rows.iter().map(|r| (r.as_str(), style)).collect();
            return centered_msg(f, inner, &msgs);
        }
        if inner.width < 2 || inner.height == 0 {
            return;
        }

        // One column per bar with a 1-column gap; resample cava's bars to fit.
        let n_disp = (inner.width as usize)
            .div_ceil(2)
            .clamp(1, app.cava_bars.len());
        let src = &app.cava_bars;
        let used_w = n_disp * 2 - 1;
        let x0 = inner.x + (inner.width as usize - used_w) as u16 / 2;
        let h8 = inner.height as f64 * 8.0;
        const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

        let buf: &mut Buffer = f.buffer_mut();
        for i in 0..n_disp {
            let a = i * src.len() / n_disp;
            let b = ((i + 1) * src.len() / n_disp).max(a + 1);
            let v = src[a..b].iter().map(|&x| x as f64).sum::<f64>() / (b - a) as f64;
            let mut level = ((v / crate::cava::MAX as f64) * h8).round() as i64;
            for row in 0..inner.height {
                let y = inner.bottom() - 1 - row;
                let cell_lvl = level.clamp(0, 8) as usize;
                level -= 8;
                if cell_lvl == 0 {
                    continue;
                }
                // gradient: muted primary at the bottom → full primary at the top
                let t = 0.55 * (1.0 - row as f32 / inner.height.max(1) as f32);
                let fg: Color = th.accent_fade(t);
                if let Some(c) = buf.cell_mut((x0 + (i * 2) as u16, y)) {
                    c.set_symbol(EIGHTHS[cell_lvl]).set_fg(fg);
                }
            }
        }
    }

    // ── footer ──────────────────────────────────────────────────────────────
    fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
        let th = &app.theme;
        let dim = Style::new().fg(th.fade(0.45));
        let source = if app.args.cover_theme {
            format!("{} + cover accent", th.source)
        } else {
            th.source.clone()
        };
        let left = Line::from(vec![
            Span::styled(" ◆ ", Style::new().fg(th.primary())),
            Span::styled(source, dim),
            Span::styled("  │  ", Style::new().fg(th.fade(0.7))),
            Span::styled(
                format!("offset {:+.1}s", app.offset),
                if app.offset != 0.0 {
                    Style::new().fg(th.primary())
                } else {
                    dim
                },
            ),
        ]);
        let hints = "space ⏯  n/p  ←/→ seek  ,/. sync  l lyrics  b big  c cava  q quit ";
        let left_w = left.width() as u16;
        f.render_widget(Paragraph::new(left), area);
        if left_w + hints.chars().count() as u16 + 2 <= area.width {
            f.render_widget(
                Paragraph::new(Line::styled(hints, dim)).alignment(Alignment::Right),
                area,
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// main: terminal setup, worker spawn, event loop, clean teardown
// ════════════════════════════════════════════════════════════════════════════
#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Args::parse();
    let theme = theme::load(&args);

    // ratatui::init = raw mode + alternate screen + a panic hook that restores both.
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        prev_hook(info);
    }));

    // Query graphics protocol + font size *after* entering the alt screen, *before* reading events.
    // (ASCII covers need no graphics protocol, so skip the terminal query entirely.)
    let mut picker = if args.ascii_cover {
        ratatui_image::picker::Picker::halfblocks()
    } else {
        ratatui_image::picker::Picker::from_query_stdio()
            .unwrap_or_else(|_| ratatui_image::picker::Picker::halfblocks())
    };
    // Only fill image padding with the surface colour when it is a real, painted colour.
    let bg = theme
        .surface
        .filter(|_| !args.transparent)
        .map(|s| image::Rgba([s.0, s.1, s.2, 255]));
    picker.set_background_color(bg);

    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
    let app = app::App::new(args, theme, picker, tx.clone());

    // Player workers: periodic poll + instant change stream.
    tokio::spawn(app.ctl.clone().poll_loop(tx.clone()));
    tokio::spawn(app.ctl.clone().follow_loop(tx.clone()));

    let res = run(&mut terminal, app, rx).await;

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: app::App,
    mut rx: mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(16));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_draw = Instant::now() - Duration::from_secs(1);

    loop {
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(ev)) => if app.handle(ev) { break },
                Some(Err(_)) | None => break,
            },
            Some(msg) = rx.recv() => {
                app.apply(msg);
                // Drain any backlog (e.g. a burst of cava frames) before drawing.
                while let Ok(m) = rx.try_recv() { app.apply(m); }
            }
            _ = tick.tick() => {
                // Redraw when something changed, while the lyric scroll is easing,
                // or ~10×/s during playback (clock, progress bar, lyric line changes).
                let periodic = app.playing() && last_draw.elapsed() >= Duration::from_millis(100);
                if app.dirty || app.animating() || periodic {
                    terminal.draw(|f| ui::draw(f, &mut app))?;
                    app.dirty = false;
                    last_draw = Instant::now();
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_parsing() {
        let l = lyrics::parse_lrc(
            "[ar:x]\n[00:12.50]hello\n[00:05.00][00:20.00]multi\n[00:30]\n[offset:+500]",
        );
        assert_eq!(l.len(), 4);
        assert!((l[0].0 - 4.5).abs() < 1e-9 && l[0].1 == "multi");
        assert_eq!(l[1].1, "hello");
        assert_eq!(l[3].1, "♪");
    }

    #[test]
    fn title_cleaning() {
        assert_eq!(
            year::clean("Come Together - Remastered 2009"),
            "Come Together"
        );
        assert_eq!(
            year::clean("Song (Live at Wembley) [Deluxe Edition]"),
            "Song"
        );
        assert_eq!(year::clean("Normal Title"), "Normal Title");
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("md-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn theme_from(extra: &[&str], file: Option<&str>) -> theme::Theme {
        let mut argv = vec!["x".to_string()];
        if let Some(content) = file {
            let p = tmp("theme").join(format!("{}.json", util::fnv1a(content)));
            std::fs::write(&p, content).unwrap();
            argv.extend(["-c".into(), p.to_str().unwrap().to_string()]);
        }
        argv.extend(extra.iter().map(|s| s.to_string()));
        theme::load(&cli::Args::parse_from(argv))
    }

    #[test]
    fn hex_and_theme_schemas() {
        assert_eq!(theme::parse_hex("#ff8000"), Some((255, 128, 0)));
        assert_eq!(theme::parse_hex("ff8000cc"), Some((255, 128, 0)));
        let cases = [
            r##"{"mSurface":"#101010","mPrimary":"#ff0000"}"##,
            r##"{"colors":{"dark":{"surface":"#101010","primary":"#ff0000"}}}"##,
            r##"{"colors":{"surface":{"dark":"#101010","light":"#ffffff"},"primary":{"dark":{"color":"#ff0000"}}}}"##,
            r##"{"dark":{"mSurface":"#101010","mPrimary":"#ff0000"},"light":{"mSurface":"#eeeeee"}}"##,
        ];
        for (i, c) in cases.iter().enumerate() {
            let t = theme_from(&[], Some(c));
            assert_eq!(t.surface, Some((16, 16, 16)), "case {i}");
            assert_eq!(t.primary, Some((255, 0, 0)), "case {i}");
            assert!(t.on_surface.is_some(), "case {i}");
        }
    }

    #[test]
    fn empty_or_invalid_json_falls_back_to_terminal_colors() {
        for bad in [
            "{}",
            "",
            "not json",
            "[]",
            r#"{"foo": 1}"#,
            r##"{"mOnSurface":"#fff"}"##,
        ] {
            let t = theme_from(&[], Some(bad));
            assert_eq!(
                (t.surface, t.primary, t.on_surface),
                (None, None, None),
                "{bad:?}"
            );
            assert!(
                t.source.starts_with("terminal colors"),
                "{bad:?}: {}",
                t.source
            );
            // terminal slots map to reset/ANSI colours, never a hex value
            assert_eq!(t.surface(), ratatui::style::Color::Reset);
            assert_eq!(t.text(), ratatui::style::Color::Reset);
            assert_eq!(t.primary(), ratatui::style::Color::Cyan);
            assert!(!matches!(t.fade(0.5), ratatui::style::Color::Rgb(..)));
            assert!(!matches!(
                t.accent_fade(0.6),
                ratatui::style::Color::Rgb(..)
            ));
        }
        // a missing explicit file behaves the same
        let t = theme::load(&cli::Args::parse_from([
            "x",
            "-c",
            "/nonexistent/colors.json",
        ]));
        assert_eq!((t.surface, t.primary), (None, None));
    }

    #[test]
    fn terminal_colors_flag_bypasses_files_and_accepts_overrides() {
        let file = r##"{"mSurface":"#101010","mPrimary":"#ff0000"}"##;
        let t = theme_from(&["--terminal-colors"], Some(file));
        assert_eq!((t.surface, t.primary, t.on_surface), (None, None, None));
        assert_eq!(t.source, "terminal colors");

        // both overrides together → fully custom
        let t = theme_from(
            &[
                "--terminal-colors",
                "--surface",
                "#112233",
                "--primary",
                "#ffaa00",
            ],
            Some(file),
        );
        assert_eq!(t.surface, Some((0x11, 0x22, 0x33)));
        assert_eq!(t.primary, Some((0xff, 0xaa, 0x00)));
        assert!(t.on_surface.is_some());
        assert!(t.source.contains("CLI overrides"));

        // only --primary: the background and text stay the terminal's own
        let t = theme_from(&["--terminal-colors", "--primary", "#ffaa00"], None);
        assert_eq!((t.surface, t.on_surface), (None, None));
        assert_eq!(t.primary, Some((0xff, 0xaa, 0x00)));
        assert_eq!(t.surface(), ratatui::style::Color::Reset);

        // only --surface: text colour is derived so it stays readable on it
        let t = theme_from(&["--terminal-colors", "--surface", "#101010"], None);
        assert_eq!(t.surface, Some((16, 16, 16)));
        assert!(theme::luminance(t.on_surface.unwrap()) > 0.7);
        assert_eq!(t.primary(), ratatui::style::Color::Cyan);
    }

    #[test]
    fn accent_from_cover() {
        let solid = |c: [u8; 3]| {
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(32, 32, image::Rgb(c)))
        };
        let a = cover::dominant_accent(&solid([220, 30, 40])).unwrap();
        assert!(a.0 > 200 && a.1 < 60 && a.2 < 70, "{a:?}");

        // a mostly-black cover with a small vivid blue patch → blue, not gray/black
        let mut img = image::RgbImage::from_pixel(64, 64, image::Rgb([8, 8, 8]));
        for y in 0..24 {
            for x in 0..24 {
                img.put_pixel(x, y, image::Rgb([20, 60, 230]));
            }
        }
        let a = cover::dominant_accent(&image::DynamicImage::ImageRgb8(img)).unwrap();
        assert!(a.2 > 180 && a.0 < 60, "{a:?}");

        // grayscale → falls back to a neutral instead of None; pure black → None
        assert!(cover::dominant_accent(&solid([140, 140, 140])).is_some());
        assert!(cover::dominant_accent(&solid([0, 0, 0])).is_none());

        // readable() lifts a dark accent on a dark surface and keeps unknown-bg accents bright
        let dark_bg = Some((16, 16, 16));
        let r = theme::readable((30, 20, 90), dark_bg);
        assert!(
            theme::luminance(r) - theme::luminance((16, 16, 16)) >= 0.4 - 1e-3,
            "{r:?}"
        );
        assert!(theme::luminance(theme::readable((10, 10, 40), None)) >= 0.35);
    }

    #[test]
    fn ascii_cover_dimensions_and_charset() {
        let mut img = image::RgbImage::new(64, 64);
        for (x, _, p) in img.enumerate_pixels_mut() {
            let v = (x * 4) as u8; // left → right gradient
            *p = image::Rgb([v, 40, 200 - v / 2]);
        }
        let art = cover::ascii_art(&image::DynamicImage::ImageRgb8(img), 20, 10);
        assert_eq!((art.cols, art.rows, art.cells.len()), (20, 10, 200));
        assert!(
            art.cells
                .iter()
                .all(|(c, _)| c.is_ascii() && !c.is_control())
        );
        // dark side is sparse, bright side is dense
        let density = |c: char| " .:-=+*#%@".find(c).unwrap();
        assert!(density(art.cells[0].0) < density(art.cells[19].0));
    }

    #[test]
    fn big_font_renders_and_falls_back() {
        let rows = bigfont::render("Hi there", 60, 3).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(
            rows.iter()
                .all(|r| r.chars().count() == rows[0].chars().count())
        );
        assert!(rows.iter().any(|r| r.contains('█')));
        // wraps into pieces (4 rows each + 1 blank row between) when narrow
        let rows = bigfont::render("go on", 24, 3).unwrap();
        assert_eq!(rows.len(), 9);
        let rows = bigfont::render("hello world again", 40, 3).unwrap();
        assert_eq!(rows.len(), 14);
        assert!(rows.iter().all(|r| r.chars().count() <= 40));
        // too many pieces / word wider than the panel / unsupported script → None
        assert!(bigfont::render("one two three four five six seven", 20, 2).is_none());
        assert!(bigfont::render("supercalifragilistic", 30, 3).is_none());
        assert!(bigfont::render("你好", 60, 3).is_none());
        // accents fold, curly quotes and ellipsis work
        assert!(bigfont::render("Café – don’t…", 100, 1).is_some());
    }

    #[test]
    fn three_panel_side_columns_are_equal() {
        for w in [80u16, 100, 120, 139, 140, 200, 301] {
            let main = ratatui::layout::Rect::new(0, 0, w, 30);
            let [l, m, r] = ui::three_panel(main);
            assert_eq!(l.width, r.width, "width {w}");
            assert_eq!(l.width + m.width + r.width, w, "width {w}");
            assert!(m.width >= 20, "width {w}");
            assert_eq!((l.x, m.x, r.x), (0, l.width, l.width + m.width));
        }
    }

    #[test]
    fn cava_runs_mono() {
        let c = cava::config(48);
        assert!(c.contains("channels = mono"));
        assert!(c.contains("bars = 48"));
        assert!(c.contains("method = raw") && c.contains("data_format = ascii"));
    }

    #[test]
    fn percent() {
        assert_eq!(util::percent_decode("/a%20b/c%C3%A9.jpg"), "/a b/cé.jpg");
    }
}
