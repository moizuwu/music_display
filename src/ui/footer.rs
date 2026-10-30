use super::*;

// ── footer ──────────────────────────────────────────────────────────────
pub(super) fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
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
            (CS::Ready, Some(a)) if !a.usable_at(app.args.chords_match) => {
                format!("chords: weak match ({}%) — hidden", a.percent())
            }
            (CS::Ready, Some(a)) => {
                let src = app.sheet_info().map_or("", |i| i.0);
                let capo = app.effective_capo();
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
    // the first key of each action, as configured
    use crate::app::Action as A;
    let k = |a: A| app.keys.first(a);
    let basics = format!(
        "{} ⏯  {}/{}  {}/{} seek  {}/{} vol",
        k(A::PlayPause), k(A::Next), k(A::Prev), k(A::SeekBack), k(A::SeekFwd), k(A::VolUp), k(A::VolDown)
    );
    let tail = format!("{} help  {} quit ", k(A::Help), k(A::Quit));
    let long = if app.spot.is_some() {
        format!("{basics}  {} library  {} like  {} chords  {tail}", k(A::Library), k(A::Like), k(A::ToggleChords))
    } else {
        format!(
            "{basics}  {} chords  {} min  {} diagrams  {tail}",
            k(A::ToggleChords), k(A::Minimal), k(A::Diagrams)
        )
    };
    let short = tail;
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
