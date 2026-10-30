//! The GUI's state: what is playing, the lyrics, chords, translation and notes, fed by the same
//! background tasks and `AppEvent`s as the terminal front end. No egui types in here, so it can
//! be driven and tested without a window.
use super::browse::Browse;
use super::extras::Extras;
use super::translation::Tr;
use music_display::annotations::{self, Annotation};
use music_display::cli::{AnnotationMode, Args, TranslateEngine, TranslateMode};
use music_display::layout::{Layout, PanelId};
use music_display::lyrics::Lyrics;
use music_display::player::{Ctl, Snapshot, Status};
use super::library::Spot;
use music_display::{AppEvent, spotify, ai, chords, cover, genre, genius, identity, tags, ug, year};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

pub enum Chords {
    Idle,
    Fetching,
    Ready,
    Missing,
    Failed(String),
}

pub enum Notes {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

pub struct Model {
    pub args: Args,
    pub layout: Layout,
    pub cfg: PathBuf,
    /// The `[gui]` look; the window loads it from the config file.
    pub gui: music_display::guiconf::GuiConf,
    pub ctl: Ctl,
    pub tx: UnboundedSender<AppEvent>,
    pub client: reqwest::Client,
    pub snap: Option<Snapshot>,
    anchor: (Instant, f64),
    pub track_key: u64,
    pub art_url: String,
    /// Decoded cover, until the window turns it into a texture.
    pub cover: Option<image::DynamicImage>,
    pub cover_fresh: bool,
    pub year: Option<String>,
    /// The year lookup has finished (so no year means "N/A", not "still looking").
    pub year_done: bool,
    pub genre: Option<String>,
    mb_genre: Option<Vec<String>>,
    /// `None` while the lyrics are being fetched.
    pub lyrics: Option<Lyrics>,
    pub offset: f64,
    pub chords_on: bool,
    pub show_diagrams: bool,
    pub sheet: Option<chords::Sheet>,
    pub aligned: Option<chords::Aligned>,
    pub chords_state: Chords,
    pub transpose: i32,
    pub capo_override: Option<u8>,
    pub translate_mode: TranslateMode,
    pub translations: HashMap<(u64, String), Vec<String>>,
    pub translate_backend: String,
    /// The translation request: state, what is in flight, the abortable task.
    pub tr: Tr,
    /// Hash of the lyric lines the translations were made from.
    lines_hash: u64,
    /// The lyrics panel is browsed by hand (auto-follow is suspended for a while).
    pub browse: Browse,
    pub line_anim: super::lyrics_view::LineAnim,
    /// Content and viewport height of the lyrics list, last frame (for plain-lyrics scrolling).
    pub lyric_dims: (f32, f32),
    /// Where the progress bar would seek to while it is being dragged.
    pub seek_preview: Option<f64>,
    /// Another player reported in at this time (see `identity::ignore_other_player`).
    player_seen: Instant,
    /// The volume was changed here at this time; polls do not move the slider for a moment.
    vol_hold: Instant,
    /// A dragged volume not sent to the player yet, and when one was sent last.
    vol_pending: Option<f64>,
    vol_sent: Instant,
    local_tags: HashMap<String, tags::Tags>,
    tags_asked: HashSet<String>,
    pub members: Vec<Vec<String>>,
    pub(super) member_src: Vec<genius::Part>,
    pub notes: Notes,
    pub anns: Vec<Annotation>,
    pub ann_match: Vec<Option<usize>>,
    pub ai_notes: Vec<Option<String>>,
    /// genius+ai: the AI part's state (`notes` is the Genius part) and why Genius gave nothing.
    pub ai_state: Notes,
    pub genius_err: Option<String>,
    pub note: Option<(String, Instant)>,
    /// Cava, minimal view, lyric versions and the big line (see extras.rs).
    pub x: Extras,
    /// The Spotify client and library (off unless `spotify = true`).
    pub spot: Spot,
    pub(super) tasks: Vec<JoinHandle<()>>,
    /// Key table, open windows and the settings window's own state (see ux.rs).
    pub ux: super::ux::Ux,
}

impl Model {
    pub fn new(args: Args, layout: Layout, cfg: PathBuf, tx: UnboundedSender<AppEvent>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("music_display/", env!("CARGO_PKG_VERSION"), " (gui; https://lrclib.net)"))
            .timeout(Duration::from_secs(15))
            .build()
            .expect("http client");
        let spot = Spot::new(&args, tx.clone(), client.clone());
        let mut m = Model {
            ctl: Ctl::new(&args.player),
            tx,
            client,
            snap: None,
            anchor: (Instant::now(), 0.0),
            track_key: 0,
            art_url: String::new(),
            cover: None,
            cover_fresh: false,
            year: None,
            year_done: false,
            genre: None,
            mb_genre: None,
            lyrics: None,
            offset: args.offset,
            chords_on: args.chords,
            show_diagrams: layout.panels.shown(PanelId::Chords),
            sheet: None,
            aligned: None,
            chords_state: Chords::Idle,
            transpose: 0,
            capo_override: None,
            translate_mode: args.start_translate_mode(),
            translations: HashMap::new(),
            translate_backend: String::new(),
            tr: Tr::default(),
            lines_hash: 0,
            browse: Browse::default(),
            line_anim: Default::default(),
            lyric_dims: (0.0, 0.0),
            seek_preview: None,
            player_seen: Instant::now() - Duration::from_secs(10),
            vol_hold: Instant::now() - Duration::from_secs(5),
            vol_pending: None,
            vol_sent: Instant::now() - Duration::from_secs(5),
            local_tags: HashMap::new(),
            tags_asked: HashSet::new(),
            members: Vec::new(),
            member_src: Vec::new(),
            notes: Notes::Idle,
            anns: Vec::new(),
            ann_match: Vec::new(),
            ai_notes: Vec::new(),
            ai_state: Notes::Idle,
            genius_err: None,
            note: None,
            x: Extras::new(&args),
            spot,
            tasks: Vec::new(),
            ux: super::ux::Ux::new(Default::default()),
            gui: Default::default(),
            args,
            layout,
            cfg,
        };
        if m.args.annotations {
            m.layout.panels.set(PanelId::Annotations, true);
        }
        m
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.note = Some((msg.into(), Instant::now()));
    }

    pub fn playing(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.status == Status::Playing)
    }

    /// Playback position, interpolated between polls.
    pub fn position(&self) -> f64 {
        let (at, base) = self.anchor;
        let t = if self.playing() { base + at.elapsed().as_secs_f64() } else { base };
        match &self.snap {
            Some(s) if s.length > 0.0 => t.clamp(0.0, s.length),
            _ => t.max(0.0),
        }
    }

    fn set_position(&mut self, p: f64) {
        self.anchor = (Instant::now(), p);
    }

    // ── events ──

    pub fn apply(&mut self, ev: AppEvent) {
        self.extras_event(&ev);
        match ev {
            AppEvent::Player(Some(s)) => {
                self.spot.set_mpris(true);
                if self.remote_active() {
                    self.player_seen = Instant::now() - Duration::from_secs(10); // MPRIS takes over
                }
                self.on_snapshot(s);
                self.spot.check_liked(self.playing_uri());
            }
            AppEvent::Player(None) => {
                self.spot.set_mpris(false);
                // no MPRIS player: show what the Spotify API says plays (the phone…)
                if !self.remote_active() {
                    if self.snap.is_some() {
                        self.snap = None;
                        self.track_key = 0;
                    }
                    // a blip of the MPRIS player must not bring back an old API answer
                    if let Some(s) = self.spot.remote_snapshot_within(spotify::REMOTE_FRESH) {
                        self.on_snapshot(s);
                    }
                }
                self.spot.check_liked(self.playing_uri());
            }
            AppEvent::Tags { url, tags } => {
                if let Some(t) = tags {
                    self.local_tags.insert(url.clone(), t);
                }
                // re-run the current snapshot with the tags merged in
                // (as if from the player, but it says nothing new: leave the clock and the
                // "last heard from the player" time alone)
                if let Some(mut s) = self.snap.clone().filter(|s| s.url == url) {
                    let seen = self.player_seen;
                    s.position = self.position();
                    self.on_snapshot(s);
                    self.player_seen = seen;
                }
            }
            AppEvent::Spotify(r) => {
                let remote = matches!(r, spotify::Reply::Remote(Ok(_)));
                self.spot.on_reply(r);
                self.take_spot_note();
                if remote && !self.spot.mpris() {
                    match self.spot.remote_snapshot() {
                        Some(s) => self.on_snapshot(s),
                        None if self.remote_active() => self.snap = None,
                        None => {}
                    }
                    self.spot.check_liked(self.playing_uri());
                }
            }
            AppEvent::Year { key, year } if key == self.track_key => {
                self.year = year;
                self.year_done = true;
            }
            AppEvent::Genre { key, genres } if key == self.track_key => {
                self.mb_genre = genres;
                self.refresh_genre();
            }
            AppEvent::Lyrics { key, lyrics } if key == self.track_key => {
                self.set_lyrics(lyrics);
                self.realign();
                self.realign_members();
                self.ensure_translation();
                self.rematch_annotations();
                // AI notes are per line number: another lyrics version needs its own
                if self.args.annotations_mode.uses_ai() && !matches!(self.notes, Notes::Idle) {
                    self.reset_annotations();
                }
                self.ensure_annotations();
            }
            AppEvent::Cover { url, loaded } if url == self.art_url => {
                self.cover = loaded.map(|l| l.image);
                self.cover_fresh = true;
            }
            AppEvent::Chords { key, sheet, capo } if key == self.track_key => {
                self.capo_override = capo.filter(|c| *c <= 12);
                match sheet {
                    Ok(Some(mut s)) => {
                        if s.key.is_none() {
                            s.key = chords::estimate_key(&s);
                        }
                        self.sheet = Some(s);
                        self.chords_state = Chords::Ready;
                    }
                    Ok(None) => {
                        self.sheet = None;
                        self.chords_state = Chords::Missing;
                    }
                    Err(e) => {
                        self.sheet = None;
                        self.chords_state = Chords::Failed(e);
                    }
                }
                self.realign();
            }
            AppEvent::Members { key, parts } if key == self.track_key => {
                self.member_src = parts;
                self.realign_members();
            }
            AppEvent::Translated { key, lang, lines } => self.on_translated(key, lang, lines),
            AppEvent::Annotations { key, result } if key == self.track_key => match result {
                Ok(a) => {
                    self.anns = a;
                    self.rematch_annotations();
                    self.notes = Notes::Ready;
                }
                // genius+ai: the AI part still comes
                Err(e) if self.args.annotations_mode == AnnotationMode::GeniusAi => {
                    self.genius_err = Some(e);
                    self.notes = Notes::Ready;
                }
                Err(e) => self.notes = Notes::Failed(e),
            },
            AppEvent::AiNotes { key, result } if key == self.track_key => {
                let state = if self.args.annotations_mode == AnnotationMode::GeniusAi { &mut self.ai_state } else { &mut self.notes };
                match result {
                    Ok(n) => {
                        self.ai_notes = n;
                        *state = Notes::Ready;
                    }
                    Err(e) => *state = Notes::Failed(e),
                }
            }
            _ => {}
        }
    }

    fn on_snapshot(&mut self, mut s: Snapshot) {
        if s.url.starts_with("file://") {
            self.merge_local_tags(&mut s);
        }
        // several players at once: stay with the one on screen unless another takes over
        if identity::ignore_other_player(self.snap.as_ref(), &s, self.player_seen.elapsed()) {
            return;
        }
        self.player_seen = Instant::now();
        let mut key = s.key();
        // the same recording under another spelling (edition, remaster) is not a new track
        if key != self.track_key && identity::same_recording(self.snap.as_ref(), self.track_key, self.position(), &s) {
            key = self.track_key;
        }
        let new_track = key != self.track_key || self.snap.is_none();
        if !new_track {
            if let Some(o) = &self.snap {
                let art = self.cover.is_some().then_some(self.art_url.as_str());
                identity::keep_known(o, &mut s, art);
            }
        }
        // our own volume change wins for a moment, so the poll does not snap the slider back
        if !identity::volume_settled(self.vol_hold.elapsed()) {
            if let Some(v) = self.snap.as_ref().and_then(|o| o.volume) {
                s.volume = s.volume.map(|_| v);
            }
        }
        let drift = (self.position() - s.position).abs();
        let changed = self.snap.as_ref().is_none_or(|o| o.status != s.status);
        if new_track || changed || drift > 0.35 {
            self.set_position(s.position);
        }
        let new_art = s.art_url != self.art_url;
        if new_track {
            self.track_key = key;
            for t in self.tasks.drain(..) {
                t.abort();
            }
            self.tr.reset();
            self.offset = self.args.offset;
            self.sheet = None;
            self.aligned = None;
            self.transpose = 0;
            self.capo_override = None;
            self.member_src.clear();
            self.members.clear();
            self.lyrics = None;
            self.lines_hash = 0;
            self.chords_state = Chords::Idle;
            self.browse.follow();
            self.seek_preview = None;
            self.reset_annotations();
            self.mb_genre = None;
            self.year = year::from_content_created(&s.content_created);
            self.year_done = self.year.is_some();
            self.spawn_lookups(&s, key);
        }
        if new_art {
            self.art_url = s.art_url.clone();
            if s.art_url.is_empty() {
                self.cover = None;
                self.cover_fresh = true;
            } else {
                let (c, tx, url) = (self.client.clone(), self.tx.clone(), s.art_url.clone());
                let look = cover::Look { surface: None, primary: None, strength: 0.0, cover_theme: false };
                tokio::spawn(async move {
                    let loaded = cover::load(c, url.clone(), look).await;
                    let _ = tx.send(AppEvent::Cover { url, loaded });
                });
            }
        }
        self.snap = Some(s);
        self.refresh_genre();
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
        identity::merge_tags(s, t);
    }

    /// Lyrics arrived (or another version was picked): a translation made from other lines is
    /// stale, whatever its length.
    fn set_lyrics(&mut self, lyrics: Lyrics) {
        self.lyrics = Some(lyrics);
        let h = self.lines().map_or(0, |l| music_display::util::fnv1a(&l.join("\n")));
        if h != self.lines_hash {
            let key = self.track_key;
            self.translations.retain(|(k, _), _| *k != key);
            self.tr.reset();
            self.lines_hash = h;
        }
    }

    fn spawn_lookups(&mut self, s: &Snapshot, key: u64) {
        let (c, tx) = (self.client.clone(), self.tx.clone());
        let (a, t, al, len) = (s.artist.clone(), s.title.clone(), s.album.clone(), s.length);
        if self.year.is_none() {
            let (c, tx, a, t, al) = (c.clone(), tx.clone(), a.clone(), t.clone(), al.clone());
            self.tasks.push(tokio::spawn(async move {
                let year = year::resolve(c, a, t, al).await;
                let _ = tx.send(AppEvent::Year { key, year });
            }));
        }
        if s.genre.trim().is_empty() && self.args.genre_lookup {
            let (c, tx, a, t) = (c.clone(), tx.clone(), a.clone(), t.clone());
            self.tasks.push(tokio::spawn(async move {
                let genres = genre::lookup(&c, &a, &t).await;
                let _ = tx.send(AppEvent::Genre { key, genres });
            }));
        }
        {
            let (c, tx, a, t, al) = (c.clone(), tx.clone(), a.clone(), t.clone(), al.clone());
            let policy = music_display::lyrics::Policy {
                retries: self.args.lyrics_retries,
                extra: self.args.lyrics_fallback.then(Default::default),
                ..Default::default()
            };
            self.tasks.push(tokio::spawn(async move {
                let lyrics = music_display::lyrics::fetch(c, a, t, al, len, policy).await;
                let _ = tx.send(AppEvent::Lyrics { key, lyrics });
            }));
        }
        if self.chords_on || self.args.chords_ug {
            self.fetch_chords(key, &s.artist, &s.title);
        }
        if self.args.member_parts {
            let (c, tx, a, t) = (c, tx, a, t);
            self.tasks.push(tokio::spawn(async move {
                let parts = genius::fetch(c, a, t).await;
                let _ = tx.send(AppEvent::Members { key, parts });
            }));
        }
    }

    pub fn fetch_chords(&mut self, key: u64, artist: &str, title: &str) {
        self.chords_state = Chords::Fetching;
        let dir = self.args.chords_dir.clone().unwrap_or_else(chords::default_dir);
        let (tx, a, t, c, use_ug) = (self.tx.clone(), artist.to_string(), title.to_string(), self.client.clone(), self.args.chords_ug);
        self.tasks.push(tokio::spawn(async move {
            let sheet = ug::resolve(c, dir, use_ug, a, t).await;
            let capo = super::extras::load_capo(key).await;
            let _ = tx.send(AppEvent::Chords { key, sheet, capo });
        }));
    }

    fn refresh_genre(&mut self) {
        let mpris = self.snap.as_ref().map(|s| s.genre.clone());
        self.genre = genre::resolve(mpris.as_deref(), None, self.mb_genre.clone());
    }

    // ── lyrics ──

    pub fn lines(&self) -> Option<Vec<String>> {
        match self.lyrics.as_ref()? {
            Lyrics::Synced(l) => Some(l.iter().map(|(_, s)| s.clone()).collect()),
            Lyrics::Plain(l) => Some(l.clone()),
            _ => None,
        }
    }

    pub fn active_line(&self) -> Option<usize> {
        match self.lyrics.as_ref()? {
            Lyrics::Synced(l) => {
                let t = self.position() + self.offset;
                l.partition_point(|(ts, _)| *ts <= t).checked_sub(1)
            }
            _ => None,
        }
    }

    /// How far into the active line we are, 0..=1 (for the karaoke fill): the terminal front
    /// end's own guess from the line times.
    pub fn line_fraction(&self) -> f64 {
        let (Some(i), Some(Lyrics::Synced(l))) = (self.active_line(), self.lyrics.as_ref()) else { return 0.0 };
        if l[i].1.trim() == "♪" {
            return 0.0;
        }
        crate::ui::line_progress(self.position() + self.offset, l[i].0, l.get(i + 1).map(|x| x.0), &l[i].1)
    }

    /// The synced lines with their times, if the lyrics are synced.
    pub fn synced(&self) -> Option<&[(f64, String)]> {
        match self.lyrics.as_ref()? {
            Lyrics::Synced(l) => Some(l),
            _ => None,
        }
    }

    /// Line `i` as written (synced or plain), without copying.
    pub fn line_at(&self, i: usize) -> Option<&str> {
        match self.lyrics.as_ref()? {
            Lyrics::Synced(l) => l.get(i).map(|x| x.1.as_str()),
            Lyrics::Plain(l) => l.get(i).map(String::as_str),
            _ => None,
        }
    }

    /// Scroll the lyrics away from the playing line by `rows` (negative = up) without touching
    /// playback; auto-follow resumes after a few idle seconds or on [`Self::follow`].
    pub fn browse_by(&mut self, rows: f32) {
        self.browse.by(rows);
    }

    /// Hand the lyrics view back to the playing line at once.
    pub fn follow(&mut self) {
        self.browse.follow();
    }

    pub fn nudge(&mut self, d: f64) {
        self.offset = (((self.offset + d) * 10.0).round() / 10.0).clamp(-60.0, 60.0);
    }

    pub fn translation(&self) -> Option<&Vec<String>> {
        if self.translate_mode == TranslateMode::Off {
            return None;
        }
        self.translations.get(&(self.track_key, self.args.translate_lang.clone()))
    }

    pub fn set_translate_mode(&mut self, mode: TranslateMode) {
        self.translate_mode = mode;
        if mode != TranslateMode::Off {
            // a new request is allowed again (a failed or skipped one is tried afresh)
            self.tr.state = super::translation::TrState::Idle;
        }
        self.ensure_translation();
    }

    pub(super) fn ai_job(&mut self, lines: &[String]) -> Option<(ai::Cfg, ai::Song)> {
        if self.args.translate_engine != TranslateEngine::Ai {
            return None;
        }
        let cfg = self.ai_cfg()?;
        let snap = self.snap.as_ref()?;
        Some((
            cfg,
            ai::Song {
                title: snap.title.clone(),
                artist: snap.artist.clone(),
                album: snap.album.clone(),
                genre: self.genre.clone(),
                year: self.year.clone(),
                source_lang: "the original language (detect it)".into(),
                target_lang: self.args.translate_lang.clone(),
                lines: lines.to_vec(),
            },
        ))
    }

    pub fn ai_cfg(&mut self) -> Option<ai::Cfg> {
        let provider = ai::Provider::parse(&self.args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
        if self.args.ai_model.trim().is_empty() {
            self.say("AI needs a model (settings → AI); using the standard engine");
            return None;
        }
        Some(ai::Cfg::new(provider, &self.args.ai_model, &self.args.ai_endpoint, &self.args.ai_key_env))
    }

    fn realign_members(&mut self) {
        self.members = match self.lines() {
            Some(l) if !self.member_src.is_empty() => genius::attribute(&self.member_src, &l),
            _ => Vec::new(),
        };
    }

    // ── chords ──

    pub fn realign(&mut self) {
        self.aligned = match (&self.sheet, &self.lyrics) {
            (Some(sh), Some(Lyrics::Synced(l))) => Some(chords::align_with(sh, l, self.args.chords_match)),
            _ => None,
        };
    }

    /// The capo a track starts with: the sheet's own, or none with `auto_zero_capo`.
    pub fn default_capo(&self) -> u8 {
        if self.args.auto_zero_capo { 0 } else { self.sheet.as_ref().map_or(0, |s| s.capo) }
    }

    pub fn effective_capo(&self) -> u8 {
        self.capo_override.unwrap_or_else(|| self.default_capo())
    }

    pub fn chord_shift(&self) -> i32 {
        chords::capo_shift(self.sheet.as_ref().map_or(0, |s| s.capo), self.effective_capo(), self.transpose)
    }

    pub fn keys(&self) -> Option<(chords::KeyInfo, chords::KeyInfo)> {
        let sh = self.sheet.as_ref()?;
        let k = chords::parse_key(sh.key.as_deref()?)?;
        Some((k.shift(sh.capo as i32), k.shift(self.chord_shift())))
    }

    pub fn use_flats(&self) -> bool {
        match self.keys() {
            Some((_, shapes)) => shapes.prefers_flats(),
            None => self.aligned.as_ref().is_some_and(|a| a.flats),
        }
    }

    pub fn toggle_chords(&mut self) {
        self.chords_on = !self.chords_on;
        if self.chords_on && matches!(self.chords_state, Chords::Idle) {
            if let Some(s) = self.snap.clone() {
                let key = self.track_key;
                self.fetch_chords(key, &s.artist, &s.title);
            }
        }
    }

    /// Every distinct chord of the song in the order it first appears, shifted.
    pub fn song_chords(&self) -> Vec<String> {
        let Some(a) = self.aligned.as_ref().filter(|a| a.usable_at(self.args.chords_match)) else { return Vec::new() };
        let (n, flats) = (self.chord_shift(), self.use_flats());
        let mut out: Vec<String> = Vec::new();
        for line in &a.per_line {
            for (_, name) in line {
                let c = chords::transpose(name, n, flats);
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }

    // ── notes ──

    pub fn annotations_shown(&self) -> bool {
        self.layout.panels.shown(PanelId::Annotations)
    }

    pub fn rematch_annotations(&mut self) {
        self.ann_match = match self.lines() {
            Some(l) => annotations::match_lines(&l, &self.anns),
            None => Vec::new(),
        };
    }

    /// The notes for lyric line `i`, Genius first (the same rule as the terminal UI).
    pub fn notes_for(&self, i: usize) -> Vec<Annotation> {
        annotations::notes_for(self.args.annotations_mode, &self.anns, &self.ann_match, &self.ai_notes, i)
    }

    pub fn reset_annotations(&mut self) {
        self.notes = Notes::Idle;
        self.ai_state = Notes::Idle;
        self.genius_err = None;
        self.anns.clear();
        self.ann_match.clear();
        self.ai_notes.clear();
    }

    /// The mode or the token changed: start over for this song.
    pub fn restart_annotations(&mut self) {
        self.reset_annotations();
        self.ensure_annotations();
    }

    pub fn ensure_annotations(&mut self) {
        if !self.annotations_shown() || !matches!(self.notes, Notes::Idle) {
            return;
        }
        if self.args.annotations_mode.uses_ai() && self.ai_blocked() {
            return;
        }
        let Some(lines) = self.lines().filter(|l| !l.is_empty()) else { return };
        let Some(snap) = self.snap.clone() else { return };
        let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
        match self.args.annotations_mode {
            AnnotationMode::Genius => {
                let Some(token) = annotations::token(&self.args.genius_token) else {
                    self.notes = Notes::Failed(annotations::NO_TOKEN.into());
                    return;
                };
                self.notes = Notes::Loading;
                self.tasks.push(tokio::spawn(async move {
                    let result = annotations::fetch(&c, &token, &snap.artist, &snap.title).await;
                    let _ = tx.send(AppEvent::Annotations { key, result });
                }));
            }
            AnnotationMode::Ai => {
                let Some(cfg) = self.ai_cfg() else {
                    self.notes = Notes::Failed("AI notes need a model (settings → AI)".into());
                    return;
                };
                self.notes = Notes::Loading;
                self.tasks.push(tokio::spawn(async move {
                    let result = annotations::ai_notes(&c, &cfg, &snap.artist, &snap.title, &lines).await;
                    let _ = tx.send(AppEvent::AiNotes { key, result });
                }));
            }
            // Genius first (shown as soon as it arrives), then one AI request that sees it
            AnnotationMode::GeniusAi => {
                let token = annotations::token(&self.args.genius_token);
                let cfg = self.ai_cfg();
                self.notes = Notes::Loading;
                self.ai_state = match cfg {
                    Some(_) => Notes::Loading,
                    None => Notes::Failed("AI extras need a model (settings → AI)".into()),
                };
                self.tasks.push(tokio::spawn(async move {
                    let genius = match token {
                        Some(t) => annotations::fetch(&c, &t, &snap.artist, &snap.title).await,
                        None => Err(annotations::NO_TOKEN.to_string()),
                    };
                    let _ = tx.send(AppEvent::Annotations { key, result: genius.clone() });
                    let Some(cfg) = cfg else { return };
                    let per_line = annotations::genius_per_line(&lines, &genius.unwrap_or_default());
                    let result = annotations::ai_extras(&c, &cfg, &snap.artist, &snap.title, &lines, &per_line).await;
                    let _ = tx.send(AppEvent::AiNotes { key, result });
                }));
            }
        }
    }

    // ── transport ──

    /// What is on screen came from the Spotify API: controls go to the API.
    pub fn remote_active(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.player == spotify::REMOTE)
    }

    /// The Spotify URI of the song playing, if it is a Spotify track.
    pub fn playing_uri(&self) -> Option<String> {
        self.snap.as_ref().and_then(|s| spotify::track_uri(&s.track_id))
    }

    /// Move a note the Spotify side wants shown to the note line.
    pub fn take_spot_note(&mut self) {
        if let Some(n) = self.spot.note.take() {
            self.say(n);
        }
    }

    pub fn play_pause(&mut self) {
        let p = self.position();
        let was_playing = self.playing();
        if self.remote_active() {
            self.spot.cmd(if was_playing { spotify::Cmd::Pause } else { spotify::Cmd::Resume });
        } else {
            self.ctl.send(&["play-pause"]);
        }
        if let Some(s) = &mut self.snap {
            s.status = if s.status == Status::Playing { Status::Paused } else { Status::Playing };
        }
        self.set_position(p);
        self.take_spot_note();
    }

    pub fn next(&mut self) {
        self.player_cmd(&["next"], spotify::Cmd::Next);
    }

    pub fn prev(&mut self) {
        self.player_cmd(&["previous"], spotify::Cmd::Prev);
    }

    /// A playback command for whatever is on screen: playerctl, or the API for another device.
    fn player_cmd(&mut self, args: &[&str], cmd: spotify::Cmd) {
        if self.remote_active() {
            self.spot.cmd(cmd);
            self.take_spot_note();
        } else {
            self.ctl.send(args);
        }
    }

    pub fn seek(&mut self, secs: f64) {
        let max = self.snap.as_ref().map_or(0.0, |s| s.length);
        let secs = if max > 0.0 { secs.clamp(0.0, max) } else { secs.max(0.0) };
        self.set_position(secs);
        if self.remote_active() {
            self.player_cmd(&[], spotify::Cmd::Seek(secs));
        } else {
            self.ctl.seek_to(secs);
        }
    }

    /// Set the volume now (a key press).
    #[allow(dead_code)] // for the volume / mute keys
    pub fn set_volume(&mut self, v: f64) {
        self.show_volume(v);
        self.vol_pending = None;
        self.send_volume(v);
    }

    /// The slider moves: show the value at once, send it at most every 150 ms (and on release,
    /// see [`Self::flush_volume`]), so a drag is not a process or an API call per frame.
    pub fn drag_volume(&mut self, v: f64) {
        let v = self.show_volume(v);
        self.vol_pending = Some(v);
        self.flush_volume(false);
    }

    /// Send the last dragged volume, when the debounce time has passed (or `force`).
    pub fn flush_volume(&mut self, force: bool) {
        let Some(v) = self.vol_pending else { return };
        if force || self.vol_sent.elapsed() >= Duration::from_millis(150) {
            self.vol_pending = None;
            self.send_volume(v);
        }
    }

    fn show_volume(&mut self, v: f64) -> f64 {
        let v = v.clamp(0.0, 1.0);
        if let Some(s) = &mut self.snap {
            s.volume = Some(v);
        }
        self.vol_hold = Instant::now();
        v
    }

    fn send_volume(&mut self, v: f64) {
        self.vol_sent = Instant::now();
        self.player_cmd(&["volume", &format!("{v:.3}")], spotify::Cmd::Volume(v));
    }

    pub fn toggle_shuffle(&mut self) {
        if let Some(s) = self.snap.as_mut().filter(|s| s.shuffle.is_some()) {
            s.shuffle = s.shuffle.map(|b| !b);
            let on = s.shuffle == Some(true);
            self.player_cmd(&["shuffle", "Toggle"], spotify::Cmd::Shuffle(on));
        }
    }

    pub fn cycle_repeat(&mut self) {
        if let Some(s) = self.snap.as_mut() {
            if let Some(r) = s.repeat.map(|r| r.next()) {
                s.repeat = Some(r);
                self.player_cmd(&["loop", r.arg()], spotify::Cmd::Repeat(r));
            }
        }
    }

    /// Persist the layout to the shared config file.
    pub fn save_layout(&mut self) {
        if let Err(e) = music_display::config::save_layout(&self.cfg, &self.layout) {
            self.say(format!("couldn't save {}: {e}", self.cfg.display()));
        }
    }

    /// Persist some keys of the shared config file.
    pub fn save_settings(&mut self, values: &[(&str, toml::Value)]) {
        if let Err(e) = music_display::config::save(&self.cfg, values) {
            self.say(format!("couldn't save {}: {e}", self.cfg.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn snap(artist: &str, title: &str) -> Snapshot {
        let line = format!("Playing\x1f5000000\x1f{title}\x1f{artist}\x1fAlbum\x1f200000000\x1f\x1f2\x1f1\x1f1999-01-02T00:00:00Z\x1f0.5\x1f\x1fid\x1fvlc\x1f[Rock, Pop]");
        music_display::player::parse(&line).unwrap()
    }

    fn model() -> (Model, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        music_display::util::enter_test_mode();
        let args = Args::parse_from(["music_display", "--genre-lookup", "false", "--no-cache-translations"]);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let layout = Layout::default();
        (Model::new(args, layout, std::env::temp_dir().join("md-gui-test.toml"), tx), rx)
    }

    #[tokio::test]
    async fn a_snapshot_fills_the_metadata_and_resets_per_track_state() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        assert_eq!(m.snap.as_ref().unwrap().title, "Song");
        assert_eq!(m.year.as_deref(), Some("1999"));
        assert_eq!(m.genre.as_deref(), Some("Rock / Pop"));
        let k = m.track_key;
        m.apply(AppEvent::Lyrics { key: k, lyrics: Lyrics::Synced(vec![(0.0, "a".into()), (10.0, "b".into())]) });
        assert_eq!(m.lines().unwrap().len(), 2);
        // a stale result for another track is dropped
        m.apply(AppEvent::Lyrics { key: k ^ 1, lyrics: Lyrics::NotFound });
        assert!(matches!(m.lyrics, Some(Lyrics::Synced(_))));
        // the next track starts clean
        let mut other = snap("Other", "Tune");
        other.track_id = "other-id".into();
        other.position = 0.5;
        m.apply(AppEvent::Player(Some(other)));
        assert!(m.lyrics.is_none() && m.sheet.is_none() && m.transpose == 0);
    }

    #[tokio::test]
    async fn the_active_line_and_karaoke_fraction_follow_the_clock() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let k = m.track_key;
        m.apply(AppEvent::Lyrics { key: k, lyrics: Lyrics::Synced(vec![(0.0, "a".into()), (10.0, "b".into()), (20.0, "c".into())]) });
        m.seek(12.5);
        m.snap.as_mut().unwrap().status = Status::Paused;
        assert_eq!(m.active_line(), Some(1));
        let f = m.line_fraction();
        // the terminal front end's guess: a 1-character line takes ~2 s, lit a little early
        assert!((f - 0.859).abs() < 0.01, "{f}");
        m.nudge(-3.0);
        assert_eq!(m.active_line(), Some(0));
        m.seek(0.0);
        m.offset = 0.0;
        assert_eq!(m.active_line(), Some(0));
    }

    #[tokio::test]
    async fn a_respelled_snapshot_of_the_same_recording_keeps_the_track_and_the_volume_hold() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let k = m.track_key;
        m.apply(AppEvent::Lyrics { key: k, lyrics: Lyrics::Plain(vec!["a".into()]) });
        // same player id, new edition name: still the same song, lyrics stay
        m.apply(AppEvent::Player(Some(snap("Band", "Song (2011 Remaster)"))));
        assert_eq!(m.track_key, k);
        assert!(m.lyrics.is_some());
        assert_eq!(m.snap.as_ref().unwrap().title, "Song", "what was known is kept");
        // our volume wins over a poll for a moment
        m.set_volume(0.9);
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        assert_eq!(m.snap.as_ref().unwrap().volume, Some(0.9));
    }

    #[tokio::test]
    async fn a_second_player_does_not_take_over_while_ours_reports() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let mut other = snap("Other", "Tune");
        other.player = "mpv".into();
        other.status = Status::Paused;
        m.apply(AppEvent::Player(Some(other)));
        assert_eq!(m.snap.as_ref().unwrap().title, "Song");
    }

    #[tokio::test]
    async fn the_volume_slider_sends_at_most_one_command_per_debounce() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        m.vol_sent = Instant::now();
        m.drag_volume(0.3);
        m.drag_volume(0.4);
        assert_eq!(m.vol_pending, Some(0.4), "held back, not sent per frame");
        assert_eq!(m.snap.as_ref().unwrap().volume, Some(0.4), "but shown at once");
        m.flush_volume(true);
        assert_eq!(m.vol_pending, None);
    }

    #[tokio::test]
    async fn seeks_are_clamped_and_nudges_round_to_a_tenth() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        m.snap.as_mut().unwrap().status = Status::Paused;
        m.seek(9999.0);
        assert_eq!(m.position(), 200.0);
        m.seek(-5.0);
        assert_eq!(m.position(), 0.0);
        for _ in 0..3 {
            m.nudge(0.1);
        }
        assert_eq!(m.offset, 0.3);
    }

    #[tokio::test]
    async fn chords_are_shifted_by_capo_and_transpose() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let k = m.track_key;
        m.apply(AppEvent::Lyrics {
            key: k,
            lyrics: Lyrics::Synced(vec![(0.0, "hello darkness my old friend".into()), (5.0, "i've come to talk with you again".into())]),
        });
        let sheet = chords::parse_chordpro("[Am]hello darkness [G]my old friend\n[C]i've come to talk [F]with you again\n");
        m.apply(AppEvent::Chords { key: k, sheet: Ok(Some(sheet)), capo: None });
        assert!(matches!(m.chords_state, Chords::Ready));
        m.args.chords_match = 0;
        m.realign();
        let before = m.song_chords();
        assert!(before.contains(&"Am".to_string()), "{before:?}");
        m.transpose = 2;
        let after = m.song_chords();
        assert!(after.contains(&"Bm".to_string()), "{after:?}");
        m.apply(AppEvent::Chords { key: k ^ 1, sheet: Ok(None), capo: None });
        assert!(matches!(m.chords_state, Chords::Ready), "stale answers are dropped");
    }

    #[tokio::test]
    async fn auto_zero_capo_shows_the_sounding_chords_and_your_own_capo_wins() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let k = m.track_key;
        m.apply(AppEvent::Lyrics {
            key: k,
            lyrics: Lyrics::Synced(vec![(0.0, "hello darkness my old friend".into()), (5.0, "i've come to talk with you again".into())]),
        });
        let sheet = chords::parse_chordpro("{capo: 2}\n[Am]hello darkness [G]my old friend\n[C]i've come to talk [F]with you again\n");
        m.apply(AppEvent::Chords { key: k, sheet: Ok(Some(sheet)), capo: None });
        m.args.chords_match = 0;
        m.realign();
        assert_eq!(m.effective_capo(), 2);
        assert!(m.song_chords().contains(&"Am".to_string()));
        m.args.auto_zero_capo = true;
        assert_eq!((m.effective_capo(), m.default_capo()), (0, 0));
        assert!(m.song_chords().contains(&"Bm".to_string()), "{:?}", m.song_chords());
        // stepping up to the sheet's capo is a choice of its own, stepping back to 0 clears it
        m.change_capo(1);
        m.change_capo(1);
        assert_eq!((m.capo_override, m.effective_capo()), (Some(2), 2));
        m.change_capo(-1);
        m.change_capo(-1);
        assert_eq!(m.capo_override, None);
        // a capo remembered for the track beats the automatic one
        m.apply(AppEvent::Chords { key: k, sheet: Ok(Some(chords::parse_chordpro("{capo: 2}\n[Am]x\n"))), capo: Some(3) });
        assert_eq!(m.effective_capo(), 3);
        let _ = std::fs::remove_file(crate::gui::extras::capo_path(k));
    }

    #[tokio::test]
    async fn translation_modes_and_notes_use_the_same_events() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let k = m.track_key;
        m.apply(AppEvent::Lyrics { key: k, lyrics: Lyrics::Plain(vec!["Jag gillar bär".into()]) });
        assert!(m.translation().is_none(), "off by default");
        m.translate_mode = TranslateMode::Both;
        m.apply(AppEvent::Translated { key: k, lang: "en".into(), lines: Ok((vec!["I like berries".into()], "google".into())) });
        assert_eq!(m.translation().unwrap()[0], "I like berries");
        m.translate_mode = TranslateMode::Off;
        assert!(m.translation().is_none());
        // annotations
        m.layout.panels.set(PanelId::Annotations, true);
        m.apply(AppEvent::Lyrics { key: k, lyrics: Lyrics::Synced(vec![(0.0, "We build castles in the sand".into())]) });
        m.apply(AppEvent::Annotations {
            key: k,
            result: Ok(vec![Annotation { fragment: "We build castles in the sand".into(), text: "About fragile things".into(), source: "Genius".into(), url: None }]),
        });
        assert_eq!(m.notes_for(0)[0].text, "About fragile things");
        assert!(m.notes_for(1).is_empty());
    }

    #[tokio::test]
    async fn genius_ai_stacks_the_ai_extra_and_survives_a_missing_token() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(snap("Band", "Song"))));
        let k = m.track_key;
        m.args.annotations_mode = AnnotationMode::GeniusAi;
        m.apply(AppEvent::Lyrics { key: k, lyrics: Lyrics::Synced(vec![(0.0, "We build castles in the sand".into())]) });
        // Genius failed: the AI part still comes, and the Genius problem is kept as a line
        m.apply(AppEvent::Annotations { key: k, result: Err("no token".into()) });
        assert!(matches!(m.notes, Notes::Ready) && m.genius_err.as_deref() == Some("no token"));
        m.apply(AppEvent::AiNotes { key: k, result: Ok(vec![Some("Sand castles stand for impermanence".into())]) });
        assert!(matches!(m.ai_state, Notes::Ready));
        let n = m.notes_for(0);
        assert_eq!((n.len(), n[0].source.as_str()), (1, music_display::annotations::AI_LABEL));
        // with a Genius note too, the AI one is the labelled extra, Genius first
        m.apply(AppEvent::Annotations {
            key: k,
            result: Ok(vec![Annotation { fragment: "We build castles in the sand".into(), text: "About fragile things".into(), source: "Genius".into(), url: None }]),
        });
        let n = m.notes_for(0);
        assert_eq!((n.len(), n[0].source.as_str(), n[1].source.as_str()), (2, "Genius", music_display::annotations::AI_EXTRA_LABEL));
        // a mode change starts over
        m.restart_annotations();
        assert!(m.anns.is_empty() && m.ai_notes.is_empty());
    }
}
