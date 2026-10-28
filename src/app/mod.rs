use super::*;
use crate::cli::{TranslateEngine, TranslateMode};
use crate::lyrics::Lyrics;
use crate::player::{Snapshot, Status};
use crate::theme::Rgb;
use crate::theme::Theme;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

mod actions;
mod customize;
mod notes;
mod remote;
mod settings;

pub use actions::*;
pub use customize::*;
pub use notes::AnnState;
pub use crate::browser::{Level, Library, TABS};
use crate::browser::{DevicePicker, Edit, TextInput};

/// Language check kept failing for a song on a never-translate list: asked again this often,
/// this many times.
const HOLD_RETRY_SECS: u64 = 20;
const HOLD_RETRIES: u32 = 6;
/// Start of the backend name when the AI failed and the standard engine answered instead.
pub const FALLBACK_TAG: &str = "fallback: ";
pub use settings::*;


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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


/// The library lists load in the background after the start: the first one after this, then
/// one every `PRELOAD_GAP`.
#[cfg(not(test))]
const PRELOAD_DELAY: Duration = Duration::from_secs(15);
#[cfg(not(test))]
const PRELOAD_GAP: Duration = Duration::from_secs(4);
#[cfg(test)]
const PRELOAD_DELAY: Duration = Duration::ZERO;
#[cfg(test)]
const PRELOAD_GAP: Duration = Duration::from_millis(150);


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
    /// The `[ settings ]` and `[ ? ]` buttons of the top row.
    pub settings_btn: Rect,
    pub help_btn: Rect,
    /// Where each metadata field was drawn (click, right-click, drag).
    pub fields: Vec<(crate::layout::MetaField, Rect)>,
    /// The borders between columns that can be dragged.
    pub borders: Vec<Border>,
    /// The rows of the open field menu.
    pub menu_items: Vec<Rect>,
    /// Chord diagrams share the annotations column (drawn above the notes) instead of docking
    /// inside the lyrics column.
    pub stack_diagrams: bool,
}

/// A draggable border between two columns.
#[derive(Clone, Copy)]
pub struct Border {
    pub rect: Rect,
    pub left: crate::layout::PanelId,
    /// Where the left column starts and the width both columns share, in cells.
    pub start: u16,
    pub total: u16,
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
    /// Per row: the columns `[x0, x1)` its text covers; a click outside them does nothing.
    pub spans: Vec<(u16, u16)>,
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
    pub(crate) anchor: (f64, Instant),
    pub track_key: u64,
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
    /// The spotifyd this program started (never one that was already running).
    pub spotifyd: Option<std::process::Child>,
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
    pub help_edit: Option<HelpEdit>,
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
    /// `k`: the translation is shown under each original line instead of replacing it.
    pub translate_both: bool,
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
    pub translate_backend: String,
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
    /// The key table with the `[keys]` overrides of config.toml.
    pub keys: crate::keymap::Keymap,
    /// Which panels, fields and buttons are shown, in which order (the `[layout]` table).
    pub layout: crate::layout::Layout,
    /// Genre of the playing track, when a source knows it.
    pub genre: Option<String>,
    /// The privacy notice for the AI engine was shown; the next press turns it on.
    pub ai_confirm: bool,
    /// Annotations panel: state, the Genius notes, their match to the lyric lines, AI notes.
    pub ann_state: AnnState,
    pub anns: Vec<crate::annotations::Annotation>,
    pub ann_match: Vec<Option<usize>>,
    pub ai_notes: Vec<Option<String>>,
    /// Test hook: another Genius API address.
    pub annotations_base: Option<String>,
    /// What MusicBrainz said for the playing track.
    mb_genre: Option<Vec<String>>,
    /// The customize page of the settings menu is open, and its selected line.
    pub customizing: bool,
    pub cust_sel: usize,
    pub field_menu: Option<FieldMenu>,
    pub drag: Option<Drag>,
    /// The task polling `/me/player` while Spotify is on.
    spot_task: Option<JoinHandle<()>>,
    /// The browser login started from the settings menu, and what it says.
    login_task: Option<JoinHandle<()>>,
    login_msg: Option<String>,
    /// A settings row being typed into.
    pub settings_edit: Option<(Setting, TextInput)>,
    /// `d`: the device picker, while it is open.
    pub devices: Option<DevicePicker>,
    /// The background load of the main lists after Spotify came on.
    /// Library tabs still to be loaded in the background, and when the next one is due.
    /// The lyrics versions lrclib has for this song (track key, kind + text) and the one shown.
    lyric_versions: Option<(u64, Vec<(String, String)>)>,
    lyric_ver: Option<usize>,
    preload_queue: std::collections::VecDeque<usize>,
    preload_at: Option<Instant>,
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
            spotifyd: None,
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
            help_edit: None,
            show_settings: false,
            settings_sel: 0,
            settings_top: 0,
            skip_cursor: 0,
            settings_note: None,
            translate_on: false,
            translate_both: false,
            translate_state: TranslateState::Idle,
            translate_inflight: None,
            translate_task: None,
            translations: Default::default(),
            translate_base: None,
            translate_backend: "google".to_string(),
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
            keys: Default::default(),
            layout: Default::default(),
            genre: None,
            ai_confirm: false,
            ann_state: AnnState::Idle,
            anns: Vec::new(),
            ann_match: Vec::new(),
            ai_notes: Vec::new(),
            annotations_base: None,
            mb_genre: None,
            customizing: false,
            cust_sel: 0,
            field_menu: None,
            drag: None,
            spot_task: None,
            login_task: None,
            login_msg: None,
            settings_edit: None,
            devices: None,
            lyric_versions: None,
            lyric_ver: None,
            preload_queue: Default::default(),
            preload_at: None,
        };
        app.sync_queue_tab();
        let start = app.args.start_translate_mode();
        app.translate_on = start != TranslateMode::Off;
        app.translate_both = start == TranslateMode::Both;
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

    #[cfg(test)]
    pub fn set_position_for_test(&mut self, p: f64) {
        self.set_position(p);
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
                self.rematch_annotations();
                self.ensure_annotations();
                self.dirty = true;
            }
            AppEvent::Annotations { key, result } if key == self.track_key => self.on_annotations(result),
            AppEvent::AiNotes { key, result } if key == self.track_key => self.on_ai_notes(result),
            AppEvent::LyricVersions { key, list } if key == self.track_key => {
                // which of them is on screen now
                self.lyric_ver = match &self.lyrics {
                    LyricsState::Ready(l) => list.iter().position(|(k, t)| lyrics::to_lyrics(k, t) == *l),
                    _ => None,
                };
                self.lyric_versions = Some((key, list));
                self.step_lyrics();
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
            AppEvent::Genre { key, genres } if key == self.track_key => {
                self.mb_genre = genres;
                self.refresh_genre();
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

    pub fn on_snapshot(&mut self, mut s: Snapshot) {
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
                keep(&o.genre, &mut s.genre);
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
            self.reset_annotations();
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

            // Genre: the player and the file's tags are merged into the snapshot; MusicBrainz is
            // asked only when neither had one
            self.mb_genre = None;
            if s.genre.trim().is_empty() && self.args.genre_lookup {
                let (c, tx) = (self.client.clone(), self.tx.clone());
                let (a, t) = (s.artist.clone(), s.title.clone());
                self.tasks.push(tokio::spawn(async move {
                    let genres = genre::lookup(&c, &a, &t).await;
                    let _ = tx.send(AppEvent::Genre { key, genres });
                }));
            }

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
        self.refresh_genre();
        if changed {
            self.dirty = true;
        }
    }

    /// A snapshot whose strings changed but that is still the item already playing: the
    /// player's own id is unchanged, or the position runs on where it was (a real track
    /// change starts near 0 or jumps) with the same length.
    /// The genre line of the metadata block: player, then file tags, then MusicBrainz.
    pub(super) fn refresh_genre(&mut self) {
        let mpris = self.snap.as_ref().map(|s| s.genre.clone());
        self.genre = genre::resolve(mpris.as_deref(), None, self.mb_genre.clone());
    }

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
        fill(&mut s.genre, &t.genre);
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
            (Some(sh), LyricsState::Ready(Lyrics::Synced(l))) => {
                Some(chords::align_with(sh, l, self.args.chords_match))
            }
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
        let ai = self.ai_job(&lines);
        // (the tests must not read or fill the real cache folder)
        let (keep, skip) = (!self.args.no_cache_translations && !crate::util::testing(), self.skip_langs());
        self.translate_task = Some(tokio::spawn(async move {
            // the language model first: one request for the whole song. If it can't deliver
            // (no key, unreachable, an answer that stays invalid after a retry) the standard
            // engine takes over and the lyrics title says so.
            let mut ai_failed: Option<String> = None;
            if let Some((acfg, song)) = ai {
                if keep {
                    if let Some(done) = ai::cached(&acfg, &song).await {
                        let _ = tx.send(AppEvent::Translated { key, lang, lines: Ok((done, ai::label(&acfg))) });
                        return;
                    }
                }
                match ai::translate(&c, &acfg, &song).await {
                    Ok((done, label)) => {
                        if keep {
                            ai::store(&acfg, &song, &done).await;
                        }
                        let _ = tx.send(AppEvent::Translated { key, lang, lines: Ok((done, label)) });
                        return;
                    }
                    Err(why) => ai_failed = Some(why),
                }
            }
            let tag = |backend: &str| match &ai_failed {
                Some(why) => format!("{FALLBACK_TAG}{backend}; AI failed: {why}"),
                None => backend.to_string(),
            };
            if keep && ai_failed.is_none() {
                if let Some((done, backend)) = translate::cached(&lang, &skip, &lines).await {
                    let _ = tx.send(AppEvent::Translated { key, lang, lines: Ok((done, tag(backend))) });
                    return;
                }
            }
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
                if let (true, Ok((done, backend))) = (keep, &r) {
                    translate::store(&lang, &skip, &lines, done, backend).await;
                }
                let _ = tx.send(AppEvent::Translated { key, lang, lines: r.map(|(l, b)| (l, tag(b))) });
                return;
            }
        }));
    }

    /// The AI request for this song, when the AI engine is chosen and set up.
    fn ai_job(&mut self, lines: &[String]) -> Option<(ai::Cfg, ai::Song)> {
        if self.args.translate_engine != TranslateEngine::Ai {
            return None;
        }
        let cfg = self.ai_cfg()?;
        // first use with a remote provider: say where the lyrics go (once, ever)
        if self.ai_notice_needed() {
            self.set_note(self.ai_notice());
            self.ack_ai_notice();
        }
        let snap = self.snap.as_ref()?;
        Some((
            cfg,
            ai::Song {
                title: snap.title.clone(),
                artist: snap.artist.clone(),
                album: snap.album.clone(),
                genre: self.genre.clone(),
                year: match &self.year {
                    YearState::Known(y) => Some(y.clone()),
                    _ => None,
                },
                source_lang: "the original language (detect it)".into(),
                target_lang: self.args.translate_lang.clone(),
                lines: lines.to_vec(),
            },
        ))
    }

    /// The AI settings as a request config; `None` (with a note) when the model is missing.
    pub fn ai_cfg(&mut self) -> Option<ai::Cfg> {
        let provider = ai::Provider::parse(&self.args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
        if self.args.ai_model.trim().is_empty() {
            self.set_note("AI translation needs a model (settings → AI model); using the standard engine");
            return None;
        }
        Some(ai::Cfg::new(provider, &self.args.ai_model, &self.args.ai_endpoint, &self.args.ai_key_env))
    }

    fn ai_ack_path() -> std::path::PathBuf {
        spotify::dir().with_file_name("ai-notice-ack")
    }

    /// A remote provider has not been confirmed yet. A local Ollama endpoint never needs it.
    pub fn ai_notice_needed(&self) -> bool {
        let provider = ai::Provider::parse(&self.args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
        let cfg = ai::Cfg::new(provider, &self.args.ai_model, &self.args.ai_endpoint, &self.args.ai_key_env);
        !cfg.is_local() && !Self::ai_ack_path().exists()
    }

    pub fn ai_notice(&self) -> String {
        let provider = ai::Provider::parse(&self.args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
        let cfg = ai::Cfg::new(provider, &self.args.ai_model, &self.args.ai_endpoint, &self.args.ai_key_env);
        format!(
            "AI translation sends the lyrics and song info (title, artist, album, genre, year) to {} ({})",
            cfg.endpoint,
            provider.name()
        )
    }

    fn ack_ai_notice(&self) {
        let p = Self::ai_ack_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, "shown\n");
    }

    /// The engine or its settings changed: whatever was translated for this song is stale.
    pub fn reset_translation(&mut self) {
        self.translations.clear();
        self.translate_state = TranslateState::Idle;
        self.translate_inflight = None;
        if let Some(t) = self.translate_task.take() {
            t.abort();
        }
        self.snap_scroll = true;
        self.ensure_translation();
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

    pub fn translate_mode(&self) -> TranslateMode {
        match (self.translate_on, self.translate_both) {
            (false, _) => TranslateMode::Off,
            (true, true) => TranslateMode::Both,
            (true, false) => TranslateMode::Translated,
        }
    }

    pub fn set_translate_mode(&mut self, mode: TranslateMode) {
        self.translate_on = mode != TranslateMode::Off;
        self.translate_both = mode == TranslateMode::Both;
        self.snap_scroll = true;
        if self.translate_on {
            self.show_lyrics = true;
            self.translate_state = TranslateState::Idle;
            self.ensure_translation();
        }
    }

    fn toggle_translate(&mut self) {
        self.translate_on = !self.translate_on;
        self.translate_both = false;
        self.snap_scroll = true;
        if self.translate_on {
            self.show_lyrics = true;
            self.translate_state = TranslateState::Idle;
            self.ensure_translation();
        }
    }

    /// `k`: translation on with the original line kept; again = off.
    fn toggle_translate_both(&mut self) {
        let off = self.translate_on && self.translate_both;
        self.translate_on = !off;
        self.translate_both = !off;
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
            Setting::ChordsMatch => match &self.settings_edit {
                Some((Setting::ChordsMatch, input)) => input.shown(),
                _ => format!("{} %", self.args.chords_match),
            },
            Setting::MemberParts => onoff(self.args.member_parts),
            Setting::Translate => onoff(self.translate_on),
            Setting::TranslateMode => format!("< {} >", self.translate_mode().name()),
            Setting::TranslateEngine => format!("< {} >", self.args.translate_engine.name()),
            Setting::AiProvider => format!("< {} >", self.args.ai_provider),
            Setting::AiModel | Setting::AiEndpoint | Setting::AiKeyEnv => match &self.settings_edit {
                Some((cur, input)) if *cur == s => input.shown(),
                _ => {
                    let (v, none) = match s {
                        Setting::AiModel => (&self.args.ai_model, "(not set)"),
                        Setting::AiEndpoint => (&self.args.ai_endpoint, "(provider default)"),
                        _ => (&self.args.ai_key_env, "(provider default)"),
                    };
                    if v.is_empty() { none.to_string() } else { v.clone() }
                }
            },
            Setting::TranslateLang => format!("< {} >", self.args.translate_lang),
            Setting::TranslateSkip => {
                // `< sv [x] >`: ←/→ pick a language, space adds or removes it
                let code = LANGS[self.skip_cursor % LANGS.len()];
                let on = self.skip_langs().contains(&translate::base_code(code));
                let list = self.args.translate_skip.clone();
                format!("< {code} {} >{}", if on { "[x]" } else { "[ ]" }, if list.is_empty() { String::new() } else { format!("  ({list})") })
            }
            Setting::TranslateFallback => onoff(self.args.translate_fallback),
            Setting::CacheTranslations => onoff(!self.args.no_cache_translations),
            Setting::Minimal => onoff(self.minimal),
            Setting::Customize => "enter →".to_string(),
            Setting::GenreLookup => onoff(self.args.genre_lookup),
            Setting::Annotations => onoff(self.annotations_shown()),
            Setting::AnnotationsMode => format!("< {} >", self.args.annotations_mode.name()),
            Setting::GeniusToken => match &self.settings_edit {
                Some((Setting::GeniusToken, input)) => input.shown(),
                _ if !self.args.genius_token.is_empty() => "set (hidden)".to_string(),
                _ if std::env::var("GENIUS_TOKEN").is_ok_and(|t| !t.trim().is_empty()) => "from GENIUS_TOKEN".to_string(),
                _ => "(not set)".to_string(),
            },
            Setting::BigLyrics => onoff(self.big_lyrics),
            Setting::Karaoke => onoff(self.args.karaoke),
            Setting::WindowTitle => format!("< {} >", self.args.window_title),
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
            Setting::SpotifyOn => onoff(self.spot.is_some()),
            Setting::SpotifyClientId => match &self.settings_edit {
                Some((Setting::SpotifyClientId, input)) => input.shown(),
                // not a secret, but only the start of it is shown
                _ => match self.args.spotify_client_id.as_deref().filter(|c| !c.is_empty()) {
                    Some(id) => format!("{}…", id.chars().take(6).collect::<String>()),
                    None => "not set".into(),
                },
            },
            Setting::SpotifyLogin => match self.login_task {
                Some(_) => "waiting… (enter cancels)".into(),
                None => String::new(),
            },
            Setting::SpotifyLogout => String::new(),
            Setting::SpotifyDevice => match &self.settings_edit {
                Some((Setting::SpotifyDevice, input)) => input.shown(),
                _ if self.args.spotify_device.is_empty() => "none".into(),
                _ => self.args.spotify_device.clone(),
            },
            Setting::UseSpotifyd => onoff(self.args.use_spotifyd),
            Setting::KillSpotifyd => onoff(!self.args.no_kill_spotifyd),
            Setting::SpotifyButton => onoff(!self.args.no_spotify_button),
            Setting::SpotifyQueueTab => onoff(!self.args.no_spotify_queue_tab),
        }
    }

    /// The status line under the "Spotify" header.
    pub fn spotify_status(&self) -> String {
        if let Some(m) = &self.login_msg {
            return m.clone();
        }
        let token = spotify::logged_in();
        if self.spot.is_none() {
            return if token { "off · logged in".into() } else { "off · not logged in".into() };
        }
        match &self.spot_status {
            Some(e) if e.starts_with("not logged in") => "not logged in".into(),
            Some(e) if e.starts_with("login failed") => "token expired: log in again".into(),
            Some(e) => e.clone(),
            None if token => "logged in".into(),
            None => "not logged in".into(),
        }
    }

    /// Change a setting (`dir` = -1 / +1 for the stepped ones; toggles ignore it) and
    /// write the new values to the config file.
    pub fn change_setting(&mut self, s: Setting, dir: i32) {
        // actions and text rows answer Enter / space / → only, never ←
        if dir < 0 && (s.is_text() || matches!(s, Setting::SpotifyLogin | Setting::SpotifyLogout)) {
            return;
        }
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
            Setting::ChordsMatch => {
                self.settings_edit = Some((s, TextInput::with(&self.args.chords_match.to_string())));
                return;
            }
            Setting::MemberParts => self.toggle_member_parts(),
            Setting::Translate => self.toggle_translate(),
            Setting::TranslateMode => self.set_translate_mode(self.translate_mode().step(dir)),
            Setting::TranslateEngine => {
                let next = if self.args.translate_engine == TranslateEngine::Ai {
                    TranslateEngine::Standard
                } else {
                    TranslateEngine::Ai
                };
                if next == TranslateEngine::Ai && self.ai_notice_needed() && !self.ai_confirm {
                    // the first time with a remote provider: say where the lyrics go, then ask again
                    self.ai_confirm = true;
                    self.settings_note = Some(format!("{}. Press again to turn it on.", self.ai_notice()));
                    return;
                }
                if next == TranslateEngine::Ai && self.ai_notice_needed() {
                    self.ack_ai_notice();
                }
                self.ai_confirm = false;
                self.args.translate_engine = next;
                self.reset_translation();
            }
            Setting::AiProvider => {
                let all = ["openai-compatible", "gemini", "claude", "openai"];
                let cur = all.iter().position(|p| *p == self.args.ai_provider).unwrap_or(0) as i32;
                self.args.ai_provider = all[(cur + dir).rem_euclid(all.len() as i32) as usize].to_string();
                self.ai_confirm = false;
                self.reset_translation();
            }
            Setting::AiModel | Setting::AiEndpoint | Setting::AiKeyEnv => {
                let cur = match s {
                    Setting::AiModel => self.args.ai_model.clone(),
                    Setting::AiEndpoint => self.args.ai_endpoint.clone(),
                    _ => self.args.ai_key_env.clone(),
                };
                self.settings_edit = Some((s, TextInput::with(&cur)));
                return;
            }
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
            Setting::CacheTranslations => self.args.no_cache_translations = !self.args.no_cache_translations,
            Setting::Minimal => self.set_minimal(!self.minimal),
            Setting::Customize => {
                self.open_customize();
                return;
            }
            Setting::GenreLookup => self.args.genre_lookup = !self.args.genre_lookup,
            Setting::Annotations => self.toggle_annotations(),
            Setting::AnnotationsMode => {
                let next = if self.args.annotations_mode == crate::cli::AnnotationMode::Genius {
                    crate::cli::AnnotationMode::Ai
                } else {
                    crate::cli::AnnotationMode::Genius
                };
                self.args.annotations_mode = next;
                self.restart_annotations();
            }
            Setting::GeniusToken => {
                self.settings_edit = Some((s, TextInput::with("")));
                return;
            }
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
            Setting::Karaoke => self.args.karaoke = !self.args.karaoke,
            Setting::WindowTitle => {
                let cur = WINDOW_TITLES.iter().position(|m| *m == self.args.window_title).unwrap_or(0) as i32;
                self.args.window_title = WINDOW_TITLES[(cur + dir).rem_euclid(WINDOW_TITLES.len() as i32) as usize].into();
            }
            Setting::LyricSpacing => {
                self.args.lyric_spacing = (self.args.lyric_spacing as i32 + dir).clamp(0, 3) as u16;
                self.snap_scroll = true;
            }
            Setting::Cava => {
                self.run(Action::ToggleCava);
            }
            Setting::ThirdPanel => {
                // written to `[layout]` too: that is what decides on the next start
                let under = self.placement == CavaPlacement::ThirdPanel;
                self.set_cava_dock(under);
            }
            Setting::CoverTheme => self.args.cover_theme = !self.args.cover_theme,
            Setting::AsciiCover => self.args.ascii_cover = !self.args.ascii_cover,
            Setting::Transparent => self.args.transparent = !self.args.transparent,
            Setting::Mouse => self.args.no_mouse = !self.args.no_mouse,
            Setting::VolumeStep => {
                self.vol_step = (self.vol_step as i32 + dir).clamp(1, 50) as u32;
                self.args.volume_step = self.vol_step;
            }
            Setting::SpotifyOn => {
                if self.spot.is_some() {
                    self.stop_spotify();
                } else {
                    self.args.spotify = true;
                    self.start_spotify();
                }
            }
            Setting::SpotifyClientId => {
                let cur = self.args.spotify_client_id.clone().unwrap_or_default();
                self.settings_edit = Some((s, TextInput::with(&cur)));
                return; // saved when Enter commits the text
            }
            Setting::SpotifyDevice => {
                self.settings_edit = Some((s, TextInput::with(&self.args.spotify_device.clone())));
                return;
            }
            Setting::SpotifyLogin => self.toggle_login(),
            Setting::SpotifyLogout => self.logout_spotify(),
            Setting::UseSpotifyd => {
                self.args.use_spotifyd = !self.args.use_spotifyd;
                if !self.args.use_spotifyd {
                    self.stop_spotifyd();
                } else if self.spotifyd.is_none() {
                    self.spotifyd = crate::spotifyd::start();
                }
                // the running client matches "this PC" by this name: rebuild it
                self.restart_spotify();
            }
            Setting::KillSpotifyd => self.args.no_kill_spotifyd = !self.args.no_kill_spotifyd,
            Setting::SpotifyButton => self.args.no_spotify_button = !self.args.no_spotify_button,
            Setting::SpotifyQueueTab => {
                self.args.no_spotify_queue_tab = !self.args.no_spotify_queue_tab;
                self.sync_queue_tab();
            }
        }
        self.save_settings(s);
    }

    /// Enter on a text row: store what was typed.
    fn commit_setting_text(&mut self, s: Setting, text: &str) {
        let text = text.trim();
        match s {
            Setting::SpotifyClientId => {
                if !text.is_empty() && !text.chars().all(|c| c.is_ascii_alphanumeric()) {
                    self.settings_note = Some("that doesn't look like a Client ID".into());
                    return;
                }
                self.args.spotify_client_id = (!text.is_empty()).then(|| text.to_string());
            }
            Setting::SpotifyDevice => self.args.spotify_device = text.to_string(),
            Setting::GeniusToken => {
                // an empty entry keeps the old token (it is never shown to be edited)
                if !text.is_empty() {
                    self.args.genius_token = text.to_string();
                    self.restart_annotations();
                    self.save_settings(s);
                }
                return;
            }
            Setting::AiModel | Setting::AiEndpoint | Setting::AiKeyEnv => {
                match s {
                    Setting::AiModel => self.args.ai_model = text.to_string(),
                    Setting::AiEndpoint => self.args.ai_endpoint = text.to_string(),
                    _ => self.args.ai_key_env = text.to_string(),
                }
                self.ai_confirm = false;
                self.reset_translation();
                self.save_settings(s);
                return;
            }
            Setting::ChordsMatch => {
                let Some(n) = text.trim_end_matches('%').trim().parse::<u32>().ok().filter(|n| *n <= 100) else {
                    self.settings_note = Some("a number from 0 to 100".into());
                    return;
                };
                self.args.chords_match = n;
                self.realign();
                self.save_settings(s);
                return;
            }
            _ => return,
        }
        // the running client keeps the old name / ID until it is rebuilt
        self.restart_spotify();
        self.save_settings(s);
    }

    /// The settings the menu owns, as config-file entries.
    pub fn settings_values(&self) -> Vec<(&'static str, toml::Value)> {
        use toml::Value::{Boolean as B, Integer as I, String as S};
        vec![
            ("player", S(self.args.player.clone())),
            ("chords", B(self.chords_on)),
            ("chords_ug", B(self.args.chords_ug)),
            ("chords_match", I(self.args.chords_match as i64)),
            ("member_parts", B(self.args.member_parts)),
            ("genre_lookup", B(self.args.genre_lookup)),
            ("annotations", B(self.annotations_shown())),
            ("annotations_mode", S(self.args.annotations_mode.name().to_string())),
            ("genius_token", S(self.args.genius_token.clone())),
            ("translate", B(self.translate_on)),
            ("translate_mode", S(self.translate_mode().name().to_string())),
            ("translate_engine", S(self.args.translate_engine.name().to_string())),
            ("ai.provider", S(self.args.ai_provider.clone())),
            ("ai.model", S(self.args.ai_model.clone())),
            ("ai.endpoint", S(self.args.ai_endpoint.clone())),
            ("ai.api_key_env", S(self.args.ai_key_env.clone())),
            ("translate_lang", S(self.args.translate_lang.clone())),
            ("translate_skip", S(self.args.translate_skip.clone())),
            ("translate_fallback", B(self.args.translate_fallback)),
            ("cache_translations", B(!self.args.no_cache_translations)),
            ("big_scale", I(self.args.big_scale as i64)),
            ("lyric_pad", I(self.args.lyric_pad as i64)),
            ("hide_color_warning", B(self.args.hide_color_warning)),
            ("minimal_view", B(self.minimal)),
            ("big_lyrics", B(self.big_lyrics)),
            ("karaoke", B(self.args.karaoke)),
            ("window_title", S(self.args.window_title.clone())),
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
            ("spotify", B(self.args.spotify)),
            ("spotify_device", S(self.args.spotify_device.clone())),
            ("use_spotifyd", B(self.args.use_spotifyd)),
            ("kill_spotifyd_on_exit", B(!self.args.no_kill_spotifyd)),
            ("spotify_button", B(!self.args.no_spotify_button)),
            ("spotify_queue_tab", B(!self.args.no_spotify_queue_tab)),
        ]
        .into_iter()
        .chain(self.args.spotify_client_id.clone().map(|id| ("spotify_client_id", S(id))))
        .collect()
    }

    /// Write only the setting that changed, so flags given for this run (`--minimal`…)
    /// don't leak into the file.
    fn save_settings(&mut self, s: Setting) {
        let path = self.args.config.clone().unwrap_or_else(config::default_path);
        let keys = s.config_keys();
        if keys.is_empty() {
            return; // an action row (log in / out): nothing to write
        }
        let values: Vec<_> = self.settings_values().into_iter().filter(|(k, _)| keys.contains(k)).collect();
        self.settings_note = config::save(&path, &values)
            .err()
            .map(|e| format!("couldn't save {}: {e}", path.display()));
    }

    fn handle_settings_key(&mut self, code: KeyCode) {
        // typing into a row: every key is text until Enter or Esc
        if let Some((s, mut input)) = self.settings_edit.take() {
            match input.key(code) {
                Some(Edit::Submit) => self.commit_setting_text(s, &input.text),
                Some(Edit::Cancel) => {}
                None => self.settings_edit = Some((s, input)),
            }
            return;
        }
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
        self.layout.panels.set(crate::layout::PanelId::Cava, on);
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
                if self.field_menu.is_some() {
                    self.handle_field_menu_key(k.code);
                    return false;
                }
                if self.show_settings && self.customizing {
                    self.handle_customize_key(k.code);
                    return false;
                }
                if self.show_settings {
                    self.handle_settings_key(k.code);
                    return false;
                }
                if self.devices.is_some() {
                    self.handle_devices_key(k.code);
                    return false;
                }
                if self.lib.open {
                    let used = self.handle_library_key(k.code);
                    // beside the side panel the player's own keys keep working
                    if used || !self.lib.hits.panel {
                        return false;
                    }
                }
                if self.show_help {
                    if self.help_edit.is_some() {
                        self.handle_help_edit(k);
                        return false;
                    }
                    // Help is open: arrows / paging scroll it, `e` edits the keys, any other key closes it.
                    match k.code {
                        KeyCode::Char('e') => self.help_edit = Some(HelpEdit::default()),
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
                if let Some(a) = self.keys.action_for(k.code) {
                    return self.run(a);
                }
            }
            Event::Mouse(m) if self.field_menu.is_some() => self.field_menu_mouse(m),
            Event::Mouse(_) if self.show_settings => {}
            Event::Mouse(m) if self.devices.is_some() => self.devices_mouse(m),
            // the toggle button works whether the browser is open or not
            Event::Mouse(m)
                if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
                    && self.lib.hits.button.contains(Position::new(m.column, m.row)) =>
            {
                self.toggle_library();
            }
            Event::Mouse(m) if self.lib.open && self.library_mouse(m) => {}
            Event::Mouse(m) if self.show_help => match m.kind {
                MouseEventKind::ScrollUp => self.help_scroll = self.help_scroll.saturating_sub(2),
                MouseEventKind::ScrollDown => self.help_scroll += 2,
                _ => {}
            },
            // right-click a metadata field: Hide / Move up / Move down
            Event::Mouse(m) if matches!(m.kind, MouseEventKind::Down(MouseButton::Right)) => {
                if let Some(f) = self.field_at(m.column, m.row) {
                    self.open_field_menu(f, m.column, m.row);
                }
            }
            Event::Mouse(m) if matches!(m.kind, MouseEventKind::Drag(MouseButton::Left)) && self.drag.is_some() => {
                self.mouse_drag(m.column, m.row);
            }
            Event::Mouse(m) if matches!(m.kind, MouseEventKind::Up(MouseButton::Left)) && self.drag.is_some() => {
                self.mouse_up(m.column, m.row);
            }
            Event::Mouse(m) if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) => {
                let pos = Position::new(m.column, m.row);
                if self.hits.settings_btn.contains(pos) {
                    self.run(Action::Settings);
                } else if self.hits.help_btn.contains(pos) {
                    self.run(Action::Help);
                } else if self.hits.progress.contains(pos) {
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
                } else if crate::ui::Panel::click(&*crate::ui::panel_for(crate::layout::PanelId::Lyrics), self, pos.x, pos.y) {
                } else {
                    // a metadata field (click: Key / BPM / Capo / Tuning open the diagrams;
                    // drag: reorder) or a column border (drag: resize)
                    self.mouse_down(pos.x, pos.y);
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
                self.layout.panels.set(crate::layout::PanelId::Lyrics, self.show_lyrics);
                self.snap_scroll = true;
            }
            Action::NextLyrics => self.next_lyrics(),
            Action::Karaoke => {
                self.args.karaoke = !self.args.karaoke;
                self.set_note(format!("word-by-word highlight {}", if self.args.karaoke { "on" } else { "off" }));
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
            Action::Annotations => self.toggle_annotations(),
            Action::Diagrams => {
                self.show_diagrams = !self.show_diagrams;
                self.layout.panels.set(crate::layout::PanelId::Chords, self.show_diagrams);
                self.snap_scroll = true;
            }
            Action::Minimal => self.set_minimal(!self.minimal),
            Action::BrowseUp => self.browse_by(-1.0),
            Action::BrowseDown => self.browse_by(1.0),
            Action::BrowsePageUp => self.browse_by(-8.0),
            Action::BrowsePageDown => self.browse_by(8.0),
            Action::BrowseEnd => self.browse = 0.0,
            Action::Translate => self.toggle_translate(),
            Action::TranslateBoth => self.toggle_translate_both(),
            Action::Settings => {
                self.show_settings = true;
                if self.login_task.is_none() {
                    self.login_msg = None; // an old result isn't news any more
                }
            }
            Action::Library => self.open_library(),
            Action::Like => self.toggle_like(),
            Action::PlayHere => self.spot_cmd(spotify::Cmd::Transfer),
            Action::Devices => self.open_devices(),
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
    pub fn click_lyric(&mut self, x: u16, y: u16) {
        let h = &self.lyric_hits;
        if y < h.area.y {
            return;
        }
        let idx = h.top + (y - h.area.y) as i64;
        let Some(i) = usize::try_from(idx).ok() else { return };
        let Some(Some(li)) = h.rows.get(i).copied() else { return };
        // only the text itself is a target, not the blank space beside it
        if h.spans.get(i).is_some_and(|&(x0, x1)| x < x0 || x >= x1) {
            return;
        }
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

    /// A key in the help overlay's key editor: arrows pick an action, Enter then a key sets it,
    /// Backspace goes back to the built-in key, Esc leaves.
    fn handle_help_edit(&mut self, k: KeyEvent) {
        let acts = self.keys.actions();
        let Some(mut ed) = self.help_edit.take() else { return };
        ed.sel = ed.sel.min(acts.len().saturating_sub(1));
        let action = acts[ed.sel];
        if ed.capture {
            ed.capture = false;
            ed.note = if k.code == KeyCode::Esc {
                Some("cancelled".into())
            } else if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                Some("Ctrl and Alt keys can't be used".into())
            } else {
                match self.keys.rebind(action, k.code) {
                    Ok(()) => Some(self.save_key(action)),
                    Err(e) => Some(e),
                }
            };
            self.help_edit = Some(ed);
            return;
        }
        let n = acts.len();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => ed.sel = (ed.sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => ed.sel = (ed.sel + 1) % n,
            KeyCode::PageUp => ed.sel = ed.sel.saturating_sub(10),
            KeyCode::PageDown => ed.sel = (ed.sel + 10).min(n - 1),
            KeyCode::Home => ed.sel = 0,
            KeyCode::End => ed.sel = n - 1,
            KeyCode::Enter => {
                ed.capture = true;
                ed.note = None;
            }
            KeyCode::Backspace | KeyCode::Delete => {
                self.keys.reset(action);
                ed.note = Some(self.save_key(action));
            }
            KeyCode::Esc | KeyCode::Char('e') | KeyCode::Char('?') => {
                return; // back to the help
            }
            _ => {}
        }
        self.help_edit = Some(ed);
    }

    /// Write what `action` is bound to into `[keys]` of the config file; the message for the editor.
    fn save_key(&self, action: Action) -> String {
        let path = self.args.config.clone().unwrap_or_else(config::default_path);
        match config::save_key(&path, action.name(), self.keys.config_value(action)) {
            Ok(()) => format!("saved to {}", path.display()),
            Err(e) => format!("couldn't save {}: {e}", path.display()),
        }
    }

    /// `L`: show the next lyrics version lrclib has for this song. The list is looked up the
    /// first time, then the key just steps through it.
    fn next_lyrics(&mut self) {
        let Some(s) = self.snap.clone().filter(|s| !s.title.is_empty()) else { return };
        if self.lyric_versions.as_ref().is_some_and(|(k, _)| *k == self.track_key) {
            return self.step_lyrics();
        }
        self.set_note("looking for other lyrics…");
        let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
        let policy = lyrics::Policy { retries: self.args.lyrics_retries, ..Default::default() };
        self.tasks.push(tokio::spawn(async move {
            let list = lyrics::versions(&c, &s.artist, &s.title, &s.album, s.length, policy).await;
            let _ = tx.send(AppEvent::LyricVersions { key, list });
        }));
    }

    fn step_lyrics(&mut self) {
        let Some(s) = self.snap.clone() else { return };
        let Some((key, list)) = self.lyric_versions.as_ref() else { return };
        let (key, n) = (*key, list.len());
        if n < 2 {
            let msg = if n == 0 { "lrclib has no lyrics for this song" } else { "lrclib has only these lyrics" };
            return self.set_note(msg);
        }
        let next = self.lyric_ver.map_or(0, |i| (i + 1) % n);
        let (kind, text) = list[next].clone();
        self.lyric_ver = Some(next);
        // the next time this song plays it opens with the version picked here
        lyrics::remember(&s.artist, &s.title, &s.album, s.length, &kind, &text);
        // a translation belongs to the lines it was made from
        self.translations.retain(|(k, _), _| *k != key);
        self.translate_inflight = None;
        self.translate_state = TranslateState::Idle;
        self.apply(AppEvent::Lyrics { key, lyrics: lyrics::to_lyrics(&kind, &text) });
        self.set_note(format!("lyrics version {} of {n}", next + 1));
    }

    /// What the terminal window should be called, or `None` to leave its title alone
    /// (`window_title`): the song, cleaned of control characters so a title can't send escapes.
    pub fn window_title(&self) -> Option<String> {
        let s = self.snap.as_ref().filter(|s| !s.title.is_empty())?;
        let text = match self.args.window_title.as_str() {
            "title" => s.title.clone(),
            "artist-title" if !s.artist.is_empty() => format!("{} – {}", s.artist, s.title),
            "artist-title" => s.title.clone(),
            _ => return None,
        };
        Some(text.chars().filter(|c| !c.is_control()).collect())
    }

    /// The active line fills up word by word: needs a redraw far more often than the clock.
    pub fn karaoke_live(&self) -> bool {
        self.args.karaoke
            && self.show_lyrics
            && self.playing()
            && matches!(self.lyrics, LyricsState::Ready(Lyrics::Synced(_)))
    }

    /// Whether the next frame would differ even without new events.
    pub fn animating(&self) -> bool {
        (self.lyric_scroll - self.lyric_scroll_target).abs() > 0.02 || self.browse != 0.0
    }
}


impl Drop for App {
    fn drop(&mut self) {
        if !self.args.no_kill_spotifyd {
            self.stop_spotifyd();
        }
    }
}
