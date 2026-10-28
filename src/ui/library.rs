use super::*;

/// `d`: where Spotify can play (this PC marked), the active one with a ▶.
pub(super) fn draw_devices(f: &mut Frame, app: &mut App) {
    use ratatui::widgets::Clear;
    let area = f.area();
    let th = app.theme.clone();
    let Some(d) = &mut app.devices else { return };
    let w = area.width.min(60);
    let h = area.height.min((d.items.len().max(1) as u16 + 5).clamp(8, 20));
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    d.area = r;
    d.rows.clear();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(
            " devices ",
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    if inner.height < 3 {
        return;
    }
    let dim = Style::new().fg(th.fade(0.45));
    let width = inner.width as usize;
    let list = Rect { height: inner.height - 1, ..inner };
    let view_h = list.height as usize;
    if d.sel < d.top {
        d.top = d.sel;
    } else if d.sel >= d.top + view_h.max(1) {
        d.top = d.sel + 1 - view_h.max(1);
    }
    let lines: Vec<Line> = if d.loading {
        vec![Line::styled("Loading…", dim)]
    } else if let Some(e) = &d.error {
        vec![Line::styled(e.clone(), dim)]
    } else if d.items.is_empty() {
        vec![
            Line::styled("No Spotify devices found.", dim),
            Line::styled("Open Spotify on a device, or start spotifyd.", dim),
        ]
    } else {
        d.items
            .iter()
            .enumerate()
            .skip(d.top)
            .take(view_h)
            .map(|(i, dev)| {
                let sel = i == d.sel;
                let style = if sel {
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
                } else if dev.restricted {
                    dim
                } else {
                    Style::new().fg(th.text())
                };
                let mut tags = vec![dev.kind.to_lowercase()];
                if dev.local {
                    tags.push("this PC".into());
                }
                if dev.restricted {
                    tags.push("restricted".into());
                }
                let tags = tags.join(" · ");
                let name_room = width.saturating_sub(crate::util::width(&tags).min(width / 2) + 6);
                let mut name = dev.name.clone();
                while crate::util::width(&name) > name_room && name.pop().is_some() {}
                let tags: String = tags.chars().take(width.saturating_sub(crate::util::width(&name) + 6)).collect();
                let pad = width.saturating_sub(crate::util::width(&name) + crate::util::width(&tags) + 4);
                Line::from(vec![
                    Span::styled(if sel { "▸ " } else { "  " }, style),
                    Span::styled(if dev.active { "▶ " } else { "  " }, Style::new().fg(th.primary())),
                    Span::styled(name, style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(tags, if sel { style } else { dim }),
                ])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), list);
    if !d.loading && d.error.is_none() {
        for (n, i) in (d.top..d.items.len()).take(view_h).enumerate() {
            d.rows.push((Rect { x: list.x, y: list.y + n as u16, width: list.width, height: 1 }, i));
        }
    }
    f.render_widget(
        Paragraph::new(Line::styled(
            crate::browser::fit_items(&["↑/↓ select", "enter play here", "r reload", "esc"], width),
            dim,
        ))
        .alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

/// `o` on a terminal too narrow for the panel: a box over the middle of the screen.
pub(super) fn draw_library(f: &mut Frame, app: &mut App) {
    use ratatui::widgets::Clear;
    let area = f.area();
    let w = area.width.min(72);
    let h = area.height.min(28);
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    draw_library_box(f, app, r);
    // the overlay covers (part of) the button: what is hidden can't be clicked
    if r.intersects(app.lib.hits.button) {
        app.lib.hits.button = Rect::default();
    }
}

/// The toggle button at the top left: shortened, never cut, and gone when nothing fits.
pub(super) fn draw_library_button(f: &mut Frame, app: &mut App, strip: Rect) {
    let th = app.theme.clone();
    let label = [("[ library ]", 11u16), ("[ lib ]", 7), ("[≡]", 3)]
        .into_iter()
        .find(|(_, w)| *w <= strip.width);
    let Some((text, w)) = label else { return };
    let style = if app.lib.open {
        Style::new().fg(th.surface()).bg(th.primary()).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(th.primary())
    };
    let r = Rect { width: w, height: 1, ..strip };
    f.render_widget(Paragraph::new(Span::styled(text, style)), r);
    app.lib.hits.button = r;
}

/// The browser itself in `r` (overlay or side panel): tabs, the list scrolled to the selection,
/// and a hint / note line. Records what can be clicked in `app.lib.hits`.
pub(super) fn draw_library_box(f: &mut Frame, app: &mut App, r: Rect) {
    use crate::app::TABS;
    let th = app.theme.clone();
    app.lib.hits.area = r;
    app.lib.hits.tabs.clear();
    app.lib.hits.rows.clear();
    app.lib.hits.field = Rect::default();
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

    // tabs: full names, or short ones when they don't fit (never a name cut in the middle)
    let shown = app.lib.visible_tabs();
    let total = |names: &[&str; 7], sep: usize| {
        shown.iter().map(|i| crate::util::width(names[*i])).sum::<usize>() + sep * (shown.len() - 1)
    };
    let names: [&str; 7] = if total(&TABS, 3) <= width {
        TABS
    } else {
        crate::browser::TABS_SHORT
    };
    let sep = if total(&names, 3) <= width { 3 } else { 1 };
    let mut tabs: Vec<Span> = Vec::new();
    let mut x = 0usize;
    for (n, i) in shown.iter().enumerate() {
        if n > 0 {
            tabs.push(Span::styled(if sep == 3 { " · " } else { " " }, dim));
            x += sep;
        }
        let style = if *i == app.lib.tab { sel_style.add_modifier(Modifier::UNDERLINED) } else { Style::new().fg(th.text()) };
        let tw = crate::util::width(names[*i]);
        // a name that doesn't fit whole is neither drawn nor clickable
        if x + tw <= width {
            app.lib.hits.tabs.push((Rect { x: inner.x + x as u16, y: inner.y, width: tw as u16, height: 1 }, *i));
            tabs.push(Span::styled(names[*i], style));
        }
        x += tw;
    }
    f.render_widget(Paragraph::new(Line::from(tabs)), Rect { height: 1, ..inner });

    // where we are: tab › album…, the playlist picker, or the search field
    let searching = app.lib.tab == crate::browser::TAB_SEARCH && app.lib.stack.is_empty() && app.lib.picker.is_none();
    let crumbs = match &app.lib.picker {
        Some((_, _, l)) => l.title.clone(),
        None => std::iter::once(TABS[app.lib.tab].to_string())
            .chain(app.lib.stack.iter().map(|l| l.title.clone()))
            .collect::<Vec<_>>()
            .join(" › "),
    };
    // cached rows stay on screen while the list is fetched again in the background
    let summary = match app.lib.level() {
        l if l.loading && !l.items.is_empty() => "refreshing…".to_string(),
        l => l.summary.clone(),
    };
    if searching {
        app.lib.hits.field = Rect { y: inner.y + 1, height: 1, ..inner };
    }
    // in a narrow box the field matters more than the result count
    let summary = if searching && width < 36 { String::new() } else { summary };
    let room = width.saturating_sub(crate::util::width(&summary) + 2);
    let (crumbs, crumb_style) = if searching {
        let shown = if app.lib.editing {
            format!("Search: {}", app.lib.input.shown())
        } else if app.lib.query.is_empty() {
            "Search: (press / to type)".to_string()
        } else {
            format!("Search: {}", app.lib.query)
        };
        let style = if app.lib.editing { Style::new().fg(th.text()).add_modifier(Modifier::BOLD) } else { dim };
        (shown, style)
    } else {
        (crumbs, Style::new().fg(th.accent_fade(0.2)))
    };
    let crumbs: String = crumbs.chars().take(room).collect();
    let pad = width.saturating_sub(crate::util::width(&crumbs) + crate::util::width(&summary));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(crumbs, crumb_style),
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
    let rows = l.scroll(view_h);
    // a new search keeps the old results (dimmed) until the new ones arrive
    let stale = l.search && l.loading && !l.items.is_empty();
    let empty = if l.loading && l.items.is_empty() {
        Some("Loading…".to_string())
    } else if let (Some(e), true) = (&l.error, l.items.is_empty()) {
        Some(e.clone())
    } else if l.items.is_empty() {
        Some(if l.search { "Type a search and press Enter" } else { "Nothing here" }.to_string())
    } else {
        None
    };
    // what can be clicked is exactly the item rows drawn now
    let mut row_hits: Vec<(Rect, usize)> = Vec::new();
    if empty.is_none() {
        for (n, row) in rows.iter().skip(l.top).take(view_h).enumerate() {
            if let crate::browser::ListRow::Item(i) = row {
                row_hits.push((Rect { x: list.x, y: list.y + n as u16, width: list.width, height: 1 }, *i));
            }
        }
    }
    let lines: Vec<Line> = match empty {
        Some(msg) => vec![Line::styled(msg, dim)],
        None => rows
            .iter()
            .skip(l.top)
            .take(view_h)
            .map(|row| {
                let i = match row {
                    crate::browser::ListRow::Head(h) => {
                        return Line::styled(
                            format!("── {h} "),
                            Style::new().fg(th.accent_fade(0.2)).add_modifier(Modifier::BOLD),
                        );
                    }
                    crate::browser::ListRow::Item(i) => *i,
                };
                let it = &l.items[i];
                let sel = i == l.sel;
                let style = if stale {
                    dim
                } else if sel {
                    sel_style
                } else {
                    Style::new().fg(th.text())
                };
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
    app.lib.hits.rows = row_hits;

    let hint = match (&app.lib.note, &app.lib.picker, app.lib.editing) {
        (_, _, true) => fit_hints(&[(1, "type"), (1, "enter search"), (1, "esc back")], width),
        (Some(n), _, _) => ellipsize(n, width),
        (None, Some(_), _) => fit_hints(&[(1, "enter add"), (1, "esc back")], width),
        // lowest number = kept longest
        (None, None, _) => {
            let tabs = if app.lib.queue_tab { "tab/1-7" } else { "tab/1-6" };
            let mut items: Vec<(u8, &str)> = vec![
                (1, "enter play"),
                (3, tabs),
                (2, "→/← open/back"),
                (3, "/ search"),
                (4, "a queue"),
                (4, "A add"),
                (5, "f like"),
                (4, "d devices"),
            ];
            if app.lib.queue_tab {
                items.push((5, "Q queue tab"));
            }
            items.extend([(5, "r reload"), (1, "esc")]);
            fit_hints(&items, width)
        }
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, dim)).alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}
