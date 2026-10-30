//! Window-side state that is not part of the playing song: the key table, which windows are
//! open, the "saved" flash of the settings window, pending lyric / note browsing and the volume
//! to come back to after a mute. No egui drawing in here (the volume control at the bottom takes
//! a `Ui`), so the logic is unit-tested without a window.
use super::model::Model;
use super::settings::SettingsUi;
use crate::keymap::Keymap;
use std::time::{Duration, Instant};

pub struct Ux {
    /// The same key table as the terminal UI (`[keys]` of the config file included).
    pub keys: Keymap,
    /// The help window (`?`).
    pub help: bool,
    /// The help window lets the keys be changed; `key_capture` waits for the next key of that action.
    pub key_edit: bool,
    pub key_capture: Option<crate::app::Action>,
    pub key_note: Option<String>,
    pub s: SettingsUi,
    /// Volume to restore when un-muting.
    pub vol_last: f64,
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
            key_edit: false,
            key_capture: None,
            key_note: None,
            s: SettingsUi::default(),
            vol_last: 0.5,
            ann_rows: 0.0,
            ai_ask: false,
            saved: None,
            spotifyd: None,
            logged_in: None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
