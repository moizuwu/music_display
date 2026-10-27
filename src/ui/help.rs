use super::*;

/// The help overlay for the default key table.
#[cfg(test)]
pub fn help_blocks(th: &Theme) -> Vec<Vec<Line<'static>>> {
    help_blocks_with(th, &crate::keymap::Keymap::default())
}

/// Lines of the help overlay, generated from the key table the input handler uses (with the
/// user's `[keys]` overrides). Returns the groups as separate blocks so they can be laid out
/// in columns.
pub fn help_blocks_with(th: &Theme, keys: &crate::keymap::Keymap) -> Vec<Vec<Line<'static>>> {
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
            v.extend(BINDS.iter().filter(|b| b.group == *g).map(|b| row(&keys.label(b.action), b.desc)));
            v.extend(MOUSE.iter().filter(|m| m.0 == *g).map(|m| row(m.1, m.2)));
            if *g == "Spotify" {
                v.push(Line::styled("in the library", Style::new().fg(th.fade(0.45))));
                v.extend(LIBRARY_KEYS.iter().map(|(k, d)| row(k, d)));
            }
            v
        })
        .collect()
}

/// The key editor of the help overlay: every action with its keys, one row each.
pub(super) fn draw_help_edit(f: &mut Frame, app: &mut App) {
    use crate::app::BINDS;
    use ratatui::widgets::Clear;
    let area = f.area();
    let th = app.theme.clone();
    let ed = app.help_edit.clone().unwrap_or_default();
    let acts = app.keys.actions();
    let w = area.width.min(66);
    let h = area.height.min(acts.len() as u16 + 4);
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(" keys ", Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    let view_h = inner.height.saturating_sub(1) as usize;
    // keep the selected row in view
    let first = ed.sel.saturating_sub(view_h.saturating_sub(1)).min(acts.len().saturating_sub(view_h));
    let keys_w = 22usize;
    let lines: Vec<Line> = acts
        .iter()
        .enumerate()
        .skip(first)
        .take(view_h)
        .map(|(i, a)| {
            let desc = BINDS.iter().find(|b| b.action == *a).map_or("", |b| b.desc);
            let selected = i == ed.sel;
            let keys = if selected && ed.capture {
                "press a key…".to_string()
            } else {
                format!("{}{}", app.keys.label(*a), if app.keys.config_value(*a).is_some() { " *" } else { "" })
            };
            let room = (inner.width as usize).saturating_sub(keys_w + 1);
            let desc: String = desc.chars().take(room).collect();
            let text = format!("{desc:<room$} {keys:>keys_w$}");
            let style = if selected {
                Style::new().fg(th.surface()).bg(th.primary()).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(th.text())
            };
            Line::styled(text, style)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { height: view_h as u16, ..inner });
    let hint = match (&ed.note, ed.capture) {
        (_, true) => "press the new key · Esc cancels".to_string(),
        (Some(n), false) => n.clone(),
        _ => "↑/↓ pick · Enter set a key · Backspace reset · Esc back  (* = changed)".to_string(),
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.55)))).alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

pub(super) fn draw_help(f: &mut Frame, app: &mut App) {
    use ratatui::widgets::Clear;
    if app.help_edit.is_some() {
        return draw_help_edit(f, app);
    }
    let area = f.area();
    let th = app.theme.clone();
    let blocks = help_blocks_with(&th, &app.keys);
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
        "↑/↓ scroll · e edit keys · any other key closes"
    } else {
        "e edit keys · any other key closes"
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.45)))).alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}
