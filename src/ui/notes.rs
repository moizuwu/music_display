use super::*;
use crate::app::AnnState;

/// The annotations panel: the note for the line being sung, with the passage it is about and
/// its source named. Short excerpts only.
pub(super) fn draw_annotations(f: &mut Frame, app: &mut App, area: Rect) {
    let th = app.theme.clone();
    let mode = match app.args.annotations_mode {
        crate::cli::AnnotationMode::Genius => "Genius",
        crate::cli::AnnotationMode::Ai => crate::annotations::AI_LABEL,
    };
    let title = format!("annotations · {mode}");
    let block = panel(app, &title);
    let inner = block.inner(area).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    f.render_widget(block, area);
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
    let Some(note) = app.note_for(i) else {
        return msg(f, "no annotation for this line");
    };
    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    if !note.fragment.is_empty() {
        for l in wrap(&format!("“{}”", note.fragment.split_whitespace().collect::<Vec<_>>().join(" ")), width) {
            lines.push(Line::styled(l, Style::new().fg(th.primary()).add_modifier(Modifier::ITALIC)));
        }
        lines.push(Line::raw(""));
    }
    for l in wrap(&note.text, width) {
        lines.push(Line::styled(l, Style::new().fg(th.text())));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(format!("— {}", note.source), dim));
    f.render_widget(Paragraph::new(lines), inner);
}
