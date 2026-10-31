use super::*;

// ── lyrics ──────────────────────────────────────────────────────────────
/// Chord columns of `orig` (char columns in its single-spaced text) moved to the same
/// relative place in `trans`, snapped to the start of the nearest word.
pub fn remap_chords(cs: &[(usize, String)], orig: &str, trans: &str) -> Vec<(usize, String)> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let (o, t) = (norm(orig), norm(trans));
    let (ol, tl) = (o.chars().count().max(1), t.chars().count());
    let starts: Vec<usize> = chords::words_pos(&t).into_iter().map(|(s, _)| s).collect();
    let mut out: Vec<(usize, String)> = Vec::new();
    for (c, name) in cs {
        let want = (*c as f64 / ol as f64 * tl as f64).round() as usize;
        let col = starts.iter().copied().min_by_key(|s| s.abs_diff(want)).unwrap_or(0);
        out.push((col, name.clone()));
    }
    out.sort_by_key(|c| c.0);
    out
}

/// Closing punctuation (CJK and Latin) that must not start a wrapped line.
pub(super) fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。' | '、' | '！' | '？' | '；' | '：' | '）' | '」' | '』' | '】' | '》' | '〉' | '”' | '’'
            | '…' | 'ー' | ',' | '.' | '!' | '?' | ';' | ':' | ')' | ']' | '}'
    )
}

/// Greedy word-wrap by display width.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for word in text.split_whitespace() {
        let ww = crate::util::width(word);
        if cur_w > 0 && cur_w + 1 + ww > width {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if ww > width {
            // hard-break very long tokens
            for ch in word.chars() {
                let cw = crate::util::cw(ch);
                if cur_w > 0 && cur_w + cw > width {
                    // closing punctuation never starts a line: carry the last char over
                    let carry = if no_line_start(ch) && cur.chars().count() > 1 { cur.pop() } else { None };
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                    if let Some(p) = carry {
                        cur.push(p);
                        cur_w = crate::util::cw(p);
                    }
                }
                cur.push(ch);
                cur_w += cw;
            }
            continue;
        }
        if cur_w > 0 {
            cur.push(' ');
            cur_w += 1;
        }
        cur.push_str(word);
        cur_w += ww;
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

/// Same wrapping as [`wrap`] but returns `(start, end)` char ranges into `norm` (the
/// single-spaced text), so chord columns can be mapped onto the wrapped rows.
pub fn wrap_ranges(norm: &str, width: usize) -> Vec<(usize, usize)> {
    if width == 0 {
        return vec![];
    }
    let chars: Vec<char> = norm.chars().collect();
    let cw = crate::util::cw;
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let (mut start, mut end, mut cur_w) = (0usize, 0usize, 0usize);
    for (s, len) in chords::words_pos(norm) {
        let ww: usize = chars[s..s + len].iter().map(|c| cw(*c)).sum();
        if cur_w > 0 && cur_w + 1 + ww > width {
            rows.push((start, end));
            cur_w = 0;
        }
        if ww > width {
            for (k, c) in chars[s..s + len].iter().enumerate() {
                if cur_w + cw(*c) > width && cur_w > 0 {
                    if no_line_start(*c) && end > start + 1 {
                        rows.push((start, end - 1));
                        start = end - 1;
                        cur_w = cw(chars[end - 1]);
                    } else {
                        rows.push((start, end));
                        cur_w = 0;
                    }
                }
                if cur_w == 0 {
                    start = s + k;
                }
                end = s + k + 1;
                cur_w += cw(*c);
            }
            continue;
        }
        if cur_w == 0 {
            start = s;
        }
        end = s + len;
        cur_w += if cur_w > 0 { 1 + ww } else { ww };
    }
    if cur_w > 0 || rows.is_empty() {
        rows.push((start, end));
    }
    rows
}

/// `Am   G   C` with each chord at its column; neighbours are pushed apart, never overlap.
pub fn chord_line(cs: &[(usize, String)], text: &[char], limit: usize, n: i32, flats: bool) -> String {
    // chord columns are char indices into `text`; place them by terminal cell width
    let cell = |col: usize| -> usize {
        text.iter().take(col).map(|c| crate::util::cw(*c)).sum::<usize>()
            + col.saturating_sub(text.len())
    };
    let items: Vec<(usize, String)> = cs.iter().map(|(col, name)| (cell(*col), chords::transpose(name, n, flats))).collect();
    place_chords(&items, limit)
}

/// Chord names (already transposed) at their cell columns, left to right: neighbours are
/// pushed apart so they never overlap, and a chord that would run past `limit` is pulled
/// left on its own, instead of the whole row shifting off its words.
pub fn place_chords(items: &[(usize, String)], limit: usize) -> String {
    let mut s = String::new();
    let mut w = 0usize;
    for (cell, name) in items {
        let floor = if w > 0 { w + 1 } else { 0 };
        let at = (*cell).min(limit.saturating_sub(crate::util::width(name))).max(floor);
        s.push_str(&" ".repeat(at - w));
        w = at + crate::util::width(name);
        s.push_str(name);
    }
    s
}

/// Wrapped rows of a lyric line, each with the chord row (left offset, text) that sits
/// right above it — the same mapping the scrolling layout uses. Lines without chords
/// just get plain wrapped rows.
pub(super) fn chord_rows(line: &str, cs: &[(usize, String)], width: usize, tr: i32, flats: bool) -> Vec<(String, Option<(usize, String)>)> {
    if cs.is_empty() {
        return wrap(line, width).into_iter().map(|t| (t, None)).collect();
    }
    let norm = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = norm.chars().collect();
    let ranges = wrap_ranges(&norm, width);
    if ranges.is_empty() {
        // no room at all: the line, without chords
        return vec![(norm, None)];
    }
    let mut per_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); ranges.len()];
    for (c, name) in cs {
        let r = ranges.iter().rposition(|(s, _)| *s <= *c).unwrap_or(0);
        per_row[r].push((c.saturating_sub(ranges[r].0), name.clone()));
    }
    ranges
        .iter()
        .zip(per_row)
        .map(|((rs, re), here)| {
            let text: String = chars[*rs..*re].iter().collect();
            let chord = (!here.is_empty()).then(|| {
                let x = width.saturating_sub(crate::util::width(&text)) / 2;
                (x, chord_line(&here, &chars[*rs..*re], width.saturating_sub(x), tr, flats))
            });
            (text, chord)
        })
        .collect()
}
