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
//!   * optional guitar chords (`g`) from local ChordPro / plain-text sheets, matched onto the
//!     synced lyrics line by line; transposable
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
        /// Use this colour file instead of auto-detecting Noctalia/Matugen [no config key]
        #[arg(short = 'c', long = "colors", value_name = "FILE")]
        pub colors: Option<PathBuf>,

        /// Force the surface (background/shadow) colour, e.g. '#1e1e2e' [no config key]
        #[arg(long, value_name = "HEX")]
        pub surface: Option<String>,

        /// Force the primary (accent/highlight) colour, e.g. '#cba6f7' [no config key]
        #[arg(long, value_name = "HEX")]
        pub primary: Option<String>,

        /// Start with the lyrics panel hidden (toggle with `l`) [no config key]
        #[arg(long)]
        pub nolyrics: bool,

        /// Start with the cava visualiser shown (toggle with `c`) [config: cava]
        #[arg(long)]
        pub cava: bool,

        /// Put cava in its own third column: Metadata | Lyrics | Cava [config: thirdpanel]
        #[arg(long, conflicts_with = "underlyrics")]
        pub thirdpanel: bool,

        /// Put cava underneath the lyrics panel (default placement) [no config key]
        #[arg(long)]
        pub underlyrics: bool,

        /// MPRIS player name passed to `playerctl -p` ("any" = playerctl's default choice) [config: player]
        #[arg(long, default_value = "spotify")]
        pub player: String,

        /// Cover tint strength: 0 = original colours, 1 = full surface→primary duotone [config: tint]
        #[arg(long, default_value_t = 1.0)]
        pub tint: f32,

        /// Default lyric offset in seconds, applied on every new track (+ = lyrics earlier) [config: offset]
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        pub offset: f64,

        /// Number of bars cava renders (resampled to the panel width) [config: cava_bars]
        #[arg(long, default_value_t = 64)]
        pub cava_bars: u16,

        /// Don't paint the surface colour as background (keep terminal transparency) [config: transparent]
        #[arg(long)]
        pub transparent: bool,

        /// Inherit the terminal's own colours: default background/foreground and the ANSI
        /// accent. Skips every colour file. --surface/--primary still override their slot.
        /// [config: terminal_colors]
        #[arg(long)]
        pub terminal_colors: bool,

        /// Draw the cover as coloured ASCII characters instead of a terminal graphics image [config: ascii_cover]
        #[arg(long)]
        pub ascii_cover: bool,

        /// Take the UI accent colour from the current album cover (updates every track) [config: cover_theme]
        #[arg(long)]
        pub cover_theme: bool,

        /// Render the active lyric line in a big block font (toggle with `b`) [config: big_lyrics]
        #[arg(long)]
        pub big_lyrics: bool,

        /// Show guitar chords above the lyrics (toggle with `g`). Sheets are read from
        /// --chords-dir as `Artist - Title.cho` (ChordPro) or `.txt` (chords over lyrics)
        /// [config: chords]
        #[arg(long)]
        pub chords: bool,

        /// Folder with your chord sheets [default: ~/.local/share/music_display/chords] [config: chords_dir]
        #[arg(long, value_name = "DIR")]
        pub chords_dir: Option<PathBuf>,

        /// Also look chords up on Ultimate Guitar when there is no local sheet (unofficial,
        /// opt-in; results are cached) [config: chords_ug]
        #[arg(long)]
        pub chords_ug: bool,

        /// Experimental: show which group member sings each line, scraped from Genius
        /// (unofficial, off by default; results are cached) [config: member_parts]
        #[arg(long)]
        pub member_parts: bool,

        /// Diagnose the Ultimate Guitar lookup for one song, print the steps and exit [no config key]
        #[arg(long, num_args = 2, value_names = ["ARTIST", "TITLE"])]
        pub chords_probe: Option<Vec<String>>,

        /// With --chords-probe: save the raw pages here (to use as test fixtures) [no config key]
        #[arg(long, value_name = "DIR")]
        pub probe_save: Option<PathBuf>,

        /// Limit chords to the active line plus N upcoming ones (default 99 = every line) [config: chords_context]
        #[arg(long, default_value_t = 99, value_parser = clap::value_parser!(u16).range(0..=99))]
        pub chords_context: u16,

        /// Blank rows between lyric lines (0-3) [config: lyric_spacing]
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(0..=3))]
        pub lyric_spacing: u16,

        /// Size of the big-font line, 1-3 times the base size (toggle the font with `b`) [config: big_scale]
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=3))]
        pub big_scale: u16,

        /// Extra blank columns on each side of the lyrics (0-10) [config: lyric_pad]
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u16).range(0..=10))]
        pub lyric_pad: u16,

        /// Hide the "terminal colors (no usable colour file)" note in the footer [config: hide_color_warning]
        #[arg(long)]
        pub hide_color_warning: bool,

        /// Volume change per `+` / `-` / mouse-wheel notch, in percent [config: volume_step]
        #[arg(long, default_value_t = DEFAULT_VOLUME_STEP, value_parser = clap::value_parser!(u32).range(1..=50))]
        pub volume_step: u32,

        /// A silence between two lyric lines longer than this many seconds shows
        /// `[ Instrumental / Solo ]` [config: interlude_threshold_secs]
        #[arg(long = "interlude-secs", default_value_t = DEFAULT_INTERLUDE_SECS)]
        pub interlude_secs: f64,

        /// Retries after a failed lyrics request (timeout, 429, 5xx), with exponential backoff
        /// [config: lyrics_retries]
        #[arg(long, default_value_t = DEFAULT_LYRICS_RETRIES, value_parser = clap::value_parser!(u32).range(0..=10))]
        pub lyrics_retries: u32,

        /// Show lyrics translated (Google Translate, unofficial; toggle with `K`) [config: translate]
        #[arg(long)]
        pub translate: bool,

        /// Language code lyrics are translated into, e.g. en, sv, de [config: translate_lang]
        #[arg(long, default_value = "en")]
        pub translate_lang: String,

        /// Fall back to LibreTranslate when Google Translate is rate-limited or unreachable
        /// [config: translate_fallback]
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set, value_name = "BOOL")]
        pub translate_fallback: bool,

        /// LibreTranslate endpoint used as the fallback [config: translate_fallback_url]
        #[arg(long, default_value = crate::translate::LIBRE_URL, value_name = "URL")]
        pub translate_fallback_url: String,

        /// API key for the LibreTranslate endpoint (libretranslate.com needs one) [config: translate_fallback_key]
        #[arg(long, value_name = "KEY")]
        pub translate_fallback_key: Option<String>,

        /// Start in the minimal teleprompter view (toggle with `M`) [config: minimal_view]
        #[arg(long)]
        pub minimal: bool,

        /// Don't capture the mouse (no click / wheel controls) [config: mouse = false]
        #[arg(long)]
        pub no_mouse: bool,

        /// Read settings from this file instead of ~/.config/music_display/config.toml [no config key]
        #[arg(long, value_name = "FILE")]
        pub config: Option<PathBuf>,

        /// Print the effective configuration (defaults + config file + flags) and exit [no config key]
        #[arg(long)]
        pub print_config: bool,
    }

    pub const DEFAULT_VOLUME_STEP: u32 = 5;
    pub const DEFAULT_INTERLUDE_SECS: f64 = 8.0;
    pub const DEFAULT_LYRICS_RETRIES: u32 = 3;
}

// ════════════════════════════════════════════════════════════════════════════
// Config file: ~/.config/music_display/config.toml (flags > file > built-in defaults)
// ════════════════════════════════════════════════════════════════════════════
mod config {
    use crate::cli::Args;
    use clap::parser::ValueSource;
    use clap::ArgMatches;
    use std::path::{Path, PathBuf};

    pub enum Kind {
        Bool,
        Int(i64, i64),
        Float(f64, f64),
        Str,
    }

    pub struct Key {
        pub name: &'static str,
        pub kind: Kind,
        /// The command-line argument it sets (an explicit flag always wins).
        pub arg: &'static str,
    }

    macro_rules! k {
        ($n:literal, $k:expr, $a:literal) => {
            Key { name: $n, kind: $k, arg: $a }
        };
    }

    /// Every setting the file understands.
    pub const KEYS: &[Key] = &[
        k!("player", Kind::Str, "player"),
        k!("offset", Kind::Float(-60.0, 60.0), "offset"),
        k!("tint", Kind::Float(0.0, 1.0), "tint"),
        k!("transparent", Kind::Bool, "transparent"),
        k!("terminal_colors", Kind::Bool, "terminal_colors"),
        k!("ascii_cover", Kind::Bool, "ascii_cover"),
        k!("cover_theme", Kind::Bool, "cover_theme"),
        k!("cava", Kind::Bool, "cava"),
        k!("cava_bars", Kind::Int(4, 1000), "cava_bars"),
        k!("thirdpanel", Kind::Bool, "thirdpanel"),
        k!("big_lyrics", Kind::Bool, "big_lyrics"),
        k!("lyric_spacing", Kind::Int(0, 3), "lyric_spacing"),
        k!("big_scale", Kind::Int(1, 3), "big_scale"),
        k!("lyric_pad", Kind::Int(0, 10), "lyric_pad"),
        k!("hide_color_warning", Kind::Bool, "hide_color_warning"),
        k!("chords", Kind::Bool, "chords"),
        k!("chords_ug", Kind::Bool, "chords_ug"),
        k!("member_parts", Kind::Bool, "member_parts"),
        k!("chords_dir", Kind::Str, "chords_dir"),
        k!("chords_context", Kind::Int(0, 99), "chords_context"),
        k!("volume_step", Kind::Int(1, 50), "volume_step"),
        k!("interlude_threshold_secs", Kind::Float(1.0, 120.0), "interlude_secs"),
        k!("lyrics_retries", Kind::Int(0, 10), "lyrics_retries"),
        k!("translate", Kind::Bool, "translate"),
        k!("translate_lang", Kind::Str, "translate_lang"),
        k!("translate_fallback", Kind::Bool, "translate_fallback"),
        k!("translate_fallback_url", Kind::Str, "translate_fallback_url"),
        k!("translate_fallback_key", Kind::Str, "translate_fallback_key"),
        k!("minimal_view", Kind::Bool, "minimal"),
        k!("mouse", Kind::Bool, "no_mouse"),
    ];

    pub fn default_path() -> PathBuf {
        // dirs::config_dir() honours $XDG_CONFIG_HOME
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("music_display")
            .join("config.toml")
    }

    fn check(kind: &Kind, v: &toml::Value) -> Result<(), String> {
        match kind {
            Kind::Bool => v.as_bool().map(|_| ()).ok_or("expected true or false".into()),
            Kind::Str => match v.as_str() {
                Some(s) if !s.trim().is_empty() => Ok(()),
                _ => Err("expected a non-empty string".into()),
            },
            Kind::Int(lo, hi) => match v.as_integer() {
                Some(n) if (*lo..=*hi).contains(&n) => Ok(()),
                _ => Err(format!("expected a whole number from {lo} to {hi}")),
            },
            Kind::Float(lo, hi) => {
                let f = v.as_float().or_else(|| v.as_integer().map(|n| n as f64));
                match f {
                    Some(f) if f.is_finite() && (*lo..=*hi).contains(&f) => Ok(()),
                    _ => Err(format!("expected a number from {lo} to {hi}")),
                }
            }
        }
    }

    /// 1-based line of `key = …` in the file, for error messages.
    fn line_of(raw: &str, key: &str) -> usize {
        raw.lines()
            .position(|l| {
                let l = l.trim_start();
                let l = l.strip_prefix('"').unwrap_or(l);
                l.strip_prefix(key).is_some_and(|rest| {
                    rest.trim_start_matches('"').trim_start().starts_with('=')
                })
            })
            .map_or(0, |i| i + 1)
    }

    fn set(args: &mut Args, name: &str, v: &toml::Value) {
        let b = || v.as_bool().unwrap_or(false);
        let f = || v.as_float().or_else(|| v.as_integer().map(|n| n as f64)).unwrap_or(0.0);
        let i = || v.as_integer().unwrap_or(0);
        let st = || v.as_str().unwrap_or("").to_string();
        match name {
            "player" => args.player = st(),
            "offset" => args.offset = f(),
            "tint" => args.tint = f() as f32,
            "transparent" => args.transparent = b(),
            "terminal_colors" => args.terminal_colors = b(),
            "ascii_cover" => args.ascii_cover = b(),
            "cover_theme" => args.cover_theme = b(),
            "cava" => args.cava = b(),
            "cava_bars" => args.cava_bars = i() as u16,
            "thirdpanel" => args.thirdpanel = b() && !args.underlyrics,
            "big_lyrics" => args.big_lyrics = b(),
            "lyric_spacing" => args.lyric_spacing = i() as u16,
            "big_scale" => args.big_scale = i() as u16,
            "lyric_pad" => args.lyric_pad = i() as u16,
            "hide_color_warning" => args.hide_color_warning = b(),
            "chords" => args.chords = b(),
            "chords_ug" => args.chords_ug = b(),
            "member_parts" => args.member_parts = b(),
            "chords_dir" => args.chords_dir = Some(PathBuf::from(st())),
            "chords_context" => args.chords_context = i() as u16,
            "volume_step" => args.volume_step = i() as u32,
            "interlude_threshold_secs" => args.interlude_secs = f(),
            "lyrics_retries" => args.lyrics_retries = i() as u32,
            "translate" => args.translate = b(),
            "translate_lang" => args.translate_lang = st(),
            "translate_fallback" => args.translate_fallback = b(),
            "translate_fallback_url" => args.translate_fallback_url = st(),
            "translate_fallback_key" => args.translate_fallback_key = Some(st()),
            "minimal_view" => args.minimal = b(),
            "mouse" => args.no_mouse = !b(),
            _ => {}
        }
    }

    /// Apply the file at `path` to `args` for every setting not given on the command line.
    /// Problems go to `warn`; none of them stop the program. Returns whether a file was read.
    pub fn apply(
        args: &mut Args,
        m: &ArgMatches,
        path: &Path,
        explicit: bool,
        warn: &mut dyn FnMut(String),
    ) -> bool {
        let raw = match std::fs::read_to_string(path) {
            Ok(r) => r,
            Err(e) => {
                // A missing default file is normal; a missing --config file deserves a word.
                if explicit || e.kind() != std::io::ErrorKind::NotFound {
                    warn(format!("{}: {e}; using defaults", path.display()));
                }
                return false;
            }
        };
        let table: toml::Table = match raw.parse() {
            Ok(t) => t,
            Err(e) => {
                warn(format!("{}: {e}; ignoring the file", path.display()));
                return true;
            }
        };
        let at = |key: &str| format!("{}:{}", path.display(), line_of(&raw, key));
        // (line, message): reported in file order, not the table's alphabetical order
        let mut problems: Vec<(usize, String)> = Vec::new();
        for (name, v) in &table {
            let line = line_of(&raw, name);
            let Some(key) = KEYS.iter().find(|k| k.name == name) else {
                problems.push((line, format!("{}: unknown key '{name}' (ignored)", at(name))));
                continue;
            };
            if let Err(why) = check(&key.kind, v) {
                problems.push((
                    line,
                    format!("{}: invalid value for '{name}': {why}; using the default", at(name)),
                ));
                continue;
            }
            if m.value_source(key.arg) == Some(ValueSource::CommandLine) {
                continue; // an explicit flag beats the file
            }
            set(args, name, v);
        }
        problems.sort();
        for (_, p) in problems {
            warn(p);
        }
        true
    }

    /// Write `values` into the config file at `path`, keeping every other key already in it
    /// (comments in the file are not preserved). Creates the folder if needed.
    pub fn save(path: &Path, values: &[(&str, toml::Value)]) -> std::io::Result<()> {
        // A file with a syntax error is left alone rather than overwritten with just our keys.
        let mut table: toml::Table = match std::fs::read_to_string(path) {
            Ok(raw) => raw.parse().map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not valid TOML, left untouched ({e})"))
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
            Err(e) => return Err(e),
        };
        for (k, v) in values {
            table.insert((*k).to_string(), v.clone());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string(&table).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("# written by music_display's settings menu (s)\n{body}"))?;
        std::fs::rename(tmp, path)
    }

    /// The effective settings as TOML (what `--print-config` shows).
    pub fn render(args: &Args, path: &Path, found: bool) -> String {
        let q = |s: &str| format!("{:?}", s);
        let mut o = format!(
            "# effective configuration: built-in defaults < {} < command-line flags\n",
            if found { path.display().to_string() } else { format!("{} (not found)", path.display()) }
        );
        let mut line = |k: &str, v: String| o.push_str(&format!("{k} = {v}\n"));
        line("player", q(&args.player));
        line("offset", format!("{:?}", args.offset));
        line("tint", format!("{:?}", args.tint));
        line("transparent", args.transparent.to_string());
        line("terminal_colors", args.terminal_colors.to_string());
        line("ascii_cover", args.ascii_cover.to_string());
        line("cover_theme", args.cover_theme.to_string());
        line("cava", args.cava.to_string());
        line("cava_bars", args.cava_bars.to_string());
        line("thirdpanel", args.thirdpanel.to_string());
        line("big_lyrics", args.big_lyrics.to_string());
        line("lyric_spacing", args.lyric_spacing.to_string());
        line("big_scale", args.big_scale.to_string());
        line("lyric_pad", args.lyric_pad.to_string());
        line("hide_color_warning", args.hide_color_warning.to_string());
        line("chords", args.chords.to_string());
        line("chords_ug", args.chords_ug.to_string());
        line("member_parts", args.member_parts.to_string());
        match &args.chords_dir {
            Some(d) => line("chords_dir", q(&d.display().to_string())),
            None => line("# chords_dir", "\"~/.local/share/music_display/chords\"  # default".into()),
        }
        line("chords_context", args.chords_context.to_string());
        line("volume_step", args.volume_step.to_string());
        line("interlude_threshold_secs", format!("{:?}", args.interlude_secs));
        line("lyrics_retries", args.lyrics_retries.to_string());
        line("translate", args.translate.to_string());
        line("translate_lang", q(&args.translate_lang));
        line("translate_fallback", args.translate_fallback.to_string());
        line("translate_fallback_url", q(&args.translate_fallback_url));
        if args.translate_fallback_key.is_some() {
            line("# translate_fallback_key", "\"…\"  # set (not shown)".into());
        }
        line("minimal_view", args.minimal.to_string());
        line("mouse", (!args.no_mouse).to_string());
        o
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Small shared helpers
// ════════════════════════════════════════════════════════════════════════════
mod util {
    use std::path::{Path, PathBuf};

    /// Aggressive identity normalisation for track/album/artist strings: drops any
    /// (...) / [...] group containing an edition noise word, trailing "- Remastered 2011"
    /// style suffixes and stray trailing hyphens, then collapses case and whitespace.
    pub fn normalize_track(s: &str) -> String {
        use regex::Regex;
        use std::sync::LazyLock;
        static BRACKET: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)\s*[\(\[][^\)\]]*\b(remaster(ed)?|re-?master|deluxe|anniversar\w*|live|version|edit|mono|stereo|bonus|demo|acoustic|explicit|radio|expanded|edition|reissue|special|collector\w*|super|single)\b[^\)\]]*[\)\]]").unwrap()
        });
        static DASH: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)\s+[-–—]\s+.*\b(remaster(ed)?|re-?master|deluxe|anniversar\w*|live|version|edit|mono|stereo|single|bonus|demo|acoustic|radio|expanded|edition|reissue)\b.*$").unwrap()
        });
        static TRAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s\-–—]+$").unwrap());
        let orig = s;
        let s = BRACKET.replace_all(s, "");
        let s = DASH.replace(&s, "");
        let s = TRAIL.replace(&s, "");
        let fold = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        let out = fold(&s);
        // "(Live)" or "---" as the whole title: keep it rather than collapse to nothing
        if out.is_empty() { fold(orig) } else { out }
    }

    /// Terminal cell width of a char (0 for combining marks, 2 for CJK/Hangul/emoji).
    pub fn cw(c: char) -> usize {
        unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
    }

    /// Terminal cell width of a string.
    pub fn width(s: &str) -> usize {
        // summed per char so it always agrees with how the wrappers measure
        s.chars().map(cw).sum()
    }

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
    const FORMAT: &str = "{{status}}\x1f{{position}}\x1f{{title}}\x1f{{artist}}\x1f{{album}}\x1f{{mpris:length}}\x1f{{mpris:artUrl}}\x1f{{xesam:trackNumber}}\x1f{{xesam:discNumber}}\x1f{{xesam:contentCreated}}\x1f{{volume}}\x1f{{xesam:url}}\x1f{{mpris:trackid}}\x1f{{playerName}}";

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
    pub fn parse_shuffle(out: &str) -> Option<bool> {
        match out.trim() {
            "On" => Some(true),
            "Off" => Some(false),
            _ => None,
        }
    }

    /// `playerctl loop` prints `None` / `Track` / `Playlist`.
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
        /// and the album are ignored, so every release of a song is the same track.
        pub fn key(&self) -> u64 {
            let first_artist = self.artist.split(", ").next().unwrap_or(&self.artist);
            crate::util::fnv1a(&format!(
                "{}\0{}",
                crate::util::normalize_track(first_artist),
                crate::util::normalize_track(&crate::year::base_title(&self.title)),
            ))
        }
    }

    fn parse(line: &str) -> Option<Snapshot> {
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
        base: Arc<std::sync::RwLock<Vec<String>>>,
        pub repoll: Arc<Notify>,
        /// The follow task restarts its `playerctl --follow` when this fires.
        switched: Arc<Notify>,
        /// Last shuffle / repeat read, stamped onto every snapshot (the follow task and the
        /// poller both send them).
        modes: Arc<std::sync::RwLock<(Option<bool>, Option<Loop>)>>,
    }

    fn player_args(player: &str) -> Vec<String> {
        if player == "any" || player.is_empty() {
            vec![]
        } else {
            vec!["-p".to_string(), player.to_string()]
        }
    }

    /// Players the settings menu cycles through; `local` is the usual desktop players.
    pub const PRESETS: &[&str] = &[
        "spotify",
        "vlc",
        "mpv",
        "rhythmbox",
        "local",
        "any",
    ];

    /// What a menu name means to playerctl (`local` = any of the common local players).
    pub fn expand(player: &str) -> String {
        match player {
            "local" => "vlc,mpv,rhythmbox,audacious,strawberry,deadbeef,elisa,lollypop,mpd".into(),
            p => p.into(),
        }
    }

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
}

// ════════════════════════════════════════════════════════════════════════════
// Release-year resolution: contentCreated → cache → MusicBrainz → iTunes
// ════════════════════════════════════════════════════════════════════════════
mod tags {
    //! Tags read straight from a local audio file (ID3, Vorbis/FLAC, MP4…), for players
    //! whose MPRIS metadata is incomplete.
    use lofty::file::TaggedFileExt;
    use lofty::tag::Accessor;
    use std::path::PathBuf;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Tags {
        pub title: String,
        pub artist: String,
        pub album: String,
        /// `YYYY` (tags rarely carry more).
        pub year: String,
        pub track: Option<u32>,
        pub disc: Option<u32>,
        /// `file://` URL of the embedded cover, written to the cache folder.
        pub art_url: String,
    }

    /// `file:///home/me/My%20Song.flac` → the path.
    pub fn path_of(url: &str) -> Option<PathBuf> {
        url.strip_prefix("file://").map(|p| PathBuf::from(crate::util::percent_decode(p)))
    }

    /// File name without folder and extension — what to show when there are no tags at all.
    pub fn title_from_url(url: &str) -> String {
        path_of(url)
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    /// Blocking: call from `spawn_blocking`.
    pub fn read(url: &str) -> Option<Tags> {
        let path = path_of(url)?;
        let file = lofty::read_from_path(&path).ok()?;
        let tag = file.primary_tag().or_else(|| file.first_tag())?;
        let s = |v: Option<std::borrow::Cow<'_, str>>| v.map(|c| c.trim().to_string()).unwrap_or_default();
        let art_url = tag
            .pictures()
            .first()
            .and_then(|pic| {
                let p = crate::util::cache_dir().join(format!("tagart-{:016x}", crate::util::fnv1a(url)));
                std::fs::create_dir_all(p.parent()?).ok()?;
                std::fs::write(&p, pic.data()).ok()?;
                Some(format!("file://{}", p.display()))
            })
            .unwrap_or_default();
        Some(Tags {
            title: s(tag.title()),
            artist: s(tag.artist()),
            album: s(tag.album()),
            year: tag.date().map(|d| d.year.to_string()).unwrap_or_default(),
            track: tag.track(),
            disc: tag.disk(),
            art_url,
        })
    }
}

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

    /// The bare song title: no (…) / […] groups, no `feat.` / `ft.` credits, nothing after
    /// a ` - ` (demo, remastered…). Used to retry a failed chord lookup.
    pub fn base_title(s: &str) -> String {
        static GROUPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[\(\[][^\)\]]*[\)\]]").unwrap());
        static FEAT: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)\s+(feat\.?|ft\.?|featuring)\s+.*$").unwrap());
        static DASH_ANY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+[-–—]\s+.*$").unwrap());
        let t = GROUPS.replace_all(s, "");
        let t = FEAT.replace(&t, "");
        let t = DASH_ANY.replace(&t, "");
        let t = t.trim();
        if t.is_empty() { s.trim().to_string() } else { t.to_string() }
    }

    /// `"2011-09-26T…"` / `"2011"` → `Some("2011")`.
    /// `YYYY-MM-DD` from an MPRIS `contentCreated` ("1969-09-26T00:00:00Z"); `None` if it
    /// only carries a year (or nothing).
    pub fn full_date(s: &str) -> Option<String> {
        let d = s.trim().get(..10)?;
        let b = d.as_bytes();
        let ok = b[4] == b'-'
            && b[7] == b'-'
            && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
        let (m, day) = (&d[5..7], &d[8..10]);
        (ok && m != "00" && day != "00").then(|| d.to_string())
    }

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

    /// `"1991-08-12T…"` → `"1991-08-12"`, `"1991-08"` → `"1991"`: the full date only when all
    /// three parts are there.
    fn date_of(date: &str) -> Option<String> {
        let y = year_of(date)?;
        Some(full_date(date).unwrap_or_else(|| y.to_string()))
    }

    /// Earlier year wins; for the same year the more precise date does.
    fn rank(d: &str) -> (i32, std::cmp::Reverse<usize>) {
        (year_of(d).unwrap_or(0), std::cmp::Reverse(d.len()))
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
    fn earliest(v: &Value, list: &str) -> Option<String> {
        v.get(list)?
            .as_array()?
            .iter()
            .filter(|r| r.get("score").and_then(Value::as_i64).unwrap_or(0) >= 90)
            .filter_map(|r| {
                r.get("first-release-date")
                    .and_then(Value::as_str)
                    .and_then(date_of)
            })
            .min_by_key(|d| rank(d))
    }

    async fn mb_recording(
        c: &reqwest::Client,
        title: &str,
        artist: &str,
    ) -> Result<Option<String>, ()> {
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
    ) -> Result<Option<String>, ()> {
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

    async fn itunes(c: &reqwest::Client, title: &str, artist: &str) -> Result<Option<String>, ()> {
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
                    .and_then(date_of)
            })
            .min_by_key(|d| rank(d)))
    }

    /// Returns `Some("YYYY-MM-DD" or "YYYY")` or `None` (= "N/A"). Never blocks the UI: runs in its own task.
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
        let mut found: Option<String> = None;

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
                let _ = tokio::fs::write(&path, &y).await;
                Some(y)
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

    pub fn is_hangul(c: char) -> bool {
        matches!(c as u32, 0xAC00..=0xD7A3 | 0x1100..=0x11FF | 0x3130..=0x318F)
    }

    /// Share of the letters in `text` that are Hangul (0.0 when there are no letters).
    pub fn hangul_ratio(text: &str) -> f32 {
        let (mut h, mut n) = (0usize, 0usize);
        for c in text.chars().filter(|c| c.is_alphabetic()) {
            n += 1;
            h += is_hangul(c) as usize;
        }
        if n == 0 { 0.0 } else { h as f32 / n as f32 }
    }

    /// Whether a lyric text is mostly Hangul.
    fn is_hangul_text(text: &str) -> bool {
        hangul_ratio(text) >= 0.3
    }

    /// Best search hit: synced with matching duration > synced > plain/instrumental. In a
    /// Korean context (`korean`, or any candidate being mostly Hangul) the more-Hangul text
    /// wins among equals, so the Hangul lyrics beat the romanized ones.
    fn pick(recs: Vec<Rec>, length: f64, korean: bool) -> Option<(&'static str, String)> {
        let close = |r: &Rec| length <= 0.0 || r.duration.is_none_or(|d| (d - length).abs() <= 4.0);
        let korean = korean || recs.iter().any(|r| classify(r).is_some_and(|c| is_hangul_text(&c.1)));
        let rank = |r: &Rec| match (classify(r).map(|c| c.0), close(r)) {
            (Some("synced"), true) => 0,
            (Some("synced"), false) => 1,
            (Some(_), true) => 2,
            (Some(_), false) => 3,
            (None, _) => 9,
        };
        // Hangul first within the synced > plain order, then the duration match
        let hangul = |r: &Rec| match classify(r) {
            Some((_, t)) if korean => 1000 - (hangul_ratio(&t) * 1000.0) as u32,
            _ => 0,
        };
        let kind = |r: &Rec| rank(r) / 2 * 2; // 0 synced, 2 plain, 8 none
        recs.iter()
            .filter(|r| rank(r) < 9)
            .min_by_key(|r| (kind(r), hangul(r), rank(r)))
            .and_then(classify)
    }

    pub const BASE: &str = "https://lrclib.net";

    /// How hard to try before giving up on a lookup.
    #[derive(Clone, Copy, Debug)]
    pub struct Policy {
        /// Retries after the first attempt (so `3` = up to 4 requests per step).
        pub retries: u32,
        /// First backoff; doubles each retry (500 ms, 1 s, 2 s …).
        pub backoff: std::time::Duration,
    }

    impl Default for Policy {
        fn default() -> Self {
            Policy {
                retries: 3,
                backoff: std::time::Duration::from_millis(500),
            }
        }
    }

    enum Reply {
        Body(String),
        /// HTTP 404: the server answered and has nothing.
        NotFound,
        /// Retries exhausted (timeout, disconnect, 429, 5xx …); the text says what.
        Failed(&'static str),
    }

    /// GET with retry + exponential backoff. 404 is a real answer (no retry). Timeouts,
    /// connection errors, 429 (honouring `Retry-After`) and 5xx are retried.
    async fn request(
        c: &reqwest::Client,
        url: &str,
        query: &[(&str, &str)],
        retries: u32,
        backoff: std::time::Duration,
    ) -> Reply {
        let mut why = "network error";
        for attempt in 0..=retries {
            let mut wait = backoff * 2u32.pow(attempt);
            match c.get(url).query(query).send().await {
                Ok(r) => {
                    let st = r.status();
                    if st == reqwest::StatusCode::NOT_FOUND {
                        return Reply::NotFound;
                    }
                    if st.is_success() {
                        match r.text().await {
                            Ok(t) => return Reply::Body(t),
                            Err(_) => why = "network error", // dropped mid-body
                        }
                    } else if st == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        why = "rate limited";
                        if let Some(s) = r
                            .headers()
                            .get(reqwest::header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.trim().parse::<u64>().ok())
                        {
                            wait = std::time::Duration::from_secs(s.min(10));
                        }
                    } else if st.is_server_error() {
                        why = "server error";
                    } else {
                        return Reply::Failed("unexpected reply"); // other 4xx: retrying won't help
                    }
                }
                Err(_) => why = "network error",
            }
            if attempt < retries {
                tokio::time::sleep(wait).await;
            }
        }
        Reply::Failed(why)
    }

    /// Result of one step of the fallback chain.
    enum Step<T> {
        Hit(T),
        Nothing,
        Failed(&'static str),
    }

    async fn get(
        c: &reqwest::Client,
        base: &str,
        q: &[(&str, &str)],
        retries: u32,
        p: &Policy,
    ) -> Step<Rec> {
        match request(c, &format!("{base}/api/get"), q, retries, p.backoff).await {
            Reply::Body(t) => match serde_json::from_str::<Rec>(&t) {
                Ok(r) => Step::Hit(r),
                Err(_) => Step::Failed("unreadable reply"),
            },
            Reply::NotFound => Step::Nothing,
            Reply::Failed(w) => Step::Failed(w),
        }
    }

    async fn search(
        c: &reqwest::Client,
        base: &str,
        artist: &str,
        title: &str,
        retries: u32,
        p: &Policy,
    ) -> Step<Vec<Rec>> {
        let q = [("artist_name", artist), ("track_name", title)];
        match request(c, &format!("{base}/api/search"), &q, retries, p.backoff).await {
            Reply::Body(t) => match serde_json::from_str::<Vec<Rec>>(&t) {
                Ok(v) => Step::Hit(v),
                Err(_) => Step::Failed("unreadable reply"),
            },
            Reply::NotFound => Step::Nothing,
            Reply::Failed(w) => Step::Failed(w),
        }
    }

    pub async fn fetch(
        c: reqwest::Client,
        artist: String,
        title: String,
        album: String,
        length: f64,
        policy: Policy,
    ) -> Lyrics {
        fetch_with(&c, BASE, &artist, &title, &album, length, policy, true).await
    }

    /// Fallback chain: (a) exact artist+title+album+duration, (b) first artist + cleaned
    /// title without album/duration, (c) search by (cleaned) title. Each step retries with
    /// backoff; once a step has exhausted its retries the rest try once each instead of
    /// multiplying the wait. "Not found" is only reported — and cached — when every step got
    /// a real answer.
    #[allow(clippy::too_many_arguments)]
    pub async fn fetch_with(
        c: &reqwest::Client,
        base: &str,
        artist: &str,
        title: &str,
        album: &str,
        length: f64,
        policy: Policy,
        use_cache: bool,
    ) -> Lyrics {
        let key = fnv1a(&format!(
            "{artist}\0{title}\0{album}\0{}",
            length.round() as i64
        ));
        let path = cache_dir().join(format!("lyrics-{key:016x}.json"));
        if use_cache {
            if let Ok(txt) = tokio::fs::read_to_string(&path).await {
                if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
                    if e.kind != "none" || now_secs().saturating_sub(e.saved) < NEG_TTL {
                        return to_lyrics(&e.kind, &e.text);
                    }
                }
            }
        }

        let cleaned = crate::year::clean(title);
        let first = artist.split(", ").next().unwrap_or(artist).to_string();
        // a Korean title or artist: romanized lyrics are worth a search for Hangul ones
        let korean = title.chars().chain(artist.chars()).any(is_hangul);
        let mut failure: Option<&'static str> = None;
        let mut result: Option<(&'static str, String)> = None;
        let mut retries = policy.retries;
        let mut note = |w: &'static str, retries: &mut u32| {
            failure = Some(w);
            *retries = 0; // network is struggling: one try per remaining step
        };

        // (a) exact signature
        let dur = format!("{}", length.round() as i64);
        let mut q = vec![("artist_name", artist), ("track_name", title)];
        if !album.is_empty() {
            q.push(("album_name", album));
        }
        if length > 0.0 {
            q.push(("duration", &dur));
        }
        match get(c, base, &q, retries, &policy).await {
            Step::Hit(r) => result = classify(&r),
            Step::Nothing => {}
            Step::Failed(w) => note(w, &mut retries),
        }

        // good enough to stop: synced, and Hangul when the song is Korean
        let done = |r: &Option<(&'static str, String)>| {
            r.as_ref().is_some_and(|r| r.0 == "synced" && (!korean || is_hangul_text(&r.1)))
        };

        // (b) first artist + cleaned title, no album / duration
        if !done(&result) {
            let q = [("artist_name", first.as_str()), ("track_name", cleaned.as_str())];
            match get(c, base, &q, retries, &policy).await {
                Step::Hit(r) => {
                    if let Some(hit) = classify(&r) {
                        let better = match &result {
                            None => true,
                            Some(x) => {
                                (x.0 != "synced" && hit.0 == "synced")
                                    || (korean && is_hangul_text(&hit.1) && !is_hangul_text(&x.1))
                            }
                        };
                        if better {
                            result = Some(hit);
                        }
                    }
                }
                Step::Nothing => {}
                Step::Failed(w) => note(w, &mut retries),
            }
        }

        // (c) search: cleaned title, then the raw title, then the first artist alone
        if !done(&result) {
            let mut tries = vec![(artist.to_string(), cleaned.clone())];
            if cleaned != title {
                tries.push((artist.to_string(), title.to_string()));
            }
            if first != artist {
                tries.push((first.clone(), cleaned.clone()));
            }
            for (a, t) in tries {
                match search(c, base, &a, &t, retries, &policy).await {
                    Step::Hit(recs) => {
                        if let Some(hit) = pick(recs, length, korean) {
                            let better = match &result {
                                None => true,
                                Some(x) => {
                                    (x.0 != "synced"
                                        && !(hit.0 != "synced" && korean && is_hangul_text(&x.1) && !is_hangul_text(&hit.1)))
                                        || (korean && is_hangul_text(&hit.1) && !is_hangul_text(&x.1))
                                }
                            };
                            if better {
                                result = Some(hit);
                            }
                            if done(&result) {
                                break;
                            }
                        }
                    }
                    Step::Nothing => {}
                    Step::Failed(w) => note(w, &mut retries),
                }
            }
        }

        let save = |kind: &str, text: &str| {
            let e = CacheEntry {
                kind: kind.into(),
                text: text.into(),
                saved: now_secs(),
            };
            serde_json::to_string(&e).ok()
        };
        match result {
            Some((kind, text)) => {
                if use_cache {
                    if let Some(s) = save(kind, &text) {
                        let _ = tokio::fs::write(&path, s).await;
                    }
                }
                to_lyrics(kind, &text)
            }
            None if failure.is_some() => Lyrics::Error(format!(
                "lyrics unavailable ({})",
                failure.unwrap_or("network error")
            )),
            None => {
                if use_cache {
                    if let Some(s) = save("none", "") {
                        let _ = tokio::fs::write(&path, s).await;
                    }
                }
                Lyrics::NotFound
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rec(synced: &str, plain: &str, duration: f64) -> Rec {
            Rec {
                synced: (!synced.is_empty()).then(|| synced.to_string()),
                plain: (!plain.is_empty()).then(|| plain.to_string()),
                instrumental: None,
                duration: Some(duration),
            }
        }

        #[test]
        fn hangul_share() {
            assert_eq!(hangul_ratio(""), 0.0);
            assert_eq!(hangul_ratio("hello world"), 0.0);
            assert_eq!(hangul_ratio("안녕하세요"), 1.0);
            let r = hangul_ratio("안녕 hi"); // 2 of 4 letters
            assert!((r - 0.5).abs() < 1e-6, "{r}");
        }

        #[test]
        fn pick_prefers_hangul_only_in_korean_context() {
            let roman = "[00:01.00]saranghae neomu\n[00:05.00]gaji ma";
            let hangul = "[00:01.00]사랑해 너무\n[00:05.00]가지 마";
            let recs = || vec![rec(roman, "", 200.0), rec(hangul, "", 200.0)];
            // Korean title/artist: the Hangul synced lyrics win over the romanized ones
            assert_eq!(pick(recs(), 200.0, true).unwrap().1, hangul);
            // a Hangul candidate also makes the context Korean by itself
            assert_eq!(pick(recs(), 200.0, false).unwrap().1, hangul);
            // no Hangul candidate and no hint: plain first-come ranking
            let eng = vec![rec("[00:01.00]one", "", 200.0), rec("[00:01.00]two", "", 200.0)];
            assert_eq!(pick(eng, 200.0, false).unwrap().1, "[00:01.00]one");
            // synced still beats plain Hangul
            let mixed = vec![rec("", "사랑해", 200.0), rec(roman, "", 200.0)];
            assert_eq!(pick(mixed, 200.0, true).unwrap().0, "synced");
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Cover art: fetch (file:// or http), cache in /tmp, downscale + duotone tint
// ════════════════════════════════════════════════════════════════════════════
mod translate {
    //! Lyric translation: Google Translate's unauthenticated web endpoints first
    //! (unofficial; they can be rate-limited or change without notice), then a
    //! LibreTranslate server when Google answers 429 or can't be reached.
    use futures::StreamExt;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicBool, Ordering};

    const GTX: &str = "https://translate.googleapis.com/translate_a/single";
    const DICT: &str = "https://clients5.google.com/translate_a/t";
    pub const LIBRE_URL: &str = "https://libretranslate.com/translate";
    const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

    #[derive(Clone, Debug, Default)]
    pub struct Cfg {
        /// Test hook: replaces both Google endpoints.
        pub google_base: Option<String>,
        pub fallback: bool,
        pub fallback_url: String,
        pub fallback_key: Option<String>,
    }

    /// Why one request failed: `Limited` / `Net` switch to the fallback, `Other` is retried.
    enum Fail {
        Limited,
        Net,
        Other,
    }

    /// Pull the translated text out of `[[["hello","hallo",…],…],…]` (the `gtx` shape).
    pub fn parse(v: &Value) -> Option<String> {
        let segs = v.get(0)?.as_array()?;
        let out: String = segs
            .iter()
            .filter_map(|s| s.get(0).and_then(Value::as_str))
            .collect();
        (!out.is_empty()).then_some(out)
    }

    /// The `dict-chrome-ex` shapes: `{"sentences":[{"trans":…}]}`, `["…"]` or `[["…","ko"]]`.
    pub fn parse_dict(v: &Value) -> Option<String> {
        let out: String = if let Some(s) = v.get("sentences").and_then(Value::as_array) {
            s.iter().filter_map(|x| x.get("trans").and_then(Value::as_str)).collect()
        } else {
            let first = v.get(0)?;
            first.as_str().or_else(|| first.get(0).and_then(Value::as_str))?.to_string()
        };
        (!out.is_empty()).then_some(out)
    }

    fn classify(e: &reqwest::Error) -> Fail {
        if e.is_timeout() || e.is_connect() || e.is_request() { Fail::Net } else { Fail::Other }
    }

    async fn get(c: &reqwest::Client, url: &str, client: &str, lang: &str, text: &str) -> Result<Value, Fail> {
        let mut q = vec![("client", client), ("sl", "auto"), ("tl", lang), ("q", text)];
        if client == "gtx" {
            q.push(("dt", "t"));
        }
        let r = c
            .get(url)
            .query(&q)
            .header(reqwest::header::USER_AGENT, UA)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| classify(&e))?;
        match r.status().as_u16() {
            200..=299 => r.json::<Value>().await.map_err(|_| Fail::Other),
            429 => Err(Fail::Limited),
            500..=599 => Err(Fail::Net),
            _ => Err(Fail::Other),
        }
    }

    /// `gtx`, then `dict-chrome-ex` when the first is limited or unreachable.
    async fn google(c: &reqwest::Client, cfg: &Cfg, lang: &str, text: &str) -> Result<String, Fail> {
        let (u1, u2) = match &cfg.google_base {
            Some(b) => (b.clone(), format!("{b}/dict")),
            None => (GTX.to_string(), DICT.to_string()),
        };
        match get(c, &u1, "gtx", lang, text).await.map(|v| parse(&v)) {
            Ok(Some(t)) => return Ok(t),
            Ok(None) | Err(Fail::Other) => {}
            Err(_) => {}
        }
        match get(c, &u2, "dict-chrome-ex", lang, text).await {
            Ok(v) => parse_dict(&v).ok_or(Fail::Other),
            Err(e) => Err(e),
        }
    }

    async fn libre(c: &reqwest::Client, cfg: &Cfg, lang: &str, text: &str) -> Option<String> {
        // LibreTranslate wants "zh", not "zh-CN"
        let target = lang.split('-').next().unwrap_or(lang);
        let mut body = json!({ "q": text, "source": "auto", "target": target, "format": "text" });
        if let Some(k) = cfg.fallback_key.as_deref().filter(|k| !k.is_empty()) {
            body["api_key"] = json!(k);
        }
        let r = c
            .post(&cfg.fallback_url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .ok()?;
        if !r.status().is_success() {
            return None;
        }
        let v = r.json::<Value>().await.ok()?;
        v.get("translatedText")?.as_str().filter(|t| !t.is_empty()).map(str::to_string)
    }

    async fn line(c: &reqwest::Client, cfg: &Cfg, use_fallback: &AtomicBool, lang: &str, l: String) -> Option<String> {
        let t = l.trim();
        if t.is_empty() || t == "♪" {
            return Some(l);
        }
        if !use_fallback.load(Ordering::Relaxed) {
            // one retry for a plain failure: a single dropped request shouldn't leave a line
            for _ in 0..2 {
                match google(c, cfg, lang, t).await {
                    Ok(x) => return Some(x),
                    Err(Fail::Other) => continue,
                    Err(_) => {
                        // limited / unreachable: stay on the fallback for the rest of the song
                        if cfg.fallback {
                            use_fallback.store(true, Ordering::Relaxed);
                        }
                        break;
                    }
                }
            }
        }
        if cfg.fallback { libre(c, cfg, lang, t).await } else { None }
    }

    /// Translate every line (order preserved; a line that fails keeps its original text).
    /// Returns the lines and which backend did the work. `Err` only when nearly everything failed.
    pub async fn lines(c: &reqwest::Client, cfg: &Cfg, lang: &str, src: &[String]) -> Result<(Vec<String>, &'static str), String> {
        let use_fallback = AtomicBool::new(false);
        let out: Vec<Option<String>> =
            futures::stream::iter(src.iter().cloned().map(|l| line(c, cfg, &use_fallback, lang, l)))
                .buffered(6)
                .collect()
                .await;
        let failed = out.iter().filter(|o| o.is_none()).count();
        let real = src.iter().filter(|l| !l.trim().is_empty() && l.trim() != "♪").count();
        if failed > 0 && failed * 5 > real {
            return Err(if cfg.fallback {
                "translation failed (Google and LibreTranslate unavailable)".into()
            } else {
                "translation failed (Google Translate unreachable or rate-limited)".into()
            });
        }
        let backend = if use_fallback.load(Ordering::Relaxed) { "libretranslate" } else { "google" };
        Ok((out.into_iter().zip(src).map(|(o, s)| o.unwrap_or_else(|| s.clone())).collect(), backend))
    }
}

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
// Guitar chords: sheet model, parsers, transpose, matching onto synced lyrics
// ════════════════════════════════════════════════════════════════════════════
mod chords {
    use std::path::{Path, PathBuf};

    /// One line of a chord sheet. `chords` are `(char column in text, chord name)`.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct SheetLine {
        pub text: String,
        pub chords: Vec<(usize, String)>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct Sheet {
        /// Where it came from (file name), for the footer.
        pub source: String,
        pub capo: u8,
        /// Key of the chords *as written* (the shapes), e.g. "Am". From `{key: Am}`, a
        /// `Key: Am` line, or Ultimate Guitar's `tonality_name`.
        pub key: Option<String>,
        /// From `{tempo: 83}` / `BPM: 83` / the page metadata when the source has it.
        pub bpm: Option<u32>,
        /// The song's original guitar tuning ("Standard", "Drop D", "Half Step Down"…),
        /// from `{tuning: …}` or Ultimate Guitar's page metadata.
        pub tuning: Option<String>,
        pub lines: Vec<SheetLine>,
    }

    impl Sheet {
        pub fn chord_count(&self) -> usize {
            self.lines.iter().map(|l| l.chords.len()).sum()
        }
        /// Flat keys (Bb, Eb…) in the sheet → show transposed chords with flats too.
        pub fn prefers_flats(&self) -> bool {
            self.lines
                .iter()
                .flat_map(|l| l.chords.iter())
                .any(|(_, c)| {
                    let b = c.as_bytes();
                    b.len() > 1 && b[1] == b'b' && b[0].is_ascii_uppercase()
                })
        }
    }

    // ── chord recognition ──────────────────────────────────────────────────
    fn root_len(s: &str) -> Option<usize> {
        let b = s.as_bytes();
        if b.is_empty() || !(b'A'..=b'G').contains(&b[0]) {
            return None;
        }
        Some(if b.len() > 1 && (b[1] == b'#' || b[1] == b'b') {
            2
        } else {
            1
        })
    }

    /// `Am7`, `F#m`, `Bb/D`, `Gsus4`, `Cadd9`, `E7b9`, `D(add9)`, `N.C.` is *not* a chord.
    pub fn is_chord(tok: &str) -> bool {
        let (main, bass) = match tok.split_once('/') {
            Some((m, b)) => (m, Some(b)),
            None => (tok, None),
        };
        if let Some(b) = bass {
            if root_len(b) != Some(b.len()) {
                return false;
            }
        }
        let Some(n) = root_len(main) else { return false };
        let mut rest = &main[n..];
        const WORDS: [&str; 8] = ["maj", "min", "dim", "aug", "sus", "add", "no", "M"];
        while !rest.is_empty() {
            if let Some(w) = WORDS.iter().find(|w| rest.starts_with(**w)) {
                rest = &rest[w.len()..];
                continue;
            }
            let c = rest.chars().next().unwrap();
            if c.is_ascii_digit() || matches!(c, 'm' | '+' | '-' | '°' | 'ø' | '#' | 'b' | '(' | ')' | ',')
            {
                rest = &rest[c.len_utf8()..];
            } else {
                return false;
            }
        }
        true
    }

    /// Shift one chord by `n` semitones (root and bass note).
    pub fn transpose(chord: &str, n: i32, flats: bool) -> String {
        if n.rem_euclid(12) == 0 {
            return chord.to_string();
        }
        let shift = |note: &str| -> String {
            const SHARP: [&str; 12] = [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ];
            const FLAT: [&str; 12] = [
                "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
            ];
            let base = match note.as_bytes()[0] {
                b'C' => 0,
                b'D' => 2,
                b'E' => 4,
                b'F' => 5,
                b'G' => 7,
                b'A' => 9,
                _ => 11,
            };
            let acc = match note.as_bytes().get(1) {
                Some(b'#') => 1,
                Some(b'b') => -1,
                _ => 0,
            };
            let i = (base + acc + n).rem_euclid(12) as usize;
            (if flats { FLAT[i] } else { SHARP[i] }).to_string()
        };
        let (main, bass) = match chord.split_once('/') {
            Some((m, b)) => (m, Some(b)),
            None => (chord, None),
        };
        let Some(r) = root_len(main) else {
            return chord.to_string();
        };
        let mut out = shift(&main[..r]);
        out.push_str(&main[r..]);
        if let Some(b) = bass {
            out.push('/');
            out.push_str(&if root_len(b).is_some() { shift(b) } else { b.to_string() });
        }
        out
    }

    /// Semitones to shift the sheet's chord shapes: a higher capo means lower shapes.
    /// `transpose` is the manual `t`/`T` shift on top.
    pub fn capo_shift(sheet_capo: u8, effective: u8, transpose: i32) -> i32 {
        sheet_capo as i32 - effective as i32 + transpose
    }

    // ── keys, Camelot wheel ────────────────────────────────────────────────
    /// A musical key: pitch class of the tonic (0 = C) and mode.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct KeyInfo {
        pub root: u8,
        pub minor: bool,
    }

    const MAJOR_NAMES: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
    const MINOR_NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B"];

    fn pitch_class(note: &str) -> Option<u8> {
        let b = note.as_bytes();
        let base: i32 = match b.first()? {
            b'C' => 0,
            b'D' => 2,
            b'E' => 4,
            b'F' => 5,
            b'G' => 7,
            b'A' => 9,
            b'B' => 11,
            _ => return None,
        };
        let acc = match b.get(1) {
            Some(b'#') => 1,
            Some(b'b') => -1,
            _ => 0,
        };
        Some((base + acc).rem_euclid(12) as u8)
    }

    /// "Am", "F#", "Bbm", "E minor", "D Major" → key.
    pub fn parse_key(s: &str) -> Option<KeyInfo> {
        let s = s.trim();
        let n = root_len(s)?;
        let root = pitch_class(&s[..n])?;
        let rest = s[n..].trim().to_lowercase();
        let minor = match rest.as_str() {
            "" | "maj" | "major" => false,
            "m" | "min" | "minor" => true,
            r if r.starts_with('m') && !r.starts_with("maj") => true, // Am7 etc.
            _ => return None,
        };
        Some(KeyInfo { root, minor })
    }

    /// Best guess at the key of the shapes from the chords alone (diatonic fit, then the
    /// tonic that is played first / last). Used when the source states no key.
    pub fn estimate_key(sheet: &Sheet) -> Option<String> {
        // (pitch class, minor) of each chord, in song order
        let seq: Vec<(u8, bool)> = sheet
            .lines
            .iter()
            .flat_map(|l| l.chords.iter())
            .filter_map(|(_, name)| {
                let main = name.split('/').next()?;
                let n = root_len(main)?;
                let root = pitch_class(&main[..n])?;
                let rest = &main[n..];
                Some((root, rest.starts_with('m') && !rest.starts_with("maj")))
            })
            .collect();
        if seq.len() < 4 {
            return None;
        }
        let count = |root: u8, minor: bool| seq.iter().filter(|c| **c == (root, minor)).count() as i32;
        let (first, last) = (seq[0], seq[seq.len() - 1]);
        // major-key fit: I ii iii IV V vi
        let fit = |r: u8| -> i32 {
            [(0, false), (2, true), (4, true), (5, false), (7, false), (9, true)]
                .iter()
                .map(|(d, m)| count((r + d) % 12, *m))
                .sum::<i32>()
        };
        let best = (0..12u8).max_by_key(|r| {
            let bonus = [first, last].iter().filter(|c| **c == (*r, false) || **c == ((*r + 9) % 12, true)).count() as i32;
            fit(*r) * 4 + bonus
        })?;
        let rel = (best + 9) % 12;
        let major_score = count(best, false) * 2 + (last == (best, false)) as i32 * 3 + (first == (best, false)) as i32;
        let minor_score = count(rel, true) * 2 + (last == (rel, true)) as i32 * 3 + (first == (rel, true)) as i32;
        let k = if minor_score > major_score { KeyInfo { root: rel, minor: true } } else { KeyInfo { root: best, minor: false } };
        Some(k.short())
    }

    impl KeyInfo {
        pub fn shift(self, n: i32) -> KeyInfo {
            KeyInfo {
                root: (self.root as i32 + n).rem_euclid(12) as u8,
                ..self
            }
        }
        /// "Am", "Bb".
        pub fn short(self) -> String {
            let names = if self.minor { &MINOR_NAMES } else { &MAJOR_NAMES };
            format!("{}{}", names[self.root as usize], if self.minor { "m" } else { "" })
        }
        /// "E Minor".
        pub fn long(self) -> String {
            let names = if self.minor { &MINOR_NAMES } else { &MAJOR_NAMES };
            format!("{} {}", names[self.root as usize], if self.minor { "Minor" } else { "Major" })
        }
        /// Camelot wheel code: "9A" = E minor, "8B" = C major.
        pub fn camelot(self) -> String {
            // minor keys share the number of their relative major
            let major_pc = if self.minor { (self.root + 3) % 12 } else { self.root } as u32;
            let n = (major_pc * 7 + 7) % 12 + 1;
            format!("{n}{}", if self.minor { 'A' } else { 'B' })
        }
        /// Flat names (Bb, Eb …) read better in F, Bb, Eb, Ab, Db major and their relatives.
        pub fn prefers_flats(self) -> bool {
            let major_pc = if self.minor { (self.root + 3) % 12 } else { self.root };
            matches!(major_pc, 5 | 10 | 3 | 8 | 1)
        }
    }

    // ── parsing ────────────────────────────────────────────────────────────
    fn strip_tags(s: &str) -> String {
        s.replace("[ch]", "")
            .replace("[/ch]", "")
            .replace("[tab]", "")
            .replace("[/tab]", "")
            .replace('\r', "")
    }

    fn parse_capo(line: &str) -> Option<u8> {
        let l = line.trim().to_lowercase();
        let rest = l.strip_prefix("capo")?;
        let digits: String = rest
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok().filter(|n| *n <= 12)
    }

    /// `[Verse 1]`, `Chorus:`, `Intro (2x)` — but not a lyric like "Bridge over troubled water".
    fn is_section(line: &str) -> bool {
        let t = line
            .trim()
            .trim_matches(|c| c == '[' || c == ']' || c == '(' || c == ')')
            .trim_end_matches(':')
            .to_lowercase();
        if t.is_empty() || t.chars().count() > 32 {
            return false;
        }
        const KINDS: [&str; 14] = [
            "verse",
            "chorus",
            "bridge",
            "intro",
            "outro",
            "pre-chorus",
            "prechorus",
            "interlude",
            "solo",
            "instrumental",
            "refrain",
            "hook",
            "coda",
            "tag",
        ];
        let mut it = t.split_whitespace();
        let Some(first) = it.next() else { return false };
        if !KINDS.contains(&first.trim_end_matches(':')) {
            return false;
        }
        it.all(|w| {
            w.chars().any(|c| c.is_ascii_digit()) || w == "x" || w.starts_with('(') || w == "play"
        })
    }

    /// `key` / `tonality` / `bpm` / `tempo` metadata; `true` if it was one.
    fn apply_meta(sheet: &mut Sheet, k: &str, v: &str) -> bool {
        match k.to_lowercase().as_str() {
            "key" | "tonality" if parse_key(v).is_some() => {
                sheet.key = Some(v.to_string());
                true
            }
            "tuning" if !v.trim().is_empty() && v.len() < 40 => {
                sheet.tuning = Some(v.trim().to_string());
                true
            }
            "bpm" | "tempo" => match v.split_whitespace().next().and_then(|n| n.parse::<u32>().ok()) {
                Some(n) if (30..=300).contains(&n) => {
                    sheet.bpm = Some(n);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn is_tab_line(line: &str) -> bool {
        line.contains("--") && line.contains('|')
    }

    fn tokens_with_pos(line: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        let mut cur = String::new();
        for (i, c) in line.chars().chain(std::iter::once(' ')).enumerate() {
            if c.is_whitespace() {
                if let Some(s) = start.take() {
                    out.push((s, std::mem::take(&mut cur)));
                }
            } else {
                if start.is_none() {
                    start = Some(i);
                }
                cur.push(c);
            }
        }
        out
    }

    fn chord_tokens(line: &str) -> Option<Vec<(usize, String)>> {
        let toks = tokens_with_pos(line);
        // `|` bar lines and `x2` repeat marks may sit among the chords.
        let toks: Vec<_> = toks
            .into_iter()
            .filter(|(_, t)| !matches!(t.as_str(), "|" | "||" | "-" | "/" | "%") && !is_repeat(t))
            .collect();
        (!toks.is_empty() && toks.iter().all(|(_, t)| is_chord(t))).then_some(toks)
    }

    fn is_repeat(t: &str) -> bool {
        let t = t.to_lowercase();
        let t = t.trim_matches(|c| c == '(' || c == ')');
        (t.starts_with('x') && t[1..].chars().all(|c| c.is_ascii_digit()) && t.len() > 1)
            || (t.ends_with('x') && t[..t.len() - 1].chars().all(|c| c.is_ascii_digit()) && t.len() > 1)
    }

    /// Chords written above the words they belong to (the common plain-text format).
    pub fn parse_text(src: &str) -> Sheet {
        let src = strip_tags(src);
        let lines: Vec<&str> = src.lines().map(str::trim_end).collect();
        let mut sheet = Sheet::default();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            i += 1;
            if line.trim().is_empty() || is_section(line) || is_tab_line(line) {
                continue;
            }
            if let Some(c) = parse_capo(line) {
                sheet.capo = c;
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                if apply_meta(&mut sheet, k.trim(), v.trim()) {
                    continue;
                }
            }
            if let Some(chords) = chord_tokens(line) {
                let next = lines.get(i).copied().unwrap_or("");
                let usable = !next.trim().is_empty()
                    && !is_section(next)
                    && !is_tab_line(next)
                    && chord_tokens(next).is_none();
                if usable {
                    sheet.lines.push(SheetLine {
                        text: next.to_string(),
                        chords,
                    });
                    i += 1;
                } else {
                    sheet.lines.push(SheetLine {
                        text: String::new(),
                        chords,
                    });
                }
            } else {
                sheet.lines.push(SheetLine {
                    text: line.to_string(),
                    chords: Vec::new(),
                });
            }
        }
        sheet
    }

    /// ChordPro: `[Am]Hello [G]world`, `{capo: 2}`, `{comment: …}`.
    pub fn parse_chordpro(src: &str) -> Sheet {
        let mut sheet = Sheet::default();
        for raw in src.replace('\r', "").lines() {
            let line = raw.trim_end();
            let t = line.trim();
            if t.starts_with('#') || t.is_empty() {
                continue;
            }
            if let Some(d) = t.strip_prefix('{') {
                let d = d.trim_end_matches('}');
                if let Some((k, v)) = d.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("capo") {
                        sheet.capo = v.trim().parse().ok().filter(|n| *n <= 12).unwrap_or(0);
                    } else {
                        apply_meta(&mut sheet, k.trim(), v.trim());
                    }
                }
                continue;
            }
            if is_section(t) {
                continue;
            }
            let mut text = String::new();
            let mut col = 0usize;
            let mut chords = Vec::new();
            let mut it = line.chars().peekable();
            while let Some(c) = it.next() {
                if c == '[' {
                    let mut inner = String::new();
                    let mut closed = false;
                    for d in it.by_ref() {
                        if d == ']' {
                            closed = true;
                            break;
                        }
                        inner.push(d);
                    }
                    if closed && is_chord(&inner) {
                        chords.push((col, inner));
                    } else {
                        text.push('[');
                        text.push_str(&inner);
                        col += 1 + inner.chars().count();
                        if closed {
                            text.push(']');
                            col += 1;
                        }
                    }
                } else {
                    text.push(c);
                    col += 1;
                }
            }
            sheet.lines.push(SheetLine { text, chords });
        }
        sheet
    }

    /// Pick the format: inline `[Am]` chords → ChordPro, otherwise chords-over-lyrics.
    pub fn parse_auto(src: &str) -> Sheet {
        let src = strip_tags(src);
        let inline = src.lines().any(|l| {
            l.match_indices('[').any(|(i, _)| {
                l[i + 1..]
                    .split_once(']')
                    .is_some_and(|(inner, after)| is_chord(inner) && !after.trim().is_empty())
            })
        });
        if inline { parse_chordpro(&src) } else { parse_text(&src) }
    }

    // ── local files ────────────────────────────────────────────────────────
    pub const EXTS: [&str; 6] = ["cho", "chopro", "chordpro", "crd", "pro", "txt"];

    pub fn default_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("music_display")
            .join("chords")
    }

    fn key(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }

    /// Find `Artist - Title.<ext>` (or just `Title.<ext>`) in `dir`, ignoring case,
    /// punctuation, "(Remastered)" suffixes and extra artists.
    pub fn find_local(dir: &Path, artist: &str, title: &str) -> Option<PathBuf> {
        let cleaned = crate::year::clean(title);
        let first = artist.split(", ").next().unwrap_or(artist);
        let mut wanted = vec![];
        for t in [title, cleaned.as_str()] {
            for a in [artist, first] {
                wanted.push(key(&format!("{a}{t}")));
                wanted.push(key(&format!("{t}{a}")));
            }
            wanted.push(key(t));
        }
        let mut best: Option<(usize, PathBuf)> = None;
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            let Some(rank) = EXTS.iter().position(|x| x.eq_ignore_ascii_case(ext)) else {
                continue;
            };
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if wanted.contains(&key(stem)) && best.as_ref().is_none_or(|(r, _)| rank < *r) {
                best = Some((rank, p));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Read + parse a local sheet. `None` when missing or it has too few chords to be real.
    pub async fn load_local(dir: PathBuf, artist: String, title: String) -> Option<Sheet> {
        let path = find_local(&dir, &artist, &title)?;
        let bytes = tokio::fs::read(&path).await.ok()?;
        let src = String::from_utf8_lossy(&bytes);
        let ext = path.extension()?.to_str()?.to_lowercase();
        let mut sheet = if ext == "txt" {
            parse_auto(&src)
        } else {
            parse_chordpro(&src)
        };
        // A .cho file may still contain chords-over-lyrics; fall back if it parsed to nothing.
        if sheet.chord_count() == 0 {
            sheet = parse_auto(&src);
        }
        if sheet.chord_count() < 4 {
            return None;
        }
        sheet.source = path.file_name()?.to_string_lossy().into_owned();
        Some(sheet)
    }

    /// ChordPro text for the cache (round-trips through [`parse_chordpro`]).
    pub fn to_chordpro(s: &Sheet) -> String {
        let mut out = String::new();
        if s.capo > 0 {
            out.push_str(&format!("{{capo: {}}}\n", s.capo));
        }
        if let Some(k) = &s.key {
            out.push_str(&format!("{{key: {k}}}\n"));
        }
        if let Some(b) = s.bpm {
            out.push_str(&format!("{{tempo: {b}}}\n"));
        }
        if let Some(t) = &s.tuning {
            out.push_str(&format!("{{tuning: {t}}}\n"));
        }
        for l in &s.lines {
            let mut cs = l.chords.clone();
            cs.sort_by_key(|c| c.0);
            let chars: Vec<char> = l.text.chars().collect();
            let mut pos = 0usize;
            for (col, name) in cs {
                while pos < col {
                    out.push(chars.get(pos).copied().unwrap_or(' '));
                    pos += 1;
                }
                out.push_str(&format!("[{name}]"));
            }
            out.extend(chars.iter().skip(pos));
            out.push('\n');
        }
        out
    }

    // ── matching a sheet onto synced lyrics ────────────────────────────────
    /// Lowercase, letters/digits only, single spaces.
    pub fn norm(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_alphanumeric() {
                out.extend(c.to_lowercase());
            } else if c.is_whitespace() || matches!(c, '-' | '/' | ',' | '.' | '!' | '?' | ';' | ':') {
                out.push(' ');
            } // apostrophes etc. vanish: "don't" → "dont"
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub fn dice(a: &str, b: &str) -> f32 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        let grams = |s: &str| -> std::collections::HashMap<(char, char), u32> {
            let v: Vec<char> = s.chars().collect();
            let mut m = std::collections::HashMap::new();
            for w in v.windows(2) {
                *m.entry((w[0], w[1])).or_insert(0) += 1;
            }
            m
        };
        let (ga, gb) = (grams(a), grams(b));
        let (na, nb): (u32, u32) = (ga.values().sum(), gb.values().sum());
        if na == 0 || nb == 0 {
            return if a == b { 1.0 } else { 0.0 };
        }
        let common: u32 = ga
            .iter()
            .map(|(k, v)| (*v).min(gb.get(k).copied().unwrap_or(0)))
            .sum();
        2.0 * common as f32 / (na + nb) as f32
    }

    /// `(start char, length)` of each whitespace-separated word.
    pub fn words_pos(s: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = None;
        let mut n = 0;
        for (i, c) in s.chars().chain(std::iter::once(' ')).enumerate() {
            if c.is_whitespace() {
                if let Some(st) = start.take() {
                    out.push((st, n));
                    n = 0;
                }
            } else {
                if start.is_none() {
                    start = Some(i);
                }
                n += 1;
            }
        }
        out
    }

    /// Move a chord column from the sheet's wording onto the lyric's wording, anchored to
    /// the word it sits on (so small wording differences don't misplace it).
    fn map_col(col: usize, sheet_text: &str, lyric: &str) -> usize {
        let sw = words_pos(sheet_text);
        let lw = words_pos(lyric);
        let lyric_len = lyric.chars().count();
        if lw.is_empty() {
            return 0;
        }
        let Some(w) = sw.iter().position(|(s, l)| col < s + l) else {
            return lyric_len; // past the last word
        };
        let off = col.saturating_sub(sw[w].0);
        let w2 = if sw.len() == lw.len() {
            w
        } else {
            (w * lw.len() / sw.len()).min(lw.len() - 1)
        };
        lw[w2].0 + off.min(lw[w2].1)
    }

    #[derive(Clone, Debug, Default)]
    pub struct Aligned {
        /// Per lyric line: chords with columns relative to the single-spaced lyric text.
        pub per_line: Vec<Vec<(usize, String)>>,
        pub matched: usize,
        pub total: usize,
        pub flats: bool,
        /// Chord-only sheet lines before the first matched lyric line (the intro), one
        /// `Vec` of chord names per sheet line.
        pub intro: Vec<Vec<String>>,
        /// Per lyric line: chord-only sheet lines that follow it (solo / instrumental break),
        /// one `Vec` of chord names per sheet line. The last entry is the outro.
        pub gaps: Vec<Vec<Vec<String>>>,
    }

    impl Aligned {
        pub fn percent(&self) -> u32 {
            (self.matched * 100).checked_div(self.total).unwrap_or(0) as u32
        }
        /// Below this the chords would more often be wrong than right, so none are shown.
        pub fn usable(&self) -> bool {
            self.total > 0 && self.matched * 100 >= self.total * 40
        }
    }

    pub fn align(sheet: &Sheet, lyrics: &[(f64, String)]) -> Aligned {
        let sung: Vec<(usize, String)> = sheet
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let n = norm(&l.text);
                (!n.is_empty()).then_some((i, n))
            })
            .collect();
        let mut out = Aligned {
            per_line: vec![Vec::new(); lyrics.len()],
            gaps: vec![Vec::new(); lyrics.len()],
            flats: sheet.prefers_flats(),
            ..Default::default()
        };
        let mut hits: Vec<(usize, usize)> = Vec::new(); // (lyric line, sheet line)
        let mut j = 0usize;
        let mut first_hit: Option<usize> = None;
        for (li, (_, text)) in lyrics.iter().enumerate() {
            let n = norm(text);
            if n.is_empty() {
                continue;
            }
            out.total += 1;

            // 1. next few sheet lines in order
            let mut best: Option<(usize, f32)> = None;
            for (k, (_, s)) in sung.iter().enumerate().skip(j).take(10) {
                let d = dice(&n, s);
                if d >= 0.55 && best.is_none_or(|(_, b)| d > b) {
                    best = Some((k, d));
                }
            }
            let mut advance = true;
            // 2. anywhere (a chorus the sheet writes once but the song repeats)
            if best.is_none() {
                for (k, (_, s)) in sung.iter().enumerate() {
                    let d = dice(&n, s);
                    if d >= 0.75 && best.is_none_or(|(_, b)| d > b) {
                        best = Some((k, d));
                    }
                }
                advance = false;
            }
            let Some((k, _)) = best else { continue };
            if first_hit.is_none() {
                first_hit = Some(sung[k].0);
            }
            if advance {
                j = k + 1;
            }
            out.matched += 1;
            hits.push((li, sung[k].0));
            let line = &sheet.lines[sung[k].0];
            let lyric_norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut cs: Vec<(usize, String)> = line
                .chords
                .iter()
                .map(|(c, name)| (map_col(*c, &line.text, &lyric_norm), name.clone()))
                .collect();
            cs.sort_by_key(|c| c.0);
            out.per_line[li] = cs;
        }
        out.intro = sheet
            .lines
            .iter()
            .take(first_hit.unwrap_or(0))
            .filter(|l| norm(&l.text).is_empty() && !l.chords.is_empty())
            .map(|l| l.chords.iter().map(|c| c.1.clone()).collect())
            .collect();
        // chord-only sheet lines between one matched line and the next: the play-along
        // chords for the break after that lyric line
        for (n, (li, si)) in hits.iter().enumerate() {
            let end = hits.iter().skip(n + 1).map(|h| h.1).find(|s| s > si).unwrap_or(sheet.lines.len());
            out.gaps[*li] = sheet.lines[si + 1..end.max(si + 1)]
                .iter()
                .filter(|l| norm(&l.text).is_empty() && !l.chords.is_empty())
                .map(|l| l.chords.iter().map(|c| c.1.clone()).collect())
                .collect();
        }
        if !out.usable() {
            for l in &mut out.per_line {
                l.clear();
            }
            for g in &mut out.gaps {
                g.clear();
            }
            out.intro.clear();
        }
        out
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Ultimate Guitar lookup (opt-in: --chords-ug). Unofficial: the site has no public API, so
// this reads the JSON blob embedded in its pages and is written to fail soft.
// ════════════════════════════════════════════════════════════════════════════
mod ug {
    use crate::chords::{self, Sheet};
    use crate::util::{cache_dir, fnv1a, now_secs};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU32, Ordering};

    pub const BASE: &str = "https://www.ultimate-guitar.com";
    // UG answers non-browser agents with 403, so this looks like a normal browser.
    const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
    const NEG_TTL: u64 = 7 * 24 * 3600;
    const MAX_FAILS: u32 = 3;
    const MIN_CHORDS: usize = 8;

    /// Consecutive hard failures (blocked / format changed / network). Three in a row
    /// switch the provider off for the rest of the session.
    static FAILS: AtomicU32 = AtomicU32::new(0);
    /// `--chords-probe --probe-save DIR`: keep every page we fetch.
    pub static SAVE_DIR: OnceLock<PathBuf> = OnceLock::new();

    pub enum Outcome {
        Found(Sheet),
        NotFound,
        Error(String),
    }

    // ── page parsing ───────────────────────────────────────────────────────
    pub fn unescape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(i) = rest.find('&') {
            out.push_str(&rest[..i]);
            rest = &rest[i..];
            let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
                out.push('&');
                rest = &rest[1..];
                continue;
            };
            let ent = &rest[1..end];
            let ch = match ent {
                "quot" => Some('"'),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            match ch {
                Some(c) => {
                    out.push(c);
                    rest = &rest[end + 1..];
                }
                None => {
                    out.push('&');
                    rest = &rest[1..];
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// The `data-content` JSON of the `js-store` element.
    pub fn extract_store(html: &str) -> Option<Value> {
        let at = html.find("js-store")?;
        let tag_start = html[..at].rfind('<')?;
        let tag_end = at + html[at..].find('>')?;
        let tag = &html[tag_start..tag_end];
        let k = tag.find("data-content=\"")? + "data-content=\"".len();
        let raw = &tag[k..k + tag[k..].find('"')?];
        serde_json::from_str(&unescape(raw)).ok()
    }

    /// First value stored under `key` anywhere in the document — survives UG moving things.
    pub fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(m) => m
                .get(key)
                .filter(|x| !x.is_null())
                .or_else(|| m.values().find_map(|x| find_key(x, key))),
            Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
            _ => None,
        }
    }

    /// The object that holds `key` — the chord page itself, so its capo / tuning / key aren't
    /// confused with those of related tabs elsewhere in the page data.
    fn parent_of<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(m) if m.get(key).is_some_and(|x| !x.is_null()) => Some(v),
            Value::Object(m) => m.values().find_map(|x| parent_of(x, key)),
            Value::Array(a) => a.iter().find_map(|x| parent_of(x, key)),
            _ => None,
        }
    }

    #[derive(Debug, Clone)]
    pub struct Cand {
        pub url: String,
        pub artist: String,
        pub song: String,
        pub rating: f64,
        pub votes: f64,
        pub score: f64,
    }

    /// Chord pages for this song, best first: rating × ln(votes), after checking that the
    /// title and artist really are the ones we asked for.
    pub fn candidates(store: &Value, artist: &str, title: &str) -> Vec<Cand> {
        let Some(Value::Array(results)) = find_key(store, "results") else {
            return vec![];
        };
        let want_t = chords::norm(&crate::year::clean(title));
        let want_a = chords::norm(artist.split(", ").next().unwrap_or(artist));
        let mut out: Vec<Cand> = results
            .iter()
            .filter_map(|r| {
                let url = r.get("tab_url")?.as_str()?.to_string();
                if r.get("type").and_then(Value::as_str).is_some_and(|t| t != "Chords") {
                    return None;
                }
                let song = r.get("song_name").and_then(Value::as_str).unwrap_or("").to_string();
                let art = r.get("artist_name").and_then(Value::as_str).unwrap_or("").to_string();
                let (st, sa) = (chords::norm(&crate::year::clean(&song)), chords::norm(&art));
                let title_ok = chords::dice(&want_t, &st) >= 0.8;
                let artist_ok = sa.contains(&want_a) || want_a.contains(&sa) || chords::dice(&want_a, &sa) >= 0.7;
                if !title_ok || !artist_ok {
                    return None;
                }
                let rating = r.get("rating").and_then(Value::as_f64).unwrap_or(0.0);
                let votes = r.get("votes").and_then(Value::as_f64).unwrap_or(0.0);
                Some(Cand {
                    url,
                    artist: art,
                    song,
                    rating,
                    votes,
                    score: rating * (1.0 + votes).ln(),
                })
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(3);
        out
    }

    /// A chord page → sheet (`None` if it has too few chords to be a real chord sheet).
    pub fn sheet_from_store(store: &Value) -> Option<Sheet> {
        let content = find_key(store, "wiki_tab")
            .and_then(|w| w.get("content"))
            .and_then(Value::as_str)?;
        let mut sheet = chords::parse_text(content);
        // metadata sits next to `wiki_tab` (in `tab_view`); search there before the whole page
        let tab = parent_of(store, "wiki_tab").unwrap_or(store);
        let find_key = |_: &Value, k: &str| find_key(tab, k).or_else(|| find_key(store, k));
        if sheet.capo == 0 {
            sheet.capo = find_key(store, "capo")
                .and_then(Value::as_u64)
                .filter(|c| (1..=12).contains(c))
                .unwrap_or(0) as u8;
        }
        if sheet.key.is_none() {
            sheet.key = find_key(store, "tonality_name")
                .and_then(Value::as_str)
                .filter(|k| chords::parse_key(k).is_some())
                .map(str::to_string);
        }
        if sheet.bpm.is_none() {
            sheet.bpm = find_key(store, "bpm")
                .and_then(|b| b.as_u64().or_else(|| b.as_str().and_then(|s| s.parse().ok())))
                .filter(|b| (30..=300).contains(b))
                .map(|b| b as u32);
        }
        if sheet.tuning.is_none() {
            // UG: "tuning": {"name": "Half Step Down", "value": "Eb Ab Db Gb Bb Eb"}
            let text = |v: Option<&Value>| {
                v.and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|n| !n.is_empty() && n.len() < 40 && !n.contains(['}', '\n']))
                    .map(str::to_string)
            };
            sheet.tuning = find_key(store, "tuning").and_then(|t| match t {
                Value::Object(m) => text(m.get("name")).or_else(|| text(m.get("value"))),
                v => text(Some(v)),
            });
        }
        (sheet.chord_count() >= MIN_CHORDS).then(|| {
            sheet.source = "ultimate-guitar".into();
            sheet
        })
    }

    // ── network ────────────────────────────────────────────────────────────
    async fn get(c: &reqwest::Client, url: &str, query: &[(&str, &str)]) -> Result<String, String> {
        let r = c
            .get(url)
            .query(query)
            .header(reqwest::header::USER_AGENT, UA)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("network error ({})", e.without_url()))?;
        let st = r.status();
        if !st.is_success() {
            return Err(format!("HTTP {} from ultimate-guitar", st.as_u16()));
        }
        let body = r.text().await.map_err(|e| format!("read error ({e})"))?;
        if let Some(dir) = SAVE_DIR.get() {
            let name = format!("{:016x}.html", fnv1a(url) ^ fnv1a(&format!("{query:?}")));
            let _ = std::fs::write(dir.join(name), &body);
        }
        Ok(body)
    }

    #[derive(Serialize, Deserialize)]
    struct CacheEntry {
        kind: String, // found | none
        chordpro: String,
        saved: u64,
    }

    fn cache_path(artist: &str, title: &str) -> PathBuf {
        cache_dir().join(format!("chords-ug-{:016x}.json", fnv1a(&format!("{artist}\0{title}"))))
    }

    fn fail(msg: String) -> Outcome {
        FAILS.fetch_add(1, Ordering::Relaxed);
        Outcome::Error(msg)
    }

    pub fn disabled() -> bool {
        FAILS.load(Ordering::Relaxed) >= MAX_FAILS
    }

    #[cfg(test)]
    pub fn reset_breaker() {
        FAILS.store(0, Ordering::Relaxed);
    }

    /// Cache → search → fetch the best page(s). At most 1 search + 3 page requests, and
    /// only for tracks that aren't cached. Errors are never cached.
    pub async fn lookup(c: &reqwest::Client, base: &str, artist: &str, title: &str) -> Outcome {
        let path = cache_path(artist, title);
        if let Ok(txt) = tokio::fs::read_to_string(&path).await {
            if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
                if e.kind == "found" {
                    let mut s = chords::parse_chordpro(&e.chordpro);
                    if s.chord_count() >= MIN_CHORDS {
                        s.source = "ultimate-guitar (cached)".into();
                        return Outcome::Found(s);
                    }
                } else if now_secs().saturating_sub(e.saved) < NEG_TTL {
                    return Outcome::NotFound;
                }
            }
        }
        if disabled() {
            return Outcome::Error("ultimate-guitar switched off after repeated failures".into());
        }

        let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::clean(title));
        let html = match get(c, &format!("{base}/search.php"), &[("search_type", "title"), ("value", &q)]).await {
            Ok(h) => h,
            Err(e) => return fail(e),
        };
        let Some(store) = extract_store(&html) else {
            return fail("ultimate-guitar page format not recognised (blocked or changed)".into());
        };
        let cands = candidates(&store, artist, title);
        let mut sheet = None;
        for cand in &cands {
            let url = if cand.url.starts_with('/') {
                format!("{base}{}", cand.url)
            } else {
                cand.url.clone()
            };
            let html = match get(c, &url, &[]).await {
                Ok(h) => h,
                Err(e) => return fail(e),
            };
            let Some(page) = extract_store(&html) else {
                return fail("ultimate-guitar tab page format not recognised".into());
            };
            if let Some(s) = sheet_from_store(&page) {
                sheet = Some(s);
                break;
            }
        }
        FAILS.store(0, Ordering::Relaxed);
        let entry = match &sheet {
            Some(s) => CacheEntry { kind: "found".into(), chordpro: chords::to_chordpro(s), saved: now_secs() },
            None => CacheEntry { kind: "none".into(), chordpro: String::new(), saved: now_secs() },
        };
        if let Ok(j) = serde_json::to_string(&entry) {
            let _ = tokio::fs::write(&path, j).await;
        }
        match sheet {
            Some(s) => Outcome::Found(s),
            None => Outcome::NotFound,
        }
    }

    /// Local folder first, then (if enabled) Ultimate Guitar.
    pub async fn resolve(
        c: reqwest::Client,
        dir: PathBuf,
        use_ug: bool,
        artist: String,
        title: String,
    ) -> Result<Option<Sheet>, String> {
        let base = crate::year::base_title(&title);
        let retry = base != title;
        if let Some(s) = chords::load_local(dir.clone(), artist.clone(), title.clone()).await {
            return Ok(Some(s));
        }
        if retry {
            if let Some(s) = chords::load_local(dir, artist.clone(), base.clone()).await {
                return Ok(Some(s));
            }
        }
        if !use_ug {
            return Ok(None);
        }
        match lookup(&c, BASE, &artist, &title).await {
            Outcome::Found(s) => Ok(Some(s)),
            // nothing under the full title: try the bare song name
            Outcome::NotFound if retry => match lookup(&c, BASE, &artist, &base).await {
                Outcome::Found(s) => Ok(Some(s)),
                Outcome::NotFound => Ok(None),
                Outcome::Error(e) => Err(e),
            },
            Outcome::NotFound => Ok(None),
            Outcome::Error(e) => Err(e),
        }
    }

    /// `music_display --chords-probe "Artist" "Title"`: show each step so a broken lookup
    /// can be diagnosed (add `--probe-save DIR` to keep the raw pages as test fixtures).
    pub async fn probe(c: &reqwest::Client, artist: &str, title: &str, save: Option<PathBuf>) {
        println!("probe: {artist} — {title}");
        if let Some(d) = save {
            let _ = std::fs::create_dir_all(&d);
            println!("saving raw pages to {}", d.display());
            let _ = SAVE_DIR.set(d);
        }
        let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::clean(title));
        println!("1. search \"{q}\"");
        let html = match get(c, &format!("{BASE}/search.php"), &[("search_type", "title"), ("value", &q)]).await {
            Ok(h) => h,
            Err(e) => return println!("   FAILED: {e}"),
        };
        println!("   {} bytes", html.len());
        let Some(store) = extract_store(&html) else {
            return println!("   FAILED: no js-store data-content JSON in the page (blocked, captcha, or format changed)");
        };
        let all = match find_key(&store, "results") {
            Some(Value::Array(a)) => a.len(),
            _ => 0,
        };
        println!("   js-store parsed; {all} results in total");
        let cands = candidates(&store, artist, title);
        if cands.is_empty() {
            return println!("   no chord pages matching this artist/title");
        }
        for (i, cd) in cands.iter().enumerate() {
            println!("   [{i}] {} — {}  rating {:.2}  votes {}  score {:.1}\n       {}", cd.artist, cd.song, cd.rating, cd.votes, cd.score, cd.url);
        }
        for (i, cd) in cands.iter().enumerate() {
            println!("2. fetch [{i}]");
            let url = if cd.url.starts_with('/') { format!("{BASE}{}", cd.url) } else { cd.url.clone() };
            let html = match get(c, &url, &[]).await {
                Ok(h) => h,
                Err(e) => return println!("   FAILED: {e}"),
            };
            let Some(page) = extract_store(&html) else {
                return println!("   FAILED: tab page has no js-store JSON");
            };
            let has_wiki = find_key(&page, "wiki_tab").is_some();
            println!("   wiki_tab present: {has_wiki}");
            match sheet_from_store(&page) {
                Some(s) => {
                    println!("   OK: {} chords on {} lines, capo {}", s.chord_count(), s.lines.len(), s.capo);
                    for l in s.lines.iter().take(8) {
                        let cs: Vec<_> = l.chords.iter().map(|(c, n)| format!("{n}@{c}")).collect();
                        println!("   | {:<40} {}", l.text.trim(), cs.join(" "));
                    }
                    return;
                }
                None => println!("   rejected (fewer than {MIN_CHORDS} recognisable chords)"),
            }
        }
        println!("no usable sheet among the candidates");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Member parts (experimental, opt-in: --member-parts). Genius has no open API for this, so
// the song page is scraped for section headers like `[Verse 1: Jimin]` and everything
// is written to fail soft and silent.
// ════════════════════════════════════════════════════════════════════════════
mod genius {
    use crate::chords;
    use crate::util::{cache_dir, fnv1a, now_secs};
    use ratatui::style::Color;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    pub const BASE: &str = "https://genius.com";
    // looks like a normal browser, as for Ultimate Guitar
    const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
    const NEG_TTL: u64 = 7 * 24 * 3600;

    /// One lyric line and the members singing it (empty = unknown / everyone).
    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Part {
        pub members: Vec<String>,
        pub text: String,
    }

    #[derive(Serialize, Deserialize)]
    struct CacheEntry {
        kind: String, // found | none
        parts: Vec<Part>,
        saved: u64,
    }

    /// A stable colour per member name, from the terminal's own ANSI palette.
    pub fn color(name: &str) -> Color {
        const PALETTE: [Color; 8] = [
            Color::LightMagenta,
            Color::LightCyan,
            Color::LightYellow,
            Color::LightGreen,
            Color::LightBlue,
            Color::LightRed,
            Color::Magenta,
            Color::Green,
        ];
        PALETTE[(fnv1a(&name.to_lowercase()) % PALETTE.len() as u64) as usize]
    }

    // ── page parsing ───────────────────────────────────────────────────────
    /// `[Verse 1: Jimin]` → `["Jimin"]`, `[Chorus: Jungkook, V]` → both, `[Pre-Chorus: RM &
    /// SUGA]` → both. `Some(vec![])` for a header without names (`[Bridge]`); `None` if the
    /// line isn't a header.
    pub fn parse_header(line: &str) -> Option<Vec<String>> {
        let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
        let Some((_, names)) = inner.split_once(':') else {
            return Some(Vec::new());
        };
        let mut out: Vec<String> = Vec::new();
        for n in names.replace(" and ", ",").split([',', '&', '+', ';', '/']) {
            // "Jin (진)" → "Jin"
            let n = n.split('(').next().unwrap_or("").trim();
            if !n.is_empty() && n.chars().count() <= 24 && !out.iter().any(|o| o == n) {
                out.push(n.to_string());
            }
        }
        Some(out)
    }

    /// The inner HTML of every `data-lyrics-container` div (tag-depth aware).
    fn containers(html: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(i) = html[from..].find("data-lyrics-container") {
            let at = from + i;
            let Some(open_end) = html[at..].find('>').map(|e| at + e + 1) else { break };
            let (mut depth, mut pos) = (1usize, open_end);
            let mut end = html.len();
            while let Some(lt) = html[pos..].find('<').map(|l| pos + l) {
                let rest = &html[lt..];
                if rest.starts_with("<div") {
                    depth += 1;
                } else if rest.starts_with("</div") {
                    depth -= 1;
                    if depth == 0 {
                        end = lt;
                        break;
                    }
                }
                pos = lt + 1;
            }
            out.push(&html[open_end..end]);
            from = end.max(open_end);
        }
        out
    }

    /// Container HTML → text: `<br>` is a newline, other tags vanish (and the contents of
    /// `data-exclude-from-selection` blocks with them), entities are decoded.
    fn text_of(html: &str) -> String {
        let (mut out, mut skip, mut pos) = (String::new(), 0usize, 0usize);
        while pos < html.len() {
            let Some(lt) = html[pos..].find('<').map(|l| pos + l) else {
                if skip == 0 {
                    out.push_str(&html[pos..]);
                }
                break;
            };
            if skip == 0 {
                out.push_str(&html[pos..lt]);
            }
            let Some(gt) = html[lt..].find('>').map(|g| lt + g) else { break };
            let tag = html[lt + 1..gt].trim().to_ascii_lowercase();
            if tag.starts_with("div") {
                if skip > 0 {
                    skip += 1;
                } else if tag.contains("data-exclude-from-selection") {
                    skip = 1;
                }
            } else if tag.starts_with("/div") {
                skip = skip.saturating_sub(1);
            } else if skip == 0 && (tag == "br" || tag.starts_with("br ") || tag.starts_with("br/")) {
                out.push('\n');
            }
            pos = gt + 1;
        }
        crate::ug::unescape(&out)
    }

    /// A song page → its lines, each with the members of the section it sits in.
    pub fn parse_page(html: &str) -> Vec<Part> {
        let mut parts = Vec::new();
        let mut members: Vec<String> = Vec::new();
        for c in containers(html) {
            for line in text_of(c).lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match parse_header(line) {
                    Some(m) => members = m,
                    None => parts.push(Part { members: members.clone(), text: line.to_string() }),
                }
            }
        }
        parts
    }

    /// Path (`/Bts-dynamite-lyrics`) of the best song hit in a `search/multi` reply: the
    /// title and artist must both really be the ones asked for.
    pub fn best_hit(reply: &Value, artist: &str, title: &str) -> Option<String> {
        let want_t = chords::norm(&crate::year::clean(title));
        let want_a = chords::norm(artist.split(", ").next().unwrap_or(artist));
        let mut best: Option<(f32, String)> = None;
        let sections = reply.pointer("/response/sections")?.as_array()?;
        for hit in sections.iter().filter_map(|s| s.get("hits")?.as_array()).flatten() {
            if hit.get("type").and_then(Value::as_str).is_some_and(|t| t != "song") {
                continue;
            }
            let Some(r) = hit.get("result") else { continue };
            let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
            let song = chords::norm(&crate::year::clean(text("title")));
            let art = chords::norm(
                r.pointer("/primary_artist/name")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| text("artist_names")),
            );
            let t = chords::dice(&want_t, &song);
            let artist_ok =
                !art.is_empty() && (art.contains(&want_a) || want_a.contains(&art) || chords::dice(&want_a, &art) >= 0.7);
            let path = match (text("path"), text("url")) {
                ("", "") => continue,
                ("", u) => u.split_once("://").and_then(|(_, h)| h.find('/').map(|i| h[i..].to_string())).unwrap_or_default(),
                (p, _) => p.to_string(),
            };
            if t >= 0.7 && artist_ok && !path.is_empty() && best.as_ref().is_none_or(|(b, _)| t > *b) {
                best = Some((t, path));
            }
        }
        best.map(|b| b.1)
    }

    /// Per synced lyric line, the members singing it: Genius lines are matched in order
    /// (a short look-ahead window, so a skipped line doesn't derail the rest). A line with
    /// no match, or in an unattributed section, gets an empty list.
    pub fn attribute(parts: &[Part], lyrics: &[String]) -> Vec<Vec<String>> {
        let norms: Vec<String> = parts.iter().map(|p| chords::norm(&p.text)).collect();
        let mut out = vec![Vec::new(); lyrics.len()];
        let mut j = 0usize;
        for (li, text) in lyrics.iter().enumerate() {
            let n = chords::norm(text);
            if n.is_empty() {
                continue;
            }
            let mut best: Option<(usize, f32)> = None;
            for (k, s) in norms.iter().enumerate().skip(j).take(12) {
                let d = chords::dice(&n, s);
                if d >= 0.6 && best.is_none_or(|(_, b)| d > b) {
                    best = Some((k, d));
                }
            }
            if let Some((k, _)) = best {
                j = k + 1;
                out[li] = parts[k].members.clone();
            }
        }
        out
    }

    // ── network ────────────────────────────────────────────────────────────
    async fn get(c: &reqwest::Client, url: &str, query: &[(&str, &str)]) -> Result<String, String> {
        let r = c
            .get(url)
            .query(query)
            .header(reqwest::header::USER_AGENT, UA)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("network error ({})", e.without_url()))?;
        let st = r.status();
        if !st.is_success() {
            return Err(format!("HTTP {} from genius", st.as_u16()));
        }
        r.text().await.map_err(|e| format!("read error ({e})"))
    }

    fn cache_path(artist: &str, title: &str) -> std::path::PathBuf {
        cache_dir().join(format!("members-{:016x}.json", fnv1a(&format!("{artist}\0{title}"))))
    }

    /// Cache → search → song page. `Ok(None)` = Genius has nothing usable (remembered for a
    /// week); `Err` is never cached.
    pub async fn lookup(c: &reqwest::Client, base: &str, artist: &str, title: &str) -> Result<Option<Vec<Part>>, String> {
        let path = cache_path(artist, title);
        if let Ok(txt) = tokio::fs::read_to_string(&path).await {
            if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
                if e.kind == "found" {
                    return Ok(Some(e.parts));
                } else if now_secs().saturating_sub(e.saved) < NEG_TTL {
                    return Ok(None);
                }
            }
        }
        let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::clean(title));
        let body = get(c, &format!("{base}/api/search/multi"), &[("q", &q)]).await?;
        let reply: Value = serde_json::from_str(&body).map_err(|_| "unreadable genius reply".to_string())?;
        let parts = match best_hit(&reply, artist, title) {
            Some(p) => parse_page(&get(c, &format!("{base}{p}"), &[]).await?),
            None => Vec::new(),
        };
        // a page without a single member header is no use, however many lines it has
        let parts = if parts.iter().any(|p| !p.members.is_empty()) { Some(parts) } else { None };
        let entry = match &parts {
            Some(p) => CacheEntry { kind: "found".into(), parts: p.clone(), saved: now_secs() },
            None => CacheEntry { kind: "none".into(), parts: Vec::new(), saved: now_secs() },
        };
        if let Ok(j) = serde_json::to_string(&entry) {
            let _ = tokio::fs::write(&path, j).await;
        }
        Ok(parts)
    }

    /// Lookup for the app: any failure just means no member tags.
    pub async fn fetch(c: reqwest::Client, artist: String, title: String) -> Vec<Part> {
        lookup(&c, BASE, &artist, &title).await.ok().flatten().unwrap_or_default()
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Guitar chord diagrams (standard tuning, strings low E → high e)
// ════════════════════════════════════════════════════════════════════════════
mod diagram {
    /// Absolute frets per string (-1 = muted, 0 = open) and finger numbers (0 = none).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Shape {
        pub frets: [i8; 6],
        pub fingers: [u8; 6],
    }

    /// Open-position shapes that beat the barre version.
    const OPEN: &[(&str, &str, &str)] = &[
        ("C", "x32010", "032010"),
        ("D", "xx0232", "000132"),
        ("G", "320003", "210004"),
        ("Dm", "xx0231", "000231"),
        ("C7", "x32310", "032410"),
        ("D7", "xx0212", "000213"),
        ("G7", "320001", "320001"),
        ("B7", "x21202", "021304"),
        ("Cmaj7", "x32000", "032000"),
        ("Dmaj7", "xx0222", "000111"),
        ("Gmaj7", "320002", "210003"),
        ("Dm7", "xx0211", "000211"),
        ("Dsus2", "xx0230", "000230"),
        ("Dsus4", "xx0233", "000123"),
        ("Gsus4", "330013", "340012"),
        ("Csus4", "x33011", "034011"),
    ];

    /// Movable shapes: (quality, E-shape relative frets, E fingers, A-shape frets, A fingers).
    #[allow(clippy::type_complexity)]
    const MOVABLE: &[(&str, [i8; 6], [u8; 6], [i8; 6], [u8; 6])] = &[
        ("", [0, 2, 2, 1, 0, 0], [1, 3, 4, 2, 1, 1], [-1, 0, 2, 2, 2, 0], [0, 1, 2, 3, 4, 1]),
        ("m", [0, 2, 2, 0, 0, 0], [1, 3, 4, 1, 1, 1], [-1, 0, 2, 2, 1, 0], [0, 1, 3, 4, 2, 1]),
        ("7", [0, 2, 0, 1, 0, 0], [1, 3, 1, 2, 1, 1], [-1, 0, 2, 0, 2, 0], [0, 1, 3, 1, 4, 1]),
        ("m7", [0, 2, 0, 0, 0, 0], [1, 3, 1, 1, 1, 1], [-1, 0, 2, 0, 1, 0], [0, 1, 3, 1, 2, 1]),
        ("maj7", [0, 2, 1, 1, 0, 0], [1, 4, 2, 3, 1, 1], [-1, 0, 2, 1, 2, 0], [0, 1, 3, 2, 4, 1]),
        ("sus4", [0, 2, 2, 2, 0, 0], [1, 2, 3, 4, 1, 1], [-1, 0, 2, 2, 3, 0], [0, 1, 2, 3, 4, 1]),
        ("sus2", [-1, -1, -1, -1, -1, -1], [0; 6], [-1, 0, 2, 2, 0, 0], [0, 1, 3, 4, 1, 1]),
        ("5", [0, 2, 2, -1, -1, -1], [1, 3, 4, 0, 0, 0], [-1, 0, 2, 2, -1, -1], [0, 1, 3, 4, 0, 0]),
    ];

    const SHARP: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

    /// `(pitch class, quality)` for plain chords; `None` for slash chords, dim/aug/9ths…
    fn split(name: &str) -> Option<(u8, &'static str)> {
        if name.contains('/') {
            return None;
        }
        let b = name.as_bytes();
        let base: i32 = match b.first()? {
            b'C' => 0,
            b'D' => 2,
            b'E' => 4,
            b'F' => 5,
            b'G' => 7,
            b'A' => 9,
            b'B' => 11,
            _ => return None,
        };
        let (acc, rest) = match b.get(1) {
            Some(b'#') => (1, &name[2..]),
            Some(b'b') => (-1, &name[2..]),
            _ => (0, &name[1..]),
        };
        let q = match rest {
            "" | "maj" | "M" => "",
            "m" | "min" | "-" => "m",
            "7" => "7",
            "m7" | "min7" | "-7" => "m7",
            "maj7" | "M7" | "Maj7" => "maj7",
            "sus4" | "sus" => "sus4",
            "sus2" => "sus2",
            "5" => "5",
            _ => return None,
        };
        Some(((base + acc).rem_euclid(12) as u8, q))
    }

    pub fn lookup(name: &str) -> Option<Shape> {
        let (pc, q) = split(name)?;
        let key = format!("{}{q}", SHARP[pc as usize]);
        if let Some((_, f, fi)) = OPEN.iter().find(|(n, _, _)| *n == key) {
            let frets = f
                .chars()
                .map(|c| if c == 'x' { -1 } else { c.to_digit(10).unwrap() as i8 })
                .collect::<Vec<_>>();
            let fingers = fi.chars().map(|c| c.to_digit(10).unwrap() as u8).collect::<Vec<_>>();
            return Some(Shape {
                frets: frets.try_into().ok()?,
                fingers: fingers.try_into().ok()?,
            });
        }
        let (_, e_f, e_fi, a_f, a_fi) = MOVABLE.iter().find(|m| m.0 == q)?;
        let f6 = (pc as i32 - 4).rem_euclid(12);
        let f5 = (pc as i32 - 9).rem_euclid(12);
        // lowest position wins; E-shape on ties (and sus2 only exists as an A-shape)
        let use_a = *e_f == [-1; 6] || f5 < f6;
        let (rel, fin, base) = if use_a { (a_f, a_fi, f5) } else { (e_f, e_fi, f6) };
        let mut frets = [-1i8; 6];
        let mut fingers = *fin;
        for i in 0..6 {
            if rel[i] >= 0 {
                frets[i] = rel[i] + base as i8;
            }
            // open-position version of a barre shape: the barre finger drops out
            if base == 0 && fingers[i] > 1 {
                fingers[i] -= 1;
            } else if base == 0 && fingers[i] == 1 && rel[i] == 0 {
                fingers[i] = 0;
            }
        }
        // a shape that would need frets above the 12th isn't a sensible diagram
        if frets.iter().any(|f| *f > 12) {
            return None;
        }
        Some(Shape { frets, fingers })
    }

    /// 8 lines, 11 columns wide: name, mute/open markers, nut, four fret rows.
    pub fn render(name: &str, s: &Shape) -> Vec<String> {
        let fretted: Vec<i8> = s.frets.iter().copied().filter(|f| *f > 0).collect();
        let max = fretted.iter().copied().max().unwrap_or(0);
        let base = if max > 4 { fretted.iter().copied().min().unwrap_or(1) } else { 1 };
        let mut out = vec![format!("{name:^11}")];
        out.push(
            s.frets
                .iter()
                .map(|f| match f {
                    -1 => 'x',
                    0 => 'o',
                    _ => ' ',
                })
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" "),
        );
        out.push(if base == 1 { "╒═╤═╤═╤═╤═╕".into() } else { "┌─┬─┬─┬─┬─┐".into() });
        for r in 0..4 {
            let fret = base + r;
            let row = (0..6)
                .map(|i| {
                    if s.frets[i] == fret {
                        match s.fingers[i] {
                            0 => "●".to_string(),
                            n => n.to_string(),
                        }
                    } else {
                        "│".to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            out.push(if r == 0 && base > 1 { format!("{row} {base}fr") } else { row });
        }
        out
    }

    /// A little tab: one column per chord, strings e B G D A E from the top.
    pub fn tab(shapes: &[Option<Shape>]) -> Vec<String> {
        const NAMES: [char; 6] = ['e', 'B', 'G', 'D', 'A', 'E'];
        (0..6)
            .map(|row| {
                let s = 5 - row;
                let mut line = format!("{}|", NAMES[row]);
                for sh in shapes {
                    let cell = match sh {
                        None => "?".to_string(),
                        Some(sh) if sh.frets[s] < 0 => "x".to_string(),
                        Some(sh) => sh.frets[s].to_string(),
                    };
                    line.push_str(&format!("-{cell:-<2}-"));
                }
                line.push('|');
                line
            })
            .collect()
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
    Chords {
        key: u64,
        sheet: Result<Option<chords::Sheet>, String>,
        /// Saved per-track capo override, if any.
        capo: Option<u8>,
    },
    /// Tags read from a local file (`None` = unreadable).
    Tags {
        url: String,
        tags: Option<tags::Tags>,
    },
    Translated {
        key: u64,
        lang: String,
        lines: Result<(Vec<String>, &'static str), String>,
    },
    /// Lines with the members singing them, from Genius (empty = nothing usable).
    Members {
        key: u64,
        parts: Vec<genius::Part>,
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

    /// Everything a key can do. The input handler and the `?` help overlay both read
    /// [`BINDS`], so the two can't drift apart.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Action {
        Quit,
        PlayPause,
        Next,
        Prev,
        Shuffle,
        Repeat,
        SeekBack,
        SeekFwd,
        SyncEarlier,
        SyncLater,
        VolUp,
        VolDown,
        Mute,
        ToggleLyrics,
        ToggleBig,
        ToggleCava,
        ToggleChords,
        TransposeUp,
        TransposeDown,
        CapoDown,
        CapoUp,
        CapoReset,
        Diagrams,
        Minimal,
        Translate,
        Settings,
        BrowseUp,
        BrowseDown,
        BrowsePageUp,
        BrowsePageDown,
        BrowseEnd,
        Help,
    }

    pub struct Bind {
        pub group: &'static str,
        /// How the key is written in the help overlay.
        pub keys: &'static str,
        pub desc: &'static str,
        pub codes: &'static [KeyCode],
        pub action: Action,
    }

    macro_rules! bind {
        ($g:literal, $k:literal, $d:literal, [$($c:expr),+], $a:ident) => {
            Bind { group: $g, keys: $k, desc: $d, codes: &[$($c),+], action: Action::$a }
        };
    }

    pub const GROUPS: [&str; 4] = ["Playback", "Lyrics", "Chords", "View"];

    pub const BINDS: &[Bind] = &[
        bind!("Playback", "space", "play / pause", [KeyCode::Char(' ')], PlayPause),
        bind!("Playback", "n", "next track", [KeyCode::Char('n')], Next),
        bind!("Playback", "p", "previous track", [KeyCode::Char('p')], Prev),
        bind!("Playback", "z", "shuffle on / off", [KeyCode::Char('z')], Shuffle),
        bind!("Playback", "r", "repeat: off / playlist / track", [KeyCode::Char('r')], Repeat),
        bind!("Playback", "←", "seek back 5 s", [KeyCode::Left], SeekBack),
        bind!("Playback", "→", "seek forward 5 s", [KeyCode::Right], SeekFwd),
        bind!("Playback", "+", "volume up", [KeyCode::Char('+'), KeyCode::Char('=')], VolUp),
        bind!("Playback", "-", "volume down", [KeyCode::Char('-')], VolDown),
        bind!("Playback", "m", "mute / unmute", [KeyCode::Char('m')], Mute),
        bind!("Lyrics", ",", "lyrics 0.1 s later", [KeyCode::Char(',')], SyncEarlier),
        bind!("Lyrics", ".", "lyrics 0.1 s earlier", [KeyCode::Char('.')], SyncLater),
        bind!("Lyrics", "l", "show / hide lyrics", [KeyCode::Char('l')], ToggleLyrics),
        bind!("Lyrics", "b", "big-font active line", [KeyCode::Char('b')], ToggleBig),
        bind!("Chords", "g", "chords on / off", [KeyCode::Char('g')], ToggleChords),
        bind!("Chords", "t", "transpose up", [KeyCode::Char('t')], TransposeUp),
        bind!("Chords", "T", "transpose down", [KeyCode::Char('T')], TransposeDown),
        bind!("Chords", "]", "capo up", [KeyCode::Char(']')], CapoUp),
        bind!("Chords", "[", "capo down", [KeyCode::Char('[')], CapoDown),
        bind!("Chords", "0", "sheet's own capo", [KeyCode::Char('0')], CapoReset),
        bind!("Chords", "D", "chord diagrams", [KeyCode::Char('D')], Diagrams),
        bind!("View", "M", "minimal view", [KeyCode::Char('M')], Minimal),
        bind!("View", "c", "cava visualiser", [KeyCode::Char('c')], ToggleCava),
        bind!("Lyrics", "↑", "browse lyrics up (playback goes on)", [KeyCode::Up], BrowseUp),
        bind!("Lyrics", "↓", "browse lyrics down", [KeyCode::Down], BrowseDown),
        bind!("Lyrics", "PgUp", "browse a page up", [KeyCode::PageUp], BrowsePageUp),
        bind!("Lyrics", "PgDn", "browse a page down", [KeyCode::PageDown], BrowsePageDown),
        bind!("Lyrics", "End", "back to the playing line", [KeyCode::End], BrowseEnd),
        bind!("Lyrics", "K", "translate lyrics on / off", [KeyCode::Char('K')], Translate),
        bind!("View", "s / S", "settings menu", [KeyCode::Char('s'), KeyCode::Char('S')], Settings),
        bind!("View", "?", "this help", [KeyCode::Char('?')], Help),
        bind!("View", "q / Esc", "quit", [KeyCode::Char('q'), KeyCode::Esc], Quit),
    ];

    /// Mouse gestures (they are hit-tested where they are drawn, so this is documentation
    /// kept next to the table): (group, gesture, what it does).
    pub const MOUSE: &[(&str, &str, &str)] = &[
        ("Playback", "click bar", "seek there"),
        ("Playback", "click buttons", "shuf / rep / prev / play / next"),
        ("Playback", "click - / +", "volume down / up"),
        ("Playback", "click vol %", "mute / restore"),
        ("Playback", "wheel: volume", "volume up / down"),
        ("Lyrics", "click a line", "seek to that line"),
    ];

    pub fn action_for(code: KeyCode) -> Option<Action> {
        BINDS
            .iter()
            .find(|b| b.codes.contains(&code))
            .map(|b| b.action)
    }

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

    pub enum ChordsState {
        /// Chords off, or nothing playing.
        Idle,
        Fetching,
        Missing,
        Failed(String),
        Ready,
    }

    pub enum TranslateState {
        Idle,
        Loading,
        Failed(String),
    }

    /// One row of the settings menu.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Setting {
        Player,
        Chords,
        ChordsUg,
        MemberParts,
        Translate,
        TranslateLang,
        TranslateFallback,
        Minimal,
        BigLyrics,
        BigScale,
        LyricSpacing,
        LyricPad,
        Cava,
        ThirdPanel,
        CoverTheme,
        AsciiCover,
        Transparent,
        HideColorWarning,
        Mouse,
        VolumeStep,
    }

    impl Setting {
        pub const ALL: &'static [Setting] = &[
            Setting::Player,
            Setting::Chords,
            Setting::ChordsUg,
            Setting::MemberParts,
            Setting::Translate,
            Setting::TranslateLang,
            Setting::TranslateFallback,
            Setting::Minimal,
            Setting::BigLyrics,
            Setting::BigScale,
            Setting::LyricSpacing,
            Setting::LyricPad,
            Setting::Cava,
            Setting::ThirdPanel,
            Setting::CoverTheme,
            Setting::AsciiCover,
            Setting::Transparent,
            Setting::HideColorWarning,
            Setting::Mouse,
            Setting::VolumeStep,
        ];

        pub fn label(self) -> &'static str {
            match self {
                Setting::Player => "Player source",
                Setting::Chords => "Chords",
                Setting::ChordsUg => "Ultimate Guitar (next track)",
                Setting::MemberParts => "Member parts (experimental)",
                Setting::Translate => "Translate lyrics",
                Setting::TranslateLang => "Translate into",
                Setting::TranslateFallback => "LibreTranslate fallback",
                Setting::Minimal => "Minimal view",
                Setting::BigLyrics => "Big active line",
                Setting::BigScale => "Big line size",
                Setting::LyricSpacing => "Lyric spacing",
                Setting::LyricPad => "Lyric side padding",
                Setting::Cava => "Cava visualiser",
                Setting::ThirdPanel => "Cava as third column",
                Setting::CoverTheme => "Accent from cover",
                Setting::AsciiCover => "ASCII cover",
                Setting::Transparent => "Transparent background",
                Setting::HideColorWarning => "Hide colour-file note",
                Setting::Mouse => "Mouse support",
                Setting::VolumeStep => "Volume step %",
            }
        }

        /// The config-file keys this row owns.
        pub fn config_keys(self) -> &'static [&'static str] {
            match self {
                Setting::Player => &["player"],
                Setting::Chords => &["chords"],
                Setting::ChordsUg => &["chords_ug"],
                Setting::MemberParts => &["member_parts"],
                Setting::Translate => &["translate"],
                Setting::TranslateLang => &["translate_lang"],
                Setting::TranslateFallback => &["translate_fallback"],
                Setting::Minimal => &["minimal_view"],
                Setting::BigLyrics => &["big_lyrics"],
                Setting::BigScale => &["big_scale"],
                Setting::LyricSpacing => &["lyric_spacing"],
                Setting::LyricPad => &["lyric_pad"],
                Setting::Cava => &["cava", "thirdpanel"],
                Setting::ThirdPanel => &["thirdpanel", "cava"],
                Setting::CoverTheme => &["cover_theme"],
                Setting::AsciiCover => &["ascii_cover"],
                Setting::Transparent => &["transparent"],
                Setting::HideColorWarning => &["hide_color_warning"],
                Setting::Mouse => &["mouse"],
                Setting::VolumeStep => &["volume_step"],
            }
        }

        /// Takes effect only after a restart.
        pub fn needs_restart(self) -> bool {
            matches!(
                self,
                Setting::CoverTheme | Setting::AsciiCover | Setting::Transparent | Setting::Mouse
            )
        }
    }

    pub const LANGS: &[&str] = &["en", "sv", "de", "fr", "es", "it", "pt", "nl", "ja", "ko", "zh-CN", "ru", "ar"];

    /// Click targets recorded during the last render.
    #[derive(Default)]
    pub struct Hits {
        pub progress: Rect,
        pub shuffle: Rect,
        pub repeat: Rect,
        pub prev: Rect,
        pub play: Rect,
        pub next: Rect,
        pub vol_minus: Rect,
        pub vol_pct: Rect,
        pub vol_plus: Rect,
        /// The whole volume control (mouse-wheel target).
        pub vol_area: Rect,
    }

    /// Where each visible lyrics row came from, recorded during the last render so a click
    /// can be mapped back to a lyric line (accounting for scroll and wrapped lines).
    #[derive(Default)]
    pub struct LyricHits {
        pub area: Rect,
        /// Row index shown at the top of `area`.
        pub top: i64,
        /// Per row: the lyric line it belongs to, or `None` if not clickable.
        pub rows: Vec<Option<usize>>,
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

        pub chords_on: bool,
        pub chords_state: ChordsState,
        sheet: Option<chords::Sheet>,
        pub aligned: Option<chords::Aligned>,
        /// Semitones the displayed chords are shifted by.
        pub transpose: i32,
        /// Per-track capo chosen with `[` / `]`; `None` = the sheet's own capo.
        pub capo_override: Option<u8>,

        /// Genius lines with their members for this track (`--member-parts`).
        member_src: Vec<genius::Part>,
        /// Per lyric line, the members singing it (empty = unknown); the lyrics panel tags
        /// lines with these.
        pub members: Vec<Vec<String>>,

        pub show_lyrics: bool,
        pub show_cava: bool,
        pub placement: CavaPlacement,
        pub cava_bars: Vec<u16>,
        pub cava_status: Option<String>,
        cava_task: Option<JoinHandle<()>>,
        tasks: Vec<JoinHandle<()>>,

        pub hits: Hits,
        pub lyric_hits: LyricHits,
        /// Player volume, 0..=1 (`None` until the player reports one).
        pub volume: Option<f64>,
        /// Last non-zero volume, restored when un-muting.
        vol_last: f64,
        /// Ignore volume reported by the poller until this long after our own change.
        vol_hold: Instant,
        /// Smoothly animated lyric scroll position (in wrapped rows).
        pub lyric_scroll: f64,
        pub lyric_scroll_target: f64,
        pub snap_scroll: bool,
        pub last_anim: Instant,
        pub dirty: bool,
        /// Volume change per `+` / `-` / wheel notch, in percent.
        pub vol_step: u32,
        /// `M`: teleprompter view (title, lyrics, thin progress bar only).
        pub minimal: bool,
        /// Cava was showing when minimal view hid it; restored on leaving.
        cava_before_minimal: bool,
        /// `D`: chord diagram panel.
        pub show_diagrams: bool,
        /// `?`: help overlay, and how far it is scrolled.
        pub show_help: bool,
        pub help_scroll: u16,
        /// `s`: settings menu and the highlighted row.
        pub show_settings: bool,
        pub settings_sel: usize,
        pub settings_top: usize,
        /// Last settings-save problem, shown in the menu.
        pub settings_note: Option<String>,
        /// `K`: show translated lyrics.
        pub translate_on: bool,
        pub translate_state: TranslateState,
        /// The (song, language) request currently running, so toggling doesn't duplicate it.
        translate_inflight: Option<(u64, String)>,
        /// Translations kept for the session, per song and language.
        pub translations: std::collections::HashMap<(u64, String), Vec<String>>,
        /// Test hook: alternative translate endpoint.
        pub translate_base: Option<String>,
        /// Which backend produced the translation on screen.
        pub translate_backend: &'static str,
        /// Rows the lyrics are scrolled away from the playing line (0 = following).
        pub browse: f64,
        browse_at: Instant,
        pub local_tags: std::collections::HashMap<String, tags::Tags>,
        /// When the player we are showing last reported in.
        player_seen: Instant,
        tags_asked: std::collections::HashSet<String>,
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
                chords_on: args.chords,
                chords_state: ChordsState::Idle,
                sheet: None,
                aligned: None,
                transpose: 0,
                capo_override: None,
                member_src: Vec::new(),
                members: Vec::new(),
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
                lyric_hits: LyricHits::default(),
                volume: None,
                vol_last: 0.5,
                vol_hold: Instant::now() - Duration::from_secs(5),
                lyric_scroll: 0.0,
                lyric_scroll_target: 0.0,
                snap_scroll: true,
                last_anim: Instant::now(),
                dirty: true,
                vol_step: 0, // set from args below
                minimal: false,
                cava_before_minimal: false,
                show_diagrams: false,
                show_help: false,
                help_scroll: 0,
                show_settings: false,
                settings_sel: 0,
                settings_top: 0,
                settings_note: None,
                translate_on: false,
                translate_state: TranslateState::Idle,
                translate_inflight: None,
                translations: Default::default(),
                translate_base: None,
                translate_backend: "google",
                browse: 0.0,
                browse_at: Instant::now(),
                local_tags: Default::default(),
                player_seen: Instant::now(),
                tags_asked: Default::default(),
            };
            app.translate_on = app.args.translate;
            app.vol_step = app.args.volume_step;
            if app.args.cava || app.args.thirdpanel || app.args.underlyrics {
                app.set_cava(true);
            }
            if app.args.minimal {
                app.set_minimal(true);
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
                    self.realign();
                    self.realign_members();
                    self.ensure_translation();
                    self.dirty = true;
                }
                AppEvent::Members { key, parts } if key == self.track_key => {
                    self.member_src = parts;
                    self.realign_members();
                    self.dirty = true;
                }
                AppEvent::Chords { key, sheet, capo } if key == self.track_key => {
                    self.capo_override = capo.filter(|c| *c <= 12);
                    match sheet {
                        Ok(Some(mut s)) => {
                            if s.key.is_none() {
                                s.key = chords::estimate_key(&s);
                            }
                            self.sheet = Some(s);
                            self.chords_state = ChordsState::Ready;
                        }
                        Ok(None) => {
                            self.sheet = None;
                            self.chords_state = ChordsState::Missing;
                        }
                        Err(e) => {
                            self.sheet = None;
                            self.chords_state = ChordsState::Failed(e);
                        }
                    }
                    self.realign();
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
                AppEvent::Tags { url, tags } => {
                    if let Some(t) = tags {
                        self.local_tags.insert(url.clone(), t);
                    }
                    // re-run the current snapshot with the tags merged in
                    if let Some(s) = self.snap.clone().filter(|s| s.url == url) {
                        self.on_snapshot(s);
                    }
                    self.dirty = true;
                }
                AppEvent::Translated { key, lang, lines } => {
                    match lines {
                        Ok((l, backend)) => {
                            self.translations.insert((key, lang.clone()), l);
                            self.translate_backend = backend;
                        }
                        Err(e) if key == self.track_key && lang == self.args.translate_lang => {
                            self.translate_state = TranslateState::Failed(e)
                        }
                        Err(_) => {}
                    }
                    if self.translate_inflight.as_ref() == Some(&(key, lang.clone())) {
                        self.translate_inflight = None;
                    }
                    if key == self.track_key && lang == self.args.translate_lang {
                        if self.translations.contains_key(&(key, lang)) {
                            self.translate_state = TranslateState::Idle;
                        }
                        self.snap_scroll = true;
                    }
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

        fn on_snapshot(&mut self, mut s: Snapshot) {
            if s.url.starts_with("file://") {
                self.merge_local_tags(&mut s);
            }
            if let Ok(path) = std::env::var("MUSIC_DISPLAY_DEBUG") {
                use std::io::Write;
                let path = if path == "1" { crate::util::cache_dir().join("snapshots.log") } else { path.into() };
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{:?}", s);
                }
            }
            // Several players at once (`any`, a list): stay with the one being shown while it
            // reports in, unless another one starts playing while ours doesn't.
            if let Some(o) = &self.snap {
                if !s.player.is_empty() && !o.player.is_empty() && s.player != o.player {
                    let ours_quiet = self.player_seen.elapsed() > Duration::from_secs(3);
                    let takeover = s.status == Status::Playing && o.status != Status::Playing;
                    if !ours_quiet && !takeover {
                        return;
                    }
                }
            }
            self.player_seen = Instant::now();
            let mut key = s.key();
            if key != self.track_key && self.same_recording(&s) {
                key = self.track_key;
            }
            let new_track = key != self.track_key;
            // Same track, different spelling of the strings (edition names, art): keep what
            // is already on screen and only fill in what was missing.
            if !new_track {
                if let Some(o) = &self.snap {
                    let keep = |old: &str, new: &mut String| {
                        if !old.is_empty() {
                            *new = old.to_string();
                        }
                    };
                    keep(&o.title, &mut s.title);
                    keep(&o.artist, &mut s.artist);
                    keep(&o.album, &mut s.album);
                    keep(&o.content_created, &mut s.content_created);
                    if !self.art_url.is_empty() && !matches!(self.cover, CoverState::Failed) {
                        s.art_url = self.art_url.clone();
                    }
                    s.track_no = o.track_no.or(s.track_no);
                    s.disc_no = o.disc_no.or(s.disc_no);
                }
            }
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
                self.sheet = None;
                self.aligned = None;
                self.transpose = 0;
                self.capo_override = None;
                self.member_src.clear();
                self.members.clear();
                self.chords_state = ChordsState::Idle;
                self.translate_state = TranslateState::Idle;
                self.translate_inflight = None;

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
                let policy = lyrics::Policy {
                    retries: self.args.lyrics_retries,
                    ..Default::default()
                };
                let (a, t, al, len) =
                    (s.artist.clone(), s.title.clone(), s.album.clone(), s.length);
                self.tasks.push(tokio::spawn(async move {
                    let lyrics = lyrics::fetch(c, a, t, al, len, policy).await;
                    let _ = tx.send(AppEvent::Lyrics { key, lyrics });
                }));
            }

            if new_track && self.args.member_parts {
                self.fetch_members(key, &s.artist, &s.title);
            }

            // with Ultimate Guitar on, the sheet is also fetched while chords are hidden, so the
            // Key / BPM / Tuning rows can fill in
            if new_track && (self.chords_on || self.args.chords_ug) {
                self.fetch_chords(key, &s.artist, &s.title);
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

            if let Some(v) = s.volume {
                // External changes (keyboard media keys, the Spotify app) show up here; our
                // own changes win for a moment so the poller doesn't flicker them back.
                if self.vol_hold.elapsed() > Duration::from_millis(1500)
                    && self.volume.is_none_or(|o| (o - v).abs() > 0.004)
                {
                    self.volume = Some(v);
                    self.dirty = true;
                }
                if v > 0.004 && self.vol_hold.elapsed() > Duration::from_millis(1500) {
                    self.vol_last = v;
                }
            } else if self.volume.is_some() {
                self.volume = None;
                self.dirty = true;
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

        /// A snapshot whose strings changed but that is still the item already playing: the
        /// player's own id is unchanged, or the position runs on where it was (a real track
        /// change starts near 0 or jumps) with the same length.
        fn same_recording(&self, s: &Snapshot) -> bool {
            let Some(o) = &self.snap else { return false };
            if self.track_key == 0 || o.player != s.player {
                return false;
            }
            if !s.track_id.is_empty() && s.track_id == o.track_id {
                return true;
            }
            if !s.url.is_empty() && s.url == o.url {
                return true;
            }
            let same_len = o.length > 0.0 && (o.length - s.length).abs() <= 1.0;
            let runs_on = s.position > 2.0 && (self.position() - s.position).abs() < 1.5;
            same_len && runs_on
        }

        /// Fill what the player left empty from the file's own tags; ask for them once.
        fn merge_local_tags(&mut self, s: &mut Snapshot) {
            let Some(t) = self.local_tags.get(&s.url) else {
                if self.tags_asked.insert(s.url.clone()) {
                    let (url, tx) = (s.url.clone(), self.tx.clone());
                    tokio::task::spawn_blocking(move || {
                        let tags = tags::read(&url);
                        let _ = tx.send(AppEvent::Tags { url, tags });
                    });
                }
                return;
            };
            let fill = |dst: &mut String, src: &str| {
                if dst.is_empty() && !src.is_empty() {
                    *dst = src.to_string();
                }
            };
            // a title that is only the file name gives way to the real one
            if !t.title.is_empty() && s.title == tags::title_from_url(&s.url) {
                s.title = t.title.clone();
            }
            fill(&mut s.artist, &t.artist);
            fill(&mut s.album, &t.album);
            fill(&mut s.content_created, &t.year);
            fill(&mut s.art_url, &t.art_url);
            s.track_no = s.track_no.or(t.track);
            s.disc_no = s.disc_no.or(t.disc);
        }

        fn capo_path(key: u64) -> std::path::PathBuf {
            crate::util::cache_dir().join(format!("capo-{key:016x}.txt"))
        }

        /// Capo chosen for this track in an earlier session (a one-number file).
        async fn load_capo(key: u64) -> Option<u8> {
            tokio::fs::read_to_string(Self::capo_path(key))
                .await
                .ok()?
                .trim()
                .parse()
                .ok()
        }

        /// `[` / `]`: capo ±1 (0..=12) as a per-track override, remembered across restarts.
        /// Landing on the sheet's own capo clears the override.
        fn change_capo(&mut self, d: i32) {
            let Some(sheet_capo) = self.sheet.as_ref().map(|s| s.capo) else {
                return;
            };
            let n = (self.effective_capo() as i32 + d).clamp(0, 12) as u8;
            self.set_capo_override((n != sheet_capo).then_some(n));
        }

        fn set_capo_override(&mut self, o: Option<u8>) {
            self.capo_override = o;
            self.snap_scroll = true;
            let path = Self::capo_path(self.track_key);
            tokio::spawn(async move {
                match o {
                    Some(n) => {
                        let _ = tokio::fs::write(path, n.to_string()).await;
                    }
                    None => {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                }
            });
        }

        fn fetch_chords(&mut self, key: u64, artist: &str, title: &str) {
            self.chords_state = ChordsState::Fetching;
            let dir = self
                .args
                .chords_dir
                .clone()
                .unwrap_or_else(chords::default_dir);
            let (tx, a, t) = (self.tx.clone(), artist.to_string(), title.to_string());
            let (c, use_ug) = (self.client.clone(), self.args.chords_ug);
            self.tasks.push(tokio::spawn(async move {
                let sheet = ug::resolve(c, dir, use_ug, a, t).await;
                let capo = App::load_capo(key).await;
                let _ = tx.send(AppEvent::Chords { key, sheet, capo });
            }));
        }

        fn fetch_members(&mut self, key: u64, artist: &str, title: &str) {
            let (c, tx) = (self.client.clone(), self.tx.clone());
            let (a, t) = (artist.to_string(), title.to_string());
            self.tasks.push(tokio::spawn(async move {
                let parts = genius::fetch(c, a, t).await;
                let _ = tx.send(AppEvent::Members { key, parts });
            }));
        }

        /// Place the Genius members on the lyric lines (cheap; redone when either side changes).
        fn realign_members(&mut self) {
            self.members = match self.lyric_lines() {
                Some(l) if !self.member_src.is_empty() => genius::attribute(&self.member_src, &l),
                _ => Vec::new(),
            };
        }

        fn toggle_member_parts(&mut self) {
            self.args.member_parts = !self.args.member_parts;
            if self.args.member_parts {
                if let Some(s) = &self.snap {
                    let (a, t, key) = (s.artist.clone(), s.title.clone(), self.track_key);
                    self.fetch_members(key, &a, &t);
                }
            } else {
                self.member_src.clear();
                self.members.clear();
            }
            self.dirty = true;
        }

        /// Match the sheet onto the synced lyrics (cheap; redone when either side changes).
        fn realign(&mut self) {
            self.aligned = match (&self.sheet, &self.lyrics) {
                (Some(sh), LyricsState::Ready(Lyrics::Synced(l))) => Some(chords::align(sh, l)),
                _ => None,
            };
        }

        pub fn sheet_info(&self) -> Option<(&str, u8)> {
            self.sheet.as_ref().map(|s| (s.source.as_str(), s.capo))
        }

        /// The capo in effect: the per-track override, else the sheet's own.
        pub fn effective_capo(&self) -> u8 {
            self.capo_override
                .unwrap_or_else(|| self.sheet.as_ref().map_or(0, |s| s.capo))
        }

        /// Semitones to shift the sheet's chord shapes by: a higher capo means lower shapes,
        /// plus the manual `t`/`T` transpose.
        pub fn chord_shift(&self) -> i32 {
            let sheet_capo = self.sheet.as_ref().map_or(0, |s| s.capo);
            chords::capo_shift(sheet_capo, self.effective_capo(), self.transpose)
        }

        /// (sounding key, key of the shapes being shown), from the sheet's written key.
        /// The sheet's key is taken to be the key of its chord shapes, so it sounds
        /// `sheet capo` semitones higher; transposing does not change it.
        pub fn keys(&self) -> Option<(chords::KeyInfo, chords::KeyInfo)> {
            let sh = self.sheet.as_ref()?;
            let k = chords::parse_key(sh.key.as_deref()?)?;
            Some((
                // the song's own key never moves with `t` / `T`; only the chord letters do
                k.shift(sh.capo as i32),
                k.shift(self.chord_shift()),
            ))
        }

        /// The song's original tuning, when the chord sheet states one.
        pub fn tuning(&self) -> Option<&str> {
            self.sheet.as_ref().and_then(|s| s.tuning.as_deref())
        }

        pub fn bpm(&self) -> Option<u32> {
            self.sheet.as_ref().and_then(|s| s.bpm)
        }

        /// Flat or sharp spelling for the chords shown.
        pub fn use_flats(&self) -> bool {
            match self.keys() {
                Some((_, shapes)) => shapes.prefers_flats(),
                None => self.aligned.as_ref().is_some_and(|a| a.flats),
            }
        }

        pub fn set_minimal(&mut self, on: bool) {
            if on == self.minimal {
                return;
            }
            self.minimal = on;
            self.snap_scroll = true;
            self.hits = Hits::default();
            if on {
                // The visualiser isn't shown, so stop its process too.
                self.cava_before_minimal = self.show_cava;
                if self.show_cava {
                    self.set_cava(false);
                }
            } else if self.cava_before_minimal {
                self.set_cava(true);
            }
        }

        /// The lyric lines as plain strings (synced or plain), if there are any.
        pub fn lyric_lines(&self) -> Option<Vec<String>> {
            match &self.lyrics {
                LyricsState::Ready(Lyrics::Synced(l)) => Some(l.iter().map(|(_, s)| s.clone()).collect()),
                LyricsState::Ready(Lyrics::Plain(l)) => Some(l.clone()),
                _ => None,
            }
        }

        /// The translation being shown, when translation is on and it has arrived.
        pub fn translation(&self) -> Option<&Vec<String>> {
            if !self.translate_on {
                return None;
            }
            self.translations
                .get(&(self.track_key, self.args.translate_lang.clone()))
        }

        /// Start fetching the translation for this song unless we have it already.
        pub fn ensure_translation(&mut self) {
            if !self.translate_on || self.translation().is_some() {
                return;
            }
            let Some(lines) = self.lyric_lines().filter(|l| !l.is_empty()) else {
                return;
            };
            let want = (self.track_key, self.args.translate_lang.clone());
            if self.translate_inflight.as_ref() == Some(&want) {
                return;
            }
            self.translate_inflight = Some(want);
            self.translate_state = TranslateState::Loading;
            let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
            let lang = self.args.translate_lang.clone();
            let cfg = translate::Cfg {
                google_base: self.translate_base.clone(),
                fallback: self.args.translate_fallback,
                fallback_url: self.args.translate_fallback_url.clone(),
                fallback_key: self.args.translate_fallback_key.clone(),
            };
            self.tasks.push(tokio::spawn(async move {
                let lines = translate::lines(&c, &cfg, &lang, &lines).await;
                let _ = tx.send(AppEvent::Translated { key, lang, lines });
            }));
        }

        /// Scroll the lyrics away from the playing line without touching playback.
        pub fn browse_by(&mut self, rows: f64) {
            self.browse += rows;
            self.browse_at = Instant::now();
        }

        /// Browsing hands the view back to the playing line after a few idle seconds.
        pub fn browse_expired(&self) -> bool {
            self.browse != 0.0 && self.browse_at.elapsed() > Duration::from_secs(6)
        }

        fn toggle_translate(&mut self) {
            self.translate_on = !self.translate_on;
            self.snap_scroll = true;
            if self.translate_on {
                self.show_lyrics = true;
                self.translate_state = TranslateState::Idle;
                self.ensure_translation();
            }
        }

        // ── settings menu ──────────────────────────────────────────────────
        pub fn setting_value(&self, s: Setting) -> String {
            let onoff = |b: bool| if b { "[x]" } else { "[ ]" }.to_string();
            match s {
                Setting::Player => format!("< {} >", self.args.player),
                Setting::Chords => onoff(self.chords_on),
                Setting::ChordsUg => onoff(self.args.chords_ug),
                Setting::MemberParts => onoff(self.args.member_parts),
                Setting::Translate => onoff(self.translate_on),
                Setting::TranslateLang => format!("< {} >", self.args.translate_lang),
                Setting::TranslateFallback => onoff(self.args.translate_fallback),
                Setting::Minimal => onoff(self.minimal),
                Setting::BigLyrics => onoff(self.big_lyrics),
                Setting::BigScale => format!("< {}x >", self.args.big_scale),
                Setting::LyricSpacing => format!("< {} >", self.args.lyric_spacing),
                Setting::LyricPad => format!("< {} >", self.args.lyric_pad),
                Setting::Cava => onoff(if self.minimal { self.cava_before_minimal } else { self.show_cava }),
                Setting::ThirdPanel => onoff(self.placement == CavaPlacement::ThirdPanel),
                Setting::CoverTheme => onoff(self.args.cover_theme),
                Setting::AsciiCover => onoff(self.args.ascii_cover),
                Setting::Transparent => onoff(self.args.transparent),
                Setting::HideColorWarning => onoff(self.args.hide_color_warning),
                Setting::Mouse => onoff(!self.args.no_mouse),
                Setting::VolumeStep => format!("< {} >", self.vol_step),
            }
        }

        /// Change a setting (`dir` = -1 / +1 for the stepped ones; toggles ignore it) and
        /// write the new values to the config file.
        pub fn change_setting(&mut self, s: Setting, dir: i32) {
            match s {
                Setting::Player => {
                    let mut all: Vec<String> = player::PRESETS.iter().map(|p| p.to_string()).collect();
                    if !all.contains(&self.args.player) {
                        all.push(self.args.player.clone());
                    }
                    let n = all.len() as i32;
                    let cur = all.iter().position(|p| *p == self.args.player).unwrap_or(0) as i32;
                    self.args.player = all[(cur + dir).rem_euclid(n) as usize].clone();
                    self.ctl.set_player(&self.args.player);
                }
                Setting::Chords => self.toggle_chords(),
                Setting::ChordsUg => self.args.chords_ug = !self.args.chords_ug,
                Setting::MemberParts => self.toggle_member_parts(),
                Setting::Translate => self.toggle_translate(),
                Setting::TranslateLang => {
                    let cur = LANGS.iter().position(|l| *l == self.args.translate_lang);
                    let n = LANGS.len() as i32;
                    let i = match cur {
                        Some(i) => (i as i32 + dir).rem_euclid(n) as usize,
                        None if dir < 0 => LANGS.len() - 1,
                        None => 0,
                    };
                    self.args.translate_lang = LANGS[i].to_string();
                    self.translate_state = TranslateState::Idle;
                    self.snap_scroll = true;
                    self.ensure_translation();
                }
                Setting::TranslateFallback => self.args.translate_fallback = !self.args.translate_fallback,
                Setting::Minimal => self.set_minimal(!self.minimal),
                Setting::BigScale => {
                    self.args.big_scale = (self.args.big_scale as i32 + dir).clamp(1, 3) as u16;
                    self.snap_scroll = true;
                }
                Setting::LyricPad => {
                    self.args.lyric_pad = (self.args.lyric_pad as i32 + dir).clamp(0, 10) as u16;
                    self.snap_scroll = true;
                }
                Setting::HideColorWarning => self.args.hide_color_warning = !self.args.hide_color_warning,
                Setting::BigLyrics => {
                    self.big_lyrics = !self.big_lyrics;
                    self.snap_scroll = true;
                }
                Setting::LyricSpacing => {
                    self.args.lyric_spacing = (self.args.lyric_spacing as i32 + dir).clamp(0, 3) as u16;
                    self.snap_scroll = true;
                }
                Setting::Cava => {
                    self.run(Action::ToggleCava);
                }
                Setting::ThirdPanel => {
                    self.placement = match self.placement {
                        CavaPlacement::ThirdPanel => CavaPlacement::UnderLyrics,
                        CavaPlacement::UnderLyrics => CavaPlacement::ThirdPanel,
                    };
                    self.args.thirdpanel = self.placement == CavaPlacement::ThirdPanel;
                    // startup shows cava whenever thirdpanel is set, so keep the two in step
                    if self.args.thirdpanel && !self.minimal && !self.show_cava {
                        self.set_cava(true);
                    }
                }
                Setting::CoverTheme => self.args.cover_theme = !self.args.cover_theme,
                Setting::AsciiCover => self.args.ascii_cover = !self.args.ascii_cover,
                Setting::Transparent => self.args.transparent = !self.args.transparent,
                Setting::Mouse => self.args.no_mouse = !self.args.no_mouse,
                Setting::VolumeStep => {
                    self.vol_step = (self.vol_step as i32 + dir).clamp(1, 50) as u32;
                    self.args.volume_step = self.vol_step;
                }
            }
            self.save_settings(s);
        }

        /// The settings the menu owns, as config-file entries.
        pub fn settings_values(&self) -> Vec<(&'static str, toml::Value)> {
            use toml::Value::{Boolean as B, Integer as I, String as S};
            vec![
                ("player", S(self.args.player.clone())),
                ("chords", B(self.chords_on)),
                ("chords_ug", B(self.args.chords_ug)),
                ("member_parts", B(self.args.member_parts)),
                ("translate", B(self.translate_on)),
                ("translate_lang", S(self.args.translate_lang.clone())),
                ("translate_fallback", B(self.args.translate_fallback)),
                ("big_scale", I(self.args.big_scale as i64)),
                ("lyric_pad", I(self.args.lyric_pad as i64)),
                ("hide_color_warning", B(self.args.hide_color_warning)),
                ("minimal_view", B(self.minimal)),
                ("big_lyrics", B(self.big_lyrics)),
                ("lyric_spacing", I(self.args.lyric_spacing as i64)),
                ("cava", B(if self.minimal { self.cava_before_minimal } else { self.show_cava })),
                // saved off while cava is off: startup would otherwise turn cava back on
                (
                    "thirdpanel",
                    B(self.placement == CavaPlacement::ThirdPanel
                        && (if self.minimal { self.cava_before_minimal } else { self.show_cava })),
                ),
                ("cover_theme", B(self.args.cover_theme)),
                ("ascii_cover", B(self.args.ascii_cover)),
                ("transparent", B(self.args.transparent)),
                ("mouse", B(!self.args.no_mouse)),
                ("volume_step", I(self.vol_step as i64)),
            ]
        }

        /// Write only the setting that changed, so flags given for this run (`--minimal`…)
        /// don't leak into the file.
        fn save_settings(&mut self, s: Setting) {
            let path = self.args.config.clone().unwrap_or_else(config::default_path);
            let keys = s.config_keys();
            let values: Vec<_> = self.settings_values().into_iter().filter(|(k, _)| keys.contains(k)).collect();
            self.settings_note = config::save(&path, &values)
                .err()
                .map(|e| format!("couldn't save {}: {e}", path.display()));
        }

        fn handle_settings_key(&mut self, code: KeyCode) {
            let n = Setting::ALL.len();
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.settings_sel = (self.settings_sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') => self.settings_sel = (self.settings_sel + 1) % n,
                KeyCode::Home => self.settings_sel = 0,
                KeyCode::End => self.settings_sel = n - 1,
                KeyCode::Left | KeyCode::Char('h') => self.change_setting(Setting::ALL[self.settings_sel], -1),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char(' ') => {
                    self.change_setting(Setting::ALL[self.settings_sel], 1)
                }
                KeyCode::Esc | KeyCode::Char('q' | 's' | 'S') => self.show_settings = false,
                _ => {}
            }
        }

        fn toggle_chords(&mut self) {
            self.chords_on = !self.chords_on;
            self.snap_scroll = true;
            if self.chords_on {
                self.show_lyrics = true;
                if matches!(self.chords_state, ChordsState::Idle) {
                    if let Some(s) = &self.snap {
                        let (a, t, key) = (s.artist.clone(), s.title.clone(), self.track_key);
                        self.fetch_chords(key, &a, &t);
                    }
                }
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
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        return true;
                    }
                    if self.show_settings {
                        self.handle_settings_key(k.code);
                        return false;
                    }
                    if self.show_help {
                        // Help is open: arrows / paging scroll it, any other key closes it.
                        match k.code {
                            KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                            KeyCode::Down | KeyCode::Char('j') => self.help_scroll += 1,
                            KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                            KeyCode::PageDown => self.help_scroll += 10,
                            KeyCode::Home => self.help_scroll = 0,
                            _ => self.show_help = false,
                        }
                        return false;
                    }
                    if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                        return false;
                    }
                    if let Some(a) = action_for(k.code) {
                        return self.run(a);
                    }
                }
                Event::Mouse(_) if self.show_settings => {}
                Event::Mouse(m) if self.show_help => match m.kind {
                    MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(2),
                    MouseEventKind::ScrollDown => self.help_scroll += 2,
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
                    } else if self.hits.shuffle.contains(pos) {
                        self.toggle_shuffle();
                    } else if self.hits.repeat.contains(pos) {
                        self.cycle_repeat();
                    } else if self.hits.prev.contains(pos) {
                        self.ctl.send(&["previous"]);
                    } else if self.hits.play.contains(pos) {
                        self.play_pause();
                    } else if self.hits.next.contains(pos) {
                        self.ctl.send(&["next"]);
                    } else if self.hits.vol_minus.contains(pos) {
                        self.volume_step(-1);
                    } else if self.hits.vol_plus.contains(pos) {
                        self.volume_step(1);
                    } else if self.hits.vol_pct.contains(pos) {
                        self.toggle_mute();
                    } else if self.lyric_hits.area.contains(pos) {
                        self.click_lyric(pos.y);
                    }
                }
                Event::Mouse(m)
                    if matches!(m.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
                        && self.lyric_hits.area.contains(Position::new(m.column, m.row)) =>
                {
                    self.browse_by(if m.kind == MouseEventKind::ScrollUp { -3.0 } else { 3.0 });
                }
                Event::Mouse(m)
                    if matches!(m.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
                        && self.hits.vol_area.contains(Position::new(m.column, m.row)) =>
                {
                    self.volume_step(if m.kind == MouseEventKind::ScrollUp { 1 } else { -1 });
                }
                _ => {}
            }
            false
        }

        /// Carry out a key action. Returns `true` to quit.
        pub fn run(&mut self, a: Action) -> bool {
            match a {
                Action::Quit => return true,
                Action::PlayPause => self.play_pause(),
                Action::Next => self.ctl.send(&["next"]),
                Action::Prev => self.ctl.send(&["previous"]),
                Action::Shuffle => self.toggle_shuffle(),
                Action::Repeat => self.cycle_repeat(),
                Action::SeekBack => self.seek_rel(-5.0),
                Action::SeekFwd => self.seek_rel(5.0),
                Action::SyncEarlier => self.nudge(-0.1),
                Action::SyncLater => self.nudge(0.1),
                Action::VolUp => self.volume_step(1),
                Action::VolDown => self.volume_step(-1),
                Action::Mute => self.toggle_mute(),
                Action::ToggleLyrics => {
                    self.show_lyrics = !self.show_lyrics;
                    self.snap_scroll = true;
                }
                Action::ToggleBig => {
                    self.big_lyrics = !self.big_lyrics;
                    self.snap_scroll = true;
                }
                Action::ToggleCava if self.minimal => {
                    // hidden for now; remember the choice for when minimal view ends
                    self.cava_before_minimal = !self.cava_before_minimal;
                }
                Action::ToggleCava => {
                    let on = !self.show_cava;
                    self.set_cava(on);
                }
                Action::ToggleChords => self.toggle_chords(),
                Action::TransposeUp => self.transpose = (self.transpose + 1).rem_euclid(12),
                Action::TransposeDown => self.transpose = (self.transpose - 1).rem_euclid(12),
                Action::CapoDown => self.change_capo(-1),
                Action::CapoUp => self.change_capo(1),
                Action::CapoReset => self.set_capo_override(None),
                Action::Diagrams => {
                    self.show_diagrams = !self.show_diagrams;
                    self.snap_scroll = true;
                }
                Action::Minimal => self.set_minimal(!self.minimal),
                Action::BrowseUp => self.browse_by(-1.0),
                Action::BrowseDown => self.browse_by(1.0),
                Action::BrowsePageUp => self.browse_by(-8.0),
                Action::BrowsePageDown => self.browse_by(8.0),
                Action::BrowseEnd => self.browse = 0.0,
                Action::Translate => self.toggle_translate(),
                Action::Settings => self.show_settings = true,
                Action::Help => {
                    self.show_help = true;
                    self.help_scroll = 0;
                }
            }
            false
        }

        /// Percent steps; clamped to 0..=100.
        pub fn volume_pct(&self) -> Option<u32> {
            self.volume.map(|v| (v * 100.0).round() as u32)
        }

        fn set_volume(&mut self, v: f64) {
            let v = v.clamp(0.0, 1.0);
            self.volume = Some(v);
            if v > 0.004 {
                self.vol_last = v;
            }
            self.vol_hold = Instant::now();
            self.ctl.send(&["volume", &format!("{v:.3}")]);
        }

        pub fn volume_step(&mut self, dir: i32) {
            let Some(pct) = self.volume_pct() else { return };
            let step = self.vol_step as i32;
            let new = (pct as i32 + dir * step).clamp(0, 100);
            self.set_volume(new as f64 / 100.0);
        }

        pub fn toggle_mute(&mut self) {
            let Some(v) = self.volume else { return };
            if v > 0.004 {
                self.vol_last = v;
                self.set_volume(0.0);
            } else {
                self.set_volume(self.vol_last.max(0.05));
            }
        }

        pub fn muted(&self) -> bool {
            self.volume.is_some_and(|v| v <= 0.004)
        }

        /// Seek to the lyric line drawn at terminal row `y` (synced lyrics only).
        fn click_lyric(&mut self, y: u16) {
            let h = &self.lyric_hits;
            if y < h.area.y {
                return;
            }
            let idx = h.top + (y - h.area.y) as i64;
            let Some(Some(li)) = usize::try_from(idx).ok().and_then(|i| h.rows.get(i)).copied()
            else {
                return;
            };
            let LyricsState::Ready(Lyrics::Synced(l)) = &self.lyrics else {
                return;
            };
            let Some((ts, _)) = l.get(li) else { return };
            // The active line is the last one with ts <= position + offset.
            let target = (ts - self.offset).max(0.0);
            self.set_position(target);
            self.ctl.seek_to(target);
        }

        /// Flip shuffle (shown at once; the next poll reports what the player really did).
        fn toggle_shuffle(&mut self) {
            if let Some(s) = self.snap.as_mut().filter(|s| s.shuffle.is_some()) {
                s.shuffle = s.shuffle.map(|b| !b);
                self.ctl.send(&["shuffle", "Toggle"]);
            }
        }

        /// Repeat: off -> playlist -> track -> off.
        fn cycle_repeat(&mut self) {
            if let Some(s) = self.snap.as_mut() {
                if let Some(r) = s.repeat.map(|r| r.next()) {
                    s.repeat = Some(r);
                    self.ctl.send(&["loop", r.arg()]);
                }
            }
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
            (self.lyric_scroll - self.lyric_scroll_target).abs() > 0.02 || self.browse != 0.0
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Big block font for the active lyric line (5×7 pixels, drawn with half blocks)
// ════════════════════════════════════════════════════════════════════════════
mod bigfont {
    /// 5×7 bitmap, one `u8` per row, bit 4 = leftmost pixel.
    fn base_glyph(c: char) -> Option<[u8; 7]> {
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

    /// Accent marks as a top row. The letter's top two rows are merged to make room for it,
    /// so accented capitals stay 7 rows tall.
    fn accented(c: char) -> Option<[u8; 7]> {
        let (base, mark): (char, u8) = match c {
            'Ä' => ('A', 0b01010),
            'Ë' => ('E', 0b01010),
            'Ï' => ('I', 0b01010),
            'Ö' => ('O', 0b01010),
            'Ü' => ('U', 0b01010),
            'Ÿ' => ('Y', 0b01010),
            'Å' => ('A', 0b00100),
            'Á' | 'É' | 'Í' | 'Ó' | 'Ú' | 'Ý' => (
                match c {
                    'Á' => 'A',
                    'É' => 'E',
                    'Í' => 'I',
                    'Ó' => 'O',
                    'Ú' => 'U',
                    _ => 'Y',
                },
                0b00010,
            ),
            'À' | 'È' | 'Ì' | 'Ò' | 'Ù' => (
                match c {
                    'À' => 'A',
                    'È' => 'E',
                    'Ì' => 'I',
                    'Ò' => 'O',
                    _ => 'U',
                },
                0b01000,
            ),
            'Â' | 'Ê' | 'Î' | 'Ô' | 'Û' => (
                match c {
                    'Â' => 'A',
                    'Ê' => 'E',
                    'Î' => 'I',
                    'Ô' => 'O',
                    _ => 'U',
                },
                0b00100,
            ),
            'Ñ' => ('N', 0b01101),
            'Ø' => return Some([0b01110, 0b10011, 0b10101, 0b10101, 0b10101, 0b11001, 0b01110]),
            _ => return None,
        };
        let g = base_glyph(base)?;
        Some([mark, g[0] | g[1], g[2], g[3], g[4], g[5], g[6]])
    }

    /// One drawable character: `w` pixel columns, bit `w - 1 - x` of a row is column `x`.
    #[derive(Clone, Copy)]
    struct Glyph {
        w: usize,
        rows: [u16; 8],
    }

    fn latin(c: char) -> Option<Glyph> {
        let g = base_glyph(c).or_else(|| accented(c))?;
        let mut rows = [0u16; 8];
        for (r, b) in rows.iter_mut().zip(g) {
            *r = b as u16;
        }
        Some(Glyph { w: 5, rows })
    }

    /// Compact 3×5 variant for panels too narrow for the normal font (letters, digits and a
    /// little punctuation; everything else is `None`).
    fn compact_glyph(c: char) -> Option<Glyph> {
        let g: [u8; 5] = match c {
            ' ' => [0; 5],
            'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
            'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
            'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
            'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
            'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
            'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
            'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
            'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
            'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
            'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
            'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
            'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
            'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
            'N' => [0b110, 0b101, 0b101, 0b101, 0b101],
            'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
            'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
            'Q' => [0b010, 0b101, 0b101, 0b110, 0b011],
            'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
            'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
            'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
            'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
            'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
            'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
            'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
            'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
            'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
            '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
            '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
            '2' => [0b110, 0b001, 0b010, 0b100, 0b111],
            '3' => [0b110, 0b001, 0b010, 0b001, 0b110],
            '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
            '5' => [0b111, 0b100, 0b110, 0b001, 0b110],
            '6' => [0b011, 0b100, 0b111, 0b101, 0b111],
            '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
            '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
            '9' => [0b111, 0b101, 0b111, 0b001, 0b110],
            '.' => [0, 0, 0, 0, 0b010],
            ',' => [0, 0, 0, 0b010, 0b100],
            '!' => [0b010, 0b010, 0b010, 0, 0b010],
            '?' => [0b110, 0b001, 0b010, 0, 0b010],
            '\'' => [0b010, 0b010, 0, 0, 0],
            '"' => [0b101, 0b101, 0, 0, 0],
            '-' => [0, 0, 0b111, 0, 0],
            ':' => [0, 0b010, 0, 0b010, 0],
            '(' => [0b010, 0b100, 0b100, 0b100, 0b010],
            ')' => [0b010, 0b001, 0b001, 0b001, 0b010],
            '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
            '+' => [0, 0b010, 0b111, 0b010, 0],
            '=' => [0, 0b111, 0, 0b111, 0],
            '_' => [0, 0, 0, 0, 0b111],
            _ => return None,
        };
        let mut rows = [0u16; 8];
        for (r, b) in rows.iter_mut().zip(g) {
            *r = b as u16;
        }
        Some(Glyph { w: 3, rows })
    }

    // ── Hangul: syllables are composed from a few jamo shapes ───────────────
    type Canvas = [[bool; 8]; 8];

    /// 5×5 shapes of the basic consonants: ㄱ ㄴ ㄷ ㄹ ㅁ ㅂ ㅅ ㅇ ㅈ ㅊ ㅋ ㅌ ㅍ ㅎ.
    const CONS: [[u8; 5]; 14] = [
        [0b11111, 0b00001, 0b00001, 0b00001, 0b00001],
        [0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        [0b11111, 0b10000, 0b10000, 0b10000, 0b11111],
        [0b11111, 0b00001, 0b11111, 0b10000, 0b11111],
        [0b11111, 0b10001, 0b10001, 0b10001, 0b11111],
        [0b10001, 0b10001, 0b11111, 0b10001, 0b11111],
        [0b00100, 0b00100, 0b01010, 0b01010, 0b10001],
        [0b01110, 0b10001, 0b10001, 0b10001, 0b01110],
        [0b11111, 0b00100, 0b01010, 0b01010, 0b10001],
        [0b00100, 0b11111, 0b00100, 0b01010, 0b10001],
        [0b11111, 0b00001, 0b11111, 0b00001, 0b00001],
        [0b11111, 0b10000, 0b11111, 0b10000, 0b11111],
        [0b11111, 0b01010, 0b01010, 0b01010, 0b11111],
        [0b00100, 0b11111, 0b01110, 0b10001, 0b01110],
    ];

    /// Initial consonant index (0–18) → (index into [`CONS`], drawn twice for ㄲ ㄸ ㅃ ㅆ ㅉ).
    fn cons_shape(id: u8) -> (usize, bool) {
        match id {
            0 => (0, false),
            1 => (0, true),
            2 => (1, false),
            3 => (2, false),
            4 => (2, true),
            5 => (3, false),
            6 => (4, false),
            7 => (5, false),
            8 => (5, true),
            9 => (6, false),
            10 => (6, true),
            11 => (7, false),
            12 => (8, false),
            13 => (8, true),
            14 => (9, false),
            15 => (10, false),
            16 => (11, false),
            17 => (12, false),
            _ => (13, false),
        }
    }

    /// Final consonant 1–27 → one or two initial-consonant indices (clusters like ㄳ are two).
    fn final_parts(t: u32) -> (u8, Option<u8>) {
        match t {
            1 => (0, None),
            2 => (1, None),
            3 => (0, Some(9)),
            4 => (2, None),
            5 => (2, Some(12)),
            6 => (2, Some(18)),
            7 => (3, None),
            8 => (5, None),
            9 => (5, Some(0)),
            10 => (5, Some(6)),
            11 => (5, Some(7)),
            12 => (5, Some(9)),
            13 => (5, Some(16)),
            14 => (5, Some(17)),
            15 => (5, Some(18)),
            16 => (6, None),
            17 => (7, None),
            18 => (7, Some(9)),
            19 => (9, None),
            20 => (10, None),
            21 => (11, None),
            22 => (12, None),
            23 => (14, None),
            24 => (15, None),
            25 => (16, None),
            26 => (17, None),
            _ => (18, None),
        }
    }

    /// Draw consonant `id` into the `w`×`h` box at (`x0`, `y0`), resampled from its 5×5 shape.
    fn stamp(cv: &mut Canvas, id: u8, x0: usize, y0: usize, w: usize, h: usize) {
        let (shape, double) = cons_shape(id);
        if double && w >= 2 {
            let w1 = w.div_ceil(2);
            stamp_one(cv, shape, x0, y0, w1, h);
            stamp_one(cv, shape, x0 + w1, y0, w - w1, h);
        } else {
            stamp_one(cv, shape, x0, y0, w, h);
        }
    }

    fn stamp_one(cv: &mut Canvas, shape: usize, x0: usize, y0: usize, w: usize, h: usize) {
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = ((2 * x + 1) * 5 / (2 * w), (2 * y + 1) * 5 / (2 * h));
                if CONS[shape][sy] & (1 << (4 - sx)) != 0 && x0 + x < 8 && y0 + y < 8 {
                    cv[y0 + y][x0 + x] = true;
                }
            }
        }
    }

    /// Vertical vowel (ㅏ ㅐ ㅑ ㅒ ㅓ ㅔ ㅕ ㅖ ㅣ = 0–7, 20) in the `w`×`h` box at (`x0`, `y0`).
    fn vowel_vertical(cv: &mut Canvas, v: u32, x0: usize, w: usize, y0: usize, h: usize) {
        let right = x0 + w - 1;
        let mid = y0 + (h - 1) / 2;
        let ticks: Vec<usize> = match v {
            0 | 1 | 4 | 5 => vec![mid],
            2 | 3 | 6 | 7 => vec![y0 + h / 4, y0 + h - 1 - h / 4],
            _ => vec![],
        };
        let stems: Vec<usize> = match v {
            0 | 2 => vec![x0],
            1 | 3 => vec![x0, right],
            4 | 6 => vec![right],
            5 | 7 => vec![right - 1, right],
            _ => vec![x0 + (w - 1) / 2],
        };
        for &x in &stems {
            for row in cv.iter_mut().skip(y0).take(h) {
                row[x] = true;
            }
        }
        for &y in &ticks {
            let span = match v {
                0 | 2 => x0 + 1..=right,
                1 | 3 => x0 + 1..=right - 1,
                4 | 6 => x0..=right - 1,
                _ => x0..=right - 2,
            };
            for x in span {
                cv[y][x] = true;
            }
        }
    }

    /// Horizontal vowel (ㅗ ㅛ ㅜ ㅠ ㅡ = 8, 12, 13, 17, 18) over columns `xa..=xb`, rows `y0..y0 + h`.
    fn vowel_horizontal(cv: &mut Canvas, v: u32, (xa, xb): (usize, usize), y0: usize, h: usize) {
        let rise = h.saturating_sub(1).min(2);
        let bar = match v {
            8 | 12 => y0 + h - 1,
            13 | 17 => y0,
            _ => y0 + h / 2,
        };
        cv[bar][xa..=xb].fill(true);
        let nubs: Vec<usize> = match v {
            8 | 13 => vec![(xa + xb) / 2],
            12 | 17 => vec![xa + 1, xb - 1],
            _ => vec![],
        };
        for x in nubs {
            for k in 1..=rise {
                let y = if matches!(v, 8 | 12) { bar - k } else { bar + k };
                cv[y][x] = true;
            }
        }
    }

    /// Compose a syllable block (U+AC00–U+D7A3) in an 8×8 pixel cell.
    fn hangul(c: char) -> Option<Glyph> {
        let idx = (c as u32).checked_sub(0xAC00).filter(|i| *i < 11172)?;
        let (l, v, t) = ((idx / 588) as u8, (idx % 588) / 28, idx % 28);
        let mut cv: Canvas = [[false; 8]; 8];
        // with a final consonant the body is squeezed into the top 5 rows
        let body = if t > 0 { 5 } else { 8 };
        match v {
            0..=7 | 20 => {
                stamp(&mut cv, l, 0, 0, 4, body);
                vowel_vertical(&mut cv, v, 5, 3, 0, body);
            }
            8 | 12 | 13 | 17 | 18 => {
                let ih = if t > 0 { 3 } else { 5 };
                stamp(&mut cv, l, 1, 0, 6, ih);
                vowel_horizontal(&mut cv, v, (1, 6), ih, body - ih);
            }
            _ => {
                // compound: horizontal part under the initial, vertical part on the right
                let hh = if t > 0 { 2 } else { 3 };
                stamp(&mut cv, l, 0, 0, 4, body - hh);
                let (hor, ver) = match v {
                    9 => (8, 0),
                    10 => (8, 1),
                    11 => (8, 20),
                    14 => (13, 4),
                    15 => (13, 5),
                    16 => (13, 20),
                    _ => (18, 20),
                };
                vowel_horizontal(&mut cv, hor, (0, 3), body - hh, hh);
                vowel_vertical(&mut cv, ver, 5, 3, 0, body);
            }
        }
        if t > 0 {
            match final_parts(t) {
                (a, None) => stamp(&mut cv, a, 1, 5, 6, 3),
                (a, Some(b)) => {
                    stamp(&mut cv, a, 1, 5, 3, 3);
                    stamp(&mut cv, b, 4, 5, 3, 3);
                }
            }
        }
        let mut rows = [0u16; 8];
        for (r, line) in rows.iter_mut().zip(cv) {
            for (x, on) in line.iter().enumerate() {
                if *on {
                    *r |= 1 << (7 - x);
                }
            }
        }
        Some(Glyph { w: 8, rows })
    }

    fn glyph(c: char, compact: bool) -> Option<Glyph> {
        if compact { compact_glyph(c) } else { latin(c).or_else(|| hangul(c)) }
    }

    /// Upper-case; `None` if the font can't draw the character (CJK ideographs, Arabic…
    /// make the caller fall back to normal text).
    fn fold(c: char, compact: bool) -> Option<char> {
        let c = match c {
            '’' | '‘' | '`' | '´' => '\'',
            '“' | '”' => '"',
            '–' | '—' => '-',
            c if c.is_ascii() => c.to_ascii_uppercase(),
            c => {
                let mut up = c.to_uppercase();
                match (up.next(), up.next()) {
                    (Some(u), None) => u, // `ß` → "SS" is not one letter: leave it undrawable
                    _ => c,
                }
            }
        };
        glyph(c, compact).map(|_| c)
    }

    /// Columns used by one glyph incl. the 1-column gap after it: Latin 6, Hangul 9; a space
    /// is 3 columns (2 in the compact font).
    fn advance(c: char, compact: bool) -> usize {
        if c == ' ' {
            if compact { 2 } else { 3 }
        } else {
            glyph(c, compact).map_or(6, |g| g.w + 1)
        }
    }

    /// Render `text` as rows of half-block characters, at most `max_cols` wide and
    /// `max_chunks` wrapped pieces tall (each piece = 4 text rows, pieces separated by one
    /// blank row). `None` when it can't be drawn — unsupported characters, a word wider than
    /// the panel, or too many pieces — so the caller can fall back to normal text.
    #[cfg(test)]
    pub fn render(text: &str, max_cols: usize, max_chunks: usize) -> Option<Vec<String>> {
        render_scaled(text, max_cols, max_chunks, 1)
    }

    /// [`render`] with every pixel drawn `scale` (1–3) times as wide and tall; a piece is
    /// then `4 * scale` rows tall.
    pub fn render_scaled(text: &str, max_cols: usize, max_chunks: usize, scale: usize) -> Option<Vec<String>> {
        render_with(text, max_cols, max_chunks, scale, false)
    }

    /// The compact 3×5 font (Latin letters and digits only): a piece is 3 rows tall.
    pub fn render_compact(text: &str, max_cols: usize, max_chunks: usize) -> Option<Vec<String>> {
        render_with(text, max_cols, max_chunks, 1, true)
    }

    fn render_with(text: &str, max_cols: usize, max_chunks: usize, scale: usize, compact: bool) -> Option<Vec<String>> {
        let scale = if compact { 1 } else { scale.clamp(1, 3) };
        let adv = |c: char| advance(c, compact) * scale;
        let text = text.replace('…', "...");
        let mut words: Vec<Vec<char>> = Vec::new();
        for w in text.split_whitespace() {
            words.push(w.chars().map(|c| fold(c, compact)).collect::<Option<Vec<_>>>()?);
        }
        if words.is_empty() {
            return None;
        }

        // Greedy wrap by rendered width.
        let mut chunks: Vec<Vec<char>> = Vec::new();
        let mut cur: Vec<char> = Vec::new();
        let mut cur_w = 0usize; // columns incl. the trailing gap
        for w in words {
            let ww: usize = w.iter().map(|&c| adv(c)).sum();
            if ww - 1 > max_cols {
                return None;
            }
            if !cur.is_empty() && cur_w + adv(' ') + ww - 1 > max_cols {
                chunks.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += adv(' ');
            }
            cur_w += ww;
            cur.extend(w);
        }
        chunks.push(cur);
        if chunks.len() > max_chunks {
            return None;
        }

        // pixel rows per piece: 8 (7 + 1 blank) or 6 (5 + 1 blank), so they pair up into
        // half-block rows
        let grid_h = if compact { 6 } else { 8 };
        let mut rows = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            if i > 0 {
                rows.push(String::new());
            }
            let mut px: Vec<Vec<bool>> = vec![Vec::new(); grid_h];
            for &c in chunk {
                if c == ' ' {
                    for row in px.iter_mut() {
                        row.extend(std::iter::repeat_n(false, advance(' ', compact)));
                    }
                    continue;
                }
                let g = glyph(c, compact).unwrap_or(Glyph { w: 5, rows: [0; 8] });
                for (y, row) in px.iter_mut().enumerate() {
                    for x in 0..g.w {
                        row.push(g.rows[y] & (1 << (g.w - 1 - x)) != 0);
                    }
                    row.push(false);
                }
            }
            for row in px.iter_mut() {
                row.pop(); // trailing gap
            }
            // each pixel becomes a scale × scale block
            if scale > 1 {
                px = px
                    .iter()
                    .flat_map(|row| {
                        let wide: Vec<bool> = row.iter().flat_map(|&b| std::iter::repeat_n(b, scale)).collect();
                        std::iter::repeat_n(wide, scale)
                    })
                    .collect();
            }
            for pair in 0..grid_h / 2 * scale {
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
    use crate::{bigfont, chords, cover, diagram};
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
        draw_main(f, app);
        if app.show_help {
            draw_help(f, app);
        }
        if app.show_settings {
            draw_settings(f, app);
        }
    }

    fn draw_settings(f: &mut Frame, app: &mut App) {
        use crate::app::Setting;
        use ratatui::widgets::Clear;
        let area = f.area();
        let th = app.theme.clone();
        let rows = Setting::ALL.len() as u16;
        let w = area.width.min(54);
        let h = area.height.min(rows + 5);
        let r = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        f.render_widget(Clear, r);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(th.primary()))
            .title(Span::styled(
                " settings ",
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            ))
            .style(Style::new().bg(th.surface()).fg(th.text()));
        let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
        f.render_widget(block, r);
        let view_h = inner.height.saturating_sub(1) as usize;
        // keep the selected row in view on short terminals
        let sel = app.settings_sel;
        if sel < app.settings_top {
            app.settings_top = sel;
        } else if sel >= app.settings_top + view_h.max(1) {
            app.settings_top = sel + 1 - view_h.max(1);
        }
        let first = app.settings_top;
        let lines: Vec<Line> = Setting::ALL
            .iter()
            .enumerate()
            .skip(first)
            .take(view_h)
            .map(|(i, s)| {
                let sel = i == app.settings_sel;
                let style = if sel {
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(th.text())
                };
                let mut label = s.label().to_string();
                if s.needs_restart() {
                    label.push_str(" *");
                }
                let value = app.setting_value(*s);
                // on a narrow terminal shorten the label so the value stays visible
                let room = (inner.width as usize).saturating_sub(crate::util::width(&value) + 4);
                while crate::util::width(&label) > room && label.pop().is_some() {}
                let pad = (inner.width as usize)
                    .saturating_sub(crate::util::width(&label) + crate::util::width(&value) + 2);
                Line::from(vec![
                    Span::styled(if sel { "▸ " } else { "  " }, style),
                    Span::styled(label, style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(value, style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines), Rect { height: view_h as u16, ..inner });
        if inner.height == 0 {
            return;
        }
        let hint = match &app.settings_note {
            Some(n) => n.chars().take(inner.width as usize).collect(),
            None => "↑/↓ select · space/←/→ change · * restart · esc".to_string(),
        };
        f.render_widget(
            Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.45)))).alignment(Alignment::Center),
            Rect {
                y: inner.bottom().saturating_sub(1),
                height: 1,
                ..inner
            },
        );
    }

    /// Lines of the help overlay, generated from the key table the input handler uses.
    /// Returns the groups as separate blocks so they can be laid out in columns.
    pub fn help_blocks(th: &Theme) -> Vec<Vec<Line<'static>>> {
        use crate::app::{BINDS, GROUPS, MOUSE};
        let head = |g: &str| {
            Line::styled(
                g.to_uppercase(),
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            )
        };
        let row = |k: &str, d: &str| {
            Line::from(vec![
                Span::styled(format!("{k:<14}"), Style::new().fg(th.primary())),
                Span::styled(d.to_string(), Style::new().fg(th.text())),
            ])
        };
        GROUPS
            .iter()
            .map(|g| {
                let mut v = vec![head(g)];
                v.extend(BINDS.iter().filter(|b| b.group == *g).map(|b| row(b.keys, b.desc)));
                v.extend(MOUSE.iter().filter(|m| m.0 == *g).map(|m| row(m.1, m.2)));
                v
            })
            .collect()
    }

    fn draw_help(f: &mut Frame, app: &mut App) {
        use ratatui::widgets::Clear;
        let area = f.area();
        let th = app.theme.clone();
        let blocks = help_blocks(&th);
        // Two columns (Playback+Lyrics | Chords+View) when there is room, else one list.
        let two = area.width >= 78;
        let cols: Vec<Vec<Line>> = if two {
            let join = |a: &[Vec<Line<'static>>]| {
                let mut out: Vec<Line> = Vec::new();
                for (i, b) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(Line::raw(""));
                    }
                    out.extend(b.iter().cloned());
                }
                out
            };
            vec![join(&blocks[..2]), join(&blocks[2..])]
        } else {
            let mut all: Vec<Line> = Vec::new();
            for (i, b) in blocks.iter().enumerate() {
                if i > 0 {
                    all.push(Line::raw(""));
                }
                all.extend(b.iter().cloned());
            }
            vec![all]
        };
        let content_h = cols.iter().map(Vec::len).max().unwrap_or(0) as u16;
        let w = area.width.min(if two { 80 } else { 44 });
        let h = area.height.min(content_h + 4); // borders + footer line
        let r = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };
        f.render_widget(Clear, r);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(th.primary()))
            .title(Span::styled(
                " help ",
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            ))
            .style(Style::new().bg(th.surface()).fg(th.text()));
        let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
        f.render_widget(block, r);
        let view_h = inner.height.saturating_sub(1);
        let max_scroll = content_h.saturating_sub(view_h);
        app.help_scroll = app.help_scroll.min(max_scroll);
        let n = cols.len() as u16;
        let col_w = inner.width / n;
        for (i, lines) in cols.into_iter().enumerate() {
            f.render_widget(
                Paragraph::new(lines).scroll((app.help_scroll, 0)),
                Rect {
                    x: inner.x + i as u16 * col_w,
                    y: inner.y,
                    width: col_w.saturating_sub(1),
                    height: view_h,
                },
            );
        }
        let hint = if max_scroll > 0 {
            "↑/↓ scroll · any other key closes"
        } else {
            "any key closes"
        };
        f.render_widget(
            Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.45)))).alignment(Alignment::Center),
            Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
        );
    }

    fn draw_main(f: &mut Frame, app: &mut App) {
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
        app.lyric_hits = Default::default();

        if app.minimal {
            draw_minimal(f, app, main);
            draw_footer(f, app, footer);
            return;
        }

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
            draw_lyrics_and_diagrams(f, app, r);
        }
        if let Some(r) = cav {
            draw_cava(f, app, r);
        }
        draw_footer(f, app, footer);
    }

    /// One-row progress bar: ━ filled, ─ remaining, ● knob.
    /// `0:42 ━━━●─── 3:10`: both timestamps sit inside the row, the bar between them.
    /// Returns the bar part (the click target for seeking).
    fn timed_bar(f: &mut Frame, th: &Theme, row: Rect, pos: f64, length: f64) -> Rect {
        let (l, r) = (fmt_time(pos), fmt_time(length));
        let side = (l.chars().count().max(r.chars().count()) + 1) as u16;
        if row.width <= side * 2 + 4 {
            let bar = row;
            progress_bar(f, th, bar, if length > 0.0 { (pos / length).clamp(0.0, 1.0) } else { 0.0 });
            return bar;
        }
        let bar = Rect { x: row.x + side, width: row.width - side * 2, ..row };
        f.render_widget(
            Paragraph::new(Span::styled(l, Style::new().fg(th.fade(0.4)))),
            Rect { width: side, ..row },
        );
        f.render_widget(
            Paragraph::new(Span::styled(r, Style::new().fg(th.fade(0.4)))).alignment(Alignment::Right),
            Rect { x: row.right() - side, width: side, ..row },
        );
        progress_bar(f, th, bar, if length > 0.0 { (pos / length).clamp(0.0, 1.0) } else { 0.0 });
        bar
    }

    fn progress_bar(f: &mut Frame, th: &Theme, r: Rect, frac: f64) {
        let w = r.width as usize;
        if w == 0 {
            return;
        }
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
            if let Some(c) = buf.cell_mut((r.x + i as u16, r.y)) {
                c.set_symbol(sym).set_fg(fg);
            }
        }
    }

    /// `M`: title, lyrics across the whole area, and a thin progress bar. No cover,
    /// metadata, buttons or visualiser.
    fn draw_minimal(f: &mut Frame, app: &mut App, main: Rect) {
        app.hits = Default::default();
        let th = app.theme.clone();
        let [title, body, prog] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(main);
        if let Some(s) = app.snap.clone() {
            let text = if s.artist.is_empty() {
                s.title.clone()
            } else {
                format!("{} — {}", s.artist, s.title)
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("◆ ", Style::new().fg(th.primary())),
                    Span::styled(text, Style::new().fg(th.text()).add_modifier(Modifier::BOLD)),
                ]))
                .alignment(Alignment::Center),
                title,
            );
            app.hits.progress = timed_bar(f, &th, prog, app.position(), s.length);
        }
        draw_lyrics_and_diagrams(f, app, body);
    }

    /// Minimum width of the lyrics column for the diagram panel to be worth showing.
    const DIAGRAM_MIN_WIDTH: u16 = 56;
    const DIAGRAM_PANEL_WIDTH: u16 = 28;

    fn draw_lyrics_and_diagrams(f: &mut Frame, app: &mut App, area: Rect) {
        if app.show_diagrams && area.width >= DIAGRAM_MIN_WIDTH {
            let [l, d] = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(DIAGRAM_PANEL_WIDTH),
            ])
            .areas(area);
            draw_lyrics(f, app, l);
            draw_diagrams(f, app, d);
        } else {
            draw_lyrics(f, app, area);
        }
    }

    /// Chords of the current stretch of the song, in order of first appearance (max 6), plus
    /// which one is being played. The active chord is estimated from how far through the
    /// line we are, since the sheet carries no timing of its own.
    fn section_chords(app: &App) -> (Vec<String>, Option<usize>) {
        let Some(a) = app.aligned.as_ref().filter(|a| a.usable()) else {
            return (vec![], None);
        };
        let LyricsState::Ready(Lyrics::Synced(l)) = &app.lyrics else {
            return (vec![], None);
        };
        let (tr, flats) = (app.chord_shift(), app.use_flats());
        let show = |n: &str| chords::transpose(n, tr, flats);
        let t = app.position() + app.offset;
        let cur = l.partition_point(|(ts, _)| *ts <= t).checked_sub(1);
        let mut names: Vec<String> = Vec::new();
        let push = |n: String, names: &mut Vec<String>| {
            if !names.contains(&n) && names.len() < 6 {
                names.push(n);
            }
        };
        match cur {
            None => {
                for n in a.intro.iter().flatten() {
                    push(show(n), &mut names);
                }
                for cs in a.per_line.iter().take(2) {
                    for (_, n) in cs {
                        push(show(n), &mut names);
                    }
                }
                let act = names.first().map(|_| 0);
                (names, act)
            }
            Some(c) => {
                for cs in a.per_line.iter().skip(c).take(4) {
                    for (_, n) in cs {
                        push(show(n), &mut names);
                    }
                }
                // which chord of the active line are we on?
                let cs = a.per_line.get(c).filter(|v| !v.is_empty());
                let active = cs.and_then(|cs| {
                    let next = l.get(c + 1).map_or(l[c].0 + 6.0, |x| x.0);
                    let dur = (next - l[c].0).min(est_sung(&l[c].1)).max(1.0);
                    let f = ((t - l[c].0) / dur).clamp(0.0, 0.999);
                    let len = l[c].1.split_whitespace().collect::<Vec<_>>().join(" ").chars().count().max(1);
                    let k = cs
                        .iter()
                        .rposition(|(col, _)| *col as f64 / len as f64 <= f)
                        .unwrap_or(0);
                    names.iter().position(|n| *n == show(&cs[k].1))
                });
                (names, active)
            }
        }
    }

    fn draw_diagrams(f: &mut Frame, app: &mut App, area: Rect) {
        let title = match app.tuning() {
            Some(t) if app.chords_on => format!("chords · {t}"),
            _ => "chords".to_string(),
        };
        let block = panel(app, &title);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let th = app.theme.clone();
        let dim = Style::new().fg(th.fade(0.5));
        if !app.chords_on {
            return centered_msg(f, inner, &[("chords are off", dim), ("press g", dim)]);
        }
        let (names, active) = section_chords(app);
        if names.is_empty() {
            return centered_msg(f, inner, &[("no chords here", dim)]);
        }
        const CELL_W: u16 = 13;
        const CELL_H: u16 = 8;
        let cols = (inner.width / CELL_W).max(1) as usize;
        let shapes: Vec<Option<diagram::Shape>> = names.iter().map(|n| diagram::lookup(n)).collect();
        // leave room for the tab underneath when there is any
        let tab_h = 7u16;
        let grid_rows_fit = (inner.height / CELL_H).max(1) as usize;
        let want_rows = names.len().div_ceil(cols);
        let rows = want_rows.min(grid_rows_fit);
        let show_tab = inner.height >= rows as u16 * CELL_H + tab_h;
        for (i, (name, shape)) in names.iter().zip(&shapes).enumerate() {
            if i / cols >= rows {
                break;
            }
            let r = Rect {
                x: inner.x + (i % cols) as u16 * CELL_W + 1,
                y: inner.y + (i / cols) as u16 * CELL_H,
                width: CELL_W - 1,
                height: CELL_H - 1,
            }
            .intersection(inner);
            let is_active = active == Some(i);
            let body = if is_active { th.primary() } else { th.fade(0.35) };
            let name_style = if is_active {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(th.text())
            };
            let lines: Vec<Line> = match shape {
                Some(sh) => diagram::render(name, sh)
                    .into_iter()
                    .enumerate()
                    .map(|(k, l)| Line::styled(l, if k == 0 { name_style } else { Style::new().fg(body) }))
                    .collect(),
                None => vec![
                    Line::styled(format!("{name:^11}"), name_style),
                    Line::styled(" diagram", dim),
                    Line::styled(" unavailable", dim),
                ],
            };
            f.render_widget(Paragraph::new(lines), r);
        }
        if show_tab {
            let max_cols = ((inner.width.saturating_sub(3)) / 4).max(1) as usize;
            let tab = diagram::tab(&shapes[..shapes.len().min(max_cols)]);
            let y0 = inner.y + rows as u16 * CELL_H;
            let lines: Vec<Line> = tab.into_iter().map(|l| Line::styled(l, Style::new().fg(th.fade(0.2)))).collect();
            f.render_widget(
                Paragraph::new(lines),
                Rect { x: inner.x + 1, y: y0, width: inner.width.saturating_sub(1), height: 6 },
            );
        }
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
    /// The metadata block: Track / Artist / Album, then Track No and Release on the left and
    /// Tuning, Capo, BPM, Key one per row in a right-hand column (see [`meta_grid`]). Those
    /// four are hidden when no source provides them.
    fn meta_lines(app: &App, snap: &crate::player::Snapshot, avail: usize) -> Vec<Line<'static>> {
        let th = &app.theme;
        let label = |s: &str| {
            Span::styled(
                format!("{s:<13}"),
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
        let date = crate::year::full_date(&snap.content_created).unwrap_or_else(|| match &app.year {
            YearState::Loading => "...".to_string(),
            YearState::Known(y) => y.clone(),
            YearState::NotFound => "N/A".to_string(),
        });

        let tag = |name: &str, v: String| {
            vec![
                Span::styled(
                    format!("{name} "),
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
                ),
                val(v),
            ]
        };

        let mut out = vec![
            Line::from(vec![label("Track"), val(dash(&snap.title))]),
            Line::from(vec![label("Artist"), val(dash(&snap.artist))]),
            Line::from(vec![label("Album"), val(dash(&snap.album))]),
            Line::from(vec![label("Track No"), val(track_no)]),
        ];
        let capo = app.effective_capo();
        let capo_tag = if app.chords_on && capo > 0 {
            let mut v = format!("{capo}");
            if let Some((_, shapes)) = app.keys() {
                v.push_str(&format!(" ({})", shapes.short()));
            }
            if app.capo_override.is_some() {
                let sheet_capo = app.sheet_info().map_or(0, |i| i.1);
                v.push_str(&format!(" [sheet {sheet_capo}]"));
            }
            Some(tag("Capo:", v))
        } else if app.chords_on && app.capo_override == Some(0) {
            Some(tag("Capo:", "none".into()))
        } else {
            None
        };
        let tuning = (app.chords_on || app.show_diagrams)
            .then(|| app.tuning().map(|t| tag("Tuning:", t.to_string())))
            .flatten();
        let keytxt = app
            .keys()
            .map(|(sounding, _)| tag("Key:", format!("{} / {}", sounding.long(), sounding.camelot())));
        let bpm = app.bpm().map(|b| tag("BPM:", b.to_string()));
        let last = out.pop().unwrap_or_default();
        let left = vec![last.spans, vec![label("Release"), val(date)]];
        let right: Vec<_> = [tuning, capo_tag, bpm, keytxt].into_iter().flatten().collect();
        out.extend(meta_grid(left, right, avail));
        out
    }

    /// Two columns: `left` rows at the start, `right` rows left-aligned at one shared column
    /// (`avail` minus the widest right item). Right items beyond the left rows get an empty
    /// left side. When the columns don't fit side by side, the right items follow below,
    /// indented under the values.
    pub fn meta_grid(
        left: Vec<Vec<Span<'static>>>,
        right: Vec<Vec<Span<'static>>>,
        avail: usize,
    ) -> Vec<Line<'static>> {
        let w = |v: &[Span]| v.iter().map(|s| s.width()).sum::<usize>();
        let left_w = left.iter().map(|v| w(v)).max().unwrap_or(0);
        let right_w = right.iter().map(|v| w(v)).max().unwrap_or(0);
        let mut out = Vec::new();
        if right.is_empty() || left_w + 2 + right_w > avail {
            out.extend(left.into_iter().map(Line::from));
            for r in right {
                let mut row = vec![Span::raw(" ".repeat(13))];
                row.extend(r);
                out.push(Line::from(row));
            }
            return out;
        }
        let start = avail - right_w;
        let mut left = left.into_iter();
        let mut right = right.into_iter();
        loop {
            let (l, r) = (left.next(), right.next());
            if l.is_none() && r.is_none() {
                break;
            }
            let mut row = l.unwrap_or_default();
            if let Some(r) = r {
                let pad = start.saturating_sub(w(&row));
                row.push(Span::raw(" ".repeat(pad)));
                row.extend(r);
            }
            out.push(Line::from(row));
        }
        out
    }

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

        let pos = app.position();
        let meta_lines = meta_lines(app, &snap, inner.width.saturating_sub(2) as usize);

        // Volume sits beside the transport buttons when it fits, else on its own row below.
        let width = inner.width.saturating_sub(2);
        let vol_shown = app.volume.is_some();
        let pct_txt = if app.muted() {
            "[ MUTE ]".to_string()
        } else {
            format!("[ {}% ]", app.volume_pct().unwrap_or(0))
        };
        let vol_w = 5 + 1 + 8 + 1 + 5; // "[ - ] [ 100% ] [ + ]"
        let rep_lbl = match snap.repeat {
            Some(crate::player::Loop::Track) => "[ rep 1 ]",
            _ => "[ rep ]",
        };
        // shuffle / repeat buttons exist only for what the player reports
        let mut labels: Vec<(&str, u8)> = Vec::new();
        if snap.shuffle.is_some() {
            labels.push(("[ shuf ]", 0));
        }
        if snap.repeat.is_some() {
            labels.push((rep_lbl, 1));
        }
        let play_lbl = if snap.status == Status::Playing {
            "[ pause ]"
        } else {
            "[ play ]"
        };
        labels.extend([("[ prev ]", 2), (play_lbl, 3), ("[ next ]", 4)]);
        let gap = 2u16;
        let total: u16 = labels.iter().map(|l| l.0.chars().count() as u16).sum::<u16>()
            + gap * (labels.len() as u16 - 1);
        let vol_inline = vol_shown && total as usize + 3 + vol_w <= width as usize;
        let vol_row = vol_shown && !vol_inline;
        let [cover_a, _, meta_a, _, prog_a, _, btn_a, vol_a] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(meta_lines.len() as u16),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(vol_row as u16),
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
        f.render_widget(Paragraph::new(meta_lines), meta_a);

        // Progress bar
        app.hits.progress = timed_bar(f, &th, prog_a, pos, snap.length);

        // Buttons
        let full = if vol_inline { total + 3 + vol_w as u16 } else { total };
        let mut x = btn_a.x + btn_a.width.saturating_sub(full) / 2;
        let transport_start = x;
        app.hits.shuffle = Rect::default();
        app.hits.repeat = Rect::default();
        for (l, id) in &labels {
            let w = l.chars().count() as u16;
            let r = Rect {
                x,
                y: btn_a.y,
                width: w.min(btn_a.right().saturating_sub(x)),
                height: 1,
            };
            let on = match id {
                0 => snap.shuffle == Some(true),
                1 => snap.repeat.is_some_and(|r| r != crate::player::Loop::None),
                _ => false,
            };
            let style = if *id == 3 || on {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else if *id < 2 {
                Style::new().fg(th.fade(0.5))
            } else {
                Style::new().fg(th.text())
            };
            f.render_widget(Paragraph::new(Span::styled(*l, style)), r);
            match id {
                0 => app.hits.shuffle = r,
                1 => app.hits.repeat = r,
                2 => app.hits.prev = r,
                3 => app.hits.play = r,
                _ => app.hits.next = r,
            }
            x = x.saturating_add(w + gap);
        }

        // Volume: [ - ] [ 100% ] [ + ]   (click the percentage to mute)
        app.hits.vol_minus = Rect::default();
        app.hits.vol_pct = Rect::default();
        app.hits.vol_plus = Rect::default();
        app.hits.vol_area = Rect::default();
        if vol_shown && (vol_inline || vol_a.height > 0) {
            let (row, x0) = if vol_inline {
                (btn_a, transport_start + total + 3)
            } else {
                (vol_a, vol_a.x + vol_a.width.saturating_sub(vol_w as u16) / 2)
            };
            let muted = app.muted();
            let pct_w = 8u16;
            let cell = |x: u16, w: u16| Rect {
                x,
                y: row.y,
                width: w.min(row.right().saturating_sub(x)),
                height: 1,
            };
            let (minus, pct, plus) = (cell(x0, 5), cell(x0 + 6, pct_w), cell(x0 + 6 + pct_w + 1, 5));
            let btn = Style::new().fg(th.text());
            let pct_style = if muted {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)
            } else {
                Style::new().fg(th.primary())
            };
            f.render_widget(Paragraph::new(Span::styled("[ - ]", btn)), minus);
            f.render_widget(
                Paragraph::new(Span::styled(format!("{pct_txt:^8}"), pct_style)),
                pct,
            );
            f.render_widget(Paragraph::new(Span::styled("[ + ]", btn)), plus);
            app.hits.vol_minus = minus;
            app.hits.vol_pct = pct;
            app.hits.vol_plus = plus;
            app.hits.vol_area = Rect {
                x: minus.x,
                y: row.y,
                width: plus.right().saturating_sub(minus.x),
                height: 1,
            };
        }
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
    /// Closing punctuation (CJK and Latin) that must not start a wrapped line.
    fn no_line_start(c: char) -> bool {
        matches!(
            c,
            '，' | '。' | '、' | '！' | '？' | '；' | '：' | '）' | '」' | '』' | '】' | '》' | '〉' | '”' | '’'
                | '…' | 'ー' | ',' | '.' | '!' | '?' | ';' | ':' | ')' | ']' | '}'
        )
    }

    /// Greedy word-wrap by display width.
    pub fn wrap(text: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![];
        }
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut cur_w = 0;
        for word in text.split_whitespace() {
            let ww = crate::util::width(word);
            if cur_w > 0 && cur_w + 1 + ww > width {
                out.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if ww > width {
                // hard-break very long tokens
                for ch in word.chars() {
                    let cw = crate::util::cw(ch);
                    if cur_w > 0 && cur_w + cw > width {
                        // closing punctuation never starts a line: carry the last char over
                        let carry = if no_line_start(ch) && cur.chars().count() > 1 { cur.pop() } else { None };
                        out.push(std::mem::take(&mut cur));
                        cur_w = 0;
                        if let Some(p) = carry {
                            cur.push(p);
                            cur_w = crate::util::cw(p);
                        }
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

    /// Same wrapping as [`wrap`] but returns `(start, end)` char ranges into `norm` (the
    /// single-spaced text), so chord columns can be mapped onto the wrapped rows.
    pub fn wrap_ranges(norm: &str, width: usize) -> Vec<(usize, usize)> {
        if width == 0 {
            return vec![];
        }
        let chars: Vec<char> = norm.chars().collect();
        let cw = crate::util::cw;
        let mut rows: Vec<(usize, usize)> = Vec::new();
        let (mut start, mut end, mut cur_w) = (0usize, 0usize, 0usize);
        for (s, len) in chords::words_pos(norm) {
            let ww: usize = chars[s..s + len].iter().map(|c| cw(*c)).sum();
            if cur_w > 0 && cur_w + 1 + ww > width {
                rows.push((start, end));
                cur_w = 0;
            }
            if ww > width {
                for (k, c) in chars[s..s + len].iter().enumerate() {
                    if cur_w + cw(*c) > width && cur_w > 0 {
                        if no_line_start(*c) && end > start + 1 {
                            rows.push((start, end - 1));
                            start = end - 1;
                            cur_w = cw(chars[end - 1]);
                        } else {
                            rows.push((start, end));
                            cur_w = 0;
                        }
                    }
                    if cur_w == 0 {
                        start = s + k;
                    }
                    end = s + k + 1;
                    cur_w += cw(*c);
                }
                continue;
            }
            if cur_w == 0 {
                start = s;
            }
            end = s + len;
            cur_w += if cur_w > 0 { 1 + ww } else { ww };
        }
        if cur_w > 0 || rows.is_empty() {
            rows.push((start, end));
        }
        rows
    }

    /// `Am   G   C` with each chord at its column; neighbours are pushed apart, never overlap.
    pub fn chord_line(cs: &[(usize, String)], text: &[char], limit: usize, n: i32, flats: bool) -> String {
        // chord columns are char indices into `text`; place them by terminal cell width
        let cell = |col: usize| -> usize {
            text.iter().take(col).map(|c| crate::util::cw(*c)).sum::<usize>()
                + col.saturating_sub(text.len())
        };
        let mut s = String::new();
        let mut w = 0usize;
        for (col, name) in cs {
            let name = chords::transpose(name, n, flats);
            let floor = if w > 0 { w + 1 } else { 0 };
            // a chord that would run past the panel edge is pulled left on its own,
            // instead of the whole row shifting off its words
            let at = cell(*col).min(limit.saturating_sub(crate::util::width(&name))).max(floor);
            s.push_str(&" ".repeat(at - w));
            w = at + crate::util::width(&name);
            s.push_str(&name);
        }
        s
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

    /// Rough time a line takes to sing, so the marker appears once the words are over
    /// rather than the moment the line starts.
    fn est_sung(text: &str) -> f64 {
        (text.chars().count() as f64 * 0.08).clamp(2.0, 6.0)
    }

    /// Style for a non-active lyric line `dist` lines away from the active one:
    /// fades with distance, and past lines fade a little faster than upcoming ones.
    fn fade_style(th: &Theme, dist: usize, past: bool) -> Style {
        let t = 0.40 + 0.08 * dist as f32 + if past { 0.10 } else { 0.0 };
        Style::new().fg(th.fade(t.min(0.78)))
    }

    /// Wrapped rows of a lyric line, each with the chord row (left offset, text) that sits
    /// right above it — the same mapping the scrolling layout uses. Lines without chords
    /// just get plain wrapped rows.
    fn chord_rows(line: &str, cs: &[(usize, String)], width: usize, tr: i32, flats: bool) -> Vec<(String, Option<(usize, String)>)> {
        if cs.is_empty() {
            return wrap(line, width).into_iter().map(|t| (t, None)).collect();
        }
        let norm = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let chars: Vec<char> = norm.chars().collect();
        let ranges = wrap_ranges(&norm, width);
        let mut per_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); ranges.len()];
        for (c, name) in cs {
            let r = ranges.iter().rposition(|(s, _)| *s <= *c).unwrap_or(0);
            per_row[r].push((c.saturating_sub(ranges[r].0), name.clone()));
        }
        ranges
            .iter()
            .zip(per_row)
            .map(|((rs, re), here)| {
                let text: String = chars[*rs..*re].iter().collect();
                let chord = (!here.is_empty()).then(|| {
                    let x = width.saturating_sub(crate::util::width(&text)) / 2;
                    (x, chord_line(&here, &chars[*rs..*re], width.saturating_sub(x), tr, flats))
                });
                (text, chord)
            })
            .collect()
    }

    /// Big-font rows for `text` in a panel `width` × `height`: the largest scale
    /// (down to one) whose wrapped pieces (up to 4) leave at least 2 rows for the neighbouring lines;
    /// failing that the compact 3×5 font. `None`: the caller falls back to bold wrapped text.
    pub fn big_stage(text: &str, width: usize, height: u16, scale: usize) -> Option<Vec<String>> {
        let fits = |rows: &Vec<String>| rows.len() as u16 + 2 <= height;
        (1..=scale.clamp(1, 3))
            .rev()
            .find_map(|sc| bigfont::render_scaled(text, width, 4, sc).filter(fits))
            .or_else(|| bigfont::render_compact(text, width, 4).filter(fits))
    }

    /// Everything [`draw_focus`] needs besides the frame, area and theme.
    pub struct Focus<'a> {
        pub lines: &'a [String],
        /// The active line; `None` before the first line (the intro chords take the stage).
        pub current: Option<usize>,
        /// Blank rows between lines.
        pub gap: usize,
        /// Largest big-font scale to try.
        pub scale: usize,
        pub chords: Option<&'a chords::Aligned>,
        pub tr: i32,
        pub flats: bool,
        /// `chords_context`: chords on the active line plus this many upcoming ones.
        pub context: usize,
        /// The active line is in an instrumental break: show its break chords under the stage.
        pub in_break: bool,
    }

    /// "Focus" layout used by the big-font mode: the active line sits on a fixed stage in the
    /// middle of the panel (big font if it fits, otherwise bold wrapped text), with the lines
    /// before and after stacked outward from it, each with its chord row when it has chords.
    /// Returns which lyric line each panel row belongs to (for click-to-seek).
    pub fn draw_focus(f: &mut Frame, inner: Rect, th: &Theme, fc: &Focus) -> Vec<Option<usize>> {
        let width = inner.width as usize;
        let (lines, c) = (fc.lines, fc.current);
        let names = |v: &[String]| {
            v.iter()
                .map(|n| chords::transpose(n, fc.tr, fc.flats))
                .collect::<Vec<_>>()
                .join("   ")
        };
        let bold = Style::new().fg(th.primary()).add_modifier(Modifier::BOLD);
        // which lyric line each panel row belongs to (chord rows and the intro belong to none)
        let hits = std::cell::RefCell::new(vec![None; inner.height as usize]);
        let (stage, stage_style, stage_li): (Vec<String>, Style, Option<usize>) = match c {
            Some(c) => match big_stage(&lines[c], width, inner.height, fc.scale) {
                Some(rows) => (rows, Style::new().fg(th.primary()), Some(c)),
                None => (wrap(&lines[c], width), bold, Some(c)),
            },
            None => (
                fc.chords.map_or_else(Vec::new, |a| a.intro.iter().map(|n| names(n)).collect()),
                bold,
                None,
            ),
        };
        let sh = (stage.len() as u16).min(inner.height);
        let top = inner.y + (inner.height - sh) / 2;

        let put = |f: &mut Frame, y: i32, li: Option<usize>, text: &str, style: Style| {
            if y >= inner.y as i32 && y < inner.bottom() as i32 && !text.is_empty() {
                hits.borrow_mut()[(y - inner.y as i32) as usize] = li;
                f.render_widget(
                    Paragraph::new(Line::styled(text.to_string(), style)).alignment(Alignment::Center),
                    Rect { y: y as u16, height: 1, ..inner },
                );
            }
        };
        // a chord row at its own left offset inside the panel
        let put_at = |f: &mut Frame, y: i32, x: usize, text: &str, style: Style| {
            if y >= inner.y as i32 && y < inner.bottom() as i32 && !text.is_empty() {
                let w = (crate::util::width(text) as u16).min(inner.width);
                let xs = (inner.x + x as u16).min(inner.right().saturating_sub(w)).max(inner.x);
                f.render_widget(
                    Paragraph::new(Line::styled(text.to_string(), style)),
                    Rect { x: xs, y: y as u16, width: w, height: 1 },
                );
            }
        };

        for (i, row) in stage.iter().take(sh as usize).enumerate() {
            put(f, top as i32 + i as i32, stage_li, row, stage_style);
        }
        // summary of the active line's chords above the big text
        if let Some(c) = c {
            let summary = fc.chords.and_then(|a| a.per_line.get(c)).filter(|cs| !cs.is_empty()).map(|cs| {
                let mut v: Vec<String> = cs.iter().map(|(_, n)| chords::transpose(n, fc.tr, fc.flats)).collect();
                v.dedup();
                v.join("   ")
            });
            if let Some(cs) = summary {
                put(f, top as i32 - 1, Some(c), &cs, bold);
            }
        }
        // chords played during an instrumental break, right under the stage
        let mut after = top as i32 + sh as i32 + 1;
        if let (Some(c), Some(a), true) = (c, fc.chords, fc.in_break) {
            for g in a.gaps.get(c).into_iter().flatten() {
                put(f, after, None, &names(g), bold);
                after += 1;
            }
        }

        // the wrapped rows of line `li` with their chord rows (hidden outside `chords_context`)
        let cur = c.map_or(-1, |c| c as i64);
        let rows_of = |li: usize| {
            let d = li as i64 - cur;
            let shown = fc.context >= 99 || (d >= 0 && d <= fc.context as i64);
            let cs = fc.chords.and_then(|a| a.per_line.get(li)).filter(|_| shown);
            chord_rows(&lines[li], cs.map_or(&[], |v| v.as_slice()), width, fc.tr, fc.flats)
        };

        // Earlier lines, nearest first, growing upward.
        let mut y = top as i32 - 2;
        for k in 1..=c.unwrap_or(0) {
            let li = c.unwrap_or(0) - k;
            let style = fade_style(th, k, true);
            let chord_style = Style::new().fg(th.accent_fade(0.55));
            for (text, chord) in rows_of(li).iter().rev() {
                put(f, y, Some(li), text, style);
                y -= 1;
                if let Some((x, cl)) = chord {
                    put_at(f, y, *x, cl, chord_style);
                    y -= 1;
                }
            }
            y -= fc.gap as i32;
            if y < inner.y as i32 {
                break;
            }
        }
        // Upcoming lines, growing downward.
        let mut y = after;
        for k in c.map_or(0, |c| c + 1)..lines.len() {
            let style = fade_style(th, (k as i64 - cur) as usize, false);
            let chord_style = Style::new().fg(th.accent_fade(0.35));
            for (text, chord) in rows_of(k) {
                if let Some((x, cl)) = chord {
                    put_at(f, y, x, &cl, chord_style);
                    y += 1;
                }
                put(f, y, Some(k), &text, style);
                y += 1;
            }
            y += fc.gap as i32;
            if y >= inner.bottom() as i32 {
                break;
            }
        }
        hits.into_inner()
    }

    /// `[Jimin] ` / `[Jimin, V] ` in each member's colour, dimmed; empty if the tag plus the
    /// text (`text_w` columns) wouldn't fit in `width`.
    fn member_tag(members: &[String], width: usize, text_w: usize) -> Vec<Span<'static>> {
        let dim = Style::new().add_modifier(Modifier::DIM);
        let mut spans = vec![Span::styled("[", dim)];
        for (i, m) in members.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", dim));
            }
            spans.push(Span::styled(m.clone(), Style::new().fg(crate::genius::color(m)).add_modifier(Modifier::DIM)));
        }
        spans.push(Span::styled("] ", dim));
        let w: usize = spans.iter().map(|s| s.width()).sum();
        if w + text_w > width { Vec::new() } else { spans }
    }

    fn draw_lyrics(f: &mut Frame, app: &mut App, area: Rect) {
        let shown = app.translation().is_some_and(|t| app.lyric_lines().is_some_and(|l| l.len() == t.len()));
        if app.browse_expired() {
            app.browse = 0.0;
        }
        let title = if app.browse != 0.0 {
            "lyrics · browsing — End to follow".to_string()
        } else if !app.translate_on {
            "lyrics".to_string()
        } else {
            use crate::app::TranslateState as TS;
            match (&app.translate_state, app.translation()) {
                (_, Some(_)) if shown && app.translate_backend != "google" => {
                    format!("lyrics · translated ({}, {})", app.args.translate_lang, app.translate_backend)
                }
                (_, Some(_)) if shown => format!("lyrics · translated ({})", app.args.translate_lang),
                (TS::Failed(e), _) => format!("lyrics · {e}"),
                _ => "lyrics · translating…".to_string(),
            }
        };
        let block = panel(app, &title);
        let inner = block.inner(area).inner(ratatui::layout::Margin {
            horizontal: 2 + app.args.lyric_pad,
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
        // Translated text replaces the original line by line; the chord sheet is matched on
        // the original wording, so chords are hidden while a translation is shown.
        let mut lines = lines;
        let translated = match app.translation() {
            Some(t) if t.len() == lines.len() => {
                for (l, t) in lines.iter_mut().zip(t) {
                    if !t.trim().is_empty() {
                        *l = t.clone();
                    }
                }
                true
            }
            _ => false,
        };
        let stamps: Vec<f64> = match &app.lyrics {
            LyricsState::Ready(Lyrics::Synced(l)) => l.iter().map(|x| x.0).collect(),
            _ => Vec::new(),
        };

        // Blank rows between lines — dropped on short panels where they'd waste the space.
        let gap = if inner.height >= 10 {
            app.args.lyric_spacing as usize
        } else {
            0
        };

        let aligned: Option<chords::Aligned> = (app.chords_on && !plain && !translated)
            .then(|| app.aligned.clone())
            .flatten()
            .filter(|a| a.usable());
        let tr = app.chord_shift();
        let flats = app.use_flats();

        // Big-font mode uses the fixed "focus" layout instead of the scrolling one — for the
        // active line, or before the first line when there are intro chords to show.
        let intro_chords = current.is_none() && aligned.as_ref().is_some_and(|a| !a.intro.is_empty());
        if app.big_lyrics && !plain && app.browse == 0.0 && (current.is_some() || intro_chords) {
            app.lyric_scroll_target = app.lyric_scroll; // nothing left to animate
            // in a long silence after the line (same rule as the scrolling layout's markers)
            let t = pos + app.offset;
            let in_break = current.is_some_and(|c| {
                let sung = stamps.get(c).copied().unwrap_or(f64::MAX) + est_sung(&lines[c]);
                lines[c].trim() != "♪"
                    && t >= sung
                    && match stamps.get(c + 1) {
                        Some(n) => n - stamps[c] > app.args.interlude_secs,
                        None => length > 0.0 && length - stamps[c] > app.args.interlude_secs + est_sung(&lines[c]),
                    }
            });
            let focus = Focus {
                lines: &lines,
                current,
                gap,
                scale: app.args.big_scale as usize,
                chords: aligned.as_ref(),
                tr,
                flats,
                context: app.args.chords_context as usize,
                in_break,
            };
            let rows = draw_focus(f, inner, &th, &focus);
            // top = 0: the row index is the offset from the panel's top edge
            app.lyric_hits = crate::app::LyricHits { area: inner, top: 0, rows };
            return;
        }

        // Wrap into rows, remembering which source line each row belongs to. Spacing rows
        // belong to the line above them but hold no text.
        // A chord row (`chord_x` = its left offset inside the panel) sits right above each
        // wrapped text row that carries chords; whether it is *drawn* depends on the active
        // line, but the row is always reserved so the layout doesn't jump on every line change.
        struct Row {
            li: usize,
            text: String,
            chord_x: Option<usize>,
            /// A chord row of the pre-lyrics intro section.
            intro: bool,
            /// `[ Instrumental / Solo ]` placeholder; only drawn while it is active.
            marker: Option<Mk>,
            /// A chord row played during this break; drawn only while the break is active.
            gap: Option<Mk>,
        }
        #[derive(Clone, Copy, PartialEq)]
        enum Mk {
            Lead,
            After(usize),
            Tail,
        }
        let width = inner.width as usize;
        let mut rows: Vec<Row> = Vec::new();
        let threshold = app.args.interlude_secs;
        let marker_row = |li: usize, m: Mk| Row {
            li,
            text: "[ Instrumental / Solo ]".into(),
            chord_x: None,
            intro: false,
            marker: Some(m),
            gap: None,
        };
        // Time before the first lyric: an instrumental marker, then the intro chords.
        if !plain && stamps.first().is_some_and(|t| *t > threshold) {
            rows.push(marker_row(0, Mk::Lead));
        }
        // Intro chords (before the first lyric timestamp) come first.
        if let Some(a) = &aligned {
            for names in &a.intro {
                let names: Vec<String> = names.iter().map(|n| chords::transpose(n, tr, flats)).collect();
                let text = names.join("   ");
                let x = width.saturating_sub(crate::util::width(&text)) / 2;
                rows.push(Row { li: 0, text, chord_x: Some(x), intro: true, marker: None, gap: None });
            }
            if !a.intro.is_empty() {
                for _ in 0..gap.max(1) {
                    rows.push(Row { li: 0, text: String::new(), chord_x: None, intro: true, marker: None, gap: None });
                }
            }
        }
        let intro_rows = rows.len();
        // chord rows for the break after lyric line `li`, shown while that break is active
        let push_gap = |rows: &mut Vec<Row>, li: usize, m: Mk| {
            let Some(a) = &aligned else { return };
            for names in a.gaps.get(li).into_iter().flatten() {
                let names: Vec<String> = names.iter().map(|n| chords::transpose(n, tr, flats)).collect();
                let text = names.join("   ");
                let x = width.saturating_sub(crate::util::width(&text)) / 2;
                rows.push(Row { li, text, chord_x: Some(x), intro: false, marker: None, gap: Some(m) });
            }
        };
        let mut starts = Vec::with_capacity(lines.len());
        let mut ends = Vec::with_capacity(lines.len());
        for (i, l) in lines.iter().enumerate() {
            starts.push(rows.len());
            let line_chords = aligned
                .as_ref()
                .and_then(|a| a.per_line.get(i))
                .filter(|c| !c.is_empty());
            match line_chords {
                None => {
                    for r in wrap(l, width) {
                        rows.push(Row { li: i, text: r, chord_x: None, intro: false, marker: None, gap: None });
                    }
                }
                Some(cs) => {
                    let norm = l.split_whitespace().collect::<Vec<_>>().join(" ");
                    let chars: Vec<char> = norm.chars().collect();
                    let ranges = wrap_ranges(&norm, width);
                    let mut per_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); ranges.len()];
                    for (c, name) in cs {
                        let r = ranges.iter().rposition(|(s, _)| *s <= *c).unwrap_or(0);
                        per_row[r].push((c.saturating_sub(ranges[r].0), name.clone()));
                    }
                    for ((rs, re), here) in ranges.iter().zip(per_row) {
                        let text: String = chars[*rs..*re].iter().collect();
                        if !here.is_empty() {
                            let x = width.saturating_sub(crate::util::width(&text)) / 2;
                            rows.push(Row {
                                li: i,
                                text: chord_line(&here, &chars[*rs..*re], width.saturating_sub(x), tr, flats),
                                chord_x: Some(x),
                                intro: false,
                                marker: None,
                                gap: None,
                            });
                        }
                        rows.push(Row { li: i, text, chord_x: None, intro: false, marker: None, gap: None });
                    }
                }
            }
            ends.push(rows.len());
            // A long silence after this line (solo / instrumental break).
            if !plain && l.trim() != "♪" {
                let next = stamps.get(i + 1).copied();
                match next {
                    Some(n) if n - stamps[i] > threshold => {
                        rows.push(marker_row(i, Mk::After(i)));
                        push_gap(&mut rows, i, Mk::After(i));
                    }
                    None if length > 0.0 && length - stamps[i] > threshold + est_sung(l) => {
                        rows.push(marker_row(i, Mk::Tail));
                        push_gap(&mut rows, i, Mk::Tail);
                    }
                    _ => {}
                }
            }
            for _ in 0..gap {
                rows.push(Row { li: i, text: String::new(), chord_x: None, intro: false, marker: None, gap: None });
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
                // before the first line: centre the intro chords if there are any
                None if intro_rows > 0 => intro_rows as f64 / 2.0 - 0.5,
                None => -1.0,
            }
        };

        // Critically-damped-ish easing toward the target; snap on big jumps.
        let now = std::time::Instant::now();
        let dt = now.duration_since(app.last_anim).as_secs_f64().min(0.1);
        app.last_anim = now;
        // browsing moves the view, not the playback: the target stays relative to the playing line
        app.browse = app.browse.clamp(-target - 1.0, rows.len() as f64 - target);
        let target = target + app.browse;
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
        // Remember which lyric line each visible row belongs to, so a click can seek to it.
        // (Spacing rows, markers and the intro belong to no line; plain lyrics aren't clickable.)
        app.lyric_hits = crate::app::LyricHits {
            area: inner,
            top,
            rows: rows
                .iter()
                .map(|r| {
                    (!plain && !r.intro && r.marker.is_none() && r.gap.is_none() && !r.text.is_empty()).then_some(r.li)
                })
                .collect(),
        };
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
            let Row { li, text, chord_x, intro, marker, gap } = &rows[idx as usize];
            if text.is_empty() {
                continue;
            }
            // (is the break happening now, seconds left in it)
            let break_state = |m: Mk| -> (bool, f64) {
                let t = pos + app.offset;
                let last = stamps.len().saturating_sub(1);
                match m {
                    Mk::Lead => (current.is_none(), stamps[0] - t),
                    Mk::After(i) => (
                        current == Some(i) && t >= stamps[i] + est_sung(&lines[i]),
                        stamps[i] + (stamps[i + 1] - stamps[i]) - t,
                    ),
                    Mk::Tail => (
                        current == Some(last) && t >= stamps[last] + est_sung(&lines[last]),
                        0.0,
                    ),
                }
            };
            if let (Some(m), Some(x)) = (gap, chord_x) {
                // play-along chords of a solo / instrumental break, only while it lasts
                if !break_state(*m).0 {
                    continue;
                }
                let w = crate::util::width(text.as_str()) as u16;
                let xs = (inner.x + *x as u16).min(inner.right().saturating_sub(w)).max(inner.x);
                f.render_widget(
                    Paragraph::new(Line::styled(
                        text.as_str(),
                        Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
                    )),
                    Rect { x: xs, y: inner.y + y, width: w.min(inner.right() - xs), height: 1 },
                );
                continue;
            }
            if let Some(m) = marker {
                // Shown only while the break is actually happening, with a countdown.
                let (active, remain) = break_state(*m);
                if !active {
                    continue;
                }
                let label = if remain > 0.5 && *m != Mk::Tail {
                    format!("{text}  · {}s", remain.ceil() as u32)
                } else {
                    text.clone()
                };
                f.render_widget(
                    Paragraph::new(Line::styled(
                        label,
                        Style::new().fg(th.primary()).add_modifier(Modifier::ITALIC),
                    ))
                    .alignment(Alignment::Center),
                    Rect { y: inner.y + y, height: 1, ..inner },
                );
                continue;
            }
            if let Some(x) = chord_x {
                let cur_i = current.map_or(-1, |c| c as i64);
                let d = *li as i64 - cur_i;
                // Default context (99) = every line; a smaller number limits chords to the
                // active line and the next N.
                let ctx = app.args.chords_context as i64;
                if !*intro && ctx < 99 && (d < 0 || d > ctx) {
                    continue;
                }
                let active = if *intro { current.is_none() } else { d == 0 && current.is_some() };
                let style = if active {
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
                } else if d < 0 && !*intro {
                    Style::new().fg(th.accent_fade(0.55))
                } else {
                    Style::new().fg(th.accent_fade(0.35))
                };
                let w = crate::util::width(text.as_str()) as u16;
                let mut xs = inner.x + *x as u16;
                if xs + w > inner.right() {
                    xs = inner.right().saturating_sub(w).max(inner.x);
                }
                f.render_widget(
                    Paragraph::new(Line::styled(text.as_str(), style)),
                    Rect {
                        x: xs,
                        y: inner.y + y,
                        width: w.min(inner.right() - xs),
                        height: 1,
                    },
                );
                continue;
            }
            let style = match current {
                Some(c) if *li == c => Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
                Some(c) => fade_style(&th, li.abs_diff(c), *li < c),
                None if plain => Style::new().fg(th.fade(0.35)),
                None => Style::new().fg(th.fade(0.55)),
            };
            // `[Jimin]` tag before the first text row of the active line and its neighbours
            let tag = current
                .filter(|c| li.abs_diff(*c) <= 2)
                .and_then(|_| app.members.get(*li))
                .filter(|m| !m.is_empty() && (starts[*li]..idx as usize).all(|j| rows[j].chord_x.is_some()))
                .map(|m| member_tag(m, width, crate::util::width(text.as_str())))
                .filter(|t| !t.is_empty());
            if let Some(tag) = tag {
                let mut spans: Vec<Span> = tag;
                spans.push(Span::styled(text.as_str(), style));
                f.render_widget(
                    Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
                    Rect { y: inner.y + y, height: 1, ..inner },
                );
                continue;
            }
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
        // "terminal colors (no usable colour file)" → "terminal colors" when hidden
        let source = match source.find(" (") {
            Some(i) if app.args.hide_color_warning && source.starts_with("terminal colors") => source[..i].to_string(),
            _ => source,
        };
        let chords_info = if app.chords_on {
            use crate::app::ChordsState as CS;
            let t = match (&app.chords_state, &app.aligned) {
                (CS::Idle, _) => "chords: waiting for a track".to_string(),
                (CS::Fetching, _) => "chords: looking…".to_string(),
                (CS::Missing, _) => "chords: no sheet found for this track".to_string(),
                (CS::Failed(e), _) => format!("chords: {e}"),
                (CS::Ready, None) => "chords: found, but there are no synced lyrics to place them on".into(),
                (CS::Ready, Some(a)) if !a.usable() => {
                    format!("chords: weak match ({}%) — hidden", a.percent())
                }
                (CS::Ready, Some(a)) => {
                    let (src, capo) = app.sheet_info().unwrap_or(("", 0));
                    let mut t = format!("chords: {src} {}%", a.percent());
                    if let Some(tu) = app.tuning() {
                        t.push_str(&format!(" · {tu}"));
                    }
                    if capo > 0 {
                        t.push_str(&format!(" · capo {capo}"));
                    }
                    if app.transpose != 0 {
                        let n = if app.transpose > 6 { app.transpose - 12 } else { app.transpose };
                        t.push_str(&format!(" · {n:+}"));
                    }
                    t
                }
            };
            Some(t)
        } else {
            None
        };
        let mut spans = vec![
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
        ];
        if let Some(c) = chords_info {
            spans.push(Span::styled("  │  ", Style::new().fg(th.fade(0.7))));
            spans.push(Span::styled(c, Style::new().fg(th.accent_fade(0.2))));
        }
        let left = Line::from(spans);
        let long = "space ⏯  n/p  ←/→ seek  +/- vol  g chords  M min  D diagrams  ? help  q quit ";
        let short = "? help  q quit ";
        let left_w = left.width() as u16;
        f.render_widget(Paragraph::new(left), area);
        for hints in [long, short] {
            if left_w + hints.chars().count() as u16 + 2 <= area.width {
                f.render_widget(
                    Paragraph::new(Line::styled(hints, dim)).alignment(Alignment::Right),
                    area,
                );
                break;
            }
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// main: terminal setup, worker spawn, event loop, clean teardown
// ════════════════════════════════════════════════════════════════════════════
#[tokio::main]
async fn main() -> Result<()> {
    // flags > config file > built-in defaults
    let matches = <cli::Args as clap::CommandFactory>::command().get_matches();
    let mut args = <cli::Args as clap::FromArgMatches>::from_arg_matches(&matches)?;
    let cfg_path = args.config.clone().unwrap_or_else(config::default_path);
    let explicit = args.config.is_some();
    let found = config::apply(&mut args, &matches, &cfg_path, explicit, &mut |w| {
        eprintln!("music_display: {w}")
    });
    if args.print_config {
        print!("{}", config::render(&args, &cfg_path, found));
        return Ok(());
    }
    if let Some(p) = &args.chords_probe {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
        ug::probe(&client, &p[0], &p[1], args.probe_save.clone()).await;
        return Ok(());
    }
    let theme = theme::load(&args);

    // ratatui::init = raw mode + alternate screen + a panic hook that restores both.
    let mut terminal = ratatui::init();
    if !args.no_mouse {
        execute!(stdout(), EnableMouseCapture)?;
    }
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
    #[test]
    fn track_identity_ignores_editions() {
        let n = util::normalize_track;
        assert_eq!(n("Master of Puppets (Remastered)"), n("Master of Puppets"));
        assert_eq!(n("Album [Deluxe Edition]"), n("Album (25th Anniversary Edition)"));
        assert_eq!(n("Song - Remastered 2011"), n("Song -"));
        assert_ne!(n("Song (Part 2)"), n("Song"));
    }

    #[test]
    fn track_identity_never_empty_and_tuning_clean() {
        assert!(!util::normalize_track("(Live)").is_empty());
        assert!(!util::normalize_track("---").is_empty());
        assert_eq!(util::normalize_track("Song - 1999"), "song - 1999");
        let store: serde_json::Value = serde_json::from_str(r#"{"tuning":{"name":"","value":""}}"#).unwrap();
        assert!(ug::find_key(&store, "tuning").is_some());
    }

    #[test]
    fn chords_align_on_cell_width() {
        // each Hangul syllable is two cells wide, so a chord over the 3rd char sits at cell 4
        let text: Vec<char> = "안녕하세요".chars().collect();
        assert_eq!(ui::chord_line(&[(2, "Am".into())], &text, 99, 0, false), "    Am");
        // pulled left to fit instead of overflowing
        assert_eq!(ui::chord_line(&[(2, "Am7".into())], &text, 6, 0, false), "   Am7");
        let text: Vec<char> = "Åäö".chars().collect();
        assert_eq!(ui::chord_line(&[(2, "G".into())], &text, 99, 0, false), "  G");
    }

    #[test]
    fn google_translate_response_parses() {
        let v: serde_json::Value = serde_json::from_str(r#"[[["Hello ","안녕",null],["world","세계",null]],null,"ko"]"#).unwrap();
        assert_eq!(translate::parse(&v).as_deref(), Some("Hello world"));
    }

    #[test]
    fn settings_save_keeps_other_keys() {
        let dir = std::env::temp_dir().join(format!("md-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let _ = std::fs::remove_dir_all(&dir);
        config::save(&path, &[("chords", toml::Value::Boolean(true))]).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{first}player = \"mpd\"\n")).unwrap();
        config::save(&path, &[("translate", toml::Value::Boolean(true))]).unwrap();
        let t: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(t["chords"].as_bool(), Some(true));
        assert_eq!(t["translate"].as_bool(), Some(true));
        assert_eq!(t["player"].as_str(), Some("mpd"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;
    use clap::Parser;

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
        // curly quotes and ellipsis work
        assert!(bigfont::render("Cafe – don’t…", 100, 1).is_some());
        // accented letters are drawn with their accent, not as the ASCII lookalike
        let plain = bigfont::render("AAO", 100, 1).unwrap();
        let acc = bigfont::render("ÅÄÖ", 100, 1).unwrap();
        assert_eq!(acc.len(), plain.len());
        assert_ne!(acc, plain);
        assert!(bigfont::render("Café", 100, 1).is_some());
        assert!(bigfont::render("ß", 100, 1).is_none());
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
    fn chord_recognition_and_transpose() {
        for ok in ["A", "Am", "F#m7", "Bb/D", "Gsus4", "Cadd9", "E7b9", "D(add9)", "Cmaj7", "G/B", "Em7"] {
            assert!(chords::is_chord(ok), "{ok}");
        }
        for bad in ["Hello", "I", "And", "Bridge", "Hm", "N.C.", "x2", "A/", "Dog"] {
            assert!(!chords::is_chord(bad), "{bad}");
        }
        assert_eq!(chords::transpose("Am7", 2, false), "Bm7");
        assert_eq!(chords::transpose("C/E", 1, false), "C#/F");
        assert_eq!(chords::transpose("C/E", 1, true), "Db/F");
        assert_eq!(chords::transpose("Bb", 2, true), "C");
        assert_eq!(chords::transpose("E", 12, false), "E");
        assert_eq!(chords::transpose("G", 7, false), "D");
    }

    const OVER: &str = "[Verse 1]\nCapo 2\n   Am        G\nHello darkness my old friend\n   C         E7\nI've come to talk with you again\n\n[Chorus]\n   F      G\nSing along tonight\n\nBridge over troubled water\n";

    #[test]
    fn parses_chords_over_lyrics() {
        let s = chords::parse_text(OVER);
        assert_eq!(s.capo, 2);
        assert_eq!(s.lines.len(), 4, "{:?}", s.lines);
        assert_eq!(s.lines[0].text, "Hello darkness my old friend");
        assert_eq!(s.lines[0].chords, vec![(3, "Am".to_string()), (13, "G".to_string())]);
        assert_eq!(s.lines[2].chords.len(), 2);
        // a lyric that merely starts with a section word stays a lyric
        assert_eq!(s.lines[3].text, "Bridge over troubled water");
        assert!(s.lines[3].chords.is_empty());
        // UG-style tags and tab lines are cleaned
        let ug = chords::parse_text("[Verse]\n[ch]Am[/ch]   [ch]G[/ch]\nhello there\ne|---0---|\n");
        assert_eq!(ug.lines.len(), 1);
        assert_eq!(ug.chord_count(), 2);
    }

    #[test]
    fn parses_chordpro_and_autodetects() {
        let src = "{title: x}\n{capo: 3}\n# comment\n[Am]Hello dar[G]kness\n[Chorus]\n[C] [G]\nplain [text] here\n";
        let s = chords::parse_chordpro(src);
        assert_eq!(s.capo, 3);
        assert_eq!(s.lines[0].text, "Hello darkness");
        assert_eq!(s.lines[0].chords, vec![(0, "Am".to_string()), (9, "G".to_string())]);
        assert_eq!(s.lines[1].text, " ");
        assert_eq!(s.lines[1].chords.len(), 2);
        assert_eq!(s.lines[2].text, "plain [text] here");
        assert_eq!(chords::parse_auto(src).lines[0].chords.len(), 2);
        assert_eq!(chords::parse_auto(OVER).lines.len(), 4);
    }

    fn lrc(lines: &[&str]) -> Vec<(f64, String)> {
        lines.iter().enumerate().map(|(i, l)| (i as f64, l.to_string())).collect()
    }

    #[test]
    fn aligns_sheet_to_synced_lyrics() {
        let sheet = chords::parse_text(OVER);
        // chorus appears twice in the song but once in the sheet; wording differs a little
        let l = lrc(&[
            "Hello darkness, my old friend",
            "♪",
            "I've come to talk with you again",
            "Sing along tonight",
            "Hello darkness, my old friend",
            "Sing along tonight",
        ]);
        let a = chords::align(&sheet, &l);
        assert_eq!((a.matched, a.total), (5, 5), "{a:?}");
        assert!(a.usable());
        assert!(a.per_line[1].is_empty());
        // "Am" over "Hello" → col 0; "G" over "my"/"old" region, anchored to a word
        assert_eq!(a.per_line[0][0], (3, "Am".into())); // col 3 is inside "Hello"
        assert_eq!(a.per_line[0].len(), 2);
        // repeated chorus line and repeated verse line (global fallback) get chords again
        assert_eq!(a.per_line[3].len(), 2);
        assert_eq!(a.per_line[5].len(), 2);
        assert_eq!(a.per_line[4].len(), 2);
        // unrelated lyrics → nothing is shown
        let bad = chords::align(&sheet, &lrc(&["completely different words here", "nothing alike at all", "zzz qqq"]));
        assert!(!bad.usable() && bad.per_line.iter().all(Vec::is_empty));
    }

    #[test]
    fn wrap_ranges_matches_wrap() {
        for text in [
            "hello darkness my old friend",
            "a verylongtokenthatmustbebroken here and there",
            "one",
            "",
        ] {
            for w in [5usize, 8, 12, 40] {
                let rows = ui::wrap(text, w);
                let chars: Vec<char> = text.chars().collect();
                let r2: Vec<String> = ui::wrap_ranges(text, w)
                    .iter()
                    .map(|(s, e)| chars[*s..*e].iter().collect())
                    .collect();
                assert_eq!(rows, r2, "{text:?} @ {w}");
            }
        }
    }

    #[test]
    fn finds_local_sheets() {
        let dir = tmp("chords");
        std::fs::write(
            dir.join("The Sound Of Silence - Simon & Garfunkel.txt"),
            "x",
        )
        .unwrap();
        std::fs::write(dir.join("simon garfunkel - the sound of silence.cho"), "[Am]a [G]b [C]c [F]d").unwrap();
        let hit = chords::find_local(&dir, "Simon & Garfunkel", "The Sound of Silence - Remastered").unwrap();
        assert_eq!(hit.extension().unwrap(), "cho"); // .cho outranks .txt
        assert!(chords::find_local(&dir, "Nobody", "Nothing").is_none());
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let sheet = rt
            .block_on(chords::load_local(dir.clone(), "Simon & Garfunkel".into(), "The Sound of Silence".into()))
            .unwrap();
        assert_eq!(sheet.chord_count(), 4);
        assert!(sheet.source.ends_with(".cho"));
    }

    fn ug_page(json: &serde_json::Value) -> String {
        let esc = json.to_string().replace('&', "&amp;").replace('"', "&quot;").replace('\'', "&#039;");
        format!("<html><body><div class=\"js-store\" data-content=\"{esc}\"></div></body></html>")
    }

    /// Tiny HTTP server: `routes` maps a path fragment to (status, body); counts requests.
    fn fake_site(routes: Vec<(&'static str, u16, String)>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h2 = hits.clone();
        std::thread::spawn(move || {
            for mut st in l.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let first = req.lines().next().unwrap_or("").to_string();
                h2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (code, body) = routes
                    .iter()
                    .find(|(frag, _, _)| first.contains(frag))
                    .map(|(_, c, b)| (*c, b.clone()))
                    .unwrap_or((404, String::new()));
                let _ = write!(st, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, hits)
    }

    #[test]
    fn ultimate_guitar_provider_with_fake_site() {
        assert_eq!(ug::unescape("a &quot;b&quot; &amp; c &#039;d&#x27; &bogus; &"), "a \"b\" & c 'd' &bogus; &");

        let tab = "[Verse 1]\n[ch]Am[/ch]        [ch]G[/ch]\nHello darkness my old friend\n[ch]C[/ch]         [ch]F[/ch]\nI've come to talk with you again\n[ch]Am[/ch]        [ch]G[/ch]\nBecause a vision softly creeping\n[ch]C[/ch]         [ch]F[/ch]\nLeft its seeds while I was sleeping\n";
        let search = serde_json::json!({"store":{"page":{"data":{"results":[
            {"song_name":"The Sound of Silence","artist_name":"Simon & Garfunkel","type":"Tabs","tab_url":"/tab/x/tabs-1","rating":5.0,"votes":9999},
            {"song_name":"The Sound of Silence (Live)","artist_name":"Someone Else","type":"Chords","tab_url":"/tab/y/chords-2","rating":5.0,"votes":9999},
            {"song_name":"The Sound of Silence","artist_name":"Simon & Garfunkel","type":"Chords","tab_url":"/tab/z/chords-3","rating":4.1,"votes":50},
            {"song_name":"The Sound of Silence","artist_name":"Simon & Garfunkel","type":"Chords","tab_url":"/tab/z/chords-4","rating":4.8,"votes":4000}
        ]}}}});
        let page = |content: &str| ug_page(&serde_json::json!({"store":{"page":{"data":{"tab_view":{"wiki_tab":{"content":content},"meta":{"capo":3}}}}}}));

        let store = ug::extract_store(&ug_page(&search)).unwrap();
        let c = ug::candidates(&store, "Simon & Garfunkel", "The Sound of Silence - Remastered");
        assert_eq!(c.len(), 2, "tabs and other artists are filtered: {c:?}");
        assert!(c[0].url.ends_with("chords-4"), "best rated × votes first");

        // best page is junk (no chords) → falls through to the next candidate
        let (base, hits) = fake_site(vec![
            ("chords-4", 200, page("just words, no chords here\nat all\n")),
            ("chords-3", 200, page(tab)),
            ("search.php", 200, ug_page(&search)),
        ]);
        let client = reqwest::Client::new();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (artist, title) = ("Simon & Garfunkel", "The Sound of Silence UGTEST");
        // title differs slightly from the fixture, so use a title that matches it exactly:
        let _ = title;
        let title = "The Sound of Silence";
        let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a(&format!("{artist}\0{title}")))));
        ug::reset_breaker();
        let out = rt.block_on(ug::lookup(&client, &base, artist, title));
        let ug::Outcome::Found(sheet) = out else { panic!("expected a sheet") };
        assert_eq!(sheet.chord_count(), 8);
        assert_eq!(sheet.capo, 3, "capo comes from the page metadata");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3, "search + 2 tab pages");
        // second lookup: from the cache, zero requests
        let again = rt.block_on(ug::lookup(&client, &base, artist, title));
        let ug::Outcome::Found(s2) = again else { panic!("cached sheet") };
        assert_eq!(s2.chord_count(), 8);
        assert!(s2.source.contains("cached") && s2.capo == 3);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);
        let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a(&format!("{artist}\0{title}")))));

        // blocked (403) three times in a row → provider switches itself off, no more requests
        let (base2, hits2) = fake_site(vec![("search.php", 403, "no".into())]);
        ug::reset_breaker();
        for i in 0..3 {
            let t = format!("Blocked Song {i}");
            let o = rt.block_on(ug::lookup(&client, &base2, "Nobody", &t));
            assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("403")));
        }
        let o = rt.block_on(ug::lookup(&client, &base2, "Nobody", "Blocked Song 9"));
        assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("switched off")));
        assert_eq!(hits2.load(std::sync::atomic::Ordering::SeqCst), 3);

        // page without js-store (captcha / redesign) is an error, never a cached "not found"
        let (base3, _) = fake_site(vec![("search.php", 200, "<html>captcha</html>".into())]);
        ug::reset_breaker();
        let o = rt.block_on(ug::lookup(&client, &base3, "Nobody", "Format Change"));
        assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("not recognised")));

        // no matching chord page → NotFound (and cached as such)
        let empty = ug_page(&serde_json::json!({"store":{"page":{"data":{"results":[]}}}}));
        let (base4, _) = fake_site(vec![("search.php", 200, empty)]);
        ug::reset_breaker();
        let o = rt.block_on(ug::lookup(&client, &base4, "Nobody", "Truly Unknown Song"));
        assert!(matches!(o, ug::Outcome::NotFound));
        let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a("Nobody\0Truly Unknown Song"))));
        ug::reset_breaker();
    }

    #[test]
    fn chordpro_cache_round_trip() {
        let s = chords::parse_text(OVER);
        let back = chords::parse_chordpro(&chords::to_chordpro(&s));
        assert_eq!(back.capo, s.capo);
        assert_eq!(back.lines.len(), s.lines.len());
        for (a, b) in s.lines.iter().zip(&back.lines) {
            assert_eq!(a.text.trim(), b.text.trim());
            assert_eq!(a.chords.iter().map(|c| &c.1).collect::<Vec<_>>(), b.chords.iter().map(|c| &c.1).collect::<Vec<_>>());
        }
    }

    fn snap(artist: &str, title: &str, album: &str) -> player::Snapshot {
        player::Snapshot {
            status: player::Status::Playing,
            position: 0.0,
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            length: 100.0,
            art_url: String::new(),
            track_no: None,
            disc_no: None,
            content_created: String::new(),
            volume: None,
            url: String::new(),
            track_id: String::new(),
            player: String::new(),
            shuffle: None,
            repeat: None,
        }
    }

    #[test]
    fn track_key_ignores_editions_and_case() {
        let k = |a, t, al| snap(a, t, al).key();
        let base = k("Metallica", "Enter Sandman", "Metallica");
        assert_eq!(base, k("Metallica", "Enter Sandman", "Metallica (Remastered Deluxe Box Set)"));
        assert_eq!(base, k("metallica ", "Enter  Sandman - Remastered 2021", "METALLICA (Anniversary Box Set)"));
        assert_eq!(base, k("Metallica", "Enter Sandman (Remastered)", "Metallica [Deluxe Edition]"));
        // different songs / artists stay different
        assert_ne!(base, k("Metallica", "Sad But True", "Metallica"));
        assert_ne!(base, k("Megadeth", "Enter Sandman", "Metallica"));
    }

    #[tokio::test]
    async fn display_stays_on_first_spelling_of_a_track() {
        use clap::Parser;
        let args = cli::Args::parse_from(["music_display"]);
        let theme = theme::load(&args);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
        let mut a = snap("Metallica", "Enter Sandman", "Metallica");
        a.content_created = "1991-08-12".into();
        app.apply(AppEvent::Player(Some(a)));
        let mut b = snap("Metallica", "Enter Sandman - Remastered 2021", "Metallica (Deluxe Edition)");
        b.content_created = "2021-09-10".into();
        for s in [b.clone(), snap("Metallica", "Enter Sandman", "Metallica"), b] {
            app.apply(AppEvent::Player(Some(s)));
            let shown = app.snap.as_ref().unwrap();
            assert_eq!(shown.album, "Metallica");
            assert_eq!(shown.title, "Enter Sandman");
            assert_eq!(shown.content_created, "1991-08-12");
        }
    }

    #[test]
    fn key_is_estimated_from_chords() {
        let sheet = |names: &[&str]| chords::Sheet {
            lines: vec![chords::SheetLine {
                text: "la la la la la la la la".into(),
                chords: names.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect(),
            }],
            ..Default::default()
        };
        assert_eq!(chords::estimate_key(&sheet(&["C", "G", "Am", "F", "C", "G", "F", "C"])).as_deref(), Some("C"));
        assert_eq!(chords::estimate_key(&sheet(&["Am", "F", "C", "G", "Am", "F", "G", "Am"])).as_deref(), Some("Am"));
        assert_eq!(chords::estimate_key(&sheet(&["C", "G"])), None);
    }

    #[tokio::test]
    async fn translation_falls_back_to_libretranslate_on_429() {
        let (site, log) = scripted_site(vec![
            ("/g/dict", vec![R::Http(429, "", String::new())]),
            ("/g?", vec![R::Http(429, "", String::new())]),
            ("/libre", vec![R::Http(200, "Content-Type: application/json\r\n", r#"{"translatedText":"hello"}"#.into())]),
        ]);
        let cfg = translate::Cfg {
            google_base: Some(format!("{site}/g")),
            fallback: true,
            fallback_url: format!("{site}/libre"),
            fallback_key: Some("k".into()),
        };
        let src = vec!["hola".to_string(), "♪".to_string(), "adiós".to_string()];
        let (out, backend) = translate::lines(&reqwest::Client::new(), &cfg, "en", &src).await.unwrap();
        assert_eq!(backend, "libretranslate");
        assert_eq!(out, vec!["hello", "♪", "hello"]);
        // once Google said 429 the rest of the song goes straight to the fallback
        assert!(log.lock().unwrap().iter().filter(|l| l.contains("/g")).count() <= 4);
        // no fallback: a limited Google is an error
        let off = translate::Cfg { fallback: false, ..cfg };
        assert!(translate::lines(&reqwest::Client::new(), &off, "en", &src).await.is_err());
    }

    #[test]
    fn dict_chrome_response_parses() {
        let a: serde_json::Value = serde_json::from_str(r#"{"sentences":[{"trans":"Hi ","orig":"x"},{"trans":"there"}]}"#).unwrap();
        assert_eq!(translate::parse_dict(&a).as_deref(), Some("Hi there"));
        let b: serde_json::Value = serde_json::from_str(r#"["Hi"]"#).unwrap();
        assert_eq!(translate::parse_dict(&b).as_deref(), Some("Hi"));
    }

    #[test]
    fn cjk_wrapping_keeps_punctuation_off_line_starts() {
        let rows = ui::wrap("今日は、いい天気です。", 8);
        assert!(rows.iter().all(|r| crate::util::width(r) <= 8), "{rows:?}");
        assert!(rows.iter().all(|r| !r.starts_with(['、', '。'])), "{rows:?}");
        assert_eq!(rows.concat(), "今日は、いい天気です。");
        let big = bigfont::render_scaled("HI", 60, 3, 2).unwrap();
        assert_eq!(big.len(), 8);
    }

    #[test]
    fn hangul_big_font_renders_blocks() {
        // one 8-pixel-tall glyph row group = 4 text rows; "안녕" = 2 × 9 − 1 columns
        let rows = bigfont::render("안녕", 60, 3).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.chars().count() == 17), "{rows:?}");
        assert!(rows.iter().any(|r| r.contains('█')));
        // scaling doubles both dimensions
        let big = bigfont::render_scaled("안녕", 60, 3, 2).unwrap();
        assert_eq!((big.len(), big[0].chars().count()), (8, 34));
        // every syllable composes (initials, vowel classes, finals, clusters)
        for c in ['가', '꿈', '봄', '왜', '읽', '값', '뷁', '힣', '쀍'] {
            assert!(bigfont::render(&c.to_string(), 20, 1).is_some(), "{c}");
        }
    }

    #[test]
    fn mixed_latin_and_hangul_render_together() {
        // H(6) I(6) space(3) 안(9) 녕(8, no trailing gap)
        let rows = bigfont::render("Hi 안녕", 60, 3).unwrap();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.chars().count() == 6 + 6 + 3 + 9 + 8), "{rows:?}");
        // ideographs and Arabic still fall back to normal text
        assert!(bigfont::render("你好", 60, 3).is_none());
        assert!(bigfont::render("مرحبا", 60, 3).is_none());
        assert!(bigfont::render_compact("안녕", 60, 3).is_none());
    }

    #[test]
    fn compact_font_takes_over_when_the_normal_one_is_too_wide() {
        // "HELLO" is 29 columns normally, 19 compact
        assert!(bigfont::render_scaled("HELLO", 20, 4, 1).is_none());
        let rows = bigfont::render_compact("HELLO", 20, 4).unwrap();
        assert_eq!((rows.len(), rows[0].chars().count()), (3, 19));
        let stage = ui::big_stage("Hello world", 20, 12, 3).unwrap();
        assert_eq!(stage.len(), 3 + 1 + 3);
        // too short for two pieces plus room for neighbours: bold text instead
        assert!(ui::big_stage("Hello world", 20, 8, 3).is_none());
        // wide panels still get the largest scale that fits
        assert_eq!(ui::big_stage("HI", 60, 40, 3).unwrap().len(), 12);
        assert_eq!(ui::big_stage("HI", 60, 12, 3).unwrap().len(), 8);
    }

    #[test]
    fn focus_layout_puts_chord_rows_on_neighbours_without_hits() {
        use clap::Parser;
        use ratatui::{Terminal, backend::TestBackend, layout::Rect};
        let args = cli::Args::parse_from(["music_display"]);
        let theme = theme::load(&args);
        let lines: Vec<String> = ["first line here", "second line here", "third line here"].map(String::from).to_vec();
        let aligned = chords::Aligned {
            per_line: vec![vec![(0, "Am".into())], vec![(0, "G".into())], vec![(0, "C".into())]],
            matched: 3,
            total: 3,
            flats: false,
            intro: vec![vec!["Dm".into()]],
            gaps: vec![vec![vec!["F".into(), "E".into()]], vec![], vec![]],
        };
        let fc = |current, in_break| ui::Focus {
            lines: &lines,
            current,
            gap: 0,
            scale: 1,
            chords: Some(&aligned),
            tr: 0,
            flats: false,
            context: 99,
            in_break,
        };
        let mut term = Terminal::new(TestBackend::new(40, 24)).unwrap();
        let area = Rect::new(0, 0, 40, 24);
        let mut hits = Vec::new();
        term.draw(|f| hits = ui::draw_focus(f, area, &theme, &fc(Some(1), false))).unwrap();
        let screen = |t: &Terminal<TestBackend>| -> Vec<String> {
            let b = t.backend().buffer();
            (0..24).map(|y| (0..40).map(|x| b[(x, y)].symbol()).collect::<String>()).collect()
        };
        let text = screen(&term);
        let row_of = |text: &[String], needle: &str| text.iter().position(|r| r.contains(needle)).unwrap();
        // the next line's chord sits directly above its text and maps to no lyric line
        let third = row_of(&text, "third line here");
        assert!(text[third - 1].contains('C'));
        assert_eq!(hits[third - 1], None);
        assert_eq!(hits[third], Some(2));
        let first = row_of(&text, "first line here");
        assert_eq!(hits[first], Some(0));
        assert_eq!(hits[first - 1], None);
        assert!(text[first - 1].contains("Am"));
        // break chords show under the stage only during a break
        assert!(!text.iter().any(|r| r.contains("F   E")));
        term.draw(|f| hits = ui::draw_focus(f, area, &theme, &fc(Some(0), true))).unwrap();
        let text = screen(&term);
        let brk = row_of(&text, "F   E");
        assert_eq!(hits[brk], None);
        // before the first line the intro chords take the stage, lines follow below
        term.draw(|f| hits = ui::draw_focus(f, area, &theme, &fc(None, false))).unwrap();
        let text = screen(&term);
        let intro = row_of(&text, "Dm");
        assert_eq!(hits[intro], None);
        assert!(row_of(&text, "first line here") > intro);
    }

    #[tokio::test]
    async fn local_files_fall_back_to_their_own_tags() {
        use clap::Parser;
        assert_eq!(tags::title_from_url("file:///music/My%20Song.flac"), "My Song");
        let args = cli::Args::parse_from(["music_display"]);
        let theme = theme::load(&args);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
        let url = "file:///music/My%20Song.flac";
        app.local_tags.insert(
            url.into(),
            tags::Tags { title: "Real Title".into(), artist: "Band".into(), album: "LP".into(), year: "1999".into(), track: Some(3), ..Default::default() },
        );
        let mut s = snap("", "My Song", "");
        s.url = url.into();
        app.apply(AppEvent::Player(Some(s)));
        let shown = app.snap.as_ref().unwrap();
        assert_eq!((shown.title.as_str(), shown.artist.as_str(), shown.album.as_str()), ("Real Title", "Band", "LP"));
        assert_eq!((shown.content_created.as_str(), shown.track_no), ("1999", Some(3)));
    }

    #[test]
    fn every_menu_row_saves_a_known_config_key() {
        for set in app::Setting::ALL {
            for k in set.config_keys() {
                assert!(config::KEYS.iter().any(|c| c.name == *k), "{k}");
            }
            assert!(!set.label().is_empty());
        }
    }

    #[test]
    fn base_title_strips_credits_and_editions() {
        let b = year::base_title;
        assert_eq!(b("Song (feat. Someone)"), "Song");
        assert_eq!(b("Song ft. Someone"), "Song");
        assert_eq!(b("Song - Demo"), "Song");
        assert_eq!(b("Song - Remastered 2011"), "Song");
        assert_eq!(b("Song [Live] (Remix)"), "Song");
        assert_eq!(b("(Intro)"), "(Intro)");
        assert_eq!(b("Plain"), "Plain");
    }

    #[test]
    fn solo_chords_are_kept_for_the_break() {
        let sheet = chords::parse_chordpro(
            "[Am]walking down the [G]road tonight\n[C]  [G]  [Am]  [F]\n[Am]standing in the [G]rain again\n",
        );
        let lyrics = vec![
            (1.0, "walking down the road tonight".to_string()),
            (30.0, "standing in the rain again".to_string()),
        ];
        let a = chords::align(&sheet, &lyrics);
        assert_eq!(a.per_line[0].len(), 2);
        assert_eq!(a.gaps[0], vec![vec!["C", "G", "Am", "F"]]);
        assert!(a.gaps[1].is_empty());
    }

    fn test_app() -> app::App {
        use clap::Parser;
        let args = cli::Args::parse_from(["music_display"]);
        let theme = theme::load(&args);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx)
    }

    #[tokio::test]
    async fn versions_of_a_playing_song_never_restart_it() {
        let mut app = test_app();
        let at = |mut s: player::Snapshot, pos: f64| {
            s.position = pos;
            s.player = "spotify".into();
            s
        };
        app.apply(AppEvent::Player(Some(at(snap("Band", "Song", "Album"), 10.0))));
        let key = app.snap.as_ref().unwrap().key();
        // a new track resets the lyric offset, so a kept offset means nothing restarted
        app.offset = 0.7;
        // the player flips between releases the strings don't reveal as the same
        for (i, (t, al)) in [
            ("Song - Live at Wembley", "Live at Wembley"),
            ("Song", "Greatest Hits"),
            ("Song (feat. Guest)", "Album: Deluxe"),
            ("Totally Different Name", "Album Super Deluxe Edition"),
        ]
        .into_iter()
        .enumerate()
        {
            app.apply(AppEvent::Player(Some(at(snap("Band", t, al), 10.2 + i as f64 * 0.2))));
            let shown = app.snap.as_ref().unwrap();
            assert_eq!((shown.title.as_str(), shown.album.as_str()), ("Song", "Album"), "{t}");
            assert_eq!(app.offset, 0.7, "track restarted on {t}");
        }
        assert_eq!(app.snap.as_ref().unwrap().key(), key);
        // a real next track (position back near 0) switches at once
        app.apply(AppEvent::Player(Some(at(snap("Band", "Next One", "Album"), 0.3))));
        assert_eq!(app.snap.as_ref().unwrap().title, "Next One");
        assert_eq!(app.offset, 0.0);
        // another player that isn't playing doesn't take over the display
        let mut other = at(snap("Someone", "Video", ""), 50.0);
        other.player = "firefox".into();
        other.status = player::Status::Paused;
        app.apply(AppEvent::Player(Some(other)));
        assert_eq!(app.snap.as_ref().unwrap().title, "Next One");
    }

    enum R {
        Http(u16, &'static str, String),
        Drop,
    }

    /// Scripted server: each route plays its responses in order, repeating the last one.
    fn scripted_site(routes: Vec<(&'static str, Vec<R>)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        let counts = std::sync::Mutex::new(vec![0usize; routes.len()]);
        std::thread::spawn(move || {
            for mut st in l.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let first = req.lines().next().unwrap_or("").to_string();
                log2.lock().unwrap().push(first.clone());
                let Some(ri) = routes.iter().position(|(f, _)| first.contains(f)) else {
                    let _ = write!(st, "HTTP/1.1 404 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                };
                let mut c = counts.lock().unwrap();
                let i = c[ri].min(routes[ri].1.len() - 1);
                c[ri] += 1;
                match &routes[ri].1[i] {
                    R::Drop => drop(st), // socket disconnect
                    R::Http(code, hdr, body) => {
                        let _ = write!(st, "HTTP/1.1 {code} X\r\n{hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    }
                }
            }
        });
        (base, log)
    }

    fn fast() -> lyrics::Policy {
        lyrics::Policy { retries: 3, backoff: Duration::from_millis(1) }
    }

    fn run_lyrics(base: &str, policy: lyrics::Policy) -> lyrics::Lyrics {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let c = reqwest::Client::builder().timeout(Duration::from_millis(400)).build().unwrap();
        rt.block_on(lyrics::fetch_with(&c, base, "Metallica", "Enter Sandman - Remastered", "Metallica (Deluxe)", 331.0, policy, false))
    }

    const LRC_JSON: &str = r#"{"syncedLyrics":"[00:01.00]hello\n[00:02.00]world","plainLyrics":"hello\nworld","duration":331}"#;

    #[test]
    fn lyrics_recover_from_429_disconnect_and_5xx() {
        let (base, log) = scripted_site(vec![(
            "api/get",
            vec![
                R::Http(429, "Retry-After: 0\r\n", String::new()),
                R::Drop,
                R::Http(503, "", String::new()),
                R::Http(200, "", LRC_JSON.into()),
            ],
        )]);
        let l = run_lyrics(&base, fast());
        assert!(matches!(l, lyrics::Lyrics::Synced(ref v) if v.len() == 2), "{l:?}");
        assert_eq!(log.lock().unwrap().len(), 4, "3 failures then success on the 4th attempt");
    }

    #[test]
    fn lyrics_timeout_is_retried() {
        // first reply never arrives within the client timeout (server just sits on the socket)
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for (i, st) in l.incoming().flatten().enumerate() {
                let mut st = st;
                let mut b = [0u8; 4096];
                let _ = st.read(&mut b);
                if i == 0 {
                    std::thread::sleep(Duration::from_millis(900)); // > client timeout
                } else {
                    let _ = write!(st, "HTTP/1.1 200 X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{LRC_JSON}", LRC_JSON.len());
                }
            }
        });
        let l = run_lyrics(&base, fast());
        assert!(matches!(l, lyrics::Lyrics::Synced(_)), "{l:?}");
    }

    #[test]
    fn lyrics_fallback_chain_and_not_found_vs_error() {
        // (a) exact (has album_name) → 404; (b) without album → hit
        let (base, log) = scripted_site(vec![
            ("album_name", vec![R::Http(404, "", String::new())]),
            ("api/get", vec![R::Http(200, "", LRC_JSON.into())]),
        ]);
        assert!(matches!(run_lyrics(&base, fast()), lyrics::Lyrics::Synced(_)));
        let calls = log.lock().unwrap().clone();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(calls[1].contains("track_name=Enter+Sandman") && !calls[1].contains("Remastered"), "{calls:?}");

        // (c) both gets 404 → search finds it
        let (base, _) = scripted_site(vec![
            ("api/get", vec![R::Http(404, "", String::new())]),
            ("api/search", vec![R::Http(200, "", format!("[{LRC_JSON}]"))]),
        ]);
        assert!(matches!(run_lyrics(&base, fast()), lyrics::Lyrics::Synced(_)));

        // 404 everywhere = "no lyrics exist", not a network error
        let (base, log) = scripted_site(vec![
            ("api/get", vec![R::Http(404, "", String::new())]),
            ("api/search", vec![R::Http(200, "", "[]".into())]),
        ]);
        assert!(matches!(run_lyrics(&base, fast()), lyrics::Lyrics::NotFound));
        assert!(log.lock().unwrap().len() <= 6, "404 is never retried");

        // everything fails → network error, after bounded effort (4 + 1 + 1…)
        let (base, log) = scripted_site(vec![("api/", vec![R::Http(500, "", String::new())])]);
        let l = run_lyrics(&base, fast());
        assert!(matches!(&l, lyrics::Lyrics::Error(e) if e.contains("server error")), "{l:?}");
        let n = log.lock().unwrap().len();
        assert!((6..=9).contains(&n), "first step retried 3x, later steps once each: {n}");

        // rate limited for good → says so
        let (base, _) = scripted_site(vec![("api/", vec![R::Http(429, "Retry-After: 0\r\n", String::new())])]);
        let l = run_lyrics(&base, fast());
        assert!(matches!(&l, lyrics::Lyrics::Error(e) if e.contains("rate limited")), "{l:?}");
    }

    #[test]
    fn keys_and_camelot() {
        let k = |x: &str| chords::parse_key(x).unwrap();
        // the full wheel, checked against the standard Camelot table
        let major = [("C", "8B"), ("Db", "3B"), ("D", "10B"), ("Eb", "5B"), ("E", "12B"), ("F", "7B"), ("F#", "2B"), ("G", "9B"), ("Ab", "4B"), ("A", "11B"), ("Bb", "6B"), ("B", "1B")];
        let minor = [("Am", "8A"), ("Bbm", "3A"), ("Bm", "10A"), ("Cm", "5A"), ("C#m", "12A"), ("Dm", "7A"), ("Ebm", "2A"), ("Em", "9A"), ("Fm", "4A"), ("F#m", "11A"), ("Gm", "6A"), ("G#m", "1A")];
        for (n, c) in major.iter().chain(minor.iter()) {
            assert_eq!(k(n).camelot(), *c, "{n}");
        }
        assert_eq!(k("Em").long(), "E Minor");
        assert_eq!(k("E minor").camelot(), "9A");
        assert_eq!(k("D#m").short(), "Ebm");
        assert_eq!(k("Am").shift(3).short(), "Cm");
        assert_eq!(k("Am").shift(-12).short(), "Am");
        assert_eq!(k("C#").shift(-3).short(), "Bb");
        assert!(chords::parse_key("Hello").is_none() && chords::parse_key("Asus4").is_none());
        assert!(k("Dm").prefers_flats() && k("F").prefers_flats() && !k("A").prefers_flats() && !k("Em").prefers_flats());
    }

    #[test]
    fn sheet_metadata_and_dates() {
        let s = chords::parse_text("Key: Em\nBPM: 120\nCapo 3\n   Am   G\nhello world\n");
        assert_eq!((s.key.as_deref(), s.bpm, s.capo), (Some("Em"), Some(120), 3));
        let c = chords::parse_chordpro("{key: F#m}\n{tempo: 83}\n[Am]x [G]y");
        assert_eq!((c.key.as_deref(), c.bpm), (Some("F#m"), Some(83)));
        let back = chords::parse_chordpro(&chords::to_chordpro(&c));
        assert_eq!((back.key.as_deref(), back.bpm), (Some("F#m"), Some(83)));
        // a lyric that happens to contain a colon is untouched
        let l = chords::parse_text("   Am\nKey to my heart: you\n");
        assert_eq!(l.lines[0].text, "Key to my heart: you");
        assert_eq!(year::full_date("1969-09-26T00:00:00Z").as_deref(), Some("1969-09-26"));
        assert_eq!(year::full_date("1969-09-26").as_deref(), Some("1969-09-26"));
        assert_eq!(year::full_date("1969"), None);
        assert_eq!(year::full_date("1969-00-00"), None);
        assert_eq!(year::full_date(""), None);
    }

    #[test]
    fn intro_chords_before_first_lyric() {
        let sheet = chords::parse_chordpro("[Am] [G] [C] [F]\n[Am] [G]\n[Am]Hello darkness my old [G]friend\n[C]I've come to talk with [F]you again\n[Am]Because a vision [G]softly creeping\n[C]Left its seeds while I was [F]sleeping\n");
        let a = chords::align(&sheet, &lrc(&["Hello darkness my old friend", "I've come to talk with you again", "Because a vision softly creeping", "Left its seeds while I was sleeping"]));
        assert_eq!(a.intro, vec![vec!["Am", "G", "C", "F"], vec!["Am", "G"]]);
        assert!(a.usable());
        // no chord-only lines before the first lyric → no intro
        let b = chords::parse_chordpro("[Am]Hello darkness my old [G]friend\n[C]I've come to talk with [F]you again\n");
        let ab = chords::align(&b, &lrc(&["Hello darkness my old friend", "I've come to talk with you again"]));
        assert!(ab.intro.is_empty());
    }

    #[test]
    fn chord_diagrams() {
        let f = |n: &str| diagram::lookup(n).map(|s| s.frets);
        assert_eq!(f("C"), Some([-1, 3, 2, 0, 1, 0]));
        assert_eq!(f("Am"), Some([-1, 0, 2, 2, 1, 0]));
        assert_eq!(f("E"), Some([0, 2, 2, 1, 0, 0]));
        assert_eq!(f("F"), Some([1, 3, 3, 2, 1, 1]));
        assert_eq!(f("Bb"), Some([-1, 1, 3, 3, 3, 1]));
        assert_eq!(f("F#m"), Some([2, 4, 4, 2, 2, 2]));
        assert_eq!(f("G"), Some([3, 2, 0, 0, 0, 3]));
        assert_eq!(f("A7"), Some([-1, 0, 2, 0, 2, 0]));
        assert_eq!(f("Em7"), Some([0, 2, 0, 0, 0, 0]));
        assert_eq!(f("A5"), Some([-1, 0, 2, 2, -1, -1]));
        assert_eq!(f("Dsus4"), Some([-1, -1, 0, 2, 3, 3]));
        assert_eq!(f("Asus2"), Some([-1, 0, 2, 2, 0, 0]));
        // enharmonics resolve to the same shape
        assert_eq!(f("A#"), f("Bb"));
        assert_eq!(f("Db"), f("C#"));
        // everything we claim to know is playable: within a 4-fret span and at least 3 strings
        for root in ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"] {
            for q in ["", "m", "7", "m7", "maj7", "sus4", "sus2", "5"] {
                let name = format!("{root}{q}");
                let Some(sh) = diagram::lookup(&name) else {
                    assert_eq!(q, "sus2", "only the high sus2 shapes are left out: {name}");
                    continue;
                };
                let fr: Vec<i8> = sh.frets.iter().copied().filter(|x| *x > 0).collect();
                if let (Some(lo), Some(hi)) = (fr.iter().min(), fr.iter().max()) {
                    assert!(hi - lo <= 3, "{name} {sh:?}");
                }
                assert!(sh.frets.iter().filter(|x| **x >= 0).count() >= 3, "{name}");
                assert!(sh.frets.iter().all(|x| *x <= 12), "{name}");
            }
        }
        // unknown / slash chords: no diagram rather than a wrong one
        for n in ["Cdim", "D/F#", "Gadd9", "Em9", "Hello", ""] {
            assert!(diagram::lookup(n).is_none(), "{n}");
        }
        let c = diagram::render("C", &diagram::lookup("C").unwrap());
        assert_eq!(c[0].trim(), "C");
        assert_eq!(c[1], "x     o   o");
        assert_eq!(c[2], "╒═╤═╤═╤═╤═╕");
        assert_eq!(c.len(), 7);
        let tab = diagram::tab(&[diagram::lookup("C"), diagram::lookup("Am"), diagram::lookup("Cdim")]);
        assert_eq!(tab[0], "e|-0---0---?--|");
        assert_eq!(tab[5], "E|-x---x---?--|");
    }

    #[test]
    fn capo_and_transposition_math() {
        // higher capo → lower shapes; wrap-around both ways
        assert_eq!(chords::capo_shift(2, 5, 0), -3);
        assert_eq!(chords::capo_shift(0, 0, 0), 0);
        assert_eq!(chords::capo_shift(3, 0, 1), 4);
        let t = |c: &str, n: i32, flats: bool| chords::transpose(c, n, flats);
        assert_eq!(t("Dm", -3, false), "Bm");
        assert_eq!(t("B", 1, false), "C");
        assert_eq!(t("C", -1, false), "B");
        assert_eq!(t("C", -13, false), "B");
        assert_eq!(t("A", 25, false), "A#");
        assert_eq!(t("A", 25, true), "Bb");
        // slash chords move both notes, spelling follows the key
        assert_eq!(t("D/F#", 1, false), "D#/G");
        assert_eq!(t("D/F#", 1, true), "Eb/G");
        assert_eq!(t("D/F#", -2, true), "C/E");
        assert_eq!(t("C#m7/G#", 2, true), "Ebm7/Bb");
        assert_eq!(t("Bb/D", 12, true), "Bb/D");
        // sounding key stays put when only the capo changes; the shapes' key moves
        let k = chords::parse_key("Dm").unwrap();
        let (sheet_capo, new_capo) = (2u8, 4u8);
        let sounding = k.shift(sheet_capo as i32);
        let shapes = k.shift(chords::capo_shift(sheet_capo, new_capo, 0));
        assert_eq!(sounding.short(), "Em");
        assert_eq!(shapes.short(), "Cm"); // capo 4 → play Cm shapes → sounds Em
        assert_eq!(shapes.shift(new_capo as i32).short(), sounding.short());
        assert!(shapes.prefers_flats()); // Cm → flat spelling
    }

    fn cfg_run(file: &str, argv: &[&str]) -> (cli::Args, Vec<String>) {
        let path = tmp("cfg").join(format!("{}.toml", util::fnv1a(file)));
        std::fs::write(&path, file).unwrap();
        let mut full = vec!["x", "--config", path.to_str().unwrap()];
        full.extend_from_slice(argv);
        let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(full);
        let mut args = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
        let mut warns = Vec::new();
        config::apply(&mut args, &m, &path, true, &mut |w| warns.push(w));
        (args, warns)
    }

    #[test]
    fn config_precedence_and_validation() {
        let file = "player = \"spotifyd\"\noffset = -0.5\ntint = 0.25\ncava = true\ncava_bars = 32\nvolume_step = 10\ninterlude_threshold_secs = 12\nlyrics_retries = 5\nminimal_view = true\nmouse = false\nchords = true\nchords_ug = true\nthirdpanel = true\n";
        // file beats built-in defaults
        let (a, w) = cfg_run(file, &[]);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((a.player.as_str(), a.offset, a.tint, a.cava, a.cava_bars), ("spotifyd", -0.5, 0.25, true, 32));
        assert_eq!((a.volume_step, a.interlude_secs, a.lyrics_retries), (10, 12.0, 5));
        assert!(a.minimal && a.no_mouse && a.chords && a.chords_ug && a.thirdpanel);
        // flags beat the file
        let (a, _) = cfg_run(file, &["--player", "vlc", "--tint", "0.9", "--volume-step", "2", "--lyrics-retries", "0"]);
        assert_eq!((a.player.as_str(), a.tint, a.volume_step, a.lyrics_retries), ("vlc", 0.9, 2, 0));
        assert_eq!(a.cava_bars, 32, "settings not given as flags still come from the file");
        // defaults when nothing is set
        let (a, w) = cfg_run("", &[]);
        assert!(w.is_empty());
        assert_eq!((a.player.as_str(), a.volume_step, a.interlude_secs, a.lyrics_retries, a.minimal, a.no_mouse), ("spotify", 5, 8.0, 3, false, false));
        // unknown key warns (with its line), valid keys still apply
        let (a, w) = cfg_run("tint = 0.3\nfoo = 1\n", &[]);
        assert_eq!(a.tint, 0.3);
        assert!(w.len() == 1 && w[0].contains(":2:") && w[0].contains("unknown key 'foo'"), "{w:?}");
        // invalid values: key + line reported, default kept, the rest unaffected
        let (a, w) = cfg_run("# c\ntint = 7\nvolume_step = \"loud\"\nlyrics_retries = -1\ncava = 1\nplayer = \"\"\noffset = 1.5\n", &[]);
        assert_eq!((a.tint, a.volume_step, a.lyrics_retries, a.cava, a.player.as_str(), a.offset), (1.0, 5, 3, false, "spotify", 1.5));
        assert_eq!(w.len(), 5, "{w:?}");
        assert!(w[0].contains(":2:") && w[0].contains("'tint'"));
        assert!(w[1].contains(":3:") && w[1].contains("'volume_step'"));
        // broken TOML: warns, everything stays default
        let (a, w) = cfg_run("tint = = 3\n", &[]);
        assert_eq!(a.tint, 1.0);
        assert!(w.len() == 1 && w[0].contains("ignoring the file"), "{w:?}");
        // a missing default file is silent, a missing explicit one is not
        let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x"]);
        let mut a = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
        let mut w = Vec::new();
        let nowhere = tmp("cfg").join("nope.toml");
        assert!(!config::apply(&mut a, &m, &nowhere, false, &mut |x| w.push(x)));
        assert!(w.is_empty());
        config::apply(&mut a, &m, &nowhere, true, &mut |x| w.push(x));
        assert_eq!(w.len(), 1);
        // print-config round-trips through the parser
        let text = config::render(&a, &nowhere, false);
        let table: toml::Table = text.parse().unwrap();
        assert!(config::KEYS.iter().all(|k| table.contains_key(k.name) || k.name == "chords_dir" || k.name == "translate_fallback_key"), "{text}");
        assert!(text.contains("# chords_dir"));
    }

    #[test]
    fn keymap_has_no_conflicts_and_help_is_generated_from_it() {
        use app::{Action as A, BINDS, GROUPS, MOUSE};
        use crossterm::event::KeyCode;
        // no key is bound twice (m / M, t / T are different keys)
        let mut seen = std::collections::HashSet::new();
        for b in BINDS {
            for c in b.codes {
                assert!(seen.insert(format!("{c:?}")), "{c:?} bound twice");
            }
        }
        assert_eq!(app::action_for(KeyCode::Char('m')), Some(A::Mute));
        assert_eq!(app::action_for(KeyCode::Char('M')), Some(A::Minimal));
        assert_eq!(app::action_for(KeyCode::Char('?')), Some(A::Help));
        assert_eq!(app::action_for(KeyCode::Esc), Some(A::Quit));
        assert_eq!(app::action_for(KeyCode::Char('z')), Some(A::Shuffle));
        assert_eq!(app::action_for(KeyCode::Char('x')), None);
        // every action can be triggered by some key
        for a in [
            A::Quit, A::PlayPause, A::Next, A::Prev, A::Shuffle, A::Repeat, A::SeekBack, A::SeekFwd, A::SyncEarlier,
            A::SyncLater, A::VolUp, A::VolDown, A::Mute, A::ToggleLyrics, A::ToggleBig,
            A::ToggleCava, A::ToggleChords, A::TransposeUp, A::TransposeDown, A::CapoDown,
            A::CapoUp, A::CapoReset, A::Diagrams, A::Minimal, A::Help,
        ] {
            assert!(BINDS.iter().any(|b| b.action == a), "{a:?} has no key");
        }
        assert!(BINDS.iter().all(|b| GROUPS.contains(&b.group)));
        assert!(MOUSE.iter().all(|m| GROUPS.contains(&m.0)));
        // the overlay lists every row of both tables
        let th = theme_from(&["--terminal-colors"], None);
        let text: String = ui::help_blocks(&th)
            .iter()
            .flatten()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>() + "\n")
            .collect();
        for b in BINDS {
            assert!(text.contains(b.keys) && text.contains(b.desc), "{} missing from help", b.keys);
        }
        for m in MOUSE {
            assert!(text.contains(m.1) && text.contains(m.2));
        }
    }


    #[test]
    fn korean_song_replaces_romanized_hit_with_hangul() {
        let roman = r#"{"syncedLyrics":"[00:01.00]saranghae neomu\n[00:05.00]gaji ma","duration":200}"#;
        let hangul = r#"{"syncedLyrics":"[00:01.00]사랑해 너무\n[00:05.00]가지 마","duration":200}"#;
        let run = |artist: &str, title: &str, base: &str| {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let c = reqwest::Client::new();
            rt.block_on(lyrics::fetch_with(&c, base, artist, title, "", 200.0, fast(), false))
        };
        let site = || {
            scripted_site(vec![
                ("api/get", vec![R::Http(200, "", roman.into())]),
                ("api/search", vec![R::Http(200, "", format!("[{roman},{hangul}]"))]),
            ])
        };
        // Korean title: the romanized exact hit is not final, the search runs and Hangul wins
        let (base, log) = site();
        let l = run("방탄소년단", "사랑해", &base);
        assert!(matches!(&l, lyrics::Lyrics::Synced(v) if v[0].1.starts_with("사랑")), "{l:?}");
        assert!(log.lock().unwrap().iter().any(|r| r.contains("api/search")));
        // not Korean: the first synced hit is kept and no search happens
        let (base, log) = site();
        let l = run("Someone", "Saranghae", &base);
        assert!(matches!(&l, lyrics::Lyrics::Synced(v) if v[0].1.starts_with("sarang")), "{l:?}");
        assert!(!log.lock().unwrap().iter().any(|r| r.contains("api/search")));
    }

    #[test]
    fn meta_grid_aligns_the_right_column() {
        let sp = |s: &str| vec![ratatui::text::Span::raw(s.to_string())];
        let text = |l: &ratatui::text::Line| l.spans.iter().map(|s| s.content.to_string()).collect::<String>();
        let left = || vec![sp("Track No  01-03"), sp("Release   1991-08-12")];
        let right = |n: usize| {
            ["Tuning: Drop D", "Capo: 2 (Am)", "BPM: 120", "Key: E Minor / 9A"]
                .iter()
                .take(n)
                .map(|s| sp(s))
                .collect::<Vec<_>>()
        };
        let rows: Vec<String> = ui::meta_grid(left(), right(4), 40).iter().map(text).collect();
        // start column = 40 - widest right item (17)
        assert_eq!(rows[0], format!("{:<23}Tuning: Drop D", "Track No  01-03"));
        assert_eq!(rows[1], format!("{:<23}Capo: 2 (Am)", "Release   1991-08-12"));
        assert_eq!(rows[2], format!("{:<23}BPM: 120", ""));
        assert_eq!(rows[3], format!("{:<23}Key: E Minor / 9A", ""));
        assert_eq!(rows.len(), 4);
        // only the key: one right row, left side untouched below it
        let rows: Vec<String> = ui::meta_grid(left(), vec![sp("Key: E Minor / 9A")], 40).iter().map(text).collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].ends_with("Key: E Minor / 9A") && rows[0].chars().count() == 40);
        assert_eq!(rows[1], "Release   1991-08-12");
        // nothing on the right
        assert_eq!(ui::meta_grid(left(), vec![], 40).len(), 2);
        // too narrow: the right items follow below, indented under the values
        let rows: Vec<String> = ui::meta_grid(left(), right(2), 30).iter().map(text).collect();
        assert_eq!(rows, ["Track No  01-03", "Release   1991-08-12", "             Tuning: Drop D", "             Capo: 2 (Am)"]);
    }

    #[test]
    fn shuffle_and_loop_output_is_parsed() {
        use player::Loop;
        assert_eq!(player::parse_shuffle("On\n"), Some(true));
        assert_eq!(player::parse_shuffle("Off\n"), Some(false));
        assert_eq!(player::parse_shuffle("No players found\n"), None);
        assert_eq!(player::parse_shuffle(""), None);
        assert_eq!(player::parse_loop("None\n"), Some(Loop::None));
        assert_eq!(player::parse_loop("Track\n"), Some(Loop::Track));
        assert_eq!(player::parse_loop("Playlist\n"), Some(Loop::Playlist));
        assert_eq!(player::parse_loop("Some error"), None);
        // off -> playlist -> track -> off, with the argument playerctl expects
        assert_eq!(Loop::None.next(), Loop::Playlist);
        assert_eq!(Loop::Playlist.next(), Loop::Track);
        assert_eq!(Loop::Track.next(), Loop::None);
        assert_eq!(Loop::Playlist.arg(), "Playlist");
        // the metadata format carries neither; the poller stamps them on afterwards
        assert!(snap("A", "T", "Al").shuffle.is_none() && snap("A", "T", "Al").repeat.is_none());
    }

    #[test]
    fn member_parts_keys_and_shuffle_binds_exist() {
        assert!(app::Setting::ALL.contains(&app::Setting::MemberParts));
        assert_eq!(app::Setting::MemberParts.config_keys(), ["member_parts"]);
        assert_eq!(app::action_for(crossterm::event::KeyCode::Char('z')), Some(app::Action::Shuffle));
        assert_eq!(app::action_for(crossterm::event::KeyCode::Char('r')), Some(app::Action::Repeat));
        let (a, _) = cfg_run("member_parts = true\n", &[]);
        assert!(a.member_parts);
        let (a, _) = cfg_run("member_parts = true\n", &["--member-parts"]);
        assert!(a.member_parts);
        let (a, _) = cfg_run("", &[]);
        assert!(!a.member_parts, "off by default");
    }

    #[test]
    fn genius_headers_become_member_lists() {
        let h = |s: &str| genius::parse_header(s);
        assert_eq!(h("[Verse 1: Jimin]"), Some(vec!["Jimin".to_string()]));
        assert_eq!(h("[Chorus: Jungkook, V]"), Some(vec!["Jungkook".to_string(), "V".to_string()]));
        assert_eq!(h("[Pre-Chorus: RM & SUGA]"), Some(vec!["RM".to_string(), "SUGA".to_string()]));
        assert_eq!(h("[Bridge: Jin (진) and J-Hope]"), Some(vec!["Jin".to_string(), "J-Hope".to_string()]));
        assert_eq!(h("[Outro]"), Some(vec![]));
        assert_eq!(h("just a lyric"), None);
    }

    const GENIUS_PAGE: &str = r#"<html><body>
<div class="Lyrics__Container" data-lyrics-container="true" class="x"><div data-exclude-from-selection="true"><div>Contributors</div></div>[Intro]<br/>Ooh<br/><a href="/1"><span>[Verse 1: Jimin]</span></a><br/>I&#x27;m in love &amp; blind<br/>Walking down the road<br/><br/>[Chorus: Jungkook, V]<br/>Stay with me tonight<br/></div>
<div data-lyrics-container="true">[Verse 2: RM &amp; SUGA]<br>Rain on the window<br>Nothing left to say</div>
<div class="footer">[Verse 9: Nobody]<br>ignored</div></body></html>"#;

    fn mp(members: &[&str], text: &str) -> genius::Part {
        genius::Part { members: members.iter().map(|m| m.to_string()).collect(), text: text.into() }
    }

    #[test]
    fn genius_page_parses_sections_and_inherits_members() {
        let parts = genius::parse_page(GENIUS_PAGE);
        assert_eq!(
            parts,
            vec![
                mp(&[], "Ooh"),
                mp(&["Jimin"], "I'm in love & blind"),
                mp(&["Jimin"], "Walking down the road"),
                mp(&["Jungkook", "V"], "Stay with me tonight"),
                mp(&["RM", "SUGA"], "Rain on the window"),
                mp(&["RM", "SUGA"], "Nothing left to say"),
            ]
        );
    }

    #[test]
    fn genius_lines_are_attributed_to_synced_lines_in_order() {
        let parts = genius::parse_page(GENIUS_PAGE);
        let lyrics: Vec<String> = [
            "Ooh",
            "I'm in love and blind", // wording differs slightly
            "something the page doesn't have",
            "Walking down the road",
            "Stay with me tonight",
            "Rain on the window",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let m = genius::attribute(&parts, &lyrics);
        assert!(m[0].is_empty(), "intro line has no member");
        assert_eq!(m[1], ["Jimin"]);
        assert!(m[2].is_empty());
        assert_eq!(m[3], ["Jimin"]);
        assert_eq!(m[4], ["Jungkook", "V"]);
        assert_eq!(m[5], ["RM", "SUGA"]);
        // stable colour per name, whatever the case
        assert_eq!(genius::color("Jimin"), genius::color("jimin"));
    }

    #[test]
    fn genius_lookup_with_fake_site_and_cache() {
        let search = serde_json::json!({"meta":{"status":200},"response":{"sections":[
            {"type":"top_hit","hits":[{"type":"song","result":{"title":"Other Song","path":"/Bts-other-lyrics","primary_artist":{"name":"BTS"}}}]},
            {"type":"song","hits":[
                {"type":"song","result":{"title":"Dynamite (Türkçe Çeviri)","path":"/Genius-tr-dynamite-lyrics","primary_artist":{"name":"Genius Türkçe Çeviri"}}},
                {"type":"song","result":{"title":"Dynamite","url":"https://genius.com/Bts-dynamite-lyrics","primary_artist":{"name":"BTS"}}}
            ]}]}});
        let artist = "BTS";
        let title = "Dynamite GENIUSTEST";
        assert_eq!(genius::best_hit(&search, artist, "Dynamite").as_deref(), Some("/Bts-dynamite-lyrics"));
        assert_eq!(genius::best_hit(&search, artist, "Totally Different"), None);
        assert_eq!(genius::best_hit(&search, "Someone Else", "Dynamite"), None);

        let cache = util::cache_dir().join(format!("members-{:016x}.json", util::fnv1a(&format!("{artist}\0{title}"))));
        let _ = std::fs::remove_file(&cache);
        let (base, hits) = fake_site(vec![
            ("search/multi", 200, search.to_string()),
            ("Bts-dynamite-lyrics", 200, GENIUS_PAGE.into()),
        ]);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let c = reqwest::Client::new();
        // the fake title differs from the hit's ("Dynamite GENIUSTEST" vs "Dynamite"), so look up the plain title
        let title2 = "Dynamite";
        let cache2 = util::cache_dir().join(format!("members-{:016x}.json", util::fnv1a(&format!("{artist}\0{title2}"))));
        let _ = std::fs::remove_file(&cache2);
        let got = rt.block_on(genius::lookup(&c, &base, artist, title2)).unwrap().expect("parts");
        assert_eq!(got.len(), 6);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2, "search + song page");
        // second time: straight from the cache
        let again = rt.block_on(genius::lookup(&c, &base, artist, title2)).unwrap().expect("cached");
        assert_eq!(again, got);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
        let _ = std::fs::remove_file(&cache2);

        // nothing found: remembered as a miss; a server error is an Err and not cached
        let _ = std::fs::remove_file(&cache);
        let (base2, hits2) = fake_site(vec![("search/multi", 200, r#"{"response":{"sections":[]}}"#.into())]);
        assert_eq!(rt.block_on(genius::lookup(&c, &base2, artist, title)).unwrap(), None);
        assert_eq!(rt.block_on(genius::lookup(&c, &base2, artist, title)).unwrap(), None);
        assert_eq!(hits2.load(std::sync::atomic::Ordering::SeqCst), 1, "negative cache");
        let _ = std::fs::remove_file(&cache);
        let (base3, _) = fake_site(vec![("search/multi", 500, "no".into())]);
        assert!(rt.block_on(genius::lookup(&c, &base3, artist, title)).is_err());
        assert!(!cache.exists());
    }

    #[test]
    fn percent() {
        assert_eq!(util::percent_decode("/a%20b/c%C3%A9.jpg"), "/a b/cé.jpg");
    }
}
