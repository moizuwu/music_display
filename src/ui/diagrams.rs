use super::*;

/// Minimum width of the lyrics column for the diagram panel to be worth showing.
pub(super) const DIAGRAM_MIN_WIDTH: u16 = 56;
pub(super) const DIAGRAM_PANEL_WIDTH: u16 = 28;

pub(super) fn draw_lyrics_and_diagrams(f: &mut Frame, app: &mut App, area: Rect) {
    if app.show_diagrams && !app.hits.stack_diagrams && area.width >= DIAGRAM_MIN_WIDTH {
        let [l, d] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(DIAGRAM_PANEL_WIDTH),
        ])
        .areas(area);
        draw_lyrics(f, app, l);
        draw_diagrams(f, app, d);
    } else {
        draw_lyrics(f, app, area);
    }
}

/// Chords of the current stretch of the song, in order of first appearance (max 6), plus
/// which one is being played. The active chord is estimated from how far through the
/// line we are, since the sheet carries no timing of its own.
pub(super) fn section_chords(app: &App) -> (Vec<String>, Option<usize>) {
    let Some(a) = app.aligned.as_ref().filter(|a| a.usable_at(app.args.chords_match)) else {
        return (vec![], None);
    };
    let LyricsState::Ready(Lyrics::Synced(l)) = &app.lyrics else {
        return (vec![], None);
    };
    let (tr, flats) = (app.chord_shift(), app.use_flats());
    let show = |n: &str| chords::transpose(n, tr, flats);
    let t = app.position() + app.offset;
    let cur = l.partition_point(|(ts, _)| *ts <= t).checked_sub(1);
    let mut names: Vec<String> = Vec::new();
    let push = |n: String, names: &mut Vec<String>| {
        if !names.contains(&n) && names.len() < 6 {
            names.push(n);
        }
    };
    match cur {
        None => {
            for n in a.intro.iter().flatten() {
                push(show(n), &mut names);
            }
            for cs in a.per_line.iter().take(2) {
                for (_, n) in cs {
                    push(show(n), &mut names);
                }
            }
            let act = names.first().map(|_| 0);
            (names, act)
        }
        Some(c) => {
            for cs in a.per_line.iter().skip(c).take(4) {
                for (_, n) in cs {
                    push(show(n), &mut names);
                }
            }
            // which chord of the active line are we on?
            let cs = a.per_line.get(c).filter(|v| !v.is_empty());
            let active = cs.and_then(|cs| {
                let next = l.get(c + 1).map_or(l[c].0 + 6.0, |x| x.0);
                let dur = (next - l[c].0).min(est_sung(&l[c].1)).max(1.0);
                let f = ((t - l[c].0) / dur).clamp(0.0, 0.999);
                let len = l[c].1.split_whitespace().collect::<Vec<_>>().join(" ").chars().count().max(1);
                let k = cs
                    .iter()
                    .rposition(|(col, _)| *col as f64 / len as f64 <= f)
                    .unwrap_or(0);
                names.iter().position(|n| *n == show(&cs[k].1))
            });
            (names, active)
        }
    }
}

pub(super) fn draw_diagrams(f: &mut Frame, app: &mut App, area: Rect) {
    let title = match app.tuning() {
        Some(t) if app.chords_on => format!("chords · {t}"),
        _ => "chords".to_string(),
    };
    let block = panel(app, &title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let th = app.theme.clone();
    let dim = Style::new().fg(th.fade(0.5));
    if !app.chords_on {
        return centered_msg(f, inner, &[("chords are off", dim), ("press g", dim)]);
    }
    let (names, active) = section_chords(app);
    if names.is_empty() {
        return centered_msg(f, inner, &[("no chords here", dim)]);
    }
    const CELL_W: u16 = 13;
    const CELL_H: u16 = 8;
    let cols = (inner.width / CELL_W).max(1) as usize;
    let shapes: Vec<Option<diagram::Shape>> = names.iter().map(|n| diagram::lookup(n)).collect();
    // leave room for the tab underneath when there is any
    let tab_h = 7u16;
    let grid_rows_fit = (inner.height / CELL_H).max(1) as usize;
    let want_rows = names.len().div_ceil(cols);
    let rows = want_rows.min(grid_rows_fit);
    let show_tab = inner.height >= rows as u16 * CELL_H + tab_h;
    for (i, (name, shape)) in names.iter().zip(&shapes).enumerate() {
        if i / cols >= rows {
            break;
        }
        let r = Rect {
            x: inner.x + (i % cols) as u16 * CELL_W + 1,
            y: inner.y + (i / cols) as u16 * CELL_H,
            width: CELL_W - 1,
            height: CELL_H - 1,
        }
        .intersection(inner);
        let is_active = active == Some(i);
        let body = if is_active { th.primary() } else { th.fade(0.35) };
        let name_style = if is_active {
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(th.text())
        };
        let lines: Vec<Line> = match shape {
            Some(sh) => diagram::render(name, sh)
                .into_iter()
                .enumerate()
                .map(|(k, l)| Line::styled(l, if k == 0 { name_style } else { Style::new().fg(body) }))
                .collect(),
            None => vec![
                Line::styled(format!("{name:^11}"), name_style),
                Line::styled(" diagram", dim),
                Line::styled(" unavailable", dim),
            ],
        };
        f.render_widget(Paragraph::new(lines), r);
    }
    if show_tab {
        let max_cols = ((inner.width.saturating_sub(3)) / 4).max(1) as usize;
        let tab = diagram::tab(&shapes[..shapes.len().min(max_cols)]);
        let y0 = inner.y + rows as u16 * CELL_H;
        let lines: Vec<Line> = tab.into_iter().map(|l| Line::styled(l, Style::new().fg(th.fade(0.2)))).collect();
        f.render_widget(
            Paragraph::new(lines),
            Rect { x: inner.x + 1, y: y0, width: inner.width.saturating_sub(1), height: 6 },
        );
    }
}
