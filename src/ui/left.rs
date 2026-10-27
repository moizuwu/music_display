use super::*;

// ── left column ─────────────────────────────────────────────────────────
/// The metadata block: Track / Artist / Album, then Track No and Release on the left and
/// Tuning, Capo, BPM, Key one per row in a right-hand column (see [`meta_grid`]). Those
/// four are hidden when no source provides them.
pub(super) fn meta_lines(app: &App, snap: &crate::player::Snapshot, avail: usize) -> Vec<Line<'static>> {
    let th = &app.theme;
    let label = |s: &str| {
        Span::styled(
            format!("{s:<13}"),
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
        )
    };
    let val = |s: String| Span::styled(s, Style::new().fg(th.text()));
    let dash = |s: &str| {
        if s.is_empty() {
            "—".to_string()
        } else {
            s.to_string()
        }
    };
    let track_no = format!(
        "{:02}-{}",
        snap.disc_no.unwrap_or(1),
        snap.track_no
            .map(|t| format!("{t:02}"))
            .unwrap_or_else(|| "--".into())
    );
    let date = crate::year::full_date(&snap.content_created).unwrap_or_else(|| match &app.year {
        YearState::Loading => "...".to_string(),
        YearState::Known(y) => y.clone(),
        YearState::NotFound => "N/A".to_string(),
    });

    let tag = |name: &str, v: String| {
        vec![
            Span::styled(
                format!("{name} "),
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            ),
            val(v),
        ]
    };

    let mut out = vec![
        Line::from(match app.liked {
            Some(on) => vec![
                label("Track"),
                val(dash(&snap.title)),
                Span::styled(if on { " ♥" } else { " ♡" }, Style::new().fg(th.primary())),
            ],
            None => vec![label("Track"), val(dash(&snap.title))],
        }),
        Line::from(vec![label("Artist"), val(dash(&snap.artist))]),
        Line::from(vec![label("Album"), val(dash(&snap.album))]),
        Line::from(vec![label("Track No"), val(track_no)]),
    ];
    let capo = app.effective_capo();
    let capo_tag = if app.chords_on && capo > 0 {
        let mut v = format!("{capo}");
        if let Some((_, shapes)) = app.keys() {
            v.push_str(&format!(" ({})", shapes.short()));
        }
        if app.capo_override.is_some() {
            let sheet_capo = app.sheet_info().map_or(0, |i| i.1);
            v.push_str(&format!(" [sheet {sheet_capo}]"));
        }
        Some(tag("Capo:", v))
    } else if app.chords_on && app.capo_override == Some(0) {
        Some(tag("Capo:", "none".into()))
    } else {
        None
    };
    let tuning = (app.chords_on || app.show_diagrams)
        .then(|| app.tuning().map(|t| tag("Tuning:", t.to_string())))
        .flatten();
    let keytxt = app
        .keys()
        .map(|(sounding, _)| tag("Key:", format!("{} / {}", sounding.long(), sounding.camelot())));
    let bpm = app.bpm().map(|b| tag("BPM:", b.to_string()));
    let last = out.pop().unwrap_or_default();
    let left = vec![last.spans, vec![label("Release"), val(date)]];
    let right: Vec<_> = [tuning, capo_tag, bpm, keytxt].into_iter().flatten().collect();
    out.extend(meta_grid(left, right, avail));
    out
}

/// Two columns: `left` rows at the start, `right` rows left-aligned at one shared column
/// (`avail` minus the widest right item). Right items beyond the left rows get an empty
/// left side. When the columns don't fit side by side, the right items follow below,
/// indented under the values.
pub fn meta_grid(
    left: Vec<Vec<Span<'static>>>,
    right: Vec<Vec<Span<'static>>>,
    avail: usize,
) -> Vec<Line<'static>> {
    let w = |v: &[Span]| v.iter().map(|s| s.width()).sum::<usize>();
    let left_w = left.iter().map(|v| w(v)).max().unwrap_or(0);
    let right_w = right.iter().map(|v| w(v)).max().unwrap_or(0);
    let mut out = Vec::new();
    if right.is_empty() || left_w + 2 + right_w > avail {
        out.extend(left.into_iter().map(Line::from));
        for r in right {
            let mut row = vec![Span::raw(" ".repeat(13))];
            row.extend(r);
            out.push(Line::from(row));
        }
        return out;
    }
    let start = avail - right_w;
    let mut left = left.into_iter();
    let mut right = right.into_iter();
    loop {
        let (l, r) = (left.next(), right.next());
        if l.is_none() && r.is_none() {
            break;
        }
        let mut row = l.unwrap_or_default();
        if let Some(r) = r {
            let pad = start.saturating_sub(w(&row));
            row.push(Span::raw(" ".repeat(pad)));
            row.extend(r);
        }
        out.push(Line::from(row));
    }
    out
}

pub(super) fn draw_left(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(app, "now playing");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let th = app.theme.clone();

    let Some(snap) = app.snap.clone() else {
        app.hits = Default::default();
        let msg = format!("waiting for {}…", app.args.player);
        let y = inner.y + inner.height / 2;
        f.render_widget(
            Paragraph::new(Line::styled(msg, Style::new().fg(th.fade(0.4))))
                .alignment(Alignment::Center),
            Rect {
                y,
                height: 1,
                ..inner
            },
        );
        return;
    };

    let pos = app.position();
    let meta_lines = meta_lines(app, &snap, inner.width.saturating_sub(2) as usize);

    let area_in = inner.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    let plan = plan_controls(
        area_in.width,
        snap.shuffle,
        snap.repeat.map(|r| r == crate::player::Loop::Track),
        snap.status == Status::Playing,
        app.volume.is_some(),
        area_in.height >= 4,
    );
    let heights = left_heights(area_in.height, meta_lines.len() as u16, plan.vol_row);
    let mut y = area_in.y;
    let mut take = |h: u16| {
        let r = Rect { x: area_in.x, y, width: area_in.width, height: h };
        y += h;
        r
    };
    let cover_a = take(heights.cover);
    take(heights.gap_cover);
    let meta_a = take(heights.meta);
    take(heights.gap_meta);
    let prog_a = take(heights.prog);
    take(heights.gap_prog);
    let btn_a = take(heights.btn);
    let vol_a = take(heights.vol);

    // Cover
    if cover_a.height >= 3 {
        draw_cover(f, app, cover_a);
    }

    // Metadata
    f.render_widget(Paragraph::new(meta_lines), meta_a);

    // Progress bar
    app.hits.progress = if prog_a.height > 0 {
        timed_bar(f, &th, prog_a, pos, snap.length)
    } else {
        Rect::default()
    };

    // Buttons: whole buttons only, and each hit area is exactly what is drawn
    app.hits.shuffle = Rect::default();
    app.hits.repeat = Rect::default();
    app.hits.prev = Rect::default();
    app.hits.play = Rect::default();
    app.hits.next = Rect::default();
    if btn_a.height > 0 {
        for b in &plan.buttons {
            let w = b.label.chars().count() as u16;
            let r = Rect { x: btn_a.x + b.x, y: btn_a.y, width: w, height: 1 };
            let on = match b.id {
                0 => snap.shuffle == Some(true),
                1 => snap.repeat.is_some_and(|r| r != crate::player::Loop::None),
                _ => false,
            };
            let style = if b.id == 3 || on {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else if b.id < 2 {
                Style::new().fg(th.fade(0.5))
            } else {
                Style::new().fg(th.text())
            };
            f.render_widget(Paragraph::new(Span::styled(b.label.clone(), style)), r);
            match b.id {
                0 => app.hits.shuffle = r,
                1 => app.hits.repeat = r,
                2 => app.hits.prev = r,
                3 => app.hits.play = r,
                _ => app.hits.next = r,
            }
        }
    }

    // Volume: [ - ] [ 100% ] [ + ]   (click the percentage to mute)
    app.hits.vol_minus = Rect::default();
    app.hits.vol_pct = Rect::default();
    app.hits.vol_plus = Rect::default();
    app.hits.vol_area = Rect::default();
    let row = if plan.vol_row { vol_a } else { btn_a };
    if let (Some((kind, vx)), true) = (plan.vol, row.height > 0) {
        let muted = app.muted();
        let pct = app.volume_pct().unwrap_or(0);
        let (mw, pw, gap) = kind.cells();
        let x0 = row.x + vx;
        let btn = Style::new().fg(th.text());
        let pct_style = if muted {
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)
        } else {
            Style::new().fg(th.primary())
        };
        let cell = |x: u16, w: u16| Rect { x, y: row.y, width: w, height: 1 };
        let minus = cell(x0, mw);
        let (pct_r, plus) = if pw > 0 {
            (cell(x0 + mw + gap, pw), cell(x0 + mw + gap + pw + gap, mw))
        } else {
            (Rect::default(), cell(x0 + mw, mw))
        };
        let (m, p) = match kind {
            VolKind::Full => ("[ - ]", "[ + ]"),
            _ => ("[-]", "[+]"),
        };
        f.render_widget(Paragraph::new(Span::styled(m, btn)), minus);
        if pw > 0 {
            let txt = match (kind, muted) {
                (VolKind::Full, true) => "[ MUTE ]".to_string(),
                (VolKind::Full, false) => format!("[ {pct}% ]"),
                (_, true) => "MUTE".to_string(),
                (_, false) => format!("{pct}%"),
            };
            f.render_widget(
                Paragraph::new(Span::styled(format!("{txt:^w$}", w = pw as usize), pct_style)),
                pct_r,
            );
        }
        f.render_widget(Paragraph::new(Span::styled(p, btn)), plus);
        app.hits.vol_minus = minus;
        app.hits.vol_pct = pct_r;
        app.hits.vol_plus = plus;
        app.hits.vol_area = Rect {
            x: minus.x,
            y: row.y,
            width: plus.right().saturating_sub(minus.x),
            height: 1,
        };
    }
}

// ── the control row: pure layout, so every size can be tested ──────────────

/// Row heights of the left panel. Priority when it is short: buttons, volume row, progress bar,
/// metadata (as many lines as fit), and the cover last, which takes whatever is left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LeftHeights {
    pub cover: u16,
    pub gap_cover: u16,
    pub meta: u16,
    pub gap_meta: u16,
    pub prog: u16,
    pub gap_prog: u16,
    pub btn: u16,
    pub vol: u16,
}

pub fn left_heights(h: u16, meta_len: u16, vol_row: bool) -> LeftHeights {
    fn take(rem: &mut u16, want: u16) -> u16 {
        let got = want.min(*rem);
        *rem -= got;
        got
    }
    let mut rem = h;
    let btn = take(&mut rem, 1);
    let vol = take(&mut rem, vol_row as u16);
    let prog = take(&mut rem, 1);
    let gap_prog = if prog > 0 { take(&mut rem, 1) } else { 0 };
    let (gap_meta, meta) = if rem >= 2 && meta_len > 0 {
        let g = take(&mut rem, 1);
        (g, take(&mut rem, meta_len))
    } else {
        // one spare row: a metadata line (no gap) beats an invisible cover
        (0, take(&mut rem, meta_len.min(1)))
    };
    let gap_cover = if meta > 0 && meta == meta_len { take(&mut rem, 1) } else { 0 };
    LeftHeights { cover: rem, gap_cover, meta, gap_meta, prog, gap_prog, btn, vol }
}
