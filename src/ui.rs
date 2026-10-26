use crate::app::{App, CavaPlacement, CoverState, LyricsState, Rendered, YearState};
use crate::lyrics::Lyrics;
use crate::player::Status;
use crate::theme::Theme;
use crate::util::fmt_time;
use crate::{bigfont, chords, cover, diagram};
use image::imageops::FilterType;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui_image::{Image, Resize};

/// Metadata | Lyrics | Cava. The left and right columns get exactly the same width, so
/// the cava panel mirrors the album/metadata column; lyrics take whatever is left.
pub fn three_panel(main: Rect) -> [Rect; 3] {
    let side = ((main.width as u32 * 28 / 100) as u16)
        .max(20)
        .min(main.width.saturating_sub(20) / 2);
    Layout::horizontal([
        Constraint::Length(side),
        Constraint::Min(1),
        Constraint::Length(side),
    ])
    .areas(main)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    draw_main(f, app);
    if app.show_help {
        draw_help(f, app);
    }
    if app.lib.open {
        draw_library(f, app);
    }
    if app.show_settings {
        draw_settings(f, app);
    }
}

/// `o`: tabs, the list (scrolled to the selection), and a hint / note line.
fn draw_library(f: &mut Frame, app: &mut App) {
    use crate::app::TABS;
    use ratatui::widgets::Clear;
    let area = f.area();
    let th = app.theme.clone();
    let w = area.width.min(72);
    let h = area.height.min(28);
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(
            " spotify ",
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    if inner.height < 4 {
        return;
    }
    let dim = Style::new().fg(th.fade(0.45));
    let sel_style = Style::new().fg(th.primary()).add_modifier(Modifier::BOLD);
    let width = inner.width as usize;

    // tabs
    let mut tabs: Vec<Span> = Vec::new();
    for (i, t) in TABS.iter().enumerate() {
        if i > 0 {
            tabs.push(Span::styled(" · ", dim));
        }
        let style = if i == app.lib.tab { sel_style.add_modifier(Modifier::UNDERLINED) } else { Style::new().fg(th.text()) };
        tabs.push(Span::styled(*t, style));
    }
    f.render_widget(Paragraph::new(Line::from(tabs)), Rect { height: 1, ..inner });

    // where we are: tab › album…, or the playlist picker
    let crumbs = match &app.lib.picker {
        Some((_, _, l)) => l.title.clone(),
        None => std::iter::once(TABS[app.lib.tab].to_string())
            .chain(app.lib.stack.iter().map(|l| l.title.clone()))
            .collect::<Vec<_>>()
            .join(" › "),
    };
    let summary = app.lib.level().summary.clone();
    let room = width.saturating_sub(crate::util::width(&summary) + 2);
    let crumbs: String = crumbs.chars().take(room).collect();
    let pad = width.saturating_sub(crate::util::width(&crumbs) + crate::util::width(&summary));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(crumbs, Style::new().fg(th.accent_fade(0.2))),
            Span::raw(" ".repeat(pad)),
            Span::styled(summary, dim),
        ])),
        Rect { y: inner.y + 1, height: 1, ..inner },
    );

    // the list
    let list = Rect { y: inner.y + 2, height: inner.height - 3, ..inner };
    let view_h = list.height as usize;
    let liked_uri = app.liked_uri.clone();
    let l = app.lib.level_mut();
    if l.sel < l.top {
        l.top = l.sel;
    } else if l.sel >= l.top + view_h.max(1) {
        l.top = l.sel + 1 - view_h.max(1);
    }
    let empty = if l.loading {
        Some("Loading…".to_string())
    } else if let Some(e) = &l.error {
        Some(e.clone())
    } else if l.items.is_empty() {
        Some("Nothing here".to_string())
    } else {
        None
    };
    let lines: Vec<Line> = match empty {
        Some(msg) => vec![Line::styled(msg, dim)],
        None => l
            .items
            .iter()
            .enumerate()
            .skip(l.top)
            .take(view_h)
            .map(|(i, it)| {
                let sel = i == l.sel;
                let style = if sel { sel_style } else { Style::new().fg(th.text()) };
                let mut sub = it.sub.clone();
                if it.kind == "track" && it.duration > 0.0 {
                    let d = it.duration.round() as u64;
                    sub = format!("{sub}  {}:{:02}", d / 60, d % 60);
                }
                let mark = if it.openable { " ›" } else if it.uri == liked_uri { " ♪" } else { "" };
                let name_room = width.saturating_sub(crate::util::width(&sub).min(width / 2) + 5);
                let mut name: String = it.name.clone();
                while crate::util::width(&name) > name_room && name.pop().is_some() {}
                let name = format!("{name}{mark}");
                let sub_room = width.saturating_sub(crate::util::width(&name) + 4);
                let sub: String = sub.chars().take(sub_room).collect();
                let pad = width.saturating_sub(crate::util::width(&name) + crate::util::width(&sub) + 2);
                Line::from(vec![
                    Span::styled(if sel { "▸ " } else { "  " }, style),
                    Span::styled(name, style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(sub, if sel { style } else { dim }),
                ])
            })
            .collect(),
    };
    f.render_widget(Paragraph::new(lines), list);

    let hint = match (&app.lib.note, &app.lib.picker) {
        (Some(n), _) => n.clone(),
        (None, Some(_)) => "enter add · esc back".to_string(),
        (None, None) => "tab/1-5 · enter play · →/← open/back · a queue · A add · f like · r reload · esc".to_string(),
    };
    let hint: String = hint.chars().take(width).collect();
    f.render_widget(
        Paragraph::new(Line::styled(hint, dim)).alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

fn draw_settings(f: &mut Frame, app: &mut App) {
    use crate::app::Setting;
    use ratatui::widgets::Clear;
    let area = f.area();
    let th = app.theme.clone();
    let rows = Setting::ALL.len() as u16;
    let w = area.width.min(54);
    let h = area.height.min(rows + 5);
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(
            " settings ",
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    let view_h = inner.height.saturating_sub(1) as usize;
    // keep the selected row in view on short terminals
    let sel = app.settings_sel;
    if sel < app.settings_top {
        app.settings_top = sel;
    } else if sel >= app.settings_top + view_h.max(1) {
        app.settings_top = sel + 1 - view_h.max(1);
    }
    let first = app.settings_top;
    let lines: Vec<Line> = Setting::ALL
        .iter()
        .enumerate()
        .skip(first)
        .take(view_h)
        .map(|(i, s)| {
            let sel = i == app.settings_sel;
            let style = if sel {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(th.text())
            };
            let mut label = s.label().to_string();
            if s.needs_restart() {
                label.push_str(" *");
            }
            let value = app.setting_value(*s);
            // on a narrow terminal shorten the label so the value stays visible
            let room = (inner.width as usize).saturating_sub(crate::util::width(&value) + 4);
            while crate::util::width(&label) > room && label.pop().is_some() {}
            let pad = (inner.width as usize)
                .saturating_sub(crate::util::width(&label) + crate::util::width(&value) + 2);
            Line::from(vec![
                Span::styled(if sel { "▸ " } else { "  " }, style),
                Span::styled(label, style),
                Span::raw(" ".repeat(pad)),
                Span::styled(value, style),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { height: view_h as u16, ..inner });
    if inner.height == 0 {
        return;
    }
    let hint = match &app.settings_note {
        Some(n) => n.chars().take(inner.width as usize).collect(),
        None => "↑/↓ select · space/←/→ change · * restart · esc".to_string(),
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.45)))).alignment(Alignment::Center),
        Rect {
            y: inner.bottom().saturating_sub(1),
            height: 1,
            ..inner
        },
    );
}

/// Lines of the help overlay, generated from the key table the input handler uses.
/// Returns the groups as separate blocks so they can be laid out in columns.
pub fn help_blocks(th: &Theme) -> Vec<Vec<Line<'static>>> {
    use crate::app::{BINDS, GROUPS, LIBRARY_KEYS, MOUSE};
    let head = |g: &str| {
        Line::styled(
            g.to_uppercase(),
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
        )
    };
    let row = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(format!("{k:<14}"), Style::new().fg(th.primary())),
            Span::styled(d.to_string(), Style::new().fg(th.text())),
        ])
    };
    GROUPS
        .iter()
        .map(|g| {
            let mut v = vec![head(g)];
            v.extend(BINDS.iter().filter(|b| b.group == *g).map(|b| row(b.keys, b.desc)));
            v.extend(MOUSE.iter().filter(|m| m.0 == *g).map(|m| row(m.1, m.2)));
            if *g == "Spotify" {
                v.push(Line::styled("in the library", Style::new().fg(th.fade(0.45))));
                v.extend(LIBRARY_KEYS.iter().map(|(k, d)| row(k, d)));
            }
            v
        })
        .collect()
}

fn draw_help(f: &mut Frame, app: &mut App) {
    use ratatui::widgets::Clear;
    let area = f.area();
    let th = app.theme.clone();
    let blocks = help_blocks(&th);
    // Two columns (Playback+Lyrics | Chords+View) when there is room, else one list.
    let two = area.width >= 78;
    let cols: Vec<Vec<Line>> = if two {
        let join = |a: &[Vec<Line<'static>>]| {
            let mut out: Vec<Line> = Vec::new();
            for (i, b) in a.iter().enumerate() {
                if i > 0 {
                    out.push(Line::raw(""));
                }
                out.extend(b.iter().cloned());
            }
            out
        };
        vec![join(&blocks[..2]), join(&blocks[2..])]
    } else {
        let mut all: Vec<Line> = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            if i > 0 {
                all.push(Line::raw(""));
            }
            all.extend(b.iter().cloned());
        }
        vec![all]
    };
    let content_h = cols.iter().map(Vec::len).max().unwrap_or(0) as u16;
    let w = area.width.min(if two { 80 } else { 44 });
    let h = area.height.min(content_h + 4); // borders + footer line
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(
            " help ",
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    let view_h = inner.height.saturating_sub(1);
    let max_scroll = content_h.saturating_sub(view_h);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let n = cols.len() as u16;
    let col_w = inner.width / n;
    for (i, lines) in cols.into_iter().enumerate() {
        f.render_widget(
            Paragraph::new(lines).scroll((app.help_scroll, 0)),
            Rect {
                x: inner.x + i as u16 * col_w,
                y: inner.y,
                width: col_w.saturating_sub(1),
                height: view_h,
            },
        );
    }
    let hint = if max_scroll > 0 {
        "↑/↓ scroll · any other key closes"
    } else {
        "any key closes"
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.45)))).alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

fn draw_main(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let th = app.theme.clone();
    // Only paint a background when we actually know the surface colour; otherwise the
    // terminal's own (possibly transparent) background is left alone.
    if !app.args.transparent && th.surface.is_some() {
        f.render_widget(
            Block::new().style(Style::new().bg(th.surface()).fg(th.text())),
            area,
        );
    }

    let [main, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    app.lyric_hits = Default::default();

    if app.minimal {
        draw_minimal(f, app, main);
        draw_footer(f, app, footer);
        return;
    }

    // ── column layout ──
    let (left, lyr, cav) = match (app.show_lyrics, app.show_cava, app.placement) {
        (false, false, _) => (main, None, None),
        (true, false, _) => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main);
            (l, Some(r), None)
        }
        (false, true, CavaPlacement::ThirdPanel) => {
            // Lyrics hidden: keep the two remaining columns the same width.
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(main);
            (l, None, Some(r))
        }
        (false, true, CavaPlacement::UnderLyrics) => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main);
            (l, None, Some(r))
        }
        (true, true, CavaPlacement::ThirdPanel) => {
            let [l, m, r] = three_panel(main);
            (l, Some(m), Some(r))
        }
        (true, true, CavaPlacement::UnderLyrics) => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main);
            let [t, b] =
                Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .areas(r);
            (l, Some(t), Some(b))
        }
    };

    draw_left(f, app, left);
    if let Some(r) = lyr {
        draw_lyrics_and_diagrams(f, app, r);
    }
    if let Some(r) = cav {
        draw_cava(f, app, r);
    }
    draw_footer(f, app, footer);
}

/// One-row progress bar: ━ filled, ─ remaining, ● knob.
/// `0:42 ━━━●─── 3:10`: both timestamps sit inside the row, the bar between them.
/// Returns the bar part (the click target for seeking).
fn timed_bar(f: &mut Frame, th: &Theme, row: Rect, pos: f64, length: f64) -> Rect {
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

fn progress_bar(f: &mut Frame, th: &Theme, r: Rect, frac: f64) {
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

/// `M`: title, lyrics across the whole area, and a thin progress bar. No cover,
/// metadata, buttons or visualiser.
fn draw_minimal(f: &mut Frame, app: &mut App, main: Rect) {
    app.hits = Default::default();
    let th = app.theme.clone();
    let [title, body, prog] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(main);
    if let Some(s) = app.snap.clone() {
        let text = if s.artist.is_empty() {
            s.title.clone()
        } else {
            format!("{} — {}", s.artist, s.title)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("◆ ", Style::new().fg(th.primary())),
                Span::styled(text, Style::new().fg(th.text()).add_modifier(Modifier::BOLD)),
            ]))
            .alignment(Alignment::Center),
            title,
        );
        app.hits.progress = timed_bar(f, &th, prog, app.position(), s.length);
    }
    draw_lyrics_and_diagrams(f, app, body);
}

/// Minimum width of the lyrics column for the diagram panel to be worth showing.
const DIAGRAM_MIN_WIDTH: u16 = 56;
const DIAGRAM_PANEL_WIDTH: u16 = 28;

fn draw_lyrics_and_diagrams(f: &mut Frame, app: &mut App, area: Rect) {
    if app.show_diagrams && area.width >= DIAGRAM_MIN_WIDTH {
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
fn section_chords(app: &App) -> (Vec<String>, Option<usize>) {
    let Some(a) = app.aligned.as_ref().filter(|a| a.usable()) else {
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

fn draw_diagrams(f: &mut Frame, app: &mut App, area: Rect) {
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

fn panel<'a>(app: &App, title: &'a str) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(app.theme.accent_fade(0.6)))
        .title(Span::styled(
            format!(" {title} "),
            Style::new()
                .fg(app.theme.primary())
                .add_modifier(Modifier::BOLD),
        ))
}

// ── left column ─────────────────────────────────────────────────────────
/// The metadata block: Track / Artist / Album, then Track No and Release on the left and
/// Tuning, Capo, BPM, Key one per row in a right-hand column (see [`meta_grid`]). Those
/// four are hidden when no source provides them.
fn meta_lines(app: &App, snap: &crate::player::Snapshot, avail: usize) -> Vec<Line<'static>> {
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

fn draw_left(f: &mut Frame, app: &mut App, area: Rect) {
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

    // Volume sits beside the transport buttons when it fits, else on its own row below.
    let width = inner.width.saturating_sub(2);
    let vol_shown = app.volume.is_some();
    let pct_txt = if app.muted() {
        "[ MUTE ]".to_string()
    } else {
        format!("[ {}% ]", app.volume_pct().unwrap_or(0))
    };
    let vol_w = 5 + 1 + 8 + 1 + 5; // "[ - ] [ 100% ] [ + ]"
    let rep_lbl = match snap.repeat {
        Some(crate::player::Loop::Track) => "[ rep 1 ]",
        _ => "[ rep ]",
    };
    // shuffle / repeat buttons exist only for what the player reports
    let mut labels: Vec<(&str, u8)> = Vec::new();
    if snap.shuffle.is_some() {
        labels.push(("[ shuf ]", 0));
    }
    if snap.repeat.is_some() {
        labels.push((rep_lbl, 1));
    }
    let play_lbl = if snap.status == Status::Playing {
        "[ pause ]"
    } else {
        "[ play ]"
    };
    labels.extend([("[ prev ]", 2), (play_lbl, 3), ("[ next ]", 4)]);
    let gap = 1u16;
    // `[ shuf ] [ rep ] - [ prev ] [ pause ] [ next ] - [ - ] [ 100% ] [ + ]`: a dash
    // (with a space each side) sets the groups apart
    let modes = labels.iter().filter(|l| l.1 < 2).count() as u16;
    let total: u16 = labels.iter().map(|l| l.0.chars().count() as u16).sum::<u16>()
        + gap * (labels.len() as u16 - 1)
        + if modes > 0 { 2 } else { 0 };
    let vol_inline = vol_shown && total as usize + 3 + vol_w <= width as usize;
    let vol_row = vol_shown && !vol_inline;
    let [cover_a, _, meta_a, _, prog_a, _, btn_a, vol_a] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(meta_lines.len() as u16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(vol_row as u16),
    ])
    .areas(inner.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    }));

    // Cover
    if cover_a.height >= 3 {
        draw_cover(f, app, cover_a);
    }

    // Metadata
    f.render_widget(Paragraph::new(meta_lines), meta_a);

    // Progress bar
    app.hits.progress = timed_bar(f, &th, prog_a, pos, snap.length);

    // Buttons
    let full = if vol_inline { total + 3 + vol_w as u16 } else { total };
    let mut x = btn_a.x + btn_a.width.saturating_sub(full) / 2;
    let transport_start = x;
    app.hits.shuffle = Rect::default();
    app.hits.repeat = Rect::default();
    let sep = Style::new().fg(th.fade(0.5));
    for (l, id) in &labels {
        let w = l.chars().count() as u16;
        if *id == 2 && modes > 0 {
            // x already includes one gap after the last mode button
            f.render_widget(
                Paragraph::new(Span::styled("-", sep)),
                Rect { x: x.min(btn_a.right().saturating_sub(1)), y: btn_a.y, width: 1, height: 1 },
            );
            x = x.saturating_add(2);
        }
        let r = Rect {
            x,
            y: btn_a.y,
            width: w.min(btn_a.right().saturating_sub(x)),
            height: 1,
        };
        let on = match id {
            0 => snap.shuffle == Some(true),
            1 => snap.repeat.is_some_and(|r| r != crate::player::Loop::None),
            _ => false,
        };
        let style = if *id == 3 || on {
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
        } else if *id < 2 {
            Style::new().fg(th.fade(0.5))
        } else {
            Style::new().fg(th.text())
        };
        f.render_widget(Paragraph::new(Span::styled(*l, style)), r);
        match id {
            0 => app.hits.shuffle = r,
            1 => app.hits.repeat = r,
            2 => app.hits.prev = r,
            3 => app.hits.play = r,
            _ => app.hits.next = r,
        }
        x = x.saturating_add(w + gap);
    }

    // Volume: [ - ] [ 100% ] [ + ]   (click the percentage to mute)
    app.hits.vol_minus = Rect::default();
    app.hits.vol_pct = Rect::default();
    app.hits.vol_plus = Rect::default();
    app.hits.vol_area = Rect::default();
    if vol_shown && (vol_inline || vol_a.height > 0) {
        let (row, x0) = if vol_inline {
            // " - " between the transport buttons and the volume control
            f.render_widget(
                Paragraph::new(Span::styled("-", sep)),
                Rect { x: (transport_start + total + 1).min(btn_a.right().saturating_sub(1)), y: btn_a.y, width: 1, height: 1 },
            );
            (btn_a, transport_start + total + 3)
        } else {
            (vol_a, vol_a.x + vol_a.width.saturating_sub(vol_w as u16) / 2)
        };
        let muted = app.muted();
        let pct_w = 8u16;
        let cell = |x: u16, w: u16| Rect {
            x,
            y: row.y,
            width: w.min(row.right().saturating_sub(x)),
            height: 1,
        };
        let (minus, pct, plus) = (cell(x0, 5), cell(x0 + 6, pct_w), cell(x0 + 6 + pct_w + 1, 5));
        let btn = Style::new().fg(th.text());
        let pct_style = if muted {
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)
        } else {
            Style::new().fg(th.primary())
        };
        f.render_widget(Paragraph::new(Span::styled("[ - ]", btn)), minus);
        f.render_widget(
            Paragraph::new(Span::styled(format!("{pct_txt:^8}"), pct_style)),
            pct,
        );
        f.render_widget(Paragraph::new(Span::styled("[ + ]", btn)), plus);
        app.hits.vol_minus = minus;
        app.hits.vol_pct = pct;
        app.hits.vol_plus = plus;
        app.hits.vol_area = Rect {
            x: minus.x,
            y: row.y,
            width: plus.right().saturating_sub(minus.x),
            height: 1,
        };
    }
}

fn draw_cover(f: &mut Frame, app: &mut App, area: Rect) {
    let th = app.theme.clone();
    let fs = app.picker.font_size();
    let (fw, fh) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
    let ascii = app.args.ascii_cover;
    let picker = &app.picker;
    match &mut app.cover {
        CoverState::Ready { src, rendered } => {
            // Largest whole-cell box with the artwork's aspect ratio, centred.
            let (w, h) = (src.width() as f64, src.height() as f64);
            let scale =
                ((area.width as f64 * fw as f64) / w).min((area.height as f64 * fh as f64) / h);
            let cw = (((w * scale) / fw as f64).floor() as u16).clamp(1, area.width);
            let ch = (((h * scale) / fh as f64).floor() as u16).clamp(1, area.height);
            let r = Rect {
                x: area.x + (area.width - cw) / 2,
                y: area.y + (area.height - ch) / 2,
                width: cw,
                height: ch,
            };

            if rendered.as_ref().map(Rendered::cells) != Some((cw, ch)) {
                let to_ascii = |src: &image::DynamicImage| Rendered::Ascii {
                    cells: (cw, ch),
                    art: cover::ascii_art(src, cw, ch),
                };
                *rendered = Some(if ascii {
                    to_ascii(src)
                } else {
                    // Pixel-exact bitmap: scaled and centre-cropped to *precisely* cw×ch
                    // cells, so the graphics protocol has nothing to pad. (Letting it fit
                    // a slightly-off aspect padded the remainder with transparent black,
                    // which showed up as a dark sliver on the right/bottom edge.)
                    let exact = src.resize_to_fill(
                        cw as u32 * fw,
                        ch as u32 * fh,
                        FilterType::Triangle,
                    );
                    match picker.new_protocol(exact, Size::new(cw, ch), Resize::Fit(None)) {
                        Ok(p) => Rendered::Graphics {
                            cells: (cw, ch),
                            proto: Box::new(p),
                        },
                        Err(_) => to_ascii(src),
                    }
                });
            }
            match rendered {
                Some(Rendered::Graphics { proto, .. }) => {
                    f.render_widget(Image::new(proto.as_ref()), r)
                }
                Some(Rendered::Ascii { art, .. }) => draw_ascii(f.buffer_mut(), r, art),
                None => {}
            }
        }
        other => {
            let msg = match other {
                CoverState::Loading => "loading cover…",
                _ => "♪",
            };
            let y = area.y + area.height / 2;
            f.render_widget(
                Paragraph::new(Line::styled(msg, Style::new().fg(th.fade(0.5))))
                    .alignment(Alignment::Center),
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );
        }
    }
}

fn draw_ascii(buf: &mut Buffer, r: Rect, art: &cover::AsciiArt) {
    for y in 0..art.rows.min(r.height) {
        for x in 0..art.cols.min(r.width) {
            let (ch, (cr, cg, cb)) = art.cells[y as usize * art.cols as usize + x as usize];
            if let Some(cell) = buf.cell_mut((r.x + x, r.y + y)) {
                let mut utf8 = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut utf8))
                    .set_fg(Color::Rgb(cr, cg, cb));
            }
        }
    }
}

// ── lyrics ──────────────────────────────────────────────────────────────
/// Chord columns of `orig` (char columns in its single-spaced text) moved to the same
/// relative place in `trans`, snapped to the start of the nearest word.
pub fn remap_chords(cs: &[(usize, String)], orig: &str, trans: &str) -> Vec<(usize, String)> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let (o, t) = (norm(orig), norm(trans));
    let (ol, tl) = (o.chars().count().max(1), t.chars().count());
    let starts: Vec<usize> = chords::words_pos(&t).into_iter().map(|(s, _)| s).collect();
    let mut out: Vec<(usize, String)> = Vec::new();
    for (c, name) in cs {
        let want = (*c as f64 / ol as f64 * tl as f64).round() as usize;
        let col = starts.iter().copied().min_by_key(|s| s.abs_diff(want)).unwrap_or(0);
        out.push((col, name.clone()));
    }
    out.sort_by_key(|c| c.0);
    out
}

/// Closing punctuation (CJK and Latin) that must not start a wrapped line.
fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。' | '、' | '！' | '？' | '；' | '：' | '）' | '」' | '』' | '】' | '》' | '〉' | '”' | '’'
            | '…' | 'ー' | ',' | '.' | '!' | '?' | ';' | ':' | ')' | ']' | '}'
    )
}

/// Greedy word-wrap by display width.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for word in text.split_whitespace() {
        let ww = crate::util::width(word);
        if cur_w > 0 && cur_w + 1 + ww > width {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if ww > width {
            // hard-break very long tokens
            for ch in word.chars() {
                let cw = crate::util::cw(ch);
                if cur_w > 0 && cur_w + cw > width {
                    // closing punctuation never starts a line: carry the last char over
                    let carry = if no_line_start(ch) && cur.chars().count() > 1 { cur.pop() } else { None };
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                    if let Some(p) = carry {
                        cur.push(p);
                        cur_w = crate::util::cw(p);
                    }
                }
                cur.push(ch);
                cur_w += cw;
            }
            continue;
        }
        if cur_w > 0 {
            cur.push(' ');
            cur_w += 1;
        }
        cur.push_str(word);
        cur_w += ww;
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

/// Same wrapping as [`wrap`] but returns `(start, end)` char ranges into `norm` (the
/// single-spaced text), so chord columns can be mapped onto the wrapped rows.
pub fn wrap_ranges(norm: &str, width: usize) -> Vec<(usize, usize)> {
    if width == 0 {
        return vec![];
    }
    let chars: Vec<char> = norm.chars().collect();
    let cw = crate::util::cw;
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let (mut start, mut end, mut cur_w) = (0usize, 0usize, 0usize);
    for (s, len) in chords::words_pos(norm) {
        let ww: usize = chars[s..s + len].iter().map(|c| cw(*c)).sum();
        if cur_w > 0 && cur_w + 1 + ww > width {
            rows.push((start, end));
            cur_w = 0;
        }
        if ww > width {
            for (k, c) in chars[s..s + len].iter().enumerate() {
                if cur_w + cw(*c) > width && cur_w > 0 {
                    if no_line_start(*c) && end > start + 1 {
                        rows.push((start, end - 1));
                        start = end - 1;
                        cur_w = cw(chars[end - 1]);
                    } else {
                        rows.push((start, end));
                        cur_w = 0;
                    }
                }
                if cur_w == 0 {
                    start = s + k;
                }
                end = s + k + 1;
                cur_w += cw(*c);
            }
            continue;
        }
        if cur_w == 0 {
            start = s;
        }
        end = s + len;
        cur_w += if cur_w > 0 { 1 + ww } else { ww };
    }
    if cur_w > 0 || rows.is_empty() {
        rows.push((start, end));
    }
    rows
}

/// `Am   G   C` with each chord at its column; neighbours are pushed apart, never overlap.
pub fn chord_line(cs: &[(usize, String)], text: &[char], limit: usize, n: i32, flats: bool) -> String {
    // chord columns are char indices into `text`; place them by terminal cell width
    let cell = |col: usize| -> usize {
        text.iter().take(col).map(|c| crate::util::cw(*c)).sum::<usize>()
            + col.saturating_sub(text.len())
    };
    let mut s = String::new();
    let mut w = 0usize;
    for (col, name) in cs {
        let name = chords::transpose(name, n, flats);
        let floor = if w > 0 { w + 1 } else { 0 };
        // a chord that would run past the panel edge is pulled left on its own,
        // instead of the whole row shifting off its words
        let at = cell(*col).min(limit.saturating_sub(crate::util::width(&name))).max(floor);
        s.push_str(&" ".repeat(at - w));
        w = at + crate::util::width(&name);
        s.push_str(&name);
    }
    s
}

fn centered_msg(f: &mut Frame, area: Rect, msgs: &[(&str, Style)]) {
    let start = area.y + area.height.saturating_sub(msgs.len() as u16) / 2;
    for (i, (m, s)) in msgs.iter().enumerate() {
        let y = start + i as u16;
        if y < area.bottom() {
            f.render_widget(
                Paragraph::new(Line::styled(*m, *s)).alignment(Alignment::Center),
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );
        }
    }
}

/// Rough time a line takes to sing, so the marker appears once the words are over
/// rather than the moment the line starts.
fn est_sung(text: &str) -> f64 {
    (text.chars().count() as f64 * 0.08).clamp(2.0, 6.0)
}

/// Style for a non-active lyric line `dist` lines away from the active one:
/// fades with distance, and past lines fade a little faster than upcoming ones.
fn fade_style(th: &Theme, dist: usize, past: bool) -> Style {
    let t = 0.40 + 0.08 * dist as f32 + if past { 0.10 } else { 0.0 };
    Style::new().fg(th.fade(t.min(0.78)))
}

/// Wrapped rows of a lyric line, each with the chord row (left offset, text) that sits
/// right above it — the same mapping the scrolling layout uses. Lines without chords
/// just get plain wrapped rows.
fn chord_rows(line: &str, cs: &[(usize, String)], width: usize, tr: i32, flats: bool) -> Vec<(String, Option<(usize, String)>)> {
    if cs.is_empty() {
        return wrap(line, width).into_iter().map(|t| (t, None)).collect();
    }
    let norm = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = norm.chars().collect();
    let ranges = wrap_ranges(&norm, width);
    let mut per_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); ranges.len()];
    for (c, name) in cs {
        let r = ranges.iter().rposition(|(s, _)| *s <= *c).unwrap_or(0);
        per_row[r].push((c.saturating_sub(ranges[r].0), name.clone()));
    }
    ranges
        .iter()
        .zip(per_row)
        .map(|((rs, re), here)| {
            let text: String = chars[*rs..*re].iter().collect();
            let chord = (!here.is_empty()).then(|| {
                let x = width.saturating_sub(crate::util::width(&text)) / 2;
                (x, chord_line(&here, &chars[*rs..*re], width.saturating_sub(x), tr, flats))
            });
            (text, chord)
        })
        .collect()
}

/// Big-font rows for `text` in a panel `width` × `height`: the largest scale
/// (down to one) whose wrapped pieces (up to 4) leave at least 2 rows for the neighbouring lines;
/// failing that the compact 3×5 font. `None`: the caller falls back to bold wrapped text.
pub fn big_stage(text: &str, width: usize, height: u16, scale: usize) -> Option<Vec<String>> {
    let fits = |rows: &Vec<String>| rows.len() as u16 + 2 <= height;
    (1..=scale.clamp(1, 3))
        .rev()
        .find_map(|sc| bigfont::render_scaled(text, width, 4, sc).filter(fits))
        .or_else(|| bigfont::render_compact(text, width, 4).filter(fits))
}

/// Everything [`draw_focus`] needs besides the frame, area and theme.
pub struct Focus<'a> {
    pub lines: &'a [String],
    /// The active line; `None` before the first line (the intro chords take the stage).
    pub current: Option<usize>,
    /// Blank rows between lines.
    pub gap: usize,
    /// Largest big-font scale to try.
    pub scale: usize,
    pub chords: Option<&'a chords::Aligned>,
    pub tr: i32,
    pub flats: bool,
    /// `chords_context`: chords on the active line plus this many upcoming ones.
    pub context: usize,
    /// The active line is in an instrumental break: show its break chords under the stage.
    pub in_break: bool,
}

/// "Focus" layout used by the big-font mode: the active line sits on a fixed stage in the
/// middle of the panel (big font if it fits, otherwise bold wrapped text), with the lines
/// before and after stacked outward from it, each with its chord row when it has chords.
/// Returns which lyric line each panel row belongs to (for click-to-seek).
pub fn draw_focus(f: &mut Frame, inner: Rect, th: &Theme, fc: &Focus) -> Vec<Option<usize>> {
    let width = inner.width as usize;
    let (lines, c) = (fc.lines, fc.current);
    let names = |v: &[String]| {
        v.iter()
            .map(|n| chords::transpose(n, fc.tr, fc.flats))
            .collect::<Vec<_>>()
            .join("   ")
    };
    let bold = Style::new().fg(th.primary()).add_modifier(Modifier::BOLD);
    // which lyric line each panel row belongs to (chord rows and the intro belong to none)
    let hits = std::cell::RefCell::new(vec![None; inner.height as usize]);
    let (stage, stage_style, stage_li): (Vec<String>, Style, Option<usize>) = match c {
        Some(c) => match big_stage(&lines[c], width, inner.height, fc.scale) {
            Some(rows) => (rows, Style::new().fg(th.primary()), Some(c)),
            None => (wrap(&lines[c], width), bold, Some(c)),
        },
        None => (
            fc.chords.map_or_else(Vec::new, |a| a.intro.iter().map(|n| names(n)).collect()),
            bold,
            None,
        ),
    };
    let sh = (stage.len() as u16).min(inner.height);
    let top = inner.y + (inner.height - sh) / 2;

    let put = |f: &mut Frame, y: i32, li: Option<usize>, text: &str, style: Style| {
        if y >= inner.y as i32 && y < inner.bottom() as i32 && !text.is_empty() {
            hits.borrow_mut()[(y - inner.y as i32) as usize] = li;
            f.render_widget(
                Paragraph::new(Line::styled(text.to_string(), style)).alignment(Alignment::Center),
                Rect { y: y as u16, height: 1, ..inner },
            );
        }
    };
    // a chord row at its own left offset inside the panel
    let put_at = |f: &mut Frame, y: i32, x: usize, text: &str, style: Style| {
        if y >= inner.y as i32 && y < inner.bottom() as i32 && !text.is_empty() {
            let w = (crate::util::width(text) as u16).min(inner.width);
            let xs = (inner.x + x as u16).min(inner.right().saturating_sub(w)).max(inner.x);
            f.render_widget(
                Paragraph::new(Line::styled(text.to_string(), style)),
                Rect { x: xs, y: y as u16, width: w, height: 1 },
            );
        }
    };

    for (i, row) in stage.iter().take(sh as usize).enumerate() {
        put(f, top as i32 + i as i32, stage_li, row, stage_style);
    }
    // summary of the active line's chords above the big text
    if let Some(c) = c {
        let summary = fc.chords.and_then(|a| a.per_line.get(c)).filter(|cs| !cs.is_empty()).map(|cs| {
            let mut v: Vec<String> = cs.iter().map(|(_, n)| chords::transpose(n, fc.tr, fc.flats)).collect();
            v.dedup();
            v.join("   ")
        });
        if let Some(cs) = summary {
            put(f, top as i32 - 1, Some(c), &cs, bold);
        }
    }
    // chords played during an instrumental break, right under the stage
    let mut after = top as i32 + sh as i32 + 1;
    if let (Some(c), Some(a), true) = (c, fc.chords, fc.in_break) {
        for g in a.gaps.get(c).into_iter().flatten() {
            put(f, after, None, &names(g), bold);
            after += 1;
        }
    }

    // the wrapped rows of line `li` with their chord rows (hidden outside `chords_context`)
    let cur = c.map_or(-1, |c| c as i64);
    let rows_of = |li: usize| {
        let d = li as i64 - cur;
        let shown = fc.context >= 99 || (d >= 0 && d <= fc.context as i64);
        let cs = fc.chords.and_then(|a| a.per_line.get(li)).filter(|_| shown);
        chord_rows(&lines[li], cs.map_or(&[], |v| v.as_slice()), width, fc.tr, fc.flats)
    };

    // Earlier lines, nearest first, growing upward.
    let mut y = top as i32 - 2;
    for k in 1..=c.unwrap_or(0) {
        let li = c.unwrap_or(0) - k;
        let style = fade_style(th, k, true);
        let chord_style = Style::new().fg(th.accent_fade(0.55));
        for (text, chord) in rows_of(li).iter().rev() {
            put(f, y, Some(li), text, style);
            y -= 1;
            if let Some((x, cl)) = chord {
                put_at(f, y, *x, cl, chord_style);
                y -= 1;
            }
        }
        y -= fc.gap as i32;
        if y < inner.y as i32 {
            break;
        }
    }
    // Upcoming lines, growing downward.
    let mut y = after;
    for k in c.map_or(0, |c| c + 1)..lines.len() {
        let style = fade_style(th, (k as i64 - cur) as usize, false);
        let chord_style = Style::new().fg(th.accent_fade(0.35));
        for (text, chord) in rows_of(k) {
            if let Some((x, cl)) = chord {
                put_at(f, y, x, &cl, chord_style);
                y += 1;
            }
            put(f, y, Some(k), &text, style);
            y += 1;
        }
        y += fc.gap as i32;
        if y >= inner.bottom() as i32 {
            break;
        }
    }
    hits.into_inner()
}

/// `[Jimin] ` / `[Jimin, V] ` in each member's colour, dimmed; empty if the tag plus the
/// text (`text_w` columns) wouldn't fit in `width`.
fn member_tag(members: &[String], width: usize, text_w: usize) -> Vec<Span<'static>> {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut spans = vec![Span::styled("[", dim)];
    for (i, m) in members.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(", ", dim));
        }
        spans.push(Span::styled(m.clone(), Style::new().fg(crate::genius::color(m)).add_modifier(Modifier::DIM)));
    }
    spans.push(Span::styled("] ", dim));
    let w: usize = spans.iter().map(|s| s.width()).sum();
    if w + text_w > width { Vec::new() } else { spans }
}

fn draw_lyrics(f: &mut Frame, app: &mut App, area: Rect) {
    let shown = app.translation().is_some_and(|t| app.lyric_lines().is_some_and(|l| l.len() == t.len()));
    if app.browse_expired() {
        app.browse = 0.0;
    }
    let title = if app.browse != 0.0 {
        "lyrics · browsing — End to follow".to_string()
    } else if !app.translate_on {
        "lyrics".to_string()
    } else {
        use crate::app::TranslateState as TS;
        match (&app.translate_state, app.translation()) {
            (_, Some(_)) if shown && app.translate_backend != "google" => {
                format!("lyrics · translated ({}, {})", app.args.translate_lang, app.translate_backend)
            }
            (_, Some(_)) if shown => format!("lyrics · translated ({})", app.args.translate_lang),
            (TS::Failed(e), _) => format!("lyrics · {e}"),
            _ => "lyrics · translating…".to_string(),
        }
    };
    let block = panel(app, &title);
    let inner = block.inner(area).inner(ratatui::layout::Margin {
        horizontal: 2 + app.args.lyric_pad,
        vertical: 1,
    });
    f.render_widget(block, area);
    let th = app.theme.clone();
    let dim = Style::new().fg(th.fade(0.5));

    if app.snap.is_none() {
        centered_msg(f, inner, &[("♪", dim)]);
        return;
    }
    let pos = app.position();
    let length = app.snap.as_ref().map(|s| s.length).unwrap_or(0.0);
    let (lines, current): (Vec<String>, Option<usize>) = match &app.lyrics {
        LyricsState::Fetching => return centered_msg(f, inner, &[("fetching lyrics…", dim)]),
        LyricsState::Ready(Lyrics::NotFound) => {
            return centered_msg(
                f,
                inner,
                &[
                    ("♪", Style::new().fg(th.primary())),
                    ("", dim),
                    ("no synced lyrics found", dim),
                ],
            );
        }
        LyricsState::Ready(Lyrics::Instrumental) => {
            return centered_msg(
                f,
                inner,
                &[("♪  instrumental  ♪", Style::new().fg(th.primary()))],
            );
        }
        LyricsState::Ready(Lyrics::Error(e)) => {
            return centered_msg(f, inner, &[("♪", dim), (e.as_str(), dim)]);
        }
        LyricsState::Ready(Lyrics::Synced(l)) => {
            let t = pos + app.offset;
            let cur = l.partition_point(|(ts, _)| *ts <= t).checked_sub(1);
            (l.iter().map(|(_, s)| s.clone()).collect(), cur)
        }
        LyricsState::Ready(Lyrics::Plain(l)) => (l.clone(), None),
    };
    let plain = matches!(app.lyrics, LyricsState::Ready(Lyrics::Plain(_)));
    // Translated text replaces the original line by line; chord columns, which point at
    // the original words, are carried over to the same relative place in the translation.
    let original = lines.clone();
    let mut lines = lines;
    let translated = match app.translation() {
        Some(t) if t.len() == lines.len() => {
            for (l, t) in lines.iter_mut().zip(t) {
                if !t.trim().is_empty() {
                    *l = t.clone();
                }
            }
            true
        }
        _ => false,
    };
    let stamps: Vec<f64> = match &app.lyrics {
        LyricsState::Ready(Lyrics::Synced(l)) => l.iter().map(|x| x.0).collect(),
        _ => Vec::new(),
    };

    // Blank rows between lines — dropped on short panels where they'd waste the space.
    let gap = if inner.height >= 10 {
        app.args.lyric_spacing as usize
    } else {
        0
    };

    let aligned: Option<chords::Aligned> = (app.chords_on && !plain)
        .then(|| app.aligned.clone())
        .flatten()
        .filter(|a| a.usable())
        .map(|mut a| {
            if translated {
                for (i, cs) in a.per_line.iter_mut().enumerate() {
                    if let (Some(o), Some(t)) = (original.get(i), lines.get(i)) {
                        *cs = remap_chords(cs, o, t);
                    }
                }
            }
            a
        });
    let tr = app.chord_shift();
    let flats = app.use_flats();

    // Big-font mode uses the fixed "focus" layout instead of the scrolling one — for the
    // active line, or before the first line when there are intro chords to show.
    let intro_chords = current.is_none() && aligned.as_ref().is_some_and(|a| !a.intro.is_empty());
    if app.big_lyrics && !plain && app.browse == 0.0 && (current.is_some() || intro_chords) {
        app.lyric_scroll_target = app.lyric_scroll; // nothing left to animate
        // in a long silence after the line (same rule as the scrolling layout's markers)
        let t = pos + app.offset;
        let in_break = current.is_some_and(|c| {
            let sung = stamps.get(c).copied().unwrap_or(f64::MAX) + est_sung(&lines[c]);
            lines[c].trim() != "♪"
                && t >= sung
                && match stamps.get(c + 1) {
                    Some(n) => n - stamps[c] > app.args.interlude_secs,
                    None => length > 0.0 && length - stamps[c] > app.args.interlude_secs + est_sung(&lines[c]),
                }
        });
        let focus = Focus {
            lines: &lines,
            current,
            gap,
            scale: app.args.big_scale as usize,
            chords: aligned.as_ref(),
            tr,
            flats,
            context: app.args.chords_context as usize,
            in_break,
        };
        let rows = draw_focus(f, inner, &th, &focus);
        // top = 0: the row index is the offset from the panel's top edge
        app.lyric_hits = crate::app::LyricHits { area: inner, top: 0, rows };
        return;
    }

    // Wrap into rows, remembering which source line each row belongs to. Spacing rows
    // belong to the line above them but hold no text.
    // A chord row (`chord_x` = its left offset inside the panel) sits right above each
    // wrapped text row that carries chords; whether it is *drawn* depends on the active
    // line, but the row is always reserved so the layout doesn't jump on every line change.
    struct Row {
        li: usize,
        text: String,
        chord_x: Option<usize>,
        /// A chord row of the pre-lyrics intro section.
        intro: bool,
        /// `[ Instrumental / Solo ]` placeholder; only drawn while it is active.
        marker: Option<Mk>,
        /// A chord row played during this break; drawn only while the break is active.
        gap: Option<Mk>,
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Mk {
        Lead,
        After(usize),
        Tail,
    }
    let width = inner.width as usize;
    let mut rows: Vec<Row> = Vec::new();
    let threshold = app.args.interlude_secs;
    let marker_row = |li: usize, m: Mk| Row {
        li,
        text: "[ Instrumental / Solo ]".into(),
        chord_x: None,
        intro: false,
        marker: Some(m),
        gap: None,
    };
    // Time before the first lyric: an instrumental marker, then the intro chords.
    if !plain && stamps.first().is_some_and(|t| *t > threshold) {
        rows.push(marker_row(0, Mk::Lead));
    }
    // Intro chords (before the first lyric timestamp) come first.
    if let Some(a) = &aligned {
        for names in &a.intro {
            let names: Vec<String> = names.iter().map(|n| chords::transpose(n, tr, flats)).collect();
            let text = names.join("   ");
            let x = width.saturating_sub(crate::util::width(&text)) / 2;
            rows.push(Row { li: 0, text, chord_x: Some(x), intro: true, marker: None, gap: None });
        }
        if !a.intro.is_empty() {
            for _ in 0..gap.max(1) {
                rows.push(Row { li: 0, text: String::new(), chord_x: None, intro: true, marker: None, gap: None });
            }
        }
    }
    let intro_rows = rows.len();
    // chord rows for the break after lyric line `li`, shown while that break is active
    let push_gap = |rows: &mut Vec<Row>, li: usize, m: Mk| {
        let Some(a) = &aligned else { return };
        for names in a.gaps.get(li).into_iter().flatten() {
            let names: Vec<String> = names.iter().map(|n| chords::transpose(n, tr, flats)).collect();
            let text = names.join("   ");
            let x = width.saturating_sub(crate::util::width(&text)) / 2;
            rows.push(Row { li, text, chord_x: Some(x), intro: false, marker: None, gap: Some(m) });
        }
    };
    let mut starts = Vec::with_capacity(lines.len());
    let mut ends = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        starts.push(rows.len());
        let line_chords = aligned
            .as_ref()
            .and_then(|a| a.per_line.get(i))
            .filter(|c| !c.is_empty());
        match line_chords {
            None => {
                for r in wrap(l, width) {
                    rows.push(Row { li: i, text: r, chord_x: None, intro: false, marker: None, gap: None });
                }
            }
            Some(cs) => {
                let norm = l.split_whitespace().collect::<Vec<_>>().join(" ");
                let chars: Vec<char> = norm.chars().collect();
                let ranges = wrap_ranges(&norm, width);
                let mut per_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); ranges.len()];
                for (c, name) in cs {
                    let r = ranges.iter().rposition(|(s, _)| *s <= *c).unwrap_or(0);
                    per_row[r].push((c.saturating_sub(ranges[r].0), name.clone()));
                }
                for ((rs, re), here) in ranges.iter().zip(per_row) {
                    let text: String = chars[*rs..*re].iter().collect();
                    if !here.is_empty() {
                        let x = width.saturating_sub(crate::util::width(&text)) / 2;
                        rows.push(Row {
                            li: i,
                            text: chord_line(&here, &chars[*rs..*re], width.saturating_sub(x), tr, flats),
                            chord_x: Some(x),
                            intro: false,
                            marker: None,
                            gap: None,
                        });
                    }
                    rows.push(Row { li: i, text, chord_x: None, intro: false, marker: None, gap: None });
                }
            }
        }
        ends.push(rows.len());
        // A long silence after this line (solo / instrumental break).
        if !plain && l.trim() != "♪" {
            let next = stamps.get(i + 1).copied();
            match next {
                Some(n) if n - stamps[i] > threshold => {
                    rows.push(marker_row(i, Mk::After(i)));
                    push_gap(&mut rows, i, Mk::After(i));
                }
                None if length > 0.0 && length - stamps[i] > threshold + est_sung(l) => {
                    rows.push(marker_row(i, Mk::Tail));
                    push_gap(&mut rows, i, Mk::Tail);
                }
                _ => {}
            }
        }
        for _ in 0..gap {
            rows.push(Row { li: i, text: String::new(), chord_x: None, intro: false, marker: None, gap: None });
        }
    }
    if rows.is_empty() {
        return centered_msg(f, inner, &[("♪", dim)]);
    }

    // Scroll target: centre of the active line's text (synced) or proportional (plain).
    let target = if plain {
        if length > 0.0 {
            (pos / length).clamp(0.0, 1.0) * rows.len() as f64
        } else {
            0.0
        }
    } else {
        match current {
            Some(c) => (starts[c] + ends[c]) as f64 / 2.0 - 0.5,
            // before the first line: centre the intro chords if there are any
            None if intro_rows > 0 => intro_rows as f64 / 2.0 - 0.5,
            None => -1.0,
        }
    };

    // Critically-damped-ish easing toward the target; snap on big jumps.
    let now = std::time::Instant::now();
    let dt = now.duration_since(app.last_anim).as_secs_f64().min(0.1);
    app.last_anim = now;
    // browsing moves the view, not the playback: the target stays relative to the playing line
    app.browse = app.browse.clamp(-target - 1.0, rows.len() as f64 - target);
    let target = target + app.browse;
    app.lyric_scroll_target = target;
    if app.snap_scroll || (target - app.lyric_scroll).abs() > inner.height as f64 {
        app.lyric_scroll = target;
        app.snap_scroll = false;
    } else {
        let k = 1.0 - (-dt * 10.0).exp();
        app.lyric_scroll += (target - app.lyric_scroll) * k;
        if (target - app.lyric_scroll).abs() < 0.02 {
            app.lyric_scroll = target;
        }
    }

    let half = inner.height as i64 / 2;
    let top = app.lyric_scroll.round() as i64 - half;
    // Remember which lyric line each visible row belongs to, so a click can seek to it.
    // (Spacing rows, markers and the intro belong to no line; plain lyrics aren't clickable.)
    app.lyric_hits = crate::app::LyricHits {
        area: inner,
        top,
        rows: rows
            .iter()
            .map(|r| {
                (!plain && !r.intro && r.marker.is_none() && r.gap.is_none() && !r.text.is_empty()).then_some(r.li)
            })
            .collect(),
    };
    if plain {
        f.render_widget(
            Paragraph::new(Line::styled(
                "no synced lyrics — plain text",
                Style::new()
                    .fg(th.accent_fade(0.4))
                    .add_modifier(Modifier::ITALIC),
            ))
            .alignment(Alignment::Center),
            Rect {
                y: inner.y.saturating_sub(1),
                height: 1,
                ..inner
            },
        );
    }
    for y in 0..inner.height {
        let idx = top + y as i64;
        if idx < 0 || idx as usize >= rows.len() {
            continue;
        }
        let Row { li, text, chord_x, intro, marker, gap } = &rows[idx as usize];
        if text.is_empty() {
            continue;
        }
        // (is the break happening now, seconds left in it)
        let break_state = |m: Mk| -> (bool, f64) {
            let t = pos + app.offset;
            let last = stamps.len().saturating_sub(1);
            match m {
                Mk::Lead => (current.is_none(), stamps[0] - t),
                Mk::After(i) => (
                    current == Some(i) && t >= stamps[i] + est_sung(&lines[i]),
                    stamps[i] + (stamps[i + 1] - stamps[i]) - t,
                ),
                Mk::Tail => (
                    current == Some(last) && t >= stamps[last] + est_sung(&lines[last]),
                    0.0,
                ),
            }
        };
        if let (Some(m), Some(x)) = (gap, chord_x) {
            // play-along chords of a solo / instrumental break, only while it lasts
            if !break_state(*m).0 {
                continue;
            }
            let w = crate::util::width(text.as_str()) as u16;
            let xs = (inner.x + *x as u16).min(inner.right().saturating_sub(w)).max(inner.x);
            f.render_widget(
                Paragraph::new(Line::styled(
                    text.as_str(),
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
                )),
                Rect { x: xs, y: inner.y + y, width: w.min(inner.right() - xs), height: 1 },
            );
            continue;
        }
        if let Some(m) = marker {
            // Shown only while the break is actually happening, with a countdown.
            let (active, remain) = break_state(*m);
            if !active {
                continue;
            }
            let label = if remain > 0.5 && *m != Mk::Tail {
                format!("{text}  · {}s", remain.ceil() as u32)
            } else {
                text.clone()
            };
            f.render_widget(
                Paragraph::new(Line::styled(
                    label,
                    Style::new().fg(th.primary()).add_modifier(Modifier::ITALIC),
                ))
                .alignment(Alignment::Center),
                Rect { y: inner.y + y, height: 1, ..inner },
            );
            continue;
        }
        if let Some(x) = chord_x {
            let cur_i = current.map_or(-1, |c| c as i64);
            let d = *li as i64 - cur_i;
            // Default context (99) = every line; a smaller number limits chords to the
            // active line and the next N.
            let ctx = app.args.chords_context as i64;
            if !*intro && ctx < 99 && (d < 0 || d > ctx) {
                continue;
            }
            let active = if *intro { current.is_none() } else { d == 0 && current.is_some() };
            let style = if active {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else if d < 0 && !*intro {
                Style::new().fg(th.accent_fade(0.55))
            } else {
                Style::new().fg(th.accent_fade(0.35))
            };
            let w = crate::util::width(text.as_str()) as u16;
            let mut xs = inner.x + *x as u16;
            if xs + w > inner.right() {
                xs = inner.right().saturating_sub(w).max(inner.x);
            }
            f.render_widget(
                Paragraph::new(Line::styled(text.as_str(), style)),
                Rect {
                    x: xs,
                    y: inner.y + y,
                    width: w.min(inner.right() - xs),
                    height: 1,
                },
            );
            continue;
        }
        let style = match current {
            Some(c) if *li == c => Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            Some(c) => fade_style(&th, li.abs_diff(c), *li < c),
            None if plain => Style::new().fg(th.fade(0.35)),
            None => Style::new().fg(th.fade(0.55)),
        };
        // `[Jimin]` tag before the first text row of the active line and its neighbours
        let tag = current
            .filter(|c| li.abs_diff(*c) <= 2)
            .and_then(|_| app.members.get(*li))
            .filter(|m| !m.is_empty() && (starts[*li]..idx as usize).all(|j| rows[j].chord_x.is_some()))
            .map(|m| member_tag(m, width, crate::util::width(text.as_str())))
            .filter(|t| !t.is_empty());
        if let Some(tag) = tag {
            let mut spans: Vec<Span> = tag;
            spans.push(Span::styled(text.as_str(), style));
            f.render_widget(
                Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
                Rect { y: inner.y + y, height: 1, ..inner },
            );
            continue;
        }
        f.render_widget(
            Paragraph::new(Line::styled(text.as_str(), style)).alignment(Alignment::Center),
            Rect {
                y: inner.y + y,
                height: 1,
                ..inner
            },
        );
    }
}

// ── cava ────────────────────────────────────────────────────────────────
fn draw_cava(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(app, "cava");
    let inner = block.inner(area).inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 0,
    });
    f.render_widget(block, area);
    let th = app.theme.clone();

    if app.cava_bars.is_empty() {
        let msg = app.cava_status.clone().unwrap_or_else(|| "…".into());
        // Long error messages wrap instead of being cut off at the panel edge.
        let style = Style::new().fg(th.fade(0.5));
        let rows = wrap(&msg, inner.width as usize);
        let msgs: Vec<(&str, Style)> = rows.iter().map(|r| (r.as_str(), style)).collect();
        return centered_msg(f, inner, &msgs);
    }
    if inner.width < 2 || inner.height == 0 {
        return;
    }

    // One column per bar with a 1-column gap; resample cava's bars to fit.
    let n_disp = (inner.width as usize)
        .div_ceil(2)
        .clamp(1, app.cava_bars.len());
    let src = &app.cava_bars;
    let used_w = n_disp * 2 - 1;
    let x0 = inner.x + (inner.width as usize - used_w) as u16 / 2;
    let h8 = inner.height as f64 * 8.0;
    const EIGHTHS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

    let buf: &mut Buffer = f.buffer_mut();
    for i in 0..n_disp {
        let a = i * src.len() / n_disp;
        let b = ((i + 1) * src.len() / n_disp).max(a + 1);
        let v = src[a..b].iter().map(|&x| x as f64).sum::<f64>() / (b - a) as f64;
        let mut level = ((v / crate::cava::MAX as f64) * h8).round() as i64;
        for row in 0..inner.height {
            let y = inner.bottom() - 1 - row;
            let cell_lvl = level.clamp(0, 8) as usize;
            level -= 8;
            if cell_lvl == 0 {
                continue;
            }
            // gradient: muted primary at the bottom → full primary at the top
            let t = 0.55 * (1.0 - row as f32 / inner.height.max(1) as f32);
            let fg: Color = th.accent_fade(t);
            if let Some(c) = buf.cell_mut((x0 + (i * 2) as u16, y)) {
                c.set_symbol(EIGHTHS[cell_lvl]).set_fg(fg);
            }
        }
    }
}

// ── footer ──────────────────────────────────────────────────────────────
fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let dim = Style::new().fg(th.fade(0.45));
    let source = if app.args.cover_theme {
        format!("{} + cover accent", th.source)
    } else {
        th.source.clone()
    };
    // "terminal colors (no usable colour file)" → "terminal colors" when hidden
    let source = match source.find(" (") {
        Some(i) if app.args.hide_color_warning && source.starts_with("terminal colors") => source[..i].to_string(),
        _ => source,
    };
    let chords_info = if app.chords_on {
        use crate::app::ChordsState as CS;
        let t = match (&app.chords_state, &app.aligned) {
            (CS::Idle, _) => "chords: waiting for a track".to_string(),
            (CS::Fetching, _) => "chords: looking…".to_string(),
            (CS::Missing, _) => "chords: no sheet found for this track".to_string(),
            (CS::Failed(e), _) => format!("chords: {e}"),
            (CS::Ready, None) => "chords: found, but there are no synced lyrics to place them on".into(),
            (CS::Ready, Some(a)) if !a.usable() => {
                format!("chords: weak match ({}%) — hidden", a.percent())
            }
            (CS::Ready, Some(a)) => {
                let (src, capo) = app.sheet_info().unwrap_or(("", 0));
                let mut t = format!("chords: {src} {}%", a.percent());
                if let Some(tu) = app.tuning() {
                    t.push_str(&format!(" · {tu}"));
                }
                if capo > 0 {
                    t.push_str(&format!(" · capo {capo}"));
                }
                if app.transpose != 0 {
                    let n = if app.transpose > 6 { app.transpose - 12 } else { app.transpose };
                    t.push_str(&format!(" · {n:+}"));
                }
                t
            }
        };
        Some(t)
    } else {
        None
    };
    let mut spans = vec![
        Span::styled(" ◆ ", Style::new().fg(th.primary())),
        Span::styled(source, dim),
        Span::styled("  │  ", Style::new().fg(th.fade(0.7))),
        Span::styled(
            format!("offset {:+.1}s", app.offset),
            if app.offset != 0.0 {
                Style::new().fg(th.primary())
            } else {
                dim
            },
        ),
    ];
    if let Some(c) = chords_info {
        spans.push(Span::styled("  │  ", Style::new().fg(th.fade(0.7))));
        spans.push(Span::styled(c, Style::new().fg(th.accent_fade(0.2))));
    }
    if app.remote_active() {
        if let Some((r, _)) = &app.remote {
            spans.push(Span::styled("  │  ", Style::new().fg(th.fade(0.7))));
            spans.push(Span::styled(format!("on {}", r.device), Style::new().fg(th.accent_fade(0.2))));
        }
    }
    let left = Line::from(spans);
    let note = app.note_text().map(|n| format!("{n} "));
    let long = if app.spot.is_some() {
        "space ⏯  n/p  ←/→ seek  +/- vol  o library  f like  g chords  ? help  q quit "
    } else {
        "space ⏯  n/p  ←/→ seek  +/- vol  g chords  M min  D diagrams  ? help  q quit "
    };
    let short = "? help  q quit ";
    let left_w = left.width() as u16;
    f.render_widget(Paragraph::new(left), area);
    if let Some(n) = note {
        let n: String = n.chars().take(area.width.saturating_sub(left_w + 2) as usize).collect();
        f.render_widget(
            Paragraph::new(Line::styled(n, Style::new().fg(th.primary()))).alignment(Alignment::Right),
            area,
        );
        return;
    }
    for hints in [long, short] {
        if left_w + hints.chars().count() as u16 + 2 <= area.width {
            f.render_widget(
                Paragraph::new(Line::styled(hints, dim)).alignment(Alignment::Right),
                area,
            );
            break;
        }
    }
}
