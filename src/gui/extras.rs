//! The window's visual extras: cava bars, the minimal (teleprompter) view, layout presets, lyric
//! versions, the per-track capo store and the configurable big active line. The state and logic
//! hang off `Model` (no egui types, unit-tested); the drawing functions are at the bottom.
use super::model::Model;
use egui::{Align, Sense, Ui, Vec2};
use music_display::cli::Args;
use music_display::layout::{Layout, PanelId};
use music_display::lyrics;
use music_display::{AppEvent, cava};
use std::path::PathBuf;
use tokio::task::JoinHandle;

/// The lyrics versions lrclib has for one track, and the one on screen.
pub struct Versions {
    key: u64,
    list: Vec<(String, String)>,
    ver: Option<usize>,
}

pub struct Extras {
    /// The minimal view: lyrics only, a large centred active line (`--minimal`, `m`).
    pub minimal: bool,
    pub cava_bars: Vec<u16>,
    /// What to say instead of bars: starting, not installed, not on Windows…
    pub cava_status: Option<String>,
    cava_task: Option<JoinHandle<()>>,
    /// cava failed; it is not started again until the panel is toggled off and on.
    cava_failed: bool,
    versions: Option<Versions>,
}

impl Extras {
    pub fn new(args: &Args) -> Self {
        Extras {
            minimal: args.minimal,
            cava_bars: Vec::new(),
            cava_status: None,
            cava_task: None,
            cava_failed: false,
            versions: None,
        }
    }
}

// ── per-track capo store (the file the terminal front end uses) ──

pub fn capo_path(key: u64) -> PathBuf {
    music_display::util::cache_dir().join(format!("capo-{key:016x}.txt"))
}

/// Capo chosen for this track in an earlier session (a one-number file).
pub async fn load_capo(key: u64) -> Option<u8> {
    tokio::fs::read_to_string(capo_path(key)).await.ok()?.trim().parse().ok()
}

impl Model {
    // ── events ──

    /// The cava and lyric-version events; called first by `apply`.
    pub fn extras_event(&mut self, ev: &AppEvent) {
        match ev {
            AppEvent::Cava(bars) if self.cava_wanted() => {
                self.x.cava_bars = bars.clone();
                self.x.cava_status = None;
            }
            AppEvent::CavaNote(n) if self.cava_wanted() && self.x.cava_bars.is_empty() => self.x.cava_status = Some(n.clone()),
            AppEvent::CavaError(e) => {
                self.x.cava_status = Some(e.clone());
                self.x.cava_bars.clear();
                self.x.cava_task = None;
                self.x.cava_failed = true;
            }
            AppEvent::LyricVersions { key, list } if *key == self.track_key => {
                // which of them is on screen now
                let ver = self.lyrics.as_ref().and_then(|l| list.iter().position(|(k, t)| lyrics::to_lyrics(k, t) == *l));
                self.x.versions = Some(Versions { key: *key, list: list.clone(), ver });
                self.step_lyrics();
            }
            _ => {}
        }
    }

    // ── cava ──

    /// Shown, and not hidden by the minimal view.
    pub fn cava_wanted(&self) -> bool {
        self.layout.panels.shown(PanelId::Cava) && !self.x.minimal
    }

    /// Start or stop the cava process to match what is shown (called every frame; cheap).
    pub fn sync_cava(&mut self) {
        if self.cava_wanted() {
            if self.x.cava_task.is_none() && !self.x.cava_failed {
                self.x.cava_status = Some("starting cava…".into());
                let (tx, bars) = (self.tx.clone(), self.args.cava_bars.max(4));
                self.x.cava_task = Some(tokio::spawn(cava::run(bars, tx)));
            }
        } else {
            if let Some(t) = self.x.cava_task.take() {
                t.abort(); // kill_on_drop stops cava: no CPU when hidden
            }
            self.x.cava_bars.clear();
            self.x.cava_failed = false;
        }
    }

    /// `v`: show or hide the bars. Returns the new state.
    pub fn toggle_cava(&mut self) -> bool {
        let on = !self.layout.panels.shown(PanelId::Cava);
        self.layout.panels.set(PanelId::Cava, on);
        self.x.cava_failed = false;
        self.sync_cava();
        on
    }

    // ── minimal view and presets ──

    pub fn set_minimal(&mut self, on: bool) {
        self.x.minimal = on;
        self.sync_cava();
    }

    /// Replace the layout by a named preset (`--layout-preset`). False for an unknown name.
    pub fn apply_preset(&mut self, name: &str) -> bool {
        let Some(l) = Layout::preset(name) else { return false };
        self.layout = l;
        self.show_diagrams = self.layout.panels.shown(PanelId::Chords);
        // the preset named minimal also turns the teleprompter view on, the others turn it off
        self.x.minimal = name.trim().eq_ignore_ascii_case("minimal");
        self.ensure_annotations();
        self.sync_cava();
        self.save_layout();
        true
    }

    // ── lyric versions ──

    /// `l`: show the next lyrics version lrclib has for this song. The list is looked up the
    /// first time, then the key just steps through it.
    pub fn next_lyrics(&mut self) {
        let Some(s) = self.snap.clone().filter(|s| !s.title.is_empty()) else { return };
        if self.x.versions.as_ref().is_some_and(|v| v.key == self.track_key) {
            return self.step_lyrics();
        }
        self.say("looking for other lyrics…");
        let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
        let policy = lyrics::Policy { retries: self.args.lyrics_retries, ..Default::default() };
        tokio::spawn(async move {
            let list = lyrics::versions(&c, &s.artist, &s.title, &s.album, s.length, policy).await;
            let _ = tx.send(AppEvent::LyricVersions { key, list });
        });
    }

    fn step_lyrics(&mut self) {
        let Some(s) = self.snap.clone() else { return };
        let Some(v) = self.x.versions.as_mut().filter(|v| v.key == self.track_key) else { return };
        let n = v.list.len();
        if n < 2 {
            let msg = if n == 0 { "lrclib has no lyrics for this song" } else { "lrclib has only these lyrics" };
            return self.say(msg);
        }
        let next = v.ver.map_or(0, |i| (i + 1) % n);
        let (kind, text) = v.list[next].clone();
        v.ver = Some(next);
        let key = self.track_key;
        // the next time this song plays it opens with the version picked here
        lyrics::remember(&s.artist, &s.title, &s.album, s.length, &kind, &text);
        // a translation belongs to the lines it was made from
        self.translations.retain(|(k, _), _| *k != key);
        self.translate_inflight = false;
        self.apply(AppEvent::Lyrics { key, lyrics: lyrics::to_lyrics(&kind, &text) });
        self.say(format!("lyrics version {} of {n}", next + 1));
    }

    // ── capo ──

    /// `[` / `]`: capo ±1 (0..=12) as a per-track override, remembered across restarts.
    /// Landing on the sheet's own capo clears the override.
    pub fn change_capo(&mut self, d: i32) {
        let Some(sheet_capo) = self.sheet.as_ref().map(|s| s.capo) else { return };
        let n = (self.effective_capo() as i32 + d).clamp(0, 12) as u8;
        self.set_capo_override((n != sheet_capo).then_some(n));
    }

    pub fn set_capo_override(&mut self, o: Option<u8>) {
        self.capo_override = o;
        // (a few bytes, written in order: separate tasks could finish out of order and leave
        // the file of an earlier press behind)
        let path = capo_path(self.track_key);
        let _ = match o {
            Some(n) => std::fs::write(path, n.to_string()),
            None => std::fs::remove_file(path),
        };
    }

    // ── lines ──

    /// Line `i` as the lyrics panel shows it: the translation when it replaces the original.
    pub fn shown_line(&self, i: usize) -> Option<String> {
        let lines = self.lines()?;
        let line = lines.get(i)?;
        let replaced = self.translate_mode == music_display::cli::TranslateMode::Translated;
        match self.translation().filter(|t| t.len() == lines.len()) {
            Some(t) if replaced && !t[i].trim().is_empty() => Some(t[i].clone()),
            _ => Some(line.clone()),
        }
    }
}

// ── drawing ─────────────────────────────────────────────────────────────────

/// The cava panel: one bar per value, cava's own 0..=1000 range; a note instead while there
/// are none (starting, not installed, not available on Windows).
pub fn cava_panel(ui: &mut Ui, m: &Model) {
    ui.heading("Cava");
    if m.x.cava_bars.is_empty() {
        let note = m.x.cava_status.as_deref().unwrap_or("waiting for audio…");
        ui.label(egui::RichText::new(note).weak());
        return;
    }
    let h = ui.available_height().clamp(40.0, 240.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    let color = ui.visuals().hyperlink_color;
    let n = m.x.cava_bars.len() as f32;
    let slot = rect.width() / n;
    for (i, v) in m.x.cava_bars.iter().enumerate() {
        let bar = (*v as f32 / cava::MAX as f32).clamp(0.0, 1.0) * rect.height();
        let x = rect.left() + slot * i as f32;
        let r = egui::Rect::from_min_max(egui::pos2(x + slot * 0.1, rect.bottom() - bar), egui::pos2(x + slot * 0.9, rect.bottom()));
        ui.painter().rect_filled(r, 1.0, color);
    }
}

/// The minimal view: the active line large and centred, the previous and next one dim around
/// it. Plain (unsynced) lyrics have no active line and fall back to the normal list.
pub fn teleprompter(ui: &mut Ui, m: &mut Model) {
    let Some(i) = m.active_line() else {
        super::view::lyrics(ui, m);
        return;
    };
    let size = m.gui.active_size as f32 * 1.6;
    let weak = ui.visuals().weak_text_color();
    let strong = ui.visuals().strong_text_color();
    let accent = ui.visuals().hyperlink_color;
    let (prev, now, next) = (i.checked_sub(1).and_then(|p| m.shown_line(p)), m.shown_line(i).unwrap_or_default(), m.shown_line(i + 1));
    let frac = m.line_fraction() as f32;
    let karaoke = m.args.karaoke;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - size * 1.5).max(0.0));
        if let Some(p) = prev {
            ui.label(egui::RichText::new(p).size(size * 0.5).color(weak));
        }
        ui.add_space(size * 0.3);
        let chars: Vec<char> = now.chars().collect();
        let lit = if karaoke { ((chars.len() as f32 * frac).round() as usize).min(chars.len()) } else { 0 };
        let (a, b): (String, String) = (chars[..lit].iter().collect(), chars[lit..].iter().collect());
        let font = super::style::active_font(&m.gui, size);
        let mut job = egui::text::LayoutJob { halign: Align::Center, ..Default::default() };
        job.wrap.max_width = ui.available_width();
        job.append(&a, 0.0, egui::TextFormat { font_id: font.clone(), color: accent, ..Default::default() });
        job.append(&b, 0.0, egui::TextFormat { font_id: font, color: strong, ..Default::default() });
        ui.add(egui::Label::new(job));
        ui.add_space(size * 0.3);
        if let Some(n) = next {
            ui.label(egui::RichText::new(n).size(size * 0.5).color(weak));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    use clap::Parser;
    use music_display::lyrics::Lyrics;

    fn model() -> (Model, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        music_display::util::enter_test_mode();
        let args = Args::parse_from(["music_display", "--genre-lookup", "false", "--no-cache-translations"]);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Model::new(args, Layout::default(), std::env::temp_dir().join("md-gui-extras.toml"), tx), rx)
    }

    #[tokio::test]
    async fn cava_events_only_count_while_the_panel_is_shown() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Cava(vec![1, 2]));
        assert!(m.x.cava_bars.is_empty());
        m.layout.panels.set(PanelId::Cava, true);
        m.apply(AppEvent::Cava(vec![1, 2]));
        assert_eq!(m.x.cava_bars, vec![1, 2]);
        // the minimal view hides it, an error leaves the note and stops restarts
        m.set_minimal(true);
        assert!(!m.cava_wanted());
        m.set_minimal(false);
        m.apply(AppEvent::CavaError("cava not found".into()));
        assert!(m.x.cava_bars.is_empty() && m.x.cava_failed);
        assert_eq!(m.x.cava_status.as_deref(), Some("cava not found"));
        m.sync_cava();
        assert!(m.x.cava_task.is_none(), "no restart loop after a failure");
        assert!(!m.toggle_cava(), "toggling hides it and forgets the failure");
        assert!(!m.x.cava_failed && m.x.cava_task.is_none());
    }

    #[tokio::test]
    async fn presets_replace_the_layout_and_minimal_follows() {
        let (mut m, _rx) = model();
        assert!(m.apply_preset("minimal"));
        assert!(m.x.minimal && !m.layout.panels.shown(PanelId::Metadata));
        assert!(m.apply_preset("cava"));
        assert!(!m.x.minimal && m.layout.panels.shown(PanelId::Cava) && !m.layout.cava_under_lyrics);
        assert!(!m.apply_preset("nope"));
        if let Some(t) = m.x.cava_task.take() {
            t.abort();
        }
    }

    #[tokio::test]
    async fn lyric_versions_step_and_wrap() {
        let (mut m, _rx) = model();
        m.apply(AppEvent::Player(Some(music_display::player::parse("Playing\x1f5000000\x1fSong\x1fBand\x1fAlbum\x1f200000000\x1f\x1f2\x1f1\x1f1999-01-02T00:00:00Z\x1f0.5\x1f\x1fid\x1fvlc\x1f[]").unwrap())));
        let key = m.track_key;
        m.apply(AppEvent::Lyrics { key, lyrics: lyrics::to_lyrics("plain", "a\nb") });
        let list = vec![("plain".to_string(), "a\nb".to_string()), ("plain".to_string(), "c\nd".to_string())];
        // a list for another track is ignored
        m.apply(AppEvent::LyricVersions { key: key ^ 1, list: list.clone() });
        assert!(m.x.versions.is_none());
        m.apply(AppEvent::LyricVersions { key, list });
        // the first was on screen, so the key moves to the second
        assert_eq!(m.lines().unwrap(), vec!["c", "d"]);
        m.next_lyrics();
        assert_eq!(m.lines().unwrap(), vec!["a", "b"]);
        assert!(matches!(m.lyrics, Some(Lyrics::Plain(_))));
    }

    #[tokio::test]
    async fn capo_override_is_stored_per_track_and_cleared_on_the_sheets_own() {
        let (mut m, _rx) = model();
        m.track_key = 0x1234_5678;
        let _ = std::fs::remove_file(capo_path(m.track_key));
        m.change_capo(1); // no sheet yet: nothing
        assert_eq!(m.capo_override, None);
        m.sheet = Some(music_display::chords::Sheet { capo: 2, ..Default::default() });
        m.change_capo(1);
        assert_eq!((m.capo_override, m.effective_capo()), (Some(3), 3));
        m.change_capo(-1);
        assert_eq!(m.capo_override, None);
        m.set_capo_override(Some(5));
        for _ in 0..50 {
            if load_capo(m.track_key).await == Some(5) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(load_capo(m.track_key).await, Some(5));
        m.set_capo_override(None);
        let _ = std::fs::remove_file(capo_path(m.track_key));
    }

    #[test]
    fn capo_files_are_named_by_track() {
        music_display::util::enter_test_mode();
        assert!(capo_path(0xab).ends_with("capo-00000000000000ab.txt"));
    }
}
