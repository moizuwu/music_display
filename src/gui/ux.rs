//! Window-side state that is not part of the playing song: the key table, which windows are
//! open, the "saved" flash of the settings window, pending lyric / note browsing and the volume
//! to come back to after a mute. No egui drawing in here (the volume control at the bottom takes
//! a `Ui`), so the logic is unit-tested without a window.
use super::model::Model;
use super::settings::SettingsUi;
use crate::keymap::Keymap;
use egui::Ui;
use std::time::{Duration, Instant};

/// How long a keyboard / wheel browse of the lyrics keeps the view off the playing line.
pub const BROWSE_HOLD: Duration = Duration::from_secs(6);

pub struct Ux {
    /// The same key table as the terminal UI (`[keys]` of the config file included).
    pub keys: Keymap,
    /// The help window (`?`).
    pub help: bool,
    pub s: SettingsUi,
    /// Volume to restore when un-muting.
    pub vol_last: f64,
    /// Rows the lyrics should be moved by (from ↑ ↓ PgUp PgDn); taken by the lyrics panel.
    browse_rows: f32,
    browse_at: Option<Instant>,
    /// Rows a long annotation should scroll (`e` / `E`).
    ann_rows: f32,
    /// The one-time "where do the lyrics go" question is waiting for an answer.
    pub ai_ask: bool,
    /// What was saved last, for the brief "saved" feedback.
    pub saved: Option<(String, Instant)>,
    /// spotifyd started by this window (`use_spotifyd`).
    pub spotifyd: Option<std::process::Child>,
    logged_in: Option<(bool, Instant)>,
}

impl Ux {
    pub fn new(keys: Keymap) -> Self {
        Ux {
            keys,
            help: false,
            s: SettingsUi::default(),
            vol_last: 0.5,
            browse_rows: 0.0,
            browse_at: None,
            ann_rows: 0.0,
            ai_ask: false,
            saved: None,
            spotifyd: None,
            logged_in: None,
        }
    }

    // ── lyric browsing (the keys only record it; the lyrics panel applies it) ──

    pub fn browse_by(&mut self, rows: f32) {
        self.browse_rows += rows;
        self.browse_at = Some(Instant::now());
    }

    /// `End`: back to the playing line.
    pub fn browse_end(&mut self) {
        self.browse_rows = 0.0;
        self.browse_at = None;
    }

    /// The view is away from the playing line, so the panel should not scroll back to it yet.
    #[allow(dead_code)] // read by the lyrics panel
    pub fn browsing(&self) -> bool {
        self.browse_at.is_some_and(|t| t.elapsed() < BROWSE_HOLD)
    }

    /// Rows to move since the last call.
    #[allow(dead_code)] // read by the lyrics panel
    pub fn take_browse(&mut self) -> f32 {
        std::mem::take(&mut self.browse_rows)
    }

    pub fn scroll_notes(&mut self, rows: f32) {
        self.ann_rows += rows;
    }

    pub fn take_ann_rows(&mut self) -> f32 {
        std::mem::take(&mut self.ann_rows)
    }

    /// Token file present? Cached for two seconds (it is read from disk).
    pub fn logged_in(&mut self) -> bool {
        match self.logged_in {
            Some((v, at)) if at.elapsed() < Duration::from_secs(2) => v,
            _ => {
                let v = music_display::spotify::logged_in();
                self.logged_in = Some((v, Instant::now()));
                v
            }
        }
    }

    pub fn forget_login_cache(&mut self) {
        self.logged_in = None;
    }

    /// Text for the "saved" flash, while it is fresh.
    pub fn saved_text(&self) -> Option<&str> {
        self.saved.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(3)).map(|(s, _)| s.as_str())
    }
}

/// A transposition (0..12 as kept by the keys) as the TUI-style signed number: 11 is -1.
pub fn signed_transpose(n: i32) -> i32 {
    let n = n.rem_euclid(12);
    if n > 6 { n - 12 } else { n }
}

/// How far the song is through a note's line (0 to 1), with a short hold at both ends so the
/// start and the end of a long note can be read (the terminal UI's rule for `annotation_scroll = "auto"`).
pub fn note_progress(t: f64, start: f64, next: Option<f64>) -> f64 {
    let end = next.unwrap_or(start + 8.0);
    let p = (t - start) / (end - start).max(0.1);
    ((p - 0.15) / 0.7).clamp(0.0, 1.0)
}

impl Model {
    /// Progress through the active synced line for the annotation panel's auto scroll.
    pub fn note_line_progress(&self, i: usize) -> f64 {
        let Some(music_display::lyrics::Lyrics::Synced(l)) = &self.lyrics else { return 0.0 };
        let Some((start, _)) = l.get(i) else { return 0.0 };
        note_progress(self.position() + self.offset, *start, l.get(i + 1).map(|x| x.0))
    }

    /// Called every frame: things that finish in the background.
    pub fn ux_tick(&mut self) {
        // a browser login finished: a fresh client reads the new token
        if self.spot.take_relogin() {
            self.ux.forget_login_cache();
            self.spot.rebuild(&self.args);
        }
    }

    pub fn volume_pct(&self) -> Option<u32> {
        self.snap.as_ref().and_then(|s| s.volume).map(|v| (v * 100.0).round() as u32)
    }

    /// `+` / `-` / the wheel: `dir` steps of `volume_step` percent.
    pub fn volume_step(&mut self, dir: i32) {
        let Some(pct) = self.volume_pct() else { return self.say("this player has no volume control") };
        let new = (pct as i32 + dir * self.args.volume_step as i32).clamp(0, 100);
        self.volume_to(new as f64 / 100.0);
    }

    pub fn volume_to(&mut self, v: f64) {
        if v > 0.004 {
            self.ux.vol_last = v;
        }
        self.set_volume(v);
    }

    pub fn muted(&self) -> bool {
        self.snap.as_ref().and_then(|s| s.volume).is_some_and(|v| v <= 0.004)
    }

    pub fn toggle_mute(&mut self) {
        let Some(v) = self.snap.as_ref().and_then(|s| s.volume) else { return self.say("this player has no volume control") };
        if v > 0.004 {
            self.ux.vol_last = v;
            self.set_volume(0.0);
        } else {
            let back = self.ux.vol_last.max(0.05);
            self.set_volume(back);
        }
    }
}

/// The volume control of the transport row: a mute button, the slider (the wheel over it changes
/// the volume by `volume_step`) and a percentage that mutes when clicked.
pub fn volume(ui: &mut Ui, m: &mut Model, v: f64) {
    let muted = m.muted();
    if ui.selectable_label(muted, if muted { "muted" } else { "mute" }).on_hover_text("mute / unmute (m)").clicked() {
        m.toggle_mute();
    }
    let mut pct = (v * 100.0) as f32;
    let r = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").show_value(false));
    if r.changed() {
        m.volume_to(pct as f64 / 100.0);
    }
    if r.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            m.volume_step(if dy > 0.0 { 1 } else { -1 });
        }
    }
    if ui.add(egui::Label::new(format!("{:.0}%", pct)).sense(egui::Sense::click())).on_hover_text("click to mute / unmute").clicked() {
        m.toggle_mute();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_is_shown_signed() {
        assert_eq!(signed_transpose(0), 0);
        assert_eq!(signed_transpose(1), 1);
        assert_eq!(signed_transpose(6), 6);
        assert_eq!(signed_transpose(11), -1);
        assert_eq!(signed_transpose(7), -5);
        assert_eq!(signed_transpose(-1), -1);
    }

    #[test]
    fn browsing_accumulates_and_ends() {
        let mut u = Ux::new(Keymap::default());
        assert!(!u.browsing());
        u.browse_by(1.0);
        u.browse_by(8.0);
        assert!(u.browsing());
        assert_eq!(u.take_browse(), 9.0);
        assert_eq!(u.take_browse(), 0.0);
        u.browse_end();
        assert!(!u.browsing());
    }

    #[test]
    fn note_progress_holds_at_both_ends() {
        assert_eq!(note_progress(0.5, 0.0, Some(10.0)), 0.0);
        assert_eq!(note_progress(9.5, 0.0, Some(10.0)), 1.0);
        let mid = note_progress(5.0, 0.0, Some(10.0));
        assert!((mid - 0.5).abs() < 0.01, "{mid}");
        // the last line has no next stamp: 8 s are assumed
        assert!(note_progress(4.0, 0.0, None) > 0.2);
    }
}
