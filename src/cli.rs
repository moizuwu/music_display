use clap::Parser;
use std::path::PathBuf;

/// Where the annotations panel gets its notes.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AnnotationMode {
    // plain comments: doc comments on variants make clap print --help in its long layout
    // Genius annotations of the song (free client token).
    #[default]
    Genius,
    // Short notes written by the AI engine, labelled as AI-generated.
    Ai,
    // Genius first, then the AI adds what Genius does not already say (labelled).
    #[value(name = "genius+ai", alias = "genius-ai", alias = "both")]
    GeniusAi,
}

impl AnnotationMode {
    pub const NAMES: &'static [&'static str] = &["genius", "ai", "genius+ai"];

    /// The label of the panel title and the GUI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Genius => "Genius",
            Self::Ai => crate::annotations::AI_LABEL,
            Self::GeniusAi => "Genius + AI",
        }
    }

    /// The next mode in the settings menu.
    pub fn next(self) -> Self {
        match self {
            Self::Genius => Self::Ai,
            Self::Ai => Self::GeniusAi,
            Self::GeniusAi => Self::Genius,
        }
    }

    pub fn uses_genius(self) -> bool {
        self != Self::Ai
    }

    pub fn uses_ai(self) -> bool {
        self != Self::Genius
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "genius" => Some(Self::Genius),
            "ai" => Some(Self::Ai),
            "genius+ai" | "genius-ai" | "both" => Some(Self::GeniusAi),
            _ => None,
        }
    }
}

/// How a long annotation that does not fit the panel moves.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AnnotationScroll {
    // plain comments: doc comments on variants make clap print --help in its long layout
    // Scrolled by hand (mouse wheel over the panel, `e` / `E`).
    #[default]
    Manual,
    // Scrolls slowly through the note while the line is sung.
    Auto,
}

impl AnnotationScroll {
    pub const NAMES: &'static [&'static str] = &["manual", "auto"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "manual" => Some(Self::Manual),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// Where the translation sits when it is shown next to the original line.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranslationPosition {
    // plain comments: doc comments on variants make clap print --help in its long layout
    // Under the original line.
    #[default]
    Below,
    // Above the original line.
    Above,
}

impl TranslationPosition {
    pub const NAMES: &'static [&'static str] = &["below", "above"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "below" => Some(Self::Below),
            "above" => Some(Self::Above),
            _ => None,
        }
    }

    /// The next (`dir` = 1) or previous value, wrapping, for the settings rows.
    pub fn step(self, dir: i32) -> Self {
        let n = Self::NAMES.len() as i32;
        Self::parse(Self::NAMES[(self as i32 + dir).rem_euclid(n) as usize]).unwrap_or_default()
    }
}

/// Where the "who sings this line" tag sits.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MemberPosition {
    // plain comments: doc comments on variants make clap print --help in its long layout
    // The terminal UI puts it before the line, the window under it.
    #[default]
    Auto,
    // Under the line (and its translation).
    Below,
    // Above the line (and its translation).
    Above,
    // In front of the line, on the same row.
    Inline,
}

impl MemberPosition {
    pub const NAMES: &'static [&'static str] = &["auto", "below", "above", "inline"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "below" => Some(Self::Below),
            "above" => Some(Self::Above),
            "inline" => Some(Self::Inline),
            _ => None,
        }
    }

    /// The next (`dir` = 1) or previous value, wrapping, for the settings rows.
    pub fn step(self, dir: i32) -> Self {
        let n = Self::NAMES.len() as i32;
        Self::parse(Self::NAMES[(self as i32 + dir).rem_euclid(n) as usize]).unwrap_or_default()
    }
}

/// Which engine translates the lyrics.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranslateEngine {
    // plain comments: doc comments on variants make clap print --help in its long layout
    // Google Translate (unofficial endpoint), LibreTranslate as the fallback.
    #[default]
    Standard,
    // A language model, one request per song (see the `[ai]` settings).
    Ai,
}

impl TranslateEngine {
    pub const NAMES: &'static [&'static str] = &["standard", "ai"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "standard" => Some(Self::Standard),
            "ai" => Some(Self::Ai),
            _ => None,
        }
    }
}

/// What the lyrics panel shows when a translation exists.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranslateMode {
    // plain comments: doc comments on variants make clap print --help in its long layout
    // The original line only.
    #[default]
    Off,
    // The original line with its translation underneath.
    Both,
    // The translation replaces the original line.
    Translated,
}

impl TranslateMode {
    pub const NAMES: &'static [&'static str] = &["off", "both", "translated"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "both" => Some(Self::Both),
            "translated" => Some(Self::Translated),
            _ => None,
        }
    }

    /// Next / previous mode, wrapping.
    pub fn step(self, dir: i32) -> Self {
        let n = Self::NAMES.len() as i32;
        Self::parse(Self::NAMES[(self as i32 + dir).rem_euclid(n) as usize]).unwrap_or_default()
    }
}

impl Args {
    /// The mode the app starts in: `--translate-mode` / `translate_mode`, else `--translate`.
    pub fn start_translate_mode(&self) -> TranslateMode {
        self.translate_mode
            .unwrap_or(if self.translate { TranslateMode::Translated } else { TranslateMode::Off })
    }
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "music_display",
    version = env!("MD_VERSION"),
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

    /// Open the window instead of the terminal UI (needs a build with `--features gui`)
    /// [no config key]
    #[arg(long)]
    pub gui: bool,

    /// Start from a named layout: default, no-lyrics, cava, under-lyrics, minimal. The flags
    /// above are shortcuts for these [config: [layout] preset]
    #[arg(long, value_name = "NAME")]
    pub layout_preset: Option<String>,

    /// Forget the saved `[layout]` (panel order, sizes, hidden fields and buttons) and start
    /// from the default [no config key]
    #[arg(long)]
    pub reset_layout: bool,

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

    /// Put the song in the terminal window's title: off, title, or artist-title
    /// [config: window_title]
    #[arg(long, default_value = "off", value_parser = ["off", "title", "artist-title"], value_name = "MODE")]
    pub window_title: String,

    /// Do not keep finished translations on disk (by default a song is translated once and
    /// then read from the cache folder) [config: cache_translations = false]
    #[arg(long)]
    pub no_cache_translations: bool,

    /// Light up the active lyric line word by word while it is sung (toggle with `a`). lrclib
    /// only has line times, so the progress inside a line is a guess [config: karaoke]
    #[arg(long)]
    pub karaoke: bool,

    /// Show guitar chords above the lyrics (toggle with `g`). Sheets are read from
    /// --chords-dir as `Artist - Title.cho` (ChordPro) or `.txt` (chords over lyrics)
    /// [config: chords]
    #[arg(long)]
    pub chords: bool,

    /// How many percent of the lyric lines must match a chord sheet before its chords are
    /// shown (lower shows weaker matches, 0 always) [config: chords_match]
    #[arg(long, default_value_t = crate::chords::DEFAULT_MATCH, value_parser = clap::value_parser!(u32).range(0..=100), value_name = "PERCENT")]
    pub chords_match: u32,

    /// Folder with your chord sheets [default: ~/.local/share/music_display/chords] [config: chords_dir]
    #[arg(long, value_name = "DIR")]
    pub chords_dir: Option<PathBuf>,

    /// Also look chords up on Ultimate Guitar when there is no local sheet (unofficial,
    /// opt-in; results are cached) [config: chords_ug]
    #[arg(long)]
    pub chords_ug: bool,

    /// A sheet that says "capo 2" is shown with capo 0, so its chords are the sounding ones
    /// (your own `[` `]` choice for a track still wins) [config: auto_zero_capo]
    #[arg(long)]
    pub auto_zero_capo: bool,

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

    /// How translated lyrics are shown: off, both (original, then the translation underneath)
    /// or translated (the translation replaces the line). Wins over --translate [config: translate_mode]
    #[arg(long, value_enum, value_name = "MODE")]
    pub translate_mode: Option<TranslateMode>,

    /// With the translation next to the original: below the line or above it [config: translation_position]
    #[arg(long, value_enum, default_value = "below", value_name = "WHERE")]
    pub translation_position: TranslationPosition,

    /// Where the member tag goes: auto (terminal: in front of the line, window: under it), below,
    /// above or inline [config: member_position]
    #[arg(long, value_enum, default_value = "auto", value_name = "WHERE")]
    pub member_position: MemberPosition,

    /// Look the genre up on MusicBrainz when neither the player nor the file's tags have one
    /// [config: genre_lookup]
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, value_name = "BOOL")]
    pub genre_lookup: bool,

    /// Translation engine: standard (Google, LibreTranslate fallback) or ai (a language model,
    /// off by default; see --ai-provider) [config: translate_engine]
    #[arg(long, value_enum, default_value = "standard", value_name = "ENGINE")]
    pub translate_engine: TranslateEngine,

    /// AI provider: openai-compatible (Ollama, LM Studio: free, local), gemini, claude, openai
    /// [config: [ai] provider]
    #[arg(long, default_value = "openai-compatible", value_name = "PROVIDER")]
    pub ai_provider: String,

    /// AI model name, e.g. `llama3.1` (Ollama), `gemini-2.5-flash`, `gpt-4o-mini` [config: [ai] model]
    #[arg(long, default_value = "", value_name = "MODEL")]
    pub ai_model: String,

    /// AI endpoint; empty = the provider's default (Ollama: http://localhost:11434/v1)
    /// [config: [ai] endpoint]
    #[arg(long, default_value = "", value_name = "URL")]
    pub ai_endpoint: String,

    /// Name of the environment variable holding the API key (the key itself is never read from
    /// a flag or the file, printed or logged); empty = the provider's usual one [config: [ai] api_key_env]
    #[arg(long, default_value = "", value_name = "VAR")]
    pub ai_key_env: String,

    /// Show the annotations panel (experimental): what the active line means, from Genius
    /// [config: annotations]
    #[arg(long)]
    pub annotations: bool,

    /// Source of the annotations: genius (free client token), ai (notes by the AI engine,
    /// labelled) or genius+ai (Genius, then AI extras that Genius does not already say)
    /// [config: annotations_mode]
    #[arg(long, value_enum, default_value = "genius", value_name = "MODE")]
    pub annotations_mode: AnnotationMode,

    /// Longest annotation shown, in characters (cut at a word with "…"); 0 = the whole note
    /// [config: annotation_max_chars]
    #[arg(long, default_value_t = 0, value_name = "N")]
    pub annotation_max_chars: usize,

    /// A note longer than the panel: manual (wheel, e / E) or auto (scrolls while the line is
    /// sung) [config: annotation_scroll]
    #[arg(long, value_enum, default_value = "manual", value_name = "MODE")]
    pub annotation_scroll: AnnotationScroll,

    /// Genius API client token (free: genius.com/api-clients). Better in the config file or the
    /// GENIUS_TOKEN environment variable than on the command line [config: genius_token]
    #[arg(long, default_value = "", value_name = "TOKEN")]
    pub genius_token: String,

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

    /// spotifyd is the player on this PC (set it up once with --setup): its `spotify_device`
    /// name then counts as "this PC". Off = only the Spotify app on this computer does
    /// [config: use_spotifyd]
    #[arg(long)]
    pub use_spotifyd: bool,

    /// Leave the spotifyd that this program started running when it quits (by default it is
    /// stopped again) [config: kill_spotifyd_on_exit = false]
    #[arg(long)]
    pub no_kill_spotifyd: bool,

    /// Hide the library button at the top left (it is only shown while Spotify is on; open the
    /// library with `o` instead) [config: spotify_button = false]
    #[arg(long)]
    pub no_spotify_button: bool,

    /// Hide the Queue tab of the library browser [config: spotify_queue_tab = false]
    #[arg(long)]
    pub no_spotify_queue_tab: bool,

    /// Log in to Spotify in the browser, save the token and exit [no config key]
    #[arg(long)]
    pub spotify_login: bool,

    /// One-time guided setup for spotifyd: check it is installed, `spotifyd authenticate`, the
    /// browser login, write spotifyd.conf and the config entries (backups first), then check
    /// the device shows up, and exit [no config key]
    #[arg(long)]
    pub setup: bool,

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
