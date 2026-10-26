use crate::cli::Args;
use clap::parser::ValueSource;
use clap::ArgMatches;
use std::path::{Path, PathBuf};

pub enum Kind {
    Bool,
    Int(i64, i64),
    Float(f64, f64),
    Str,
    /// A string that may be empty.
    Text,
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
    k!("translate_skip", Kind::Text, "translate_skip"),
    k!("translate_fallback", Kind::Bool, "translate_fallback"),
    k!("translate_fallback_url", Kind::Str, "translate_fallback_url"),
    k!("translate_fallback_key", Kind::Str, "translate_fallback_key"),
    k!("minimal_view", Kind::Bool, "minimal"),
    k!("mouse", Kind::Bool, "no_mouse"),
    k!("spotify", Kind::Bool, "spotify"),
    k!("spotify_client_id", Kind::Str, "spotify_client_id"),
    k!("spotify_device", Kind::Str, "spotify_device"),
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
        Kind::Text => v.as_str().map(|_| ()).ok_or("expected a string".into()),
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
        "translate_skip" => args.translate_skip = st(),
        "translate_fallback" => args.translate_fallback = b(),
        "translate_fallback_url" => args.translate_fallback_url = st(),
        "translate_fallback_key" => args.translate_fallback_key = Some(st()),
        "minimal_view" => args.minimal = b(),
        "mouse" => args.no_mouse = !b(),
        "spotify" => args.spotify = b(),
        "spotify_client_id" => args.spotify_client_id = Some(st()),
        "spotify_device" => args.spotify_device = st(),
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
    line("translate_skip", q(&args.translate_skip));
    line("translate_fallback", args.translate_fallback.to_string());
    line("translate_fallback_url", q(&args.translate_fallback_url));
    if args.translate_fallback_key.is_some() {
        line("# translate_fallback_key", "\"…\"  # set (not shown)".into());
    }
    line("minimal_view", args.minimal.to_string());
    line("mouse", (!args.no_mouse).to_string());
    line("spotify", args.spotify.to_string());
    match &args.spotify_client_id {
        Some(id) => line("spotify_client_id", q(id)),
        None => line("# spotify_client_id", "\"…\"  # your Spotify app's Client ID".into()),
    }
    line("spotify_device", q(&args.spotify_device));
    o
}
