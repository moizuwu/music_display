use super::*;

pub(super) fn draw_settings(f: &mut Frame, app: &mut App) {
    use crate::app::{Setting, SettingsRow};
    use ratatui::widgets::Clear;
    let area = f.area();
    let th = app.theme.clone();
    let layout = Setting::rows();
    let w = area.width.min(54);
    let h = area.height.min(layout.len() as u16 + 5);
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
        .title_top(
            Line::from(Span::styled(format!(" v{} ", env!("MD_VERSION")), Style::new().fg(th.fade(0.55))))
                .right_aligned(),
        )
        .style(Style::new().bg(th.surface()).fg(th.text()));
    let inner = block.inner(r).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, r);
    let view_h = inner.height.saturating_sub(1) as usize;
    // keep the selected row in view on short terminals (scrolling counts drawn rows,
    // headers and the status line included)
    let sel = layout
        .iter()
        .position(|r| *r == SettingsRow::Item(app.settings_sel))
        .unwrap_or(0);
    if sel < app.settings_top {
        app.settings_top = sel;
        // show the group header above the first row of a group
        if sel > 0 && matches!(layout[sel - 1], SettingsRow::Header(_)) {
            app.settings_top = sel - 1;
        }
    } else if sel >= app.settings_top + view_h.max(1) {
        app.settings_top = sel + 1 - view_h.max(1);
    }
    let first = app.settings_top;
    let width = inner.width as usize;
    let lines: Vec<Line> = layout
        .iter()
        .skip(first)
        .take(view_h)
        .map(|row| match row {
            SettingsRow::Header(h) => Line::styled(
                format!("── {h} "),
                Style::new().fg(th.accent_fade(0.2)).add_modifier(Modifier::BOLD),
            ),
            SettingsRow::Status => {
                let text: String = format!("  status: {}", app.spotify_status()).chars().take(width).collect();
                Line::styled(text, Style::new().fg(th.fade(0.45)))
            }
            SettingsRow::Item(i) => {
                let s = Setting::ALL[*i];
                let sel = *i == app.settings_sel;
                let style = if sel {
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(th.text())
                };
                let mut label = s.label().to_string();
                if s.needs_restart() {
                    label.push_str(" *");
                }
                let value = app.setting_value(s);
                // on a narrow terminal shorten the label so the value stays visible
                let room = width.saturating_sub(crate::util::width(&value) + 4);
                while crate::util::width(&label) > room && label.pop().is_some() {}
                let pad = width.saturating_sub(crate::util::width(&label) + crate::util::width(&value) + 2);
                Line::from(vec![
                    Span::styled(if sel { "▸ " } else { "  " }, style),
                    Span::styled(label, style),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(value, style),
                ])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { height: view_h as u16, ..inner });
    if inner.height == 0 {
        return;
    }
    let hint = match (&app.settings_note, &app.settings_edit) {
        (Some(n), _) => n.chars().take(inner.width as usize).collect(),
        (None, Some(_)) => crate::browser::fit_items(&["type", "enter save", "esc cancel"], width),
        (None, None) => crate::browser::fit_items(
            &["↑/↓ select", "space/←/→ change", "enter edit / run", "* restart", "esc"],
            width,
        ),
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
