use super::*;
use crate::app::AnnState;
use crate::cli::{AnnotationMode, AnnotationScroll};

/// The annotations panel: the notes for the line being sung (genius+ai: the Genius note, then
/// the AI extra under a rule), with the passage it is about and its source named. A note that
/// does not fit scrolls (wheel, `e` / `E`, or by itself with `annotation_scroll = "auto"`).
pub(super) fn draw_annotations(f: &mut Frame, app: &mut App, area: Rect) {
    let th = app.theme.clone();
    let mode = app.args.annotations_mode;
    // the source is named in the body too; the title only carries it when there is room
    let title = if area.width >= 30 && app.args.panel_sources { format!("annotations · {}", mode.label()) } else { "annotations".to_string() };
    let block = panel(app, &title);
    let inner = block.inner(area).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, area);
    app.hits.notes = area;
    let dim = Style::new().fg(th.fade(0.5));
    let msg = |f: &mut Frame, t: &str| {
        let wrapped = wrap(t, inner.width.max(1) as usize);
        let rows: Vec<(&str, Style)> = wrapped.iter().map(|l| (l.as_str(), dim)).collect();
        centered_msg(f, inner, &rows)
    };
    match &app.ann_state {
        AnnState::Idle => {
            let why = match &app.lyrics {
                LyricsState::Ready(Lyrics::Synced(_)) | LyricsState::Ready(Lyrics::Plain(_)) => "waiting…",
                _ => "needs lyrics",
            };
            return msg(f, why);
        }
        AnnState::Loading => return msg(f, "looking up annotations…"),
        AnnState::Failed(e) => return msg(f, e),
        AnnState::Ready => {}
    }
    let Some(i) = app.active_line() else {
        return msg(f, "annotations follow the synced lyrics");
    };
    if app.ann_scroll_line != Some(i) {
        app.ann_scroll_line = Some(i);
        app.ann_scroll = 0;
    }
    let notes = app.notes_for(i);
    // genius+ai: how each half is doing, as dim lines around the notes
    let (mut head, mut foot) = (Vec::new(), Vec::new());
    if mode == AnnotationMode::GeniusAi {
        if let Some(e) = &app.genius_err {
            head.push(format!("Genius: {e}"));
        }
        match &app.ai_state {
            AnnState::Loading => foot.push("AI extras: looking up…".to_string()),
            AnnState::Failed(e) => foot.push(format!("AI extras: {e}")),
            _ => {}
        }
    }
    if notes.is_empty() && head.is_empty() && foot.is_empty() {
        return msg(f, "no annotation for this line");
    }
    let width = inner.width.max(1) as usize;
    let max = app.args.annotation_max_chars;
    let mut lines: Vec<Line> = Vec::new();
    for h in &head {
        lines.extend(wrap(h, width).into_iter().map(|l| Line::styled(l, dim)));
        lines.push(Line::raw(""));
    }
    if notes.is_empty() {
        lines.push(Line::styled("no annotation for this line", dim));
    }
    for (k, note) in notes.iter().enumerate() {
        if k > 0 {
            let label = " AI extra ";
            let side = width.saturating_sub(label.chars().count()) / 2;
            lines.push(Line::raw(""));
            lines.push(Line::styled(format!("{}{label}{}", "─".repeat(side), "─".repeat(side)), dim));
            lines.push(Line::raw(""));
        }
        if !note.fragment.is_empty() {
            for l in wrap(&format!("“{}”", note.fragment.split_whitespace().collect::<Vec<_>>().join(" ")), width) {
                lines.push(Line::styled(l, Style::new().fg(th.primary()).add_modifier(Modifier::ITALIC)));
            }
            lines.push(Line::raw(""));
        }
        for l in wrap(&crate::annotations::shown(&note.text, max), width) {
            lines.push(Line::styled(l, Style::new().fg(th.text())));
        }
        lines.push(Line::raw(""));
        lines.extend(wrap(&format!("— {}", note.source), width).into_iter().map(|l| Line::styled(l, dim)));
    }
    for t in &foot {
        lines.push(Line::raw(""));
        lines.extend(wrap(t, width).into_iter().map(|l| Line::styled(l, dim)));
    }
    let over = (lines.len() as u16).saturating_sub(inner.height);
    app.ann_scroll = match app.args.annotation_scroll {
        AnnotationScroll::Manual => app.ann_scroll.min(over),
        AnnotationScroll::Auto => (line_progress(app, i) * over as f64).round() as u16,
    };
    f.render_widget(Paragraph::new(lines).scroll((app.ann_scroll, 0)), inner);
    // say there is more, on the bottom border
    if over > 0 && area.height >= 2 {
        let hint = match (app.ann_scroll > 0, app.ann_scroll < over) {
            (true, true) => " ↑↓ more ",
            (false, _) => " ↓ more ",
            (true, false) => " ↑ ",
        };
        let w = hint.chars().count() as u16;
        if area.width > w + 2 {
            let r = Rect::new(area.x + area.width - w - 2, area.y + area.height - 1, w, 1);
            f.render_widget(Paragraph::new(Line::styled(hint, dim)), r);
        }
    }
}

/// How far the song is through synced line `i` (0 to 1), with a short hold at both ends so
/// the start and the end of a note can be read.
fn line_progress(app: &App, i: usize) -> f64 {
    let LyricsState::Ready(Lyrics::Synced(l)) = &app.lyrics else { return 0.0 };
    let Some((start, _)) = l.get(i) else { return 0.0 };
    let end = l.get(i + 1).map_or(start + 8.0, |(t, _)| *t);
    let p = (app.position() + app.offset - start) / (end - start).max(0.1);
    ((p - 0.15) / 0.7).clamp(0.0, 1.0)
}
