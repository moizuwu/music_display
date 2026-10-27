use super::*;
use crate::app::{CustItem, FieldMenu};
use ratatui::widgets::Clear;

/// The customize page of the settings menu: panels, metadata fields and control buttons, each
/// with a check box, in the order they are shown.
pub(super) fn draw_customize(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let th = app.theme.clone();
    let items = app.cust_items();
    // group headers between the three lists
    let mut rows: Vec<(Option<usize>, &'static str)> = Vec::new();
    let mut last = "";
    for (i, it) in items.iter().enumerate() {
        if it.group() != last {
            last = it.group();
            rows.push((None, last));
        }
        rows.push((Some(i), ""));
    }
    let w = area.width.min(58);
    let h = area.height.min(rows.len() as u16 + 4);
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(" customize layout ", Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    if inner.height < 2 {
        return;
    }
    let view_h = inner.height.saturating_sub(1) as usize;
    let sel_row = rows.iter().position(|(i, _)| *i == Some(app.cust_sel)).unwrap_or(0);
    let top = if rows.len() <= view_h { 0 } else { sel_row.saturating_sub(view_h / 2).min(rows.len() - view_h) };
    let width = inner.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .skip(top)
        .take(view_h)
        .map(|(i, head)| match i {
            None => Line::styled(
                format!("── {head} "),
                Style::new().fg(th.accent_fade(0.2)).add_modifier(Modifier::BOLD),
            ),
            Some(i) => {
                let it = items[*i];
                let sel = *i == app.cust_sel;
                let style = if sel {
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
                } else if app.cust_shown(it) {
                    Style::new().fg(th.text())
                } else {
                    Style::new().fg(th.fade(0.5))
                };
                let size = match it {
                    CustItem::Panel(p) => match app.cust_size(p) {
                        Some(n) => format!("{n}%"),
                        None => "auto".to_string(),
                    },
                    _ => String::new(),
                };
                let check = if app.cust_shown(it) { "[x]" } else { "[ ]" };
                let left = format!("{} {check} {}", if sel { "▸" } else { " " }, it.label());
                let pad = width.saturating_sub(crate::util::width(&left) + crate::util::width(&size));
                Line::from(vec![
                    Span::styled(left, style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(size, Style::new().fg(th.fade(0.4))),
                ])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { height: view_h as u16, ..inner });
    let hint = match &app.settings_note {
        Some(n) => n.chars().take(width).collect(),
        None => crate::browser::fit_items(
            &["↑/↓ select", "space show/hide", "u/d move", "+/- size", "a auto sizes", "r reset", "esc back"],
            width,
        ),
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::new().fg(th.fade(0.45)))).alignment(Alignment::Center),
        Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner },
    );
}

/// The little menu of a right-clicked metadata field. Its rows are recorded for the mouse.
pub(super) fn draw_field_menu(f: &mut Frame, app: &mut App) {
    let Some(menu) = app.field_menu else { return };
    let th = app.theme.clone();
    let area = f.area();
    let w = (FieldMenu::ITEMS.iter().map(|s| s.chars().count()).max().unwrap_or(0) + 4) as u16;
    let h = FieldMenu::ITEMS.len() as u16 + 2;
    let x = menu.at.0.min(area.right().saturating_sub(w));
    let y = menu.at.1.min(area.bottom().saturating_sub(h));
    let r = Rect { x, y, width: w.min(area.width), height: h.min(area.height) };
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.primary()))
        .title(Span::styled(format!(" {} ", menu.field.label()), Style::new().fg(th.primary())))
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r);
    f.render_widget(block, r);
    app.hits.menu_items.clear();
    for (i, label) in FieldMenu::ITEMS.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let row = Rect { x: inner.x, y: inner.y + i as u16, width: inner.width, height: 1 };
        let style = if i == menu.sel {
            Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(th.text())
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!("{} {label}", if i == menu.sel { "▸" } else { " " }), style)),
            row,
        );
        app.hits.menu_items.push(row);
    }
}
