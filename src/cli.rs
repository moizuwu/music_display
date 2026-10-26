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

    /// Languages that are never translated, comma separated, e.g. `sv,fi` (lines the service
    /// detects as one of them stay as written) [config: translate_skip]
    #[arg(long, default_value = "", value_name = "CODES")]
    pub translate_skip: String,

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

    /// Spotify Web API: library browser (`o`), like (`f`), play here (`H`), and showing /
    /// controlling playback on other devices (the phone). Log in once with --spotify-login
    /// [config: spotify]
    #[arg(long)]
    pub spotify: bool,

    /// Client ID of your Spotify developer app (only needed for --spotify-login) [config: spotify_client_id]
    #[arg(long, value_name = "ID")]
    pub spotify_client_id: Option<String>,

    /// Spotify Connect name of spotifyd on this PC (its `device_name`); empty = only the Spotify
    /// app on this computer counts as "this PC" [config: spotify_device]
    #[arg(long, default_value = "", value_name = "NAME")]
    pub spotify_device: String,

    /// Log in to Spotify in the browser, save the token and exit [no config key]
    #[arg(long)]
    pub spotify_login: bool,

    /// Forget the saved Spotify token and exit [no config key]
    #[arg(long)]
    pub spotify_logout: bool,

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
