use super::*;

/// Rough time a line takes to sing, so the marker appears once the words are over
/// rather than the moment the line starts.
pub(super) fn est_sung(text: &str) -> f64 {
    (text.chars().count() as f64 * 0.08).clamp(2.0, 6.0)
}

/// Share (0..=1) of the line starting at `start` that is sung at `t`. lrclib only has line
/// times, so this is a guess: the line takes its own length (`est_sung`, up to 1.6x that when
/// the next line is further away, so a break is not filled), and is lit a little before it ends.
pub fn line_progress(t: f64, start: f64, next: Option<f64>, text: &str) -> f64 {
    let est = est_sung(text);
    let dur = next.map_or(est, |n| (n - start).min(est * 1.6)).max(0.5);
    ((t - start) / dur * 1.1).clamp(0.0, 1.0)
}

pub(super) fn non_space(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// How many non-space characters of each wrapped row are lit when `f` of the whole line is.
pub(super) fn spread_lit(counts: &[usize], f: f64) -> Vec<usize> {
    let mut left = (f * counts.iter().sum::<usize>() as f64).round() as usize;
    counts
        .iter()
        .map(|c| {
            let k = left.min(*c);
            left -= k;
            k
        })
        .collect()
}

/// `row` cut right after its `lit`-th non-space character.
pub(super) fn split_lit(row: &str, lit: usize) -> (&str, &str) {
    if lit == 0 {
        return ("", row);
    }
    let mut seen = 0;
    for (i, c) in row.char_indices() {
        if !c.is_whitespace() {
            seen += 1;
            if seen == lit {
                return row.split_at(i + c.len_utf8());
            }
        }
    }
    (row, "")
}

#[cfg(test)]
pub fn split_lit_for_test(row: &str, lit: usize) -> (&str, &str) {
    split_lit(row, lit)
}

/// `row` cut after `cols` display columns.
pub(super) fn split_cols(row: &str, cols: usize) -> (&str, &str) {
    let mut w = 0;
    for (i, c) in row.char_indices() {
        if w >= cols {
            return row.split_at(i);
        }
        w += crate::util::width(c.encode_utf8(&mut [0; 4]));
    }
    (row, "")
}

/// Index (into `text.chars()`) of the first character not lit yet when `lit` non-space
/// characters are; `usize::MAX` when all are.
pub(super) fn lit_boundary(text: &str, lit: usize) -> usize {
    if lit == 0 {
        return 0;
    }
    let mut seen = 0;
    for (i, c) in text.chars().enumerate() {
        if !c.is_whitespace() {
            seen += 1;
            if seen == lit {
                return i + 1;
            }
        }
    }
    usize::MAX
}

/// The lit and the not yet lit part of a row, side by side.
pub(super) fn lit_spans<'a>(lit: &'a str, rest: &'a str, lit_style: Style, rest_style: Style) -> Vec<Span<'a>> {
    vec![Span::styled(lit, lit_style), Span::styled(rest, rest_style)]
}

/// Style for a non-active lyric line `dist` lines away from the active one:
/// fades with distance, and past lines fade a little faster than upcoming ones.
pub(super) fn fade_style(th: &Theme, dist: usize, past: bool) -> Style {
    let t = 0.40 + 0.08 * dist as f32 + if past { 0.10 } else { 0.0 };
    Style::new().fg(th.fade(t.min(0.78)))
}
