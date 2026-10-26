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
    BigScale,
    LineSpacing,
    LyricPad,
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
    Library,
    Like,
    PlayHere,
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

/// Language check kept failing for a song on a never-translate list: asked again this often,
/// this many times.
const HOLD_RETRY_SECS: u64 = 20;
const HOLD_RETRIES: u32 = 6;

pub const GROUPS: [&str; 5] = ["Playback", "Lyrics", "Chords", "View", "Spotify"];

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
    bind!("Lyrics", "x", "big-font size 1 / 2 / 3", [KeyCode::Char('x')], BigScale),
    bind!("Lyrics", "i", "line spacing 0-3", [KeyCode::Char('i')], LineSpacing),
    bind!("Lyrics", "u", "side padding 0-10", [KeyCode::Char('u')], LyricPad),
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
    bind!("Spotify", "o", "library: playlists, liked, albums…", [KeyCode::Char('o')], Library),
    bind!("Spotify", "f", "like / unlike this song", [KeyCode::Char('f')], Like),
    bind!("Spotify", "H", "play here (move to this PC)", [KeyCode::Char('H')], PlayHere),
];

/// Keys inside the library browser (documented in the help and the README).
pub const LIBRARY_KEYS: &[(&str, &str)] = &[
    ("Tab / 1-5", "switch tab"),
    ("Enter", "play"),
    ("→ / ←", "open / back"),
    ("a / A", "queue / add to playlist"),
    ("f / r", "like / reload"),
];

/// The library browser's tabs, in order.
pub const TABS: [&str; 5] = ["Recent", "Playlists", "Liked", "Albums", "Artists"];
const TAB_LIKED: usize = 2;
const TAB_PLAYLISTS: usize = 1;

/// One list in the library browser: a tab, or an album / playlist / artist opened from one.
#[derive(Default)]
pub struct Level {
    pub title: String,
    /// The context its tracks play in (`liked` for Liked Songs).
    pub uri: String,
    pub summary: String,
    pub items: Vec<spotify::Item>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub sel: usize,
    pub top: usize,
}

/// `o`: the Spotify library browser.
#[derive(Default)]
pub struct Library {
    pub open: bool,
    pub tab: usize,
    pub tabs: Vec<Level>,
    /// Albums / playlists / artists opened from the tab, innermost last.
    pub stack: Vec<Level>,
    /// "Add to playlist": the track (uri, name) and the list of your own playlists.
    pub picker: Option<(String, String, Level)>,
    pub note: Option<String>,
}

impl Library {
    fn new() -> Self {
        let tabs = TABS
            .iter()
            .enumerate()
            .map(|(i, t)| Level {
                title: t.to_string(),
                uri: if i == TAB_LIKED { "liked".into() } else { String::new() },
                ..Default::default()
            })
            .collect();
        Library { tabs, ..Default::default() }
    }

    /// The list on screen.
    pub fn level(&self) -> &Level {
        match &self.picker {
            Some((_, _, l)) => l,
            None => self.stack.last().unwrap_or(&self.tabs[self.tab]),
        }
    }

    pub fn level_mut(&mut self) -> &mut Level {
        match &mut self.picker {
            Some((_, _, l)) => l,
            None => self.stack.last_mut().unwrap_or(&mut self.tabs[self.tab]),
        }
    }
}

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
    TranslateSkip,
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
        Setting::TranslateSkip,
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
            Setting::TranslateSkip => "Never translate",
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
            Setting::TranslateSkip => &["translate_skip"],
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
    /// The language the "Never translate" row is pointing at.
    skip_cursor: usize,
    /// Last settings-save problem, shown in the menu.
    pub settings_note: Option<String>,
    /// `K`: show translated lyrics.
    pub translate_on: bool,
    pub translate_state: TranslateState,
    /// The (song, language) request currently running, so toggling doesn't duplicate it.
    translate_inflight: Option<(u64, String)>,
    /// The task doing it, stopped when the song or the never-translate list changes.
    translate_task: Option<JoinHandle<()>>,
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

    /// Spotify Web API (`--spotify`); `None` when off.
    pub spot: Option<spotify::Api>,
    /// The last `/me/player` answer and when it came.
    pub remote: Option<(spotify::Remote, Instant)>,
    /// The last `playerctl` poll found a player.
    mpris_present: bool,
    /// Whether the song playing is in Liked Songs (`None` = unknown / not a Spotify track).
    pub liked: Option<bool>,
    /// The Spotify URI of the song playing (what `liked` is about).
    pub liked_uri: String,
    /// The last Spotify login / connection problem.
    pub spot_status: Option<String>,
    /// A short message in the footer (a Spotify command finished or failed).
    pub note: Option<(String, Instant)>,
    pub lib: Library,
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
            skip_cursor: 0,
            settings_note: None,
            translate_on: false,
            translate_state: TranslateState::Idle,
            translate_inflight: None,
            translate_task: None,
            translations: Default::default(),
            translate_base: None,
            translate_backend: "google",
            browse: 0.0,
            browse_at: Instant::now(),
            local_tags: Default::default(),
            player_seen: Instant::now(),
            tags_asked: Default::default(),
            spot: None,
            remote: None,
            mpris_present: false,
            liked: None,
            liked_uri: String::new(),
            spot_status: None,
            note: None,
            lib: Library::new(),
        };
        if app.args.spotify {
            let id = app.args.spotify_client_id.clone().unwrap_or_default();
            app.spot = Some(spotify::Api::new(app.client.clone(), &id, &app.args.spotify_device));
            if !spotify::logged_in() {
                app.spot_status = Some("not logged in: run music_display --spotify-login".into());
            }
        }
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
                self.set_mpris(false);
                // No MPRIS player: show what the Spotify API says plays (the phone…)
                if !self.remote_active() {
                    if self.snap.is_some() {
                        self.snap = None;
                        self.track_key = 0;
                        self.dirty = true;
                    }
                    if let Some(s) = self.remote_snapshot() {
                        self.on_snapshot(s);
                    }
                }
                self.check_liked();
            }
            AppEvent::Player(Some(s)) => {
                self.set_mpris(true);
                if self.remote_active() {
                    self.player_seen = Instant::now() - Duration::from_secs(10); // MPRIS takes over
                }
                self.on_snapshot(s);
                self.check_liked();
            }
            AppEvent::Spotify(r) => self.on_spotify(r),
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
                let held_msg = lines.as_ref().err().cloned();
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
                // a held song keeps its request: the task retries the language check itself
                let held = held_msg.as_deref().is_some_and(translate::is_held);
                if !held && self.translate_inflight.as_ref() == Some(&(key, lang.clone())) {
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
            if let Some(t) = self.translate_task.take() {
                t.abort();
            }

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
        // its language is on the never-translate list: settled until the list or the song changes
        if matches!(&self.translate_state, TranslateState::Failed(e) if translate::is_skipped(e)) {
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
            skip: self.skip_langs(),
        };
        if let Some(t) = self.translate_task.take() {
            t.abort();
        }
        self.translate_task = Some(tokio::spawn(async move {
            // language not detectable (rate-limited) while the never-translate list is in use:
            // say so once, then ask again every HOLD_RETRY_SECS
            let mut told = false;
            for attempt in 1..=HOLD_RETRIES {
                let mut r = translate::lines(&c, &cfg, &lang, &lines).await;
                if r.as_ref().is_err_and(|e| translate::is_held(e)) {
                    if attempt < HOLD_RETRIES {
                        if !told {
                            told = true;
                            let held = Err(translate::HELD.to_string());
                            let _ = tx.send(AppEvent::Translated { key, lang: lang.clone(), lines: held });
                        }
                        tokio::time::sleep(Duration::from_secs(HOLD_RETRY_SECS)).await;
                        continue;
                    }
                    r = Err(translate::HELD_FINAL.to_string());
                }
                let _ = tx.send(AppEvent::Translated { key, lang, lines: r });
                return;
            }
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

    /// The `translate_skip` codes, normalised (`SV, zh-CN` → `["sv", "zh"]`).
    pub fn skip_langs(&self) -> Vec<String> {
        self.args
            .translate_skip
            .split(',')
            .map(translate::base_code)
            .filter(|c| !c.is_empty())
            .collect()
    }

    /// Add / remove one language from the never-translate list.
    pub fn toggle_skip(&mut self, code: &str) {
        let mut all = self.skip_langs();
        let code = translate::base_code(code);
        match all.iter().position(|c| *c == code) {
            Some(i) => {
                all.remove(i);
            }
            None => all.push(code),
        }
        self.args.translate_skip = all.join(",");
        // what was translated under the old list is stale
        self.translations.clear();
        self.translate_inflight = None;
        if let Some(t) = self.translate_task.take() {
            t.abort(); // its answer was made under the old list
        }
        self.translate_state = TranslateState::Idle;
        self.snap_scroll = true;
        self.ensure_translation();
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
            Setting::TranslateSkip => {
                // `< sv [x] >`: ←/→ pick a language, space adds or removes it
                let code = LANGS[self.skip_cursor % LANGS.len()];
                let on = self.skip_langs().contains(&translate::base_code(code));
                let list = self.args.translate_skip.clone();
                format!("< {code} {} >{}", if on { "[x]" } else { "[ ]" }, if list.is_empty() { String::new() } else { format!("  ({list})") })
            }
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
            Setting::TranslateSkip => {
                if dir == 0 {
                    self.toggle_skip(LANGS[self.skip_cursor % LANGS.len()]);
                } else {
                    let n = LANGS.len() as i32;
                    self.skip_cursor = (self.skip_cursor as i32 + dir).rem_euclid(n) as usize;
                }
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
            ("translate_skip", S(self.args.translate_skip.clone())),
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
            KeyCode::Enter | KeyCode::Char(' ') if Setting::ALL[self.settings_sel] == Setting::TranslateSkip => {
                self.change_setting(Setting::TranslateSkip, 0)
            }
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
                if self.lib.open {
                    self.handle_library_key(k.code);
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
            Event::Mouse(m) if self.lib.open => match m.kind {
                MouseEventKind::ScrollUp => self.handle_library_key(KeyCode::Up),
                MouseEventKind::ScrollDown => self.handle_library_key(KeyCode::Down),
                _ => {}
            },
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
                        self.seek_to(target);
                    }
                } else if self.hits.shuffle.contains(pos) {
                    self.toggle_shuffle();
                } else if self.hits.repeat.contains(pos) {
                    self.cycle_repeat();
                } else if self.hits.prev.contains(pos) {
                    self.prev_track();
                } else if self.hits.play.contains(pos) {
                    self.play_pause();
                } else if self.hits.next.contains(pos) {
                    self.next_track();
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
            Action::Next => self.next_track(),
            Action::Prev => self.prev_track(),
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
            // runtime only (the settings menu saves them): unshifted keys, each one cycles
            Action::BigScale => {
                self.args.big_scale = self.args.big_scale % 3 + 1;
                self.snap_scroll = true;
                self.set_note(format!("big-font size {}", self.args.big_scale));
            }
            Action::LineSpacing => {
                self.args.lyric_spacing = (self.args.lyric_spacing + 1) % 4;
                self.snap_scroll = true;
                self.set_note(format!("line spacing {}", self.args.lyric_spacing));
            }
            Action::LyricPad => {
                let pad = self.args.lyric_pad + 2;
                self.args.lyric_pad = if pad > 10 { 0 } else { pad };
                self.snap_scroll = true;
                self.set_note(format!("side padding {}", self.args.lyric_pad));
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
            Action::Library => self.open_library(),
            Action::Like => self.toggle_like(),
            Action::PlayHere => self.spot_cmd(spotify::Cmd::Transfer),
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
        self.player_cmd(&["volume", &format!("{v:.3}")], spotify::Cmd::Volume(v));
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
        self.seek_to(target);
    }

    /// Flip shuffle (shown at once; the next poll reports what the player really did).
    fn toggle_shuffle(&mut self) {
        if let Some(s) = self.snap.as_mut().filter(|s| s.shuffle.is_some()) {
            s.shuffle = s.shuffle.map(|b| !b);
            let on = s.shuffle == Some(true);
            self.player_cmd(&["shuffle", "Toggle"], spotify::Cmd::Shuffle(on));
        }
    }

    /// Repeat: off -> playlist -> track -> off.
    fn cycle_repeat(&mut self) {
        if let Some(s) = self.snap.as_mut() {
            if let Some(r) = s.repeat.map(|r| r.next()) {
                s.repeat = Some(r);
                self.player_cmd(&["loop", r.arg()], spotify::Cmd::Repeat(r));
            }
        }
    }

    fn play_pause(&mut self) {
        // Optimistic update so the button flips immediately.
        let p = self.position();
        let was_playing = self.playing();
        if let Some(s) = &mut self.snap {
            s.status = if s.status == Status::Playing {
                Status::Paused
            } else {
                Status::Playing
            };
        }
        self.set_position(p);
        let cmd = if was_playing { spotify::Cmd::Pause } else { spotify::Cmd::Resume };
        self.player_cmd(&["play-pause"], cmd);
    }

    fn seek_rel(&mut self, d: f64) {
        let p = self.position() + d;
        self.set_position(p);
        if self.remote_active() {
            self.spot_cmd(spotify::Cmd::Seek(p));
            return;
        }
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

// ── Spotify (--spotify) ────────────────────────────────────────────────
impl App {
    /// What is on screen came from the Spotify API, not MPRIS: controls go to the API.
    pub fn remote_active(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.player == spotify::REMOTE)
    }

    fn set_mpris(&mut self, present: bool) {
        self.mpris_present = present;
        if let Some(api) = &self.spot {
            api.mpris.store(present, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The last API snapshot, its position moved on by the time since it came.
    fn remote_snapshot(&self) -> Option<Snapshot> {
        let (r, at) = self.remote.as_ref()?;
        let mut s = r.snap.clone();
        if s.status == Status::Playing {
            s.position += at.elapsed().as_secs_f64();
        }
        Some(s)
    }

    fn set_note(&mut self, msg: impl Into<String>) {
        self.note = Some((msg.into(), Instant::now()));
        self.dirty = true;
    }

    /// The footer note, while it is fresh.
    pub fn note_text(&self) -> Option<&str> {
        self.note
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
            .map(|(n, _)| n.as_str())
    }

    /// The API, or a note saying why there is none.
    fn api(&mut self) -> Option<spotify::Api> {
        match &self.spot {
            None => {
                self.set_note("Spotify is off: start with --spotify (or spotify = true)");
                None
            }
            Some(api) => Some(api.clone()),
        }
    }

    /// Run a command in the background; the result comes back as a note.
    fn spot_cmd(&mut self, cmd: spotify::Cmd) {
        let Some(api) = self.api() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = api.run(cmd).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Done(r)));
            // the player state lags the command a little
            tokio::time::sleep(Duration::from_millis(400)).await;
            api.wake.notify_one();
        });
    }

    /// A playback command for whatever is on screen: playerctl, or the API for another device.
    fn player_cmd(&mut self, args: &[&str], cmd: spotify::Cmd) {
        if self.remote_active() {
            self.spot_cmd(cmd);
        } else {
            self.ctl.send(args);
        }
    }

    fn next_track(&mut self) {
        self.player_cmd(&["next"], spotify::Cmd::Next);
    }

    fn prev_track(&mut self) {
        self.player_cmd(&["previous"], spotify::Cmd::Prev);
    }

    fn seek_to(&mut self, secs: f64) {
        if self.remote_active() {
            self.spot_cmd(spotify::Cmd::Seek(secs));
        } else {
            self.ctl.seek_to(secs);
        }
    }

    /// The Spotify URI of the song playing, if it is a Spotify track.
    fn playing_uri(&self) -> Option<String> {
        self.snap.as_ref().and_then(|s| spotify::track_uri(&s.track_id))
    }

    /// Ask once per song whether it is in Liked Songs.
    fn check_liked(&mut self) {
        let uri = self.playing_uri().unwrap_or_default();
        if uri == self.liked_uri {
            return;
        }
        self.liked_uri = uri.clone();
        self.liked = None;
        let (Some(api), false) = (self.spot.clone(), uri.is_empty()) else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(liked) = api.liked(&uri).await {
                let _ = tx.send(AppEvent::Spotify(spotify::Reply::Liked { uri, liked }));
            }
        });
    }

    fn toggle_like(&mut self) {
        let Some(uri) = self.playing_uri() else {
            self.set_note("not a Spotify song");
            return;
        };
        if self.spot.is_none() {
            self.api();
            return;
        }
        let on = self.liked != Some(true);
        self.liked = Some(on);
        self.lib.tabs[TAB_LIKED].loaded = false;
        self.spot_cmd(spotify::Cmd::Like(uri, on));
    }

    fn on_spotify(&mut self, r: spotify::Reply) {
        use spotify::Reply;
        self.dirty = true;
        match r {
            Reply::Remote(Err(e)) => self.spot_status = Some(e),
            Reply::Remote(Ok(r)) => {
                self.spot_status = None;
                self.remote = r.map(|r| (*r, Instant::now()));
                if self.mpris_present {
                    return;
                }
                match self.remote_snapshot() {
                    Some(s) => self.on_snapshot(s),
                    None if self.remote_active() => {
                        self.snap = None;
                        self.track_key = 0;
                    }
                    None => {}
                }
                self.check_liked();
            }
            Reply::Liked { uri, liked } if uri == self.liked_uri => self.liked = Some(liked),
            Reply::Liked { .. } => {}
            Reply::Done(Ok(n)) if n.is_empty() => {}
            Reply::Done(Ok(n)) => {
                self.lib.note = Some(n.clone());
                self.set_note(n);
            }
            Reply::Done(Err(e)) => {
                // the optimistic heart was wrong: ask again
                self.liked_uri.clear();
                self.check_liked();
                self.lib.note = Some(e.clone());
                self.set_note(e);
            }
            Reply::Tab(i, res) => {
                let l = &mut self.lib.tabs[i];
                l.loading = false;
                match res {
                    Ok(items) => {
                        l.items = items;
                        l.loaded = true;
                        l.error = None;
                        l.sel = l.sel.min(l.items.len().saturating_sub(1));
                    }
                    Err(e) => l.error = Some(e),
                }
                if i == TAB_PLAYLISTS {
                    self.fill_picker();
                }
            }
            Reply::Opened { uri, result } => {
                let Some(l) = self.lib.stack.iter_mut().rev().find(|l| l.uri == uri && l.loading) else {
                    return;
                };
                l.loading = false;
                match result {
                    Ok((summary, items)) => {
                        l.summary = summary;
                        l.items = items;
                        l.loaded = true;
                    }
                    Err(e) => l.error = Some(e),
                }
            }
        }
    }

    // ── library browser ────────────────────────────────────────────────

    fn open_library(&mut self) {
        if self.api().is_none() {
            return;
        }
        self.lib.open = true;
        self.lib.note = self.spot_status.clone();
        self.load_tab(self.lib.tab, false);
    }

    fn load_tab(&mut self, i: usize, force: bool) {
        let l = &mut self.lib.tabs[i];
        if l.loading || (l.loaded && !force) {
            return;
        }
        let Some(api) = self.spot.clone() else { return };
        l.loading = true;
        l.error = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = match i {
                0 => api.recent().await,
                1 => api.playlists().await,
                2 => api.liked_tracks().await,
                3 => api.albums().await,
                _ => api.artists().await,
            };
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Tab(i, r.map_err(|e| e.msg))));
        });
    }

    fn switch_tab(&mut self, i: usize) {
        self.lib.tab = i % TABS.len();
        self.lib.stack.clear();
        self.lib.picker = None;
        self.load_tab(self.lib.tab, false);
    }

    /// Open an album / own playlist / artist / Liked Songs as a new level.
    fn open_item(&mut self, it: spotify::Item) {
        if !it.openable {
            self.lib.note = Some(match it.kind.as_str() {
                "playlist" => "Spotify only lets apps open your own playlists: Enter plays it".into(),
                _ => "Enter plays it".into(),
            });
            return;
        }
        let Some(api) = self.spot.clone() else { return };
        let uri = if it.kind == "liked" { "liked".to_string() } else { it.uri.clone() };
        self.lib.stack.push(Level { title: it.name, uri: uri.clone(), loading: true, ..Default::default() });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.open(&uri).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Opened { uri, result }));
        });
    }

    fn reload_level(&mut self) {
        if self.lib.picker.is_some() {
            self.load_tab(TAB_PLAYLISTS, true);
            return;
        }
        match self.lib.stack.pop() {
            Some(l) => {
                let kind = l.uri.split(':').nth(1).unwrap_or("liked").to_string();
                self.open_item(spotify::Item { kind, uri: l.uri, name: l.title, openable: true, ..Default::default() });
            }
            None => self.load_tab(self.lib.tab, true),
        }
    }

    /// Enter: a track plays in the list's context (from that song on), anything else plays itself.
    fn play_selected(&mut self) {
        let l = self.lib.level();
        let Some(it) = l.items.get(l.sel).cloned() else { return };
        let ctx = l.uri.clone();
        let cmd = match it.kind.as_str() {
            "track" if !ctx.is_empty() => spotify::Cmd::Play { ctx, track: it.uri },
            "track" => spotify::Cmd::Play { ctx: it.uri, track: String::new() },
            "liked" => spotify::Cmd::Play { ctx: "liked".into(), track: String::new() },
            _ => spotify::Cmd::Play { ctx: it.uri, track: String::new() },
        };
        self.spot_cmd(cmd);
        self.lib.open = false;
    }

    fn selected_track(&mut self) -> Option<spotify::Item> {
        let l = self.lib.level();
        let it = l.items.get(l.sel).filter(|it| it.kind == "track").cloned();
        if it.is_none() {
            self.lib.note = Some("pick a song first".into());
        }
        it
    }

    /// The "add to playlist" list: your own playlists (loaded with the Playlists tab).
    fn fill_picker(&mut self) {
        let tab = &self.lib.tabs[TAB_PLAYLISTS];
        let (items, loading, error) = (
            tab.items.iter().filter(|p| p.openable).cloned().collect::<Vec<_>>(),
            tab.loading,
            tab.error.clone(),
        );
        if let Some((_, _, l)) = &mut self.lib.picker {
            l.items = items;
            l.loaded = !loading && error.is_none();
            l.loading = loading;
            l.error = error;
            l.sel = l.sel.min(l.items.len().saturating_sub(1));
        }
    }

    fn open_picker(&mut self, it: spotify::Item) {
        let title = format!("Add “{}” to…", it.name);
        self.lib.picker = Some((it.uri, it.name, Level { title, ..Default::default() }));
        self.load_tab(TAB_PLAYLISTS, false);
        self.fill_picker();
    }

    pub fn handle_library_key(&mut self, code: KeyCode) {
        let n = self.lib.level().items.len();
        let page = 10;
        let move_sel = |l: &mut Level, to: usize| l.sel = to.min(n.saturating_sub(1));
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                let l = self.lib.level_mut();
                l.sel = l.sel.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let l = self.lib.level_mut();
                let to = l.sel + 1;
                move_sel(l, to);
            }
            KeyCode::PageUp => {
                let l = self.lib.level_mut();
                l.sel = l.sel.saturating_sub(page);
            }
            KeyCode::PageDown => {
                let l = self.lib.level_mut();
                let to = l.sel + page;
                move_sel(l, to);
            }
            KeyCode::Home => self.lib.level_mut().sel = 0,
            KeyCode::End => move_sel(self.lib.level_mut(), n),
            KeyCode::Esc | KeyCode::Char('q') if self.lib.picker.is_some() => self.lib.picker = None,
            KeyCode::Esc | KeyCode::Char('q' | 'o') => self.lib.open = false,
            KeyCode::Enter if self.lib.picker.is_some() => {
                let Some((track, _, l)) = &self.lib.picker else { return };
                let Some(pl) = l.items.get(l.sel).cloned() else { return };
                let track = track.clone();
                self.lib.picker = None;
                self.spot_cmd(spotify::Cmd::Add { playlist: pl.uri, name: pl.name, track });
            }
            _ if self.lib.picker.is_some() => {
                if code == KeyCode::Char('r') {
                    self.reload_level();
                }
            }
            KeyCode::Tab => self.switch_tab(self.lib.tab + 1),
            KeyCode::BackTab => self.switch_tab(self.lib.tab + TABS.len() - 1),
            KeyCode::Char(c @ '1'..='5') => self.switch_tab(c as usize - '1' as usize),
            KeyCode::Enter | KeyCode::Char(' ') => self.play_selected(),
            KeyCode::Right | KeyCode::Char('l') => {
                let l = self.lib.level();
                if let Some(it) = l.items.get(l.sel).cloned() {
                    self.open_item(it);
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                self.lib.stack.pop();
            }
            KeyCode::Char('a') => {
                if let Some(it) = self.selected_track() {
                    self.spot_cmd(spotify::Cmd::Queue(it.uri));
                }
            }
            KeyCode::Char('A') => {
                if let Some(it) = self.selected_track() {
                    self.open_picker(it);
                }
            }
            KeyCode::Char('f') => {
                if let Some(it) = self.selected_track() {
                    if it.uri == self.liked_uri {
                        self.liked = Some(true);
                    }
                    self.lib.tabs[TAB_LIKED].loaded = false;
                    self.spot_cmd(spotify::Cmd::Like(it.uri, true));
                }
            }
            KeyCode::Char('r') => self.reload_level(),
            _ => {}
        }
    }
}
