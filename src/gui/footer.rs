//! The status line at the bottom of the window: the chord sheet's state, the Spotify device being shown, and the current note (wrapped).
use super::model::{Chords, Model};
use egui::{RichText, Ui};
use music_display::chords::Aligned;

/// What to say about the chords: why there are none, or where they came from. `None` = chords
/// are off. `transpose` is 0..12 as stored; it is shown signed (`-1`, not `+11`).
#[allow(clippy::too_many_arguments)]
pub fn chords_status(
    on: bool,
    state: &Chords,
    aligned: Option<&Aligned>,
    min_pct: u32,
    source: &str,
    tuning: Option<&str>,
    capo: u8,
    transpose: i32,
) -> Option<String> {
    if !on {
        return None;
    }
    Some(match (state, aligned) {
        (Chords::Idle, _) => "chords: waiting for a track".to_string(),
        (Chords::Fetching, _) => "chords: looking…".to_string(),
        (Chords::Missing, _) => "chords: no sheet found for this track".to_string(),
        (Chords::Failed(e), _) => format!("chords: {e}"),
        (Chords::Ready, None) => "chords: found, but there are no synced lyrics to place them on".into(),
        (Chords::Ready, Some(a)) if !a.usable_at(min_pct) => format!("chords: weak match ({}%) — hidden", a.percent()),
        (Chords::Ready, Some(a)) => {
            let mut t = format!("chords: {source} {}%", a.percent());
            if let Some(tu) = tuning {
                t.push_str(&format!(" · {tu}"));
            }
            if capo > 0 {
                t.push_str(&format!(" · capo {capo}"));
            }
            if transpose != 0 {
                t.push_str(&format!(" · {:+}", signed_transpose(transpose)));
            }
            t
        }
    })
}

/// 0..12 semitones as -5..=6.
pub fn signed_transpose(t: i32) -> i32 {
    let t = t.rem_euclid(12);
    if t > 6 { t - 12 } else { t }
}

pub fn footer(ui: &mut Ui, m: &mut Model) {
    let status = chords_status(
        m.chords_on,
        &m.chords_state,
        m.aligned.as_ref(),
        m.args.chords_match,
        m.sheet.as_ref().map_or("", |s| s.source.as_str()),
        m.sheet.as_ref().and_then(|s| s.tuning.as_deref()),
        m.effective_capo(),
        m.transpose,
    );
    let accent = ui.visuals().hyperlink_color;
    let note = m.note.as_ref().map(|(n, _)| n.clone());
    ui.horizontal_wrapped(|ui| {
        let has_status = status.is_some();
        if let Some(s) = status {
            ui.label(RichText::new(s).small().weak());
        }
        if m.remote_active() {
            if let Some(d) = m.spot.device() {
                if has_status {
                    ui.separator();
                }
                ui.label(RichText::new(format!("on {d}")).small().color(accent));
            }
        }
    });
    if let Some(n) = note {
        ui.add(egui::Label::new(RichText::new(n).color(accent)).wrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aligned(matched: usize, total: usize) -> Aligned {
        Aligned { per_line: Vec::new(), matched, total, flats: false, intro: Vec::new(), gaps: Vec::new() }
    }

    #[test]
    fn the_chord_status_names_each_situation() {
        let st = |s: &Chords, a: Option<&Aligned>, tr| chords_status(true, s, a, 40, "song.cho", Some("Drop D"), 2, tr);
        assert_eq!(chords_status(false, &Chords::Idle, None, 40, "", None, 0, 0), None);
        assert_eq!(st(&Chords::Fetching, None, 0).unwrap(), "chords: looking…");
        assert_eq!(st(&Chords::Missing, None, 0).unwrap(), "chords: no sheet found for this track");
        assert!(st(&Chords::Ready, None, 0).unwrap().contains("no synced lyrics"));
        let weak = aligned(1, 10);
        assert_eq!(st(&Chords::Ready, Some(&weak), 0).unwrap(), "chords: weak match (10%) — hidden");
        let good = aligned(9, 10);
        assert_eq!(st(&Chords::Ready, Some(&good), 0).unwrap(), "chords: song.cho 90% · Drop D · capo 2");
        assert!(st(&Chords::Ready, Some(&good), 11).unwrap().ends_with("· -1"));
        assert!(st(&Chords::Ready, Some(&good), 2).unwrap().ends_with("· +2"));
    }

    #[test]
    fn transpose_is_shown_as_a_small_signed_number() {
        assert_eq!([0, 1, 6, 7, 11].map(signed_transpose), [0, 1, 6, -5, -1]);
    }
}
