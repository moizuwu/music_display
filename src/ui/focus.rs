use super::*;

/// Big-font rows for `text` in a panel `width` × `height`: the largest scale
/// (down to one) whose wrapped pieces (up to 4) leave at least 2 rows for the neighbouring lines;
/// failing that the compact 3×5 font. `None`: the caller falls back to bold wrapped text.
/// `spare` leaves that many more rows free (the member tag above the stage).
#[cfg(test)]
pub fn big_stage(text: &str, width: usize, height: u16, scale: usize, spare: u16) -> Option<Vec<String>> {
    big_stage_map(text, width, height, scale, spare).map(|r| r.rows)
}

/// [`big_stage`] with the column of every source character (for chords above the glyphs).
/// A line with Hangul tries the finer 12×12 font first at each scale and falls back to the
/// 8×8 one, so a narrow or short panel keeps working exactly as before.
pub fn big_stage_map(text: &str, width: usize, height: u16, scale: usize, spare: u16) -> Option<bigfont::Rendered> {
    use bigfont::Font;
    let fits = |r: &bigfont::Rendered| r.rows.len() as u16 + 2 + spare <= height;
    let hangul = bigfont::has_hangul(text);
    (1..=scale.clamp(1, 3))
        .rev()
        .find_map(|sc| {
            let fine = hangul.then(|| bigfont::render_map(text, width, 4, sc, Font::Fine).filter(fits));
            fine.flatten().or_else(|| bigfont::render_map(text, width, 4, sc, Font::Normal).filter(fits))
        })
        .or_else(|| bigfont::render_map(text, width, 4, 1, Font::Compact).filter(fits))
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
    /// Members singing the active line (`--member-parts`): a dim tag row above the stage.
    pub members: &'a [String],
    /// This song has member data: keep one row free above the stage for the tag on every
    /// line, so the layout doesn't jump between tagged and untagged lines.
    pub reserve_tag: bool,
    /// The tag goes under the stage instead of above it (`member_position = below`).
    pub tag_below: bool,
    /// Karaoke: how much of the active line is sung (0..=1); `None` = plain bold line.
    pub progress: Option<f64>,
}

/// "Focus" layout used by the big-font mode: the active line sits on a fixed stage in the
/// middle of the panel (big font if it fits, otherwise bold wrapped text), with the lines
/// before and after stacked outward from it, each with its chord row when it has chords.
/// Returns which lyric line each panel row belongs to (for click-to-seek).
#[cfg(test)]
pub fn draw_focus(f: &mut Frame, inner: Rect, th: &Theme, fc: &Focus) -> Vec<Option<usize>> {
    draw_focus_spans(f, inner, th, fc).0
}

/// [`draw_focus`] plus the columns `[x0, x1)` each panel row's text covers.
pub(super) fn draw_focus_spans(f: &mut Frame, inner: Rect, th: &Theme, fc: &Focus) -> (Vec<Option<usize>>, Vec<(u16, u16)>) {
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
    let spans = std::cell::RefCell::new(vec![(inner.x, inner.right()); inner.height as usize]);
    let tag_rows = (fc.reserve_tag && c.is_some()) as u16;
    // (the one reserved row is above the stage, or below it with `tag_below`)
    let (rows_above, rows_below) = if fc.tag_below { (0, tag_rows) } else { (tag_rows, 0) };
    type Stage = (Vec<String>, Style, Option<usize>, Option<bigfont::Rendered>);
    let (stage, stage_style, stage_li, map): Stage = match c {
        Some(c) => match big_stage_map(&lines[c], width, inner.height, fc.scale, tag_rows) {
            Some(r) => (r.rows.clone(), Style::new().fg(th.primary()), Some(c), Some(r)),
            None => (wrap(&lines[c], width), bold, Some(c), None),
        },
        None => (
            fc.chords.map_or_else(Vec::new, |a| a.intro.iter().map(|n| names(n)).collect()),
            bold,
            None,
            None,
        ),
    };
    let sh = (stage.len() as u16).min(inner.height);
    // the spare row for the tag sits above the stage, so the stage moves down by half of it
    let top = inner.y + (inner.height - sh + rows_above).saturating_sub(rows_below) / 2;

    let put = |f: &mut Frame, y: i32, li: Option<usize>, text: &str, style: Style| {
        if y >= inner.y as i32 && y < inner.bottom() as i32 && !text.is_empty() {
            let w = (crate::util::width(text) as u16).min(inner.width);
            let x0 = inner.x + (inner.width - w) / 2;
            hits.borrow_mut()[(y - inner.y as i32) as usize] = li;
            spans.borrow_mut()[(y - inner.y as i32) as usize] = (x0, x0 + w);
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
            spans.borrow_mut()[(y - inner.y as i32) as usize] = (xs, xs + w);
            f.render_widget(
                Paragraph::new(Line::styled(text.to_string(), style)),
                Rect { x: xs, y: y as u16, width: w, height: 1 },
            );
        }
    };

    // karaoke: split every stage row where the sung part ends
    let lit_rows: Vec<usize> = match (fc.progress, c) {
        (Some(fr), Some(c)) => match &map {
            // big font: cut at the column of the first glyph that is not lit yet
            Some(r) => {
                let boundary = lit_boundary(&lines[c], (fr * non_space(&lines[c]) as f64).round() as usize);
                (0..stage.len())
                    .map(|i| {
                        let k = r.chunks.iter().rposition(|ch| ch.row <= i).unwrap_or(0);
                        let ch = &r.chunks[k];
                        ch.cols.iter().find(|(s, _)| *s >= boundary).map_or(ch.width, |(_, col)| *col)
                    })
                    .collect()
            }
            None => spread_lit(&stage.iter().map(|s| non_space(s)).collect::<Vec<_>>(), fr),
        },
        _ => Vec::new(),
    };
    let kar_dim = Style::new().fg(th.fade(0.55)).add_modifier(Modifier::BOLD);
    for (i, row) in stage.iter().take(sh as usize).enumerate() {
        match lit_rows.get(i) {
            Some(k) if !row.is_empty() => {
                let y = top as i32 + i as i32;
                let (a, b) = if map.is_some() { split_cols(row, *k) } else { split_lit(row, *k) };
                let w = (crate::util::width(row) as u16).min(inner.width);
                let x0 = inner.x + (inner.width - w) / 2;
                if y >= inner.y as i32 && y < inner.bottom() as i32 {
                    hits.borrow_mut()[(y - inner.y as i32) as usize] = stage_li;
                    spans.borrow_mut()[(y - inner.y as i32) as usize] = (x0, x0 + w);
                    f.render_widget(
                        Paragraph::new(Line::from(lit_spans(a, b, stage_style, kar_dim))).alignment(Alignment::Center),
                        Rect { y: y as u16, height: 1, ..inner },
                    );
                }
            }
            _ => put(f, top as i32 + i as i32, stage_li, row, stage_style),
        }
    }
    // the active line's chords: each above its own glyph when the big font draws it (one row
    // above every wrapped piece, in the blank row between pieces), else a summary above
    if let Some(c) = c {
        let line_chords = fc.chords.and_then(|a| a.per_line.get(c)).filter(|cs| !cs.is_empty());
        let mut has_summary = false;
        if let (Some(cs), Some(r)) = (line_chords, &map) {
            let mut per_piece: Vec<Vec<(usize, String)>> = vec![Vec::new(); r.chunks.len()];
            for (idx, name) in cs {
                if let Some((k, col)) = r.column_of(*idx) {
                    per_piece[k].push((col, chords::transpose(name, fc.tr, fc.flats)));
                }
            }
            for (k, items) in per_piece.iter().enumerate().filter(|(_, i)| !i.is_empty()) {
                let piece = &r.chunks[k];
                let x = width.saturating_sub(piece.width) / 2;
                let y = top as i32 + piece.row as i32 - 1;
                put_at(f, y, x, &place_chords(items, width - x), bold);
                if y >= inner.y as i32 && y < inner.bottom() as i32 {
                    hits.borrow_mut()[(y - inner.y as i32) as usize] = Some(c);
                }
                has_summary |= k == 0;
            }
        } else if let Some(cs) = line_chords {
            let mut v: Vec<String> = cs.iter().map(|(_, n)| chords::transpose(n, fc.tr, fc.flats)).collect();
            v.dedup();
            put(f, top as i32 - 1, Some(c), &v.join("   "), bold);
            has_summary = true;
        }
        // who sings it: one dim row right above the big text (or above the chord summary), or
        // right under the stage
        let y = if fc.tag_below { top as i32 + sh as i32 } else { top as i32 - 1 - has_summary as i32 };
        let tag = member_tag_line(fc.members, width);
        if tag_rows > 0 && !tag.is_empty() && y >= inner.y as i32 {
            f.render_widget(
                Paragraph::new(Line::from(tag)).alignment(Alignment::Center),
                Rect { y: y as u16, height: 1, ..inner },
            );
        }
    }
    // chords played during an instrumental break, right under the stage
    let mut after = top as i32 + sh as i32 + 1 + rows_below as i32;
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

    // Earlier lines, nearest first, growing upward (above the tag row when there is one).
    let mut y = top as i32 - 2 - rows_above as i32;
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
    (hits.into_inner(), spans.into_inner())
}

/// `[Jimin] ` / `[Jimin, V] ` in each member's colour, dimmed; empty if the tag plus the
/// text (`text_w` columns) wouldn't fit in `width`.
pub(super) fn member_tag(members: &[String], width: usize, text_w: usize) -> Vec<Span<'static>> {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut spans = vec![Span::styled("[", dim)];
    for (i, m) in members.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(", ", dim));
        }
        spans.push(Span::styled(m.clone(), Style::new().fg(member_color(m)).add_modifier(Modifier::DIM)));
    }
    spans.push(Span::styled("] ", dim));
    let w: usize = spans.iter().map(|s| s.width()).sum();
    if w + text_w > width { Vec::new() } else { spans }
}

/// The tag for the big layout, one line that always fits `width`: `[Jimin, V]`, else
/// `[Jimin +1]`, else the first name shortened with `…`; empty when even that doesn't fit.
pub(super) fn member_tag_line(members: &[String], width: usize) -> Vec<Span<'static>> {
    let Some(first) = members.first() else { return Vec::new() };
    let dim = Style::new().add_modifier(Modifier::DIM);
    let name = |m: &str, text: String| Span::styled(text, Style::new().fg(member_color(m)).add_modifier(Modifier::DIM));
    let fits = |s: &[Span]| s.iter().map(|s| s.width()).sum::<usize>() <= width;
    let mut full = vec![Span::styled("[", dim)];
    for (i, m) in members.iter().enumerate() {
        if i > 0 {
            full.push(Span::styled(", ", dim));
        }
        full.push(name(m, m.clone()));
    }
    full.push(Span::styled("]", dim));
    if fits(&full) {
        return full;
    }
    if members.len() > 1 {
        let more = vec![
            Span::styled("[", dim),
            name(first, first.clone()),
            Span::styled(format!(" +{}]", members.len() - 1), dim),
        ];
        if fits(&more) {
            return more;
        }
    }
    let room = width.saturating_sub(2);
    if room < 2 {
        return Vec::new();
    }
    vec![Span::styled("[", dim), name(first, ellipsize(first, room)), Span::styled("]", dim)]
}

/// A stable colour per member name, from the terminal's own ANSI palette.
pub fn member_color(name: &str) -> Color {
    const PALETTE: [Color; 8] = [
        Color::LightMagenta,
        Color::LightCyan,
        Color::LightYellow,
        Color::LightGreen,
        Color::LightBlue,
        Color::LightRed,
        Color::Magenta,
        Color::Green,
    ];
    PALETTE[(crate::util::fnv1a(&name.to_lowercase()) % PALETTE.len() as u64) as usize]
}
