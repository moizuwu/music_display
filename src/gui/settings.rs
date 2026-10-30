//! The settings window: tabs that mirror the terminal menu's groups (General, Lyrics, Chords,
//! Translation, Annotations, Spotify) plus the window-only Appearance and Layout tabs. Every
//! row uses the terminal menu's config keys (`app::Setting::config_keys`), applies at once and
//! is written to `config.toml` (text fields on Enter or when they lose focus, sliders when
//! released). The setters live on `Model` and have no egui in them, so the keys they save are
//! unit-tested.
use super::model::Model;
use crate::app::LANGS;
use egui::{RichText, Ui};
use music_display::cli::{AnnotationMode, AnnotationScroll, Args, TranslateEngine, TranslateMode};
use music_display::guiconf::GuiConf;
use music_display::layout::{Layout, PRESETS, PanelId};
use music_display::{ai, genius, player, spotifyd, AppEvent};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    General,
    Lyrics,
    Chords,
    Translation,
    Annotations,
    Spotify,
    Appearance,
    Layout,
}

impl Tab {
    const ALL: [(Tab, &'static str); 8] = [
        (Tab::General, "General"),
        (Tab::Lyrics, "Lyrics"),
        (Tab::Chords, "Chords"),
        (Tab::Translation, "Translation"),
        (Tab::Annotations, "Annotations"),
        (Tab::Spotify, "Spotify"),
        (Tab::Appearance, "Appearance"),
        (Tab::Layout, "Layout"),
    ];
}

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    pub tab: Tab,
    /// Text being typed per field, until it is committed.
    edits: HashMap<&'static str, String>,
}

/// Window keys that only take effect at the next start (the GUI's counterpart of
/// `Setting::needs_restart`, whose rows are terminal-only).
pub const RESTART_KEYS: &[&str] = &["transparent", "cover_theme", "remember_window"];

/// `transparent = true` in the main config means a see-through window: this opacity (percent)
/// unless `[gui]` sets `opacity` itself.
pub const TRANSPARENT_OPACITY: u8 = 85;

/// The terminal's `transparent` flag as the window's `opacity`, when `[gui]` leaves it open.
/// (`cover_theme` needs nothing: the window's accent follows the cover unless `[gui] accent` pins one.)
pub fn main_flags_into_gui(g: &mut GuiConf, transparent: bool, gui_sets_opacity: bool) {
    if transparent && !gui_sets_opacity {
        g.opacity = TRANSPARENT_OPACITY;
    }
}

/// Does the `[gui]` table of the file at `path` set `key`?
pub fn gui_sets(path: &Path, key: &str) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.parse::<toml::Table>().ok())
        .and_then(|t| t.get("gui").and_then(|g| g.get(key)).map(|_| true))
        .unwrap_or(false)
}

/// The preset the layout still is, if any.
pub fn preset_of(l: &Layout) -> Option<&'static str> {
    PRESETS.iter().copied().find(|n| Layout::preset(n).as_ref() == Some(l))
}

// ── setters (no egui) ───────────────────────────────────────────────────────

pub enum AiField {
    Model,
    Endpoint,
    KeyEnv,
}

impl Model {
    /// Write some keys of the shared config file and flash "saved".
    pub fn persist(&mut self, label: &str, values: &[(&str, toml::Value)]) {
        if values.is_empty() {
            return;
        }
        match music_display::config::save(&self.cfg, values) {
            Ok(()) => self.ux.saved = Some((label.to_string(), Instant::now())),
            Err(e) => self.say(format!("couldn't save {}: {e}", self.cfg.display())),
        }
    }

    pub fn set_player_source(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.args.player = name.to_string();
        self.ctl.set_player(name);
        self.persist("Player", &[("player", name.into())]);
    }

    pub fn set_chords_ug(&mut self, on: bool) {
        self.args.chords_ug = on;
        self.persist("Ultimate Guitar", &[("chords_ug", on.into())]);
    }

    pub fn set_chords_match(&mut self, n: u32) {
        self.args.chords_match = n.min(100);
        self.realign();
    }

    pub fn save_chords_match(&mut self) {
        self.persist("Chord match", &[("chords_match", (self.args.chords_match as i64).into())]);
    }

    pub fn set_member_parts(&mut self, on: bool) {
        self.args.member_parts = on;
        if on {
            if let Some(s) = self.snap.clone() {
                let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
                self.tasks.push(tokio::spawn(async move {
                    let parts = genius::fetch(c, s.artist, s.title).await;
                    let _ = tx.send(AppEvent::Members { key, parts });
                }));
            }
        } else {
            self.member_src.clear();
            self.members.clear();
        }
        self.persist("Member parts", &[("member_parts", on.into())]);
    }

    /// An AI feature is on: the one-time question comes before anything is sent.
    fn check_ai_notice(&mut self) {
        if self.args.translate_engine == TranslateEngine::Ai || (self.annotations_shown() && self.args.annotations_mode.uses_ai()) {
            self.ai_blocked();
        }
    }

    pub fn set_translate_engine(&mut self, e: TranslateEngine) {
        self.args.translate_engine = e;
        self.check_ai_notice();
        self.reset_translation();
        self.persist("Translation engine", &[("translate_engine", e.name().into())]);
    }

    pub fn set_ai_provider(&mut self, name: &str) {
        self.args.ai_provider = name.to_string();
        self.check_ai_notice();
        self.reset_translation();
        if self.args.annotations_mode.uses_ai() {
            self.restart_annotations();
        }
        self.persist("AI provider", &[("ai.provider", name.into())]);
    }

    /// Model, endpoint or the NAME of the key's environment variable (never a key).
    pub fn set_ai_field(&mut self, f: AiField, text: &str) {
        let text = text.trim().to_string();
        let (key, label) = match f {
            AiField::Model => {
                self.args.ai_model = text.clone();
                ("ai.model", "AI model")
            }
            AiField::Endpoint => {
                self.args.ai_endpoint = text.clone();
                ("ai.endpoint", "AI endpoint")
            }
            AiField::KeyEnv => {
                self.args.ai_key_env = text.clone();
                ("ai.api_key_env", "AI key variable")
            }
        };
        self.check_ai_notice();
        self.reset_translation();
        if self.args.annotations_mode.uses_ai() {
            self.restart_annotations();
        }
        self.persist(label, &[(key, text.into())]);
    }

    pub fn set_translate_mode_saved(&mut self, mode: TranslateMode) {
        self.set_translate_mode(mode);
        if mode != TranslateMode::Off {
            self.layout.panels.set(PanelId::Lyrics, true);
        }
        self.persist("Translation", &[("translate_mode", mode.name().into()), ("translate", (mode != TranslateMode::Off).into())]);
    }

    pub fn set_translate_lang(&mut self, lang: &str) {
        let lang = lang.trim();
        if lang.is_empty() {
            return;
        }
        self.args.translate_lang = lang.to_string();
        self.tr.reset();
        self.ensure_translation();
        self.persist("Language", &[("translate_lang", lang.into())]);
    }

    /// Add / remove one language from the never-translate list.
    pub fn toggle_skip(&mut self, code: &str) {
        let mut all = self.skip_langs();
        let code = music_display::translate::base_code(code);
        match all.iter().position(|c| *c == code) {
            Some(i) => {
                all.remove(i);
            }
            None => all.push(code),
        }
        self.args.translate_skip = all.join(",");
        self.reset_translation();
        self.persist("Never translate", &[("translate_skip", self.args.translate_skip.clone().into())]);
    }

    pub fn set_fallback(&mut self, on: bool) {
        self.args.translate_fallback = on;
        self.persist("LibreTranslate fallback", &[("translate_fallback", on.into())]);
    }

    pub fn set_fallback_url(&mut self, url: &str) {
        self.args.translate_fallback_url = url.trim().to_string();
        self.persist("LibreTranslate URL", &[("translate_fallback_url", self.args.translate_fallback_url.clone().into())]);
    }

    /// The LibreTranslate key (empty removes it). Saved to the file like the terminal config does,
    /// never shown again and never put in a message.
    pub fn set_fallback_key(&mut self, key: &str) {
        let key = key.trim().to_string();
        self.args.translate_fallback_key = (!key.is_empty()).then(|| key.clone());
        self.persist("LibreTranslate key", &[("translate_fallback_key", key.into())]);
    }

    pub fn set_cache_translations(&mut self, keep: bool) {
        self.args.no_cache_translations = !keep;
        self.persist("Keep translations", &[("cache_translations", keep.into())]);
    }

    pub fn set_genre_lookup(&mut self, on: bool) {
        self.args.genre_lookup = on;
        self.persist("Genre lookup", &[("genre_lookup", on.into())]);
    }

    pub fn set_annotations_shown(&mut self, on: bool) {
        self.layout.panels.set(PanelId::Annotations, on);
        self.args.annotations = on;
        self.check_ai_notice();
        self.ensure_annotations();
        self.persist("Annotations", &[("annotations", on.into())]);
    }

    pub fn set_annotations_mode(&mut self, mode: AnnotationMode) {
        self.args.annotations_mode = mode;
        self.check_ai_notice();
        self.restart_annotations();
        self.persist("Annotations source", &[("annotations_mode", mode.name().into())]);
    }

    /// The Genius token. An empty entry keeps the old one (use [`Model::clear_genius_token`]).
    /// Never shown, never in a message.
    pub fn set_genius_token(&mut self, token: &str) {
        let token = token.trim();
        if token.is_empty() {
            return;
        }
        self.args.genius_token = token.to_string();
        self.restart_annotations();
        self.persist("Genius token", &[("genius_token", token.into())]);
    }

    pub fn clear_genius_token(&mut self) {
        self.args.genius_token.clear();
        self.restart_annotations();
        self.persist("Genius token removed", &[("genius_token", "".into())]);
    }

    pub fn set_annotation_scroll(&mut self, s: AnnotationScroll) {
        self.args.annotation_scroll = s;
        self.persist("Annotation scroll", &[("annotation_scroll", s.name().into())]);
    }

    pub fn set_cava_shown(&mut self, on: bool) {
        if on != self.layout.panels.shown(PanelId::Cava) {
            self.toggle_cava();
        }
        let shown = self.layout.panels.shown(PanelId::Cava);
        self.persist("Cava", &[("cava", shown.into())]);
    }

    /// Cava as a column of its own (`thirdpanel`) instead of under the lyrics.
    pub fn set_third_column(&mut self, third: bool) {
        self.layout.cava_under_lyrics = !third;
        self.args.thirdpanel = third;
        if third && !self.x.minimal && !self.layout.panels.shown(PanelId::Cava) {
            self.toggle_cava();
        }
        self.save_layout();
        let shown = self.layout.panels.shown(PanelId::Cava);
        self.persist("Cava column", &[("thirdpanel", (third && shown).into()), ("cava", shown.into())]);
    }

    pub fn set_minimal_saved(&mut self, on: bool) {
        self.set_minimal(on);
        self.persist("Minimal view", &[("minimal_view", on.into())]);
    }

    // ── Spotify: every change rebuilds the client, which reads the flags only at start ──

    fn rebuild_spotify(&mut self) {
        let was_remote = self.remote_active();
        self.spot.rebuild(&self.args);
        if was_remote && !self.spot.on() {
            self.snap = None;
            self.track_key = 0;
        }
    }

    pub fn set_spotify_on(&mut self, on: bool) {
        self.args.spotify = on;
        self.rebuild_spotify();
        self.persist("Spotify", &[("spotify", on.into())]);
    }

    /// A Client ID is letters and digits. Returns why it was refused.
    pub fn set_spotify_client_id(&mut self, id: &str) -> Result<(), String> {
        let id = id.trim();
        if !id.is_empty() && !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("that doesn't look like a Client ID".into());
        }
        self.args.spotify_client_id = (!id.is_empty()).then(|| id.to_string());
        self.rebuild_spotify();
        self.persist("Client ID", &[("spotify_client_id", id.into())]);
        Ok(())
    }

    pub fn set_spotify_device(&mut self, name: &str) {
        self.args.spotify_device = name.trim().to_string();
        self.rebuild_spotify();
        self.persist("Device name", &[("spotify_device", self.args.spotify_device.clone().into())]);
    }

    pub fn set_use_spotifyd(&mut self, on: bool) {
        self.args.use_spotifyd = on;
        if on {
            if self.ux.spotifyd.is_none() {
                self.ux.spotifyd = spotifyd::start();
            }
        } else if let Some(mut c) = self.ux.spotifyd.take() {
            spotifyd::stop(&mut c);
        }
        // the client matches "this PC" by the device name only while spotifyd is in use
        self.rebuild_spotify();
        self.persist("Use spotifyd", &[("use_spotifyd", on.into())]);
    }

    pub fn set_kill_spotifyd(&mut self, on: bool) {
        self.args.no_kill_spotifyd = !on;
        self.persist("Stop spotifyd on exit", &[("kill_spotifyd_on_exit", on.into())]);
    }

    pub fn set_spotify_button(&mut self, on: bool) {
        self.args.no_spotify_button = !on;
        self.persist("Library button", &[("spotify_button", on.into())]);
    }

    pub fn set_spotify_queue_tab(&mut self, on: bool) {
        self.args.no_spotify_queue_tab = !on;
        self.spot.lib.queue_tab = on;
        if !on && self.spot.lib.tab == crate::browser::TAB_QUEUE {
            self.spot.lib.tab = 0;
            self.spot.lib.stack.clear();
        }
        self.persist("Queue tab", &[("spotify_queue_tab", on.into())]);
    }

    pub fn spotify_login(&mut self) {
        let id = self.args.spotify_client_id.clone().unwrap_or_default();
        self.spot.toggle_login(&id);
    }

    pub fn spotify_logout(&mut self) {
        self.spot.logout();
        self.ux.forget_login_cache();
    }

    // ── lyrics / playback numbers ──

    pub fn save_lyric_numbers(&mut self) {
        let a = &self.args;
        let v = [
            ("lyric_spacing", (a.lyric_spacing as i64).into()),
            ("lyric_pad", (a.lyric_pad as i64).into()),
            ("big_scale", (a.big_scale as i64).into()),
            ("interlude_threshold_secs", a.interlude_secs.into()),
            ("volume_step", (a.volume_step as i64).into()),
        ];
        self.persist("Lyrics", &v);
    }
}

// ── widgets ─────────────────────────────────────────────────────────────────

fn check(ui: &mut Ui, label: &str, v: bool) -> Option<bool> {
    let mut x = v;
    ui.checkbox(&mut x, label).changed().then_some(x)
}

fn combo<T: PartialEq + Clone>(ui: &mut Ui, id: &str, label: &str, cur: &T, opts: &[(T, &str)]) -> Option<T> {
    let shown = opts.iter().find(|(v, _)| v == cur).map_or("custom", |(_, n)| *n);
    let mut pick = None;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
            for (v, n) in opts {
                if ui.selectable_label(v == cur, *n).clicked() {
                    pick = Some(v.clone());
                }
            }
        });
    });
    pick
}

/// A slider: (it changed, the change is final: released or keyboard-set).
fn slider<N: egui::emath::Numeric>(ui: &mut Ui, label: &str, v: &mut N, range: std::ops::RangeInclusive<N>, suffix: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        let r = ui.add(egui::Slider::new(v, range).suffix(suffix));
        (r.changed(), r.drag_stopped() || (r.changed() && !r.dragged()))
    })
    .inner
}

/// A text field that commits on Enter or when it loses focus (Esc puts the old text back).
/// Returns the new text when it changed. `password` hides it and never fills it from `current`.
fn text_row(ui: &mut Ui, edits: &mut HashMap<&'static str, String>, id: &'static str, label: &str, current: &str, hint: &str, password: bool) -> Option<String> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.label(label);
        let buf = edits.entry(id).or_insert_with(|| current.to_string());
        let r = ui.add(egui::TextEdit::singleline(buf).password(password).hint_text(hint).desired_width(220.0));
        if r.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *buf = current.to_string();
            } else if buf.trim() != current {
                out = Some(buf.trim().to_string());
            }
        } else if !r.has_focus() && *buf != current {
            *buf = current.to_string();
        }
    });
    out
}

fn weak(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().weak()).wrap());
}

fn engine_name(e: TranslateEngine) -> &'static str {
    match e {
        TranslateEngine::Standard => "Standard (Google, free)",
        TranslateEngine::Ai => "An AI model",
    }
}

fn mode_name(m: TranslateMode) -> &'static str {
    match m {
        TranslateMode::Off => "Off",
        TranslateMode::Both => "Original + translation",
        TranslateMode::Translated => "Translation only",
    }
}

const PROVIDERS: [(&str, &str); 4] = [
    ("openai-compatible", "OpenAI-compatible (Ollama, LM Studio)"),
    ("gemini", "Google Gemini"),
    ("claude", "Anthropic Claude"),
    ("openai", "OpenAI"),
];

fn scroll_name(s: AnnotationScroll) -> &'static str {
    match s {
        AnnotationScroll::Manual => "Manual (wheel, e / E)",
        AnnotationScroll::Auto => "Automatic (follows the song)",
    }
}

fn ai_provider_name(args: &Args) -> String {
    let p = ai::Provider::parse(&args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
    p.name().to_string()
}

// ── the window ──────────────────────────────────────────────────────────────

impl super::Gui {
    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.m.ux.s.open;
        egui::Window::new("Settings").open(&mut open).default_width(440.0).default_height(520.0).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (t, name) in Tab::ALL {
                    ui.selectable_value(&mut self.m.ux.s.tab, t, name);
                }
            });
            ui.separator();
            let footer = 22.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((ui.available_height() - footer).max(80.0)).show(ui, |ui| {
                match self.m.ux.s.tab {
                    Tab::General => general(ui, &mut self.m),
                    Tab::Lyrics => lyrics(ui, &mut self.m),
                    Tab::Chords => chords(ui, &mut self.m),
                    Tab::Translation => translation(ui, &mut self.m),
                    Tab::Annotations => annotations(ui, &mut self.m),
                    Tab::Spotify => spotify(ui, &mut self.m),
                    Tab::Appearance => {
                        weak(ui, &format!("Only the window's own look lives in [gui] of config.toml. `transparent` in the main config sets the opacity to {TRANSPARENT_OPACITY} % when [gui] has none; the accent follows the cover unless you pick a fixed colour. Read at start (restart needed): {}.", RESTART_KEYS.join(", ")));
                        style_tab(ui, self);
                    }
                    Tab::Layout => {
                        self.layout_editor(ui);
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                match self.m.ux.saved_text() {
                    Some(s) => ui.label(RichText::new(format!("saved: {s}")).small().color(ui.visuals().hyperlink_color)),
                    None => ui.label(RichText::new("changes apply at once and are saved to config.toml").small().weak()),
                };
            });
        });
        self.m.ux.s.open = open;
    }
}

fn style_tab(ui: &mut Ui, g: &mut super::Gui) {
    super::style::appearance(ui, &mut g.m, &mut g.look);
}

fn general(ui: &mut Ui, m: &mut Model) {
    ui.heading("Player");
    let mut names: Vec<String> = player::PRESETS.iter().map(|p| p.to_string()).collect();
    if !names.contains(&m.args.player) {
        names.push(m.args.player.clone());
    }
    let opts: Vec<(String, &str)> = names.iter().map(|n| (n.clone(), n.as_str())).collect();
    if let Some(p) = combo(ui, "player", "Player source", &m.args.player, &opts) {
        m.set_player_source(&p);
    }
    if let Some(v) = check(ui, "Look up the genre (MusicBrainz) when the player gives none", m.args.genre_lookup) {
        m.set_genre_lookup(v);
    }
    let mut step = m.args.volume_step;
    let (ch, done) = slider(ui, "Volume step", &mut step, 1..=50, "%");
    if ch {
        m.args.volume_step = step;
    }
    if done {
        m.save_lyric_numbers();
    }
    let titles = [("off".to_string(), "off"), ("title".to_string(), "song title"), ("artist-title".to_string(), "artist - title")];
    if let Some(v) = combo(ui, "wintitle", "Song in the window title", &m.args.window_title, &titles) {
        m.args.window_title = v.clone();
        m.persist("Window title", &[("window_title", v.into())]);
    }
    ui.separator();
    ui.heading("View");
    if let Some(v) = check(ui, "Minimal view (lyrics only, large centred line)", m.x.minimal) {
        m.set_minimal_saved(v);
    }
    if let Some(v) = check(ui, "Cava bars (needs cava installed)", m.layout.panels.shown(PanelId::Cava)) {
        m.set_cava_shown(v);
    }
    if let Some(v) = check(ui, "Cava in a column of its own (third column)", !m.layout.cava_under_lyrics) {
        m.set_third_column(v);
    }
    let mut bars = m.args.cava_bars as u32;
    ui.horizontal(|ui| {
        ui.label("Cava bars");
        let r = ui.add(egui::DragValue::new(&mut bars).range(4..=1000).speed(1));
        if r.changed() {
            m.args.cava_bars = bars as u16;
        }
        if r.drag_stopped() || r.lost_focus() {
            m.persist("Cava bars", &[("cava_bars", (bars as i64).into())]);
        }
        weak(ui, "used the next time cava starts");
    });
    let cur = preset_of(&m.layout);
    let shown = cur.unwrap_or("custom");
    let mut chosen: Option<&str> = None;
    ui.horizontal(|ui| {
        ui.label("Layout preset");
        egui::ComboBox::from_id_salt("preset").selected_text(shown).show_ui(ui, |ui| {
            for name in PRESETS {
                if ui.selectable_label(cur == Some(*name), *name).clicked() {
                    chosen = Some(name);
                }
            }
        });
    });
    if let Some(name) = chosen {
        m.apply_preset(name);
        let minimal = m.x.minimal;
        m.persist("Layout preset", &[("minimal_view", minimal.into())]);
    }
    weak(ui, "Panels, fields and buttons are in the Layout tab. Press ? for the key list.");
    ui.separator();
    weak(ui, "Terminal-only settings (colour files and the cover tint that uses them, ASCII cover, mouse, hide colour-file note) are not here; the Appearance tab has the window's own colours.");
}

fn lyrics(ui: &mut Ui, m: &mut Model) {
    ui.heading("Lyrics");
    if let Some(v) = check(ui, "Light up the line word by word", m.args.karaoke) {
        m.args.karaoke = v;
        m.persist("Karaoke", &[("karaoke", v.into())]);
    }
    if let Some(v) = check(ui, "Big active line", m.args.big_lyrics) {
        m.args.big_lyrics = v;
        m.persist("Big active line", &[("big_lyrics", v.into())]);
    }
    let (mut sc, mut sp, mut pad, mut gap) = (m.args.big_scale, m.args.lyric_spacing, m.args.lyric_pad, m.args.interlude_secs);
    let mut ch = false;
    let mut done = false;
    for (c, d) in [
        slider(ui, "Big line size", &mut sc, 1..=3, "x"),
        slider(ui, "Line spacing", &mut sp, 0..=3, ""),
        slider(ui, "Side padding", &mut pad, 0..=10, ""),
        slider(ui, "Instrumental marker after a gap of", &mut gap, 1.0..=120.0, " s"),
    ] {
        ch |= c;
        done |= d;
    }
    if ch {
        (m.args.big_scale, m.args.lyric_spacing, m.args.lyric_pad, m.args.interlude_secs) = (sc, sp, pad, gap);
    }
    if done {
        m.save_lyric_numbers();
    }
    let mut off = m.args.offset;
    let (och, odone) = slider(ui, "Default lyric offset", &mut off, -10.0..=10.0, " s");
    if och {
        let off = (off * 10.0).round() / 10.0;
        m.args.offset = off;
        m.offset = off;
    }
    if odone {
        let off = m.args.offset;
        m.persist("Lyric offset", &[("offset", off.into())]);
    }
    let mut tries = m.args.lyrics_retries;
    let (rch, rdone) = slider(ui, "Lyrics lookup retries", &mut tries, 0..=10, "");
    if rch {
        m.args.lyrics_retries = tries;
    }
    if rdone {
        m.persist("Lyrics retries", &[("lyrics_retries", (tries as i64).into())]);
    }
    weak(ui, "The offset is the starting value for every song; , and . nudge the current one.");
    weak(ui, "Keys: b big line, x size, i spacing, u padding, a word by word (the keys change them for this run only, like in the terminal).");
}

fn chords(ui: &mut Ui, m: &mut Model) {
    ui.heading("Chords");
    if let Some(_v) = check(ui, "Chords above the lyrics (g)", m.chords_on) {
        m.toggle_chords();
        let on = m.chords_on;
        m.persist("Chords", &[("chords", on.into())]);
    }
    if let Some(v) = check(ui, "Also look on Ultimate Guitar (from the next track)", m.args.chords_ug) {
        m.set_chords_ug(v);
    }
    let mut n = m.args.chords_match;
    let (ch, done) = slider(ui, "Chord match threshold", &mut n, 0..=100, "%");
    if ch {
        m.set_chords_match(n);
    }
    if done {
        m.save_chords_match();
    }
    weak(ui, "Lower shows weaker matches of a sheet to the lyrics; 0 always shows it.");
    let mut ctx = m.args.chords_context as u32;
    ui.horizontal(|ui| {
        ui.label("Chords on the active line and");
        let r = ui.add(egui::DragValue::new(&mut ctx).range(0..=99).speed(0.2));
        ui.label("upcoming ones");
        if r.changed() {
            m.args.chords_context = ctx as u16;
        }
        if r.drag_stopped() || r.lost_focus() {
            m.persist("Chord lines", &[("chords_context", (ctx as i64).into())]);
        }
    });
    weak(ui, "99 shows the chords on every line.");
    let dir = m.args.chords_dir.clone().unwrap_or_else(music_display::chords::default_dir).display().to_string();
    if let Some(d) = text_row(ui, &mut m.ux.s.edits, "chords_dir", "Chord sheet folder", &dir, "", false) {
        // an empty field keeps the folder (the default one is shown as the text)
        if !d.is_empty() {
            m.args.chords_dir = Some(std::path::PathBuf::from(&d));
            m.persist("Chord folder", &[("chords_dir", d.into())]);
        }
    }
    weak(ui, "Your own sheets are read from here from the next track on.");
    if let Some(v) = check(ui, "Member parts: who sings which line (experimental)", m.args.member_parts) {
        m.set_member_parts(v);
    }
}

fn translation(ui: &mut Ui, m: &mut Model) {
    ui.heading("Translation");
    let modes: Vec<(TranslateMode, &str)> = [TranslateMode::Off, TranslateMode::Translated, TranslateMode::Both].into_iter().map(|t| (t, mode_name(t))).collect();
    if let Some(v) = combo(ui, "tmode", "Show", &m.translate_mode, &modes) {
        m.set_translate_mode_saved(v);
    }
    let mut langs: Vec<String> = LANGS.iter().map(|l| l.to_string()).collect();
    if !langs.contains(&m.args.translate_lang) {
        langs.push(m.args.translate_lang.clone());
    }
    let lopts: Vec<(String, &str)> = langs.iter().map(|n| (n.clone(), n.as_str())).collect();
    if let Some(v) = combo(ui, "tlang", "Translate into", &m.args.translate_lang, &lopts) {
        m.set_translate_lang(&v);
    }
    if let Some(v) = check(ui, "Keep translations on disk", !m.args.no_cache_translations) {
        m.set_cache_translations(v);
    }
    ui.separator();
    ui.label(RichText::new("Engine").strong());
    let engines = [(TranslateEngine::Standard, engine_name(TranslateEngine::Standard)), (TranslateEngine::Ai, engine_name(TranslateEngine::Ai))];
    if let Some(v) = combo(ui, "tengine", "Translate with", &m.args.translate_engine, &engines) {
        m.set_translate_engine(v);
    }
    // the AI fields are always here: they also serve the AI annotations
    ui.label(RichText::new("AI model (translation and AI annotations)").strong());
    let cur = ai_provider_name(&m.args);
    let provs: Vec<(String, &str)> = PROVIDERS.iter().map(|(k, n)| (k.to_string(), *n)).collect();
    if let Some(v) = combo(ui, "aiprov", "Provider", &cur, &provs) {
        m.set_ai_provider(&v);
    }
    let (model, endpoint, keyenv) = (m.args.ai_model.clone(), m.args.ai_endpoint.clone(), m.args.ai_key_env.clone());
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "ai_model", "Model", &model, "e.g. llama3.1", false) {
        m.set_ai_field(AiField::Model, &t);
    }
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "ai_endpoint", "Endpoint", &endpoint, "empty = the provider's default", false) {
        m.set_ai_field(AiField::Endpoint, &t);
    }
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "ai_keyenv", "Key variable", &keyenv, "name of an environment variable", false) {
        m.set_ai_field(AiField::KeyEnv, &t);
    }
    weak(ui, "The key itself is never typed here or stored: put it in that environment variable. Lyrics and song info go to the provider; a local endpoint (Ollama) keeps them on this computer.");
    if super::privacy::notice_needed(&m.args, super::privacy::ack_path().exists()) {
        weak(ui, &format!("{}. You are asked once before the first request.", super::privacy::notice(&m.args)));
    }
    ui.separator();
    ui.label(RichText::new("Never translate these languages").strong());
    let skip = m.skip_langs();
    ui.horizontal_wrapped(|ui| {
        for code in LANGS {
            let on = skip.contains(&music_display::translate::base_code(code));
            if ui.selectable_label(on, *code).clicked() {
                m.toggle_skip(code);
            }
        }
    });
    let own = m.args.translate_skip.clone();
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "skip", "List", &own, "comma-separated, e.g. sv,en", false) {
        m.args.translate_skip = t;
        m.reset_translation();
        m.persist("Never translate", &[("translate_skip", m.args.translate_skip.clone().into())]);
    }
    ui.separator();
    ui.label(RichText::new("LibreTranslate fallback").strong());
    if let Some(v) = check(ui, "Use it when the standard engine fails", m.args.translate_fallback) {
        m.set_fallback(v);
    }
    let url = m.args.translate_fallback_url.clone();
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "fb_url", "URL", &url, "", false) {
        m.set_fallback_url(&t);
    }
    let has_key = m.args.translate_fallback_key.as_deref().is_some_and(|k| !k.is_empty());
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "fb_key", "API key", "", if has_key { "set (hidden)" } else { "none" }, true) {
        m.set_fallback_key(&t);
    }
}

fn annotations(ui: &mut Ui, m: &mut Model) {
    ui.heading("Annotations");
    if let Some(v) = check(ui, "Annotations panel (experimental)", m.annotations_shown()) {
        m.set_annotations_shown(v);
    }
    let modes: Vec<(AnnotationMode, &str)> = [AnnotationMode::Genius, AnnotationMode::Ai, AnnotationMode::GeniusAi].into_iter().map(|a| (a, a.label())).collect();
    if let Some(v) = combo(ui, "annmode", "Notes from", &m.args.annotations_mode, &modes) {
        m.set_annotations_mode(v);
    }
    let scrolls = [(AnnotationScroll::Manual, scroll_name(AnnotationScroll::Manual)), (AnnotationScroll::Auto, scroll_name(AnnotationScroll::Auto))];
    if let Some(v) = combo(ui, "annscroll", "Long notes", &m.args.annotation_scroll, &scrolls) {
        m.set_annotation_scroll(v);
    }
    let mut max = m.args.annotation_max_chars as u32;
    ui.horizontal(|ui| {
        ui.label("Note length");
        let r = ui.add(egui::DragValue::new(&mut max).range(0..=4000).speed(10).suffix(" characters"));
        if r.changed() {
            m.args.annotation_max_chars = max as usize;
        }
        if r.drag_stopped() || r.lost_focus() {
            m.persist("Note length", &[("annotation_max_chars", (max as i64).into())]);
        }
        weak(ui, "0 = the whole note");
    });
    ui.separator();
    ui.label(RichText::new("Genius").strong());
    let status = if !m.args.genius_token.is_empty() {
        "token set (hidden)"
    } else if std::env::var("GENIUS_TOKEN").is_ok_and(|t| !t.trim().is_empty()) {
        "token from GENIUS_TOKEN"
    } else {
        "no token"
    };
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "genius", "Token", "", status, true) {
        m.set_genius_token(&t);
    }
    ui.horizontal(|ui| {
        weak(ui, "Free at genius.com/api-clients. An empty entry keeps the old token.");
        if !m.args.genius_token.is_empty() && ui.small_button("remove").clicked() {
            m.clear_genius_token();
        }
    });
}

fn spotify(ui: &mut Ui, m: &mut Model) {
    ui.heading("Spotify");
    let status = match (&m.spot.login_msg, m.spot.on(), m.ux.logged_in()) {
        (Some(msg), _, _) => msg.clone(),
        (None, false, true) => "off · logged in".to_string(),
        (None, false, false) => "off · not logged in".to_string(),
        (None, true, true) => m.spot.status.clone().unwrap_or_else(|| "logged in".into()),
        (None, true, false) => "not logged in".to_string(),
    };
    weak(ui, &status);
    if let Some(v) = check(ui, "Spotify features (library, likes, other devices)", m.spot.on()) {
        m.set_spotify_on(v);
    }
    let id = m.args.spotify_client_id.clone().unwrap_or_default();
    let mut refused = None;
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "client_id", "Client ID", &id, "from developer.spotify.com/dashboard", false) {
        refused = m.set_spotify_client_id(&t).err();
    }
    if let Some(e) = refused {
        m.say(e);
    }
    ui.horizontal(|ui| {
        let busy = m.spot.logging_in();
        if ui.button(if busy { "Cancel login" } else { "Log in (opens your browser)" }).clicked() {
            m.spotify_login();
        }
        if ui.button("Log out").on_hover_text("forget the saved login").clicked() {
            m.spotify_logout();
        }
    });
    if let Some(u) = m.spot.login_url.clone() {
        ui.add(egui::Label::new(RichText::new("If the browser did not open, copy this link:").small().weak()).wrap());
        let mut link = u;
        ui.add(egui::TextEdit::singleline(&mut link).desired_width(380.0));
    }
    ui.separator();
    let dev = m.args.spotify_device.clone();
    if let Some(t) = text_row(ui, &mut m.ux.s.edits, "device", "spotifyd device name", &dev, "none", false) {
        m.set_spotify_device(&t);
    }
    if let Some(v) = check(ui, "Use spotifyd as the player on this PC", m.args.use_spotifyd) {
        m.set_use_spotifyd(v);
    }
    if let Some(v) = check(ui, "Stop spotifyd when the window closes", !m.args.no_kill_spotifyd) {
        m.set_kill_spotifyd(v);
    }
    ui.separator();
    if let Some(v) = check(ui, "Library button in the transport row (o still works)", !m.args.no_spotify_button) {
        m.set_spotify_button(v);
    }
    if let Some(v) = check(ui, "Queue tab in the library", !m.args.no_spotify_queue_tab) {
        m.set_spotify_queue_tab(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn model(name: &str) -> (Model, std::path::PathBuf, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        music_display::util::enter_test_mode();
        let args = Args::parse_from(["music_display", "--genre-lookup", "false", "--no-cache-translations"]);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let path = std::env::temp_dir().join(format!("md-gui-settings-{}-{name}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (Model::new(args, Layout::default(), path.clone(), tx), path, rx)
    }

    fn saved(path: &Path) -> toml::Table {
        std::fs::read_to_string(path).unwrap_or_default().parse().unwrap_or_default()
    }

    #[tokio::test]
    async fn rows_save_the_terminal_menus_keys() {
        let (mut m, p, _rx) = model("keys");
        m.set_player_source("vlc");
        m.set_chords_ug(true);
        m.set_chords_match(40);
        m.save_chords_match();
        m.set_cache_translations(true);
        m.set_genre_lookup(true);
        m.set_translate_lang("de");
        m.toggle_skip("sv");
        m.set_fallback(true);
        m.set_fallback_url(" https://lt.example/translate ");
        m.set_ai_provider("gemini");
        m.set_ai_field(AiField::Model, "gemini-2.5-flash");
        m.set_ai_field(AiField::Endpoint, "");
        m.set_ai_field(AiField::KeyEnv, "MY_GEMINI_KEY");
        m.set_annotation_scroll(AnnotationScroll::Auto);
        m.set_annotations_mode(AnnotationMode::Genius);
        m.args.lyric_spacing = 2;
        m.args.lyric_pad = 4;
        m.args.big_scale = 3;
        m.args.interlude_secs = 12.0;
        m.args.volume_step = 7;
        m.save_lyric_numbers();
        m.set_spotify_queue_tab(false);
        m.set_spotify_button(false);
        m.set_kill_spotifyd(false);
        let t = saved(&p);
        assert_eq!(t["player"].as_str(), Some("vlc"));
        assert_eq!(t["chords_ug"].as_bool(), Some(true));
        assert_eq!(t["chords_match"].as_integer(), Some(40));
        assert_eq!(t["cache_translations"].as_bool(), Some(true));
        assert_eq!(t["genre_lookup"].as_bool(), Some(true));
        assert_eq!(t["translate_lang"].as_str(), Some("de"));
        assert_eq!(t["translate_skip"].as_str(), Some("sv"));
        assert_eq!(t["translate_fallback"].as_bool(), Some(true));
        assert_eq!(t["translate_fallback_url"].as_str(), Some("https://lt.example/translate"));
        let ai = t["ai"].as_table().unwrap();
        assert_eq!(ai["provider"].as_str(), Some("gemini"));
        assert_eq!(ai["model"].as_str(), Some("gemini-2.5-flash"));
        assert_eq!(ai["api_key_env"].as_str(), Some("MY_GEMINI_KEY"));
        assert_eq!(t["annotation_scroll"].as_str(), Some("auto"));
        assert_eq!(t["lyric_spacing"].as_integer(), Some(2));
        assert_eq!(t["lyric_pad"].as_integer(), Some(4));
        assert_eq!(t["big_scale"].as_integer(), Some(3));
        assert_eq!(t["interlude_threshold_secs"].as_float(), Some(12.0));
        assert_eq!(t["volume_step"].as_integer(), Some(7));
        assert_eq!(t["spotify_queue_tab"].as_bool(), Some(false));
        assert_eq!(t["spotify_button"].as_bool(), Some(false));
        assert_eq!(t["kill_spotifyd_on_exit"].as_bool(), Some(false));
        assert!(m.ux.saved_text().is_some());
        // every key written is one the config file knows
        for k in t.keys().filter(|k| *k != "ai") {
            assert!(music_display::config::KEYS.iter().any(|x| x.name == k), "{k} is not a config key");
        }
        let _ = std::fs::remove_file(p);
    }

    #[tokio::test]
    async fn the_genius_token_is_saved_but_never_in_a_message_and_empty_keeps_it() {
        let (mut m, p, _rx) = model("token");
        m.set_genius_token("s3cret-token");
        assert_eq!(saved(&p)["genius_token"].as_str(), Some("s3cret-token"));
        m.set_genius_token("   ");
        assert_eq!(m.args.genius_token, "s3cret-token", "an empty entry keeps the old token");
        assert!(m.note.as_ref().is_none_or(|(n, _)| !n.contains("s3cret")));
        assert!(m.ux.saved_text().is_some_and(|s| !s.contains("s3cret")));
        m.clear_genius_token();
        assert_eq!(saved(&p)["genius_token"].as_str(), Some(""));
        let _ = std::fs::remove_file(p);
    }

    #[tokio::test]
    async fn the_client_id_is_checked_and_changes_rebuild_spotify() {
        let (mut m, p, _rx) = model("spot");
        assert!(m.set_spotify_client_id("not an id!").is_err());
        assert!(m.args.spotify_client_id.is_none());
        assert!(m.set_spotify_client_id("abc123DEF").is_ok());
        assert!(!m.spot.on(), "setting the ID alone does not switch Spotify on");
        m.set_spotify_on(true);
        assert!(m.spot.on());
        m.set_spotify_on(false);
        assert!(!m.spot.on());
        let t = saved(&p);
        assert_eq!(t["spotify"].as_bool(), Some(false));
        assert_eq!(t["spotify_client_id"].as_str(), Some("abc123DEF"));
        let _ = std::fs::remove_file(p);
    }

    #[tokio::test]
    async fn the_third_column_and_the_cava_switch_save_both_keys() {
        let (mut m, p, _rx) = model("cava");
        m.set_third_column(true);
        assert!(!m.layout.cava_under_lyrics && m.layout.panels.shown(PanelId::Cava));
        let t = saved(&p);
        assert_eq!(t["thirdpanel"].as_bool(), Some(true));
        assert_eq!(t["cava"].as_bool(), Some(true));
        m.set_cava_shown(false);
        assert_eq!(saved(&p)["cava"].as_bool(), Some(false));
        let _ = std::fs::remove_file(p);
    }

    #[tokio::test]
    async fn turning_the_ai_engine_on_asks_first_for_a_remote_provider() {
        let (mut m, p, _rx) = model("ai");
        m.args.ai_provider = "claude".into();
        m.args.ai_model = "some-model".into();
        let had_ack = super::super::privacy::ack_path().exists();
        let _ = std::fs::remove_file(super::super::privacy::ack_path());
        m.set_translate_engine(TranslateEngine::Ai);
        assert!(m.ux.ai_ask, "the question opens");
        // nothing is requested while it is open
        m.translate_mode = TranslateMode::Both;
        m.apply(AppEvent::Lyrics { key: m.track_key, lyrics: music_display::lyrics::Lyrics::Plain(vec!["a".into()]) });
        m.ensure_translation();
        assert!(m.tr.inflight.is_none(), "no request before the answer");
        m.ai_decline();
        assert!(!m.ux.ai_ask && m.args.translate_engine == TranslateEngine::Standard);
        assert_eq!(saved(&p)["translate_engine"].as_str(), Some("standard"));
        if had_ack {
            let _ = std::fs::write(super::super::privacy::ack_path(), "shown\n");
        }
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn transparent_in_the_main_config_becomes_window_opacity_unless_gui_sets_it() {
        let mut g = GuiConf::default();
        main_flags_into_gui(&mut g, false, false);
        assert_eq!(g.opacity, 100);
        main_flags_into_gui(&mut g, true, true);
        assert_eq!(g.opacity, 100);
        main_flags_into_gui(&mut g, true, false);
        assert_eq!(g.opacity, TRANSPARENT_OPACITY);
        let p = std::env::temp_dir().join(format!("md-gui-gui-sets-{}.toml", std::process::id()));
        std::fs::write(&p, "transparent = true\n[gui]\nopacity = 70\n").unwrap();
        assert!(gui_sets(&p, "opacity") && !gui_sets(&p, "accent"));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn restart_keys_cover_what_the_terminal_menu_marks() {
        use crate::app::Setting;
        for s in Setting::ALL.iter().filter(|s| s.needs_restart()) {
            for k in s.config_keys() {
                if matches!(*k, "transparent" | "cover_theme") {
                    assert!(RESTART_KEYS.contains(k));
                }
            }
        }
    }

    #[test]
    fn the_preset_combo_knows_which_preset_the_layout_is() {
        assert_eq!(preset_of(&Layout::default()), Some("default"));
        assert_eq!(preset_of(&Layout::preset("cava").unwrap()), Some("cava"));
        let mut l = Layout::default();
        l.set_size(PanelId::Metadata, 11);
        assert_eq!(preset_of(&l), None);
    }
}
