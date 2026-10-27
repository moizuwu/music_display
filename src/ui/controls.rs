use super::*;

/// Blank columns between control buttons (full / compact labels).
pub(super) const CTL_GAP: u16 = 3;
pub(super) const CTL_GAP_COMPACT: u16 = 2;

/// One-row progress bar: ━ filled, ─ remaining, ● knob.
/// `0:42 ━━━●─── 3:10`: both timestamps sit inside the row, the bar between them.
/// Returns the bar part (the click target for seeking).
pub(super) fn timed_bar(f: &mut Frame, th: &Theme, row: Rect, pos: f64, length: f64) -> Rect {
    let (l, r) = (fmt_time(pos), fmt_time(length));
    let side = (l.chars().count().max(r.chars().count()) + 1) as u16;
    if row.width <= side * 2 + 4 {
        let bar = row;
        progress_bar(f, th, bar, if length > 0.0 { (pos / length).clamp(0.0, 1.0) } else { 0.0 });
        return bar;
    }
    let bar = Rect { x: row.x + side, width: row.width - side * 2, ..row };
    f.render_widget(
        Paragraph::new(Span::styled(l, Style::new().fg(th.fade(0.4)))),
        Rect { width: side, ..row },
    );
    f.render_widget(
        Paragraph::new(Span::styled(r, Style::new().fg(th.fade(0.4)))).alignment(Alignment::Right),
        Rect { x: row.right() - side, width: side, ..row },
    );
    progress_bar(f, th, bar, if length > 0.0 { (pos / length).clamp(0.0, 1.0) } else { 0.0 });
    bar
}

pub(super) fn progress_bar(f: &mut Frame, th: &Theme, r: Rect, frac: f64) {
    let w = r.width as usize;
    if w == 0 {
        return;
    }
    let filled = ((frac * w as f64).round() as usize).min(w);
    let knob = filled.saturating_sub(1).min(w - 1);
    let buf = f.buffer_mut();
    for i in 0..w {
        let (sym, fg) = if i == knob && filled > 0 {
            ("●", th.primary())
        } else if i < filled {
            ("━", th.primary())
        } else {
            ("─", th.fade(0.7))
        };
        if let Some(c) = buf.cell_mut((r.x + i as u16, r.y)) {
            c.set_symbol(sym).set_fg(fg);
        }
    }
}

/// How much of the volume control is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolKind {
    /// `[ - ] [ 65% ] [ + ]`
    Full,
    /// `[-] 65% [+]`
    Compact,
    /// `[-][+]`
    Tiny,
}

impl VolKind {
    /// (width of `[-]`, width of the percentage (0 = none), gap between the three parts).
    pub fn cells(self) -> (u16, u16, u16) {
        match self {
            VolKind::Full => (5, 8, 1),
            VolKind::Compact => (3, 4, 1),
            VolKind::Tiny => (3, 0, 0),
        }
    }

    pub fn width(self) -> u16 {
        let (m, p, g) = self.cells();
        if p == 0 { m * 2 } else { m * 2 + p + g * 2 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtlButton {
    /// 0 shuffle, 1 repeat, 2 previous, 3 play / pause, 4 next.
    pub id: u8,
    pub label: String,
    /// Column offset from the left edge of the row.
    pub x: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ControlPlan {
    pub buttons: Vec<CtlButton>,
    /// Volume control: what is drawn and its column offset in its row.
    pub vol: Option<(VolKind, u16)>,
    /// The volume sits on its own row below the buttons.
    pub vol_row: bool,
}

/// The buttons for one label set: shuffle / repeat only for what the player reports.
pub(super) fn ctl_labels(compact: bool, modes: bool, shuffle: Option<bool>, rep: Option<bool>, playing: bool) -> Vec<(u8, &'static str)> {
    let mut v = Vec::new();
    if modes {
        if shuffle.is_some() {
            v.push((0, if compact { "[sh]" } else { "[ shuf ]" }));
        }
        if let Some(one) = rep {
            v.push((1, match (compact, one) {
                (false, false) => "[ rep ]",
                (false, true) => "[ rep 1 ]",
                (true, false) => "[rp]",
                (true, true) => "[r1]",
            }));
        }
    }
    v.push((2, if compact { "[<<]" } else { "[ prev ]" }));
    v.push((3, match (compact, playing) {
        (false, true) => "[ pause ]",
        (false, false) => "[ play ]",
        (true, true) => "[||]",
        (true, false) => "[|>]",
    }));
    v.push((4, if compact { "[>>]" } else { "[ next ]" }));
    v
}

pub(super) fn labels_width(labels: &[(u8, &str)], gap: u16) -> u16 {
    labels.iter().map(|l| l.1.chars().count() as u16).sum::<u16>() + gap * (labels.len() as u16).saturating_sub(1)
}

/// Lay the control row out for `width` columns. Whole buttons only: when something doesn't fit
/// it is dropped, never cut. Order of giving up: full labels with the volume beside them, volume
/// on its own row (`row_ok`: there is a spare row), compact labels, shuffle / repeat, the
/// volume (percentage, then the whole control), and last prev / next; play stays longest.
pub fn plan_controls(width: u16, shuffle: Option<bool>, rep: Option<bool>, playing: bool, volume: bool, row_ok: bool) -> ControlPlan {
    // (compact labels, with shuffle / repeat, volume kind, volume on its own row)
    let mut cands: Vec<(bool, bool, Option<VolKind>, bool)> = Vec::new();
    let vols = |k: VolKind| if volume { Some(k) } else { None };
    cands.push((false, true, vols(VolKind::Full), false));
    if volume && row_ok {
        cands.push((false, true, Some(VolKind::Full), true));
    }
    cands.push((true, true, vols(VolKind::Compact), false));
    if volume && row_ok {
        cands.push((true, true, Some(VolKind::Compact), true));
    }
    cands.push((true, false, vols(VolKind::Compact), false));
    if volume && row_ok {
        cands.push((true, false, Some(VolKind::Compact), true));
    }
    if volume {
        cands.push((true, false, Some(VolKind::Tiny), false));
    }
    cands.push((true, false, None, false));
    for (compact, modes, vol, on_row) in cands {
        let gap = if compact { CTL_GAP_COMPACT } else { CTL_GAP };
        let labels = ctl_labels(compact, modes, shuffle, rep, playing);
        let t = labels_width(&labels, gap);
        let vw = vol.map_or(0, VolKind::width);
        let total = if vol.is_some() && !on_row { t + gap + vw } else { t };
        if total > width || (on_row && vw > width) {
            continue;
        }
        let mut x = (width - total) / 2;
        let mut buttons = Vec::new();
        for (id, label) in &labels {
            buttons.push(CtlButton { id: *id, label: label.to_string(), x });
            x += label.chars().count() as u16 + gap;
        }
        let vol = vol.map(|k| (k, if on_row { (width - vw) / 2 } else { x }));
        return ControlPlan { buttons, vol, vol_row: on_row };
    }
    // Too narrow even for the compact row: play first, then next, then prev, as many as fit
    let labels = ctl_labels(true, false, None, None, playing);
    for keep in [&[3u8][..], &[3, 4], &[2, 3, 4]].iter().rev() {
        let set: Vec<_> = labels.iter().filter(|l| keep.contains(&l.0)).copied().collect();
        let t = labels_width(&set, CTL_GAP_COMPACT);
        if t <= width {
            let mut x = (width - t) / 2;
            let mut buttons = Vec::new();
            for (id, label) in &set {
                buttons.push(CtlButton { id: *id, label: label.to_string(), x });
                x += label.chars().count() as u16 + CTL_GAP_COMPACT;
            }
            return ControlPlan { buttons, vol: None, vol_row: false };
        }
    }
    ControlPlan::default()
}
