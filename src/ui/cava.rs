use super::*;

// ── cava ────────────────────────────────────────────────────────────────
pub(super) fn draw_cava(f: &mut Frame, app: &mut App, area: Rect) {
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
