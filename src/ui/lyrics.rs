use super::*;

pub(super) fn draw_lyrics(f: &mut Frame, app: &mut App, area: Rect) {
    let shown =app.translation().is_some_and(|t| app.lyric_lines().is_some_and(|l| l.len() == t.len()));
    if app.browse_expired() {
        app.browse = 0.0;
    }
    let title = if app.browse != 0.0 {
        "lyrics · browsing — End to follow".to_string()
    } else if !app.translate_on {
        "lyrics".to_string()
    } else {
        use crate::app::TranslateState as TS;
        match (&app.translate_state, app.translation()) {
            (_, Some(_)) if shown && app.translate_backend.starts_with("AI ") => {
                format!("lyrics · translated by {} ({})", app.args.ai_model, app.args.translate_lang)
            }
            (_, Some(_)) if shown && app.translate_backend.starts_with(crate::app::FALLBACK_TAG) => format!(
                "lyrics · translated ({}, {})",
                app.args.translate_lang,
                &app.translate_backend[crate::app::FALLBACK_TAG.len()..]
            ),
            (_, Some(_)) if shown && app.translate_backend != "google" => {
                format!("lyrics · translated ({}, {})", app.args.translate_lang, app.translate_backend)
            }
            (_, Some(_)) if shown && app.translate_both => {
                format!("lyrics · original + translated ({})", app.args.translate_lang)
            }
            (_, Some(_)) if shown => format!("lyrics · translated ({})", app.args.translate_lang),
            (TS::Failed(e), _) => format!("lyrics · {e}"),
            _ => "lyrics · translating…".to_string(),
        }
    };
    let block = panel(app, &title);
    let inner = block.inner(area).inner(ratatui::layout::Margin {
        horizontal: 2 + app.args.lyric_pad,
        vertical: 1,
    });
    f.render_widget(block, area);
    let th = app.theme.clone();
    let dim = Style::new().fg(th.fade(0.5));

    if app.snap.is_none() {
        centered_msg(f, inner, &[("♪", dim)]);
        return;
    }
    let pos = app.position();
    let length = app.snap.as_ref().map(|s| s.length).unwrap_or(0.0);
    let (lines, current): (Vec<String>, Option<usize>) = match &app.lyrics {
        LyricsState::Fetching => return centered_msg(f, inner, &[("fetching lyrics…", dim)]),
        LyricsState::Ready(Lyrics::NotFound) => {
            return centered_msg(
                f,
                inner,
                &[
                    ("♪", Style::new().fg(th.primary())),
                    ("", dim),
                    ("no synced lyrics found", dim),
                ],
            );
        }
        LyricsState::Ready(Lyrics::Instrumental) => {
            return centered_msg(
                f,
                inner,
                &[("♪  instrumental  ♪", Style::new().fg(th.primary()))],
            );
        }
        LyricsState::Ready(Lyrics::Error(e)) => {
            return centered_msg(f, inner, &[("♪", dim), (e.as_str(), dim)]);
        }
        LyricsState::Ready(Lyrics::Synced(l)) => {
            let t = pos + app.offset;
            let cur = l.partition_point(|(ts, _)| *ts <= t).checked_sub(1);
            (l.iter().map(|(_, s)| s.clone()).collect(), cur)
        }
        LyricsState::Ready(Lyrics::Plain(l)) => (l.clone(), None),
    };
    let plain = matches!(app.lyrics, LyricsState::Ready(Lyrics::Plain(_)));
    // Translated text replaces the original line by line; chord columns, which point at
    // the original words, are carried over to the same relative place in the translation.
    let original = lines.clone();
    let mut lines = lines;
    // `k`: keep the original and put the translation under it. The fixed big-font layout has
    // no room for a second line, so there the translation replaces the original as with `K`.
    let big_focus = app.big_lyrics && !plain && app.browse == 0.0;
    let both = app.translate_both && !big_focus;
    let subs: Vec<String> = match app.translation() {
        Some(t) if both && t.len() == lines.len() => t.clone(),
        _ => Vec::new(),
    };
    let translated = match app.translation() {
        Some(_) if both => false,
        Some(t) if t.len() == lines.len() => {
            for (l, t) in lines.iter_mut().zip(t) {
                if !t.trim().is_empty() {
                    *l = t.clone();
                }
            }
            true
        }
        _ => false,
    };
    let stamps: Vec<f64> = match &app.lyrics {
        LyricsState::Ready(Lyrics::Synced(l)) => l.iter().map(|x| x.0).collect(),
        _ => Vec::new(),
    };
    // karaoke: how much of the active line is sung (guessed from the line times)
    let kar_progress = current
        .filter(|c| app.karaoke_live() && lines[*c].trim() != "♪")
        .map(|c| line_progress(pos + app.offset, stamps[c], stamps.get(c + 1).copied(), &lines[c]));

    // Blank rows between lines — dropped on short panels where they'd waste the space.
    let gap = if inner.height >= 10 {
        app.args.lyric_spacing as usize
    } else {
        0
    };

    let aligned: Option<chords::Aligned> = (app.chords_on && !plain)
        .then(|| app.aligned.clone())
        .flatten()
        .filter(|a| a.usable_at(app.args.chords_match))
        .map(|mut a| {
            if translated {
                for (i, cs) in a.per_line.iter_mut().enumerate() {
                    if let (Some(o), Some(t)) = (original.get(i), lines.get(i)) {
                        *cs = remap_chords(cs, o, t);
                    }
                }
            }
            a
        });
    let tr = app.chord_shift();
    let flats = app.use_flats();

    // Big-font mode uses the fixed "focus" layout instead of the scrolling one — for the
    // active line, or before the first line when there are intro chords to show.
    let intro_chords = current.is_none() && aligned.as_ref().is_some_and(|a| !a.intro.is_empty());
    if app.big_lyrics && !plain && app.browse == 0.0 && (current.is_some() || intro_chords) {
        app.lyric_scroll_target = app.lyric_scroll; // nothing left to animate
        // in a long silence after the line (same rule as the scrolling layout's markers)
        let t = pos + app.offset;
        let in_break = current.is_some_and(|c| {
            let sung = stamps.get(c).copied().unwrap_or(f64::MAX) + est_sung(&lines[c]);
            lines[c].trim() != "♪"
                && t >= sung
                && match stamps.get(c + 1) {
                    Some(n) => n - stamps[c] > app.args.interlude_secs,
                    None => length > 0.0 && length - stamps[c] > app.args.interlude_secs + est_sung(&lines[c]),
                }
        });
        let focus = Focus {
            lines: &lines,
            current,
            gap,
            scale: app.args.big_scale as usize,
            chords: aligned.as_ref(),
            tr,
            flats,
            context: app.args.chords_context as usize,
            in_break,
            members: current.and_then(|c| app.members.get(c)).map_or(&[][..], |m| m.as_slice()),
            reserve_tag: app.members.iter().any(|m| !m.is_empty()),
            progress: kar_progress,
        };
        let (rows, spans) = draw_focus_spans(f, inner, &th, &focus);
        // top = 0: the row index is the offset from the panel's top edge
        app.lyric_hits = crate::app::LyricHits { area: inner, top: 0, rows, spans };
        return;
    }

    // Wrap into rows, remembering which source line each row belongs to. Spacing rows
    // belong to the line above them but hold no text.
    // A chord row (`chord_x` = its left offset inside the panel) sits right above each
    // wrapped text row that carries chords; whether it is *drawn* depends on the active
    // line, but the row is always reserved so the layout doesn't jump on every line change.
    struct Row {
        li: usize,
        text: String,
        chord_x: Option<usize>,
        /// A chord row of the pre-lyrics intro section.
        intro: bool,
        /// `[ Instrumental / Solo ]` placeholder; only drawn while it is active.
        marker: Option<Mk>,
        /// A chord row played during this break; drawn only while the break is active.
        gap: Option<Mk>,
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Mk {
        Lead,
        After(usize),
        Tail,
    }
    let width = inner.width as usize;
    let mut rows: Vec<Row> = Vec::new();
    let threshold = app.args.interlude_secs;
    let marker_row = |li: usize, m: Mk| Row {
        li,
        text: "[ Instrumental / Solo ]".into(),
        chord_x: None,
        intro: false,
        marker: Some(m),
        gap: None,
    };
    // Time before the first lyric: an instrumental marker, then the intro chords.
    if !plain && stamps.first().is_some_and(|t| *t > threshold) {
        rows.push(marker_row(0, Mk::Lead));
    }
    // Intro chords (before the first lyric timestamp) come first.
    if let Some(a) = &aligned {
        for names in &a.intro {
            let names: Vec<String> = names.iter().map(|n| chords::transpose(n, tr, flats)).collect();
            let text = names.join("   ");
            let x = width.saturating_sub(crate::util::width(&text)) / 2;
            rows.push(Row { li: 0, text, chord_x: Some(x), intro: true, marker: None, gap: None });
        }
        if !a.intro.is_empty() {
            for _ in 0..gap.max(1) {
                rows.push(Row { li: 0, text: String::new(), chord_x: None, intro: true, marker: None, gap: None });
            }
        }
    }
    let intro_rows = rows.len();
    // chord rows for the break after lyric line `li`, shown while that break is active
    let push_gap = |rows: &mut Vec<Row>, li: usize, m: Mk| {
        let Some(a) = &aligned else { return };
        for names in a.gaps.get(li).into_iter().flatten() {
            let names: Vec<String> = names.iter().map(|n| chords::transpose(n, tr, flats)).collect();
            let text = names.join("   ");
            let x = width.saturating_sub(crate::util::width(&text)) / 2;
            rows.push(Row { li, text, chord_x: Some(x), intro: false, marker: None, gap: Some(m) });
        }
    };
    let mut sub_rows: std::collections::HashSet<usize> = Default::default();
    let mut starts = Vec::with_capacity(lines.len());
    let mut ends = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        starts.push(rows.len());
        let line_chords = aligned
            .as_ref()
            .and_then(|a| a.per_line.get(i))
            .filter(|c| !c.is_empty());
        match line_chords {
            None => {
                for r in wrap(l, width) {
                    rows.push(Row { li: i, text: r, chord_x: None, intro: false, marker: None, gap: None });
                }
            }
            Some(cs) => {
                let norm = l.split_whitespace().collect::<Vec<_>>().join(" ");
                let chars: Vec<char> = norm.chars().collect();
                let ranges = wrap_ranges(&norm, width);
                let mut per_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); ranges.len()];
                for (c, name) in cs {
                    let r = ranges.iter().rposition(|(s, _)| *s <= *c).unwrap_or(0);
                    per_row[r].push((c.saturating_sub(ranges[r].0), name.clone()));
                }
                for ((rs, re), here) in ranges.iter().zip(per_row) {
                    let text: String = chars[*rs..*re].iter().collect();
                    if !here.is_empty() {
                        let x = width.saturating_sub(crate::util::width(&text)) / 2;
                        rows.push(Row {
                            li: i,
                            text: chord_line(&here, &chars[*rs..*re], width.saturating_sub(x), tr, flats),
                            chord_x: Some(x),
                            intro: false,
                            marker: None,
                            gap: None,
                        });
                    }
                    rows.push(Row { li: i, text, chord_x: None, intro: false, marker: None, gap: None });
                }
            }
        }
        // `k`: the translation as extra rows of the same line (skipped when it adds nothing)
        let sub_from = rows.len();
        if let Some(t) = subs.get(i).filter(|t| !t.trim().is_empty() && t.trim() != l.trim()) {
            for r in wrap(t, width) {
                rows.push(Row { li: i, text: r, chord_x: None, intro: false, marker: None, gap: None });
            }
        }
        sub_rows.extend(sub_from..rows.len());
        ends.push(rows.len());
        // A long silence after this line (solo / instrumental break).
        if !plain && l.trim() != "♪" {
            let next = stamps.get(i + 1).copied();
            match next {
                Some(n) if n - stamps[i] > threshold => {
                    rows.push(marker_row(i, Mk::After(i)));
                    push_gap(&mut rows, i, Mk::After(i));
                }
                None if length > 0.0 && length - stamps[i] > threshold + est_sung(l) => {
                    rows.push(marker_row(i, Mk::Tail));
                    push_gap(&mut rows, i, Mk::Tail);
                }
                _ => {}
            }
        }
        for _ in 0..gap {
            rows.push(Row { li: i, text: String::new(), chord_x: None, intro: false, marker: None, gap: None });
        }
    }
    if rows.is_empty() {
        return centered_msg(f, inner, &[("♪", dim)]);
    }

    // Scroll target: centre of the active line's text (synced) or proportional (plain).
    let target = if plain {
        if length > 0.0 {
            (pos / length).clamp(0.0, 1.0) * rows.len() as f64
        } else {
            0.0
        }
    } else {
        match current {
            Some(c) => (starts[c] + ends[c]) as f64 / 2.0 - 0.5,
            // before the first line: centre the intro chords if there are any
            None if intro_rows > 0 => intro_rows as f64 / 2.0 - 0.5,
            None => -1.0,
        }
    };

    // Critically-damped-ish easing toward the target; snap on big jumps.
    let now = std::time::Instant::now();
    let dt = now.duration_since(app.last_anim).as_secs_f64().min(0.1);
    app.last_anim = now;
    // browsing moves the view, not the playback: the target stays relative to the playing line
    app.browse = app.browse.clamp(-target - 1.0, rows.len() as f64 - target);
    let target = target + app.browse;
    app.lyric_scroll_target = target;
    if app.snap_scroll || (target - app.lyric_scroll).abs() > inner.height as f64 {
        app.lyric_scroll = target;
        app.snap_scroll = false;
    } else {
        let k = 1.0 - (-dt * 10.0).exp();
        app.lyric_scroll += (target - app.lyric_scroll) * k;
        if (target - app.lyric_scroll).abs() < 0.02 {
            app.lyric_scroll = target;
        }
    }

    let half = inner.height as i64 / 2;
    let top = app.lyric_scroll.round() as i64 - half;
    // Remember which lyric line each visible row belongs to, so a click can seek to it.
    // (Spacing rows, markers and the intro belong to no line; plain lyrics aren't clickable.)
    app.lyric_hits = crate::app::LyricHits {
        area: inner,
        top,
        rows: rows
            .iter()
            .map(|r| {
                (!plain && !r.intro && r.marker.is_none() && r.gap.is_none() && !r.text.is_empty()).then_some(r.li)
            })
            .collect(),
        spans: rows
            .iter()
            .map(|r| match r.chord_x {
                Some(x) => {
                    let w = (crate::util::width(&r.text) as u16).min(inner.width);
                    let xs = (inner.x + x as u16).min(inner.right().saturating_sub(w)).max(inner.x);
                    (xs, xs + w)
                }
                None => {
                    let w = (crate::util::width(&r.text) as u16).min(inner.width);
                    let x0 = inner.x + (inner.width - w) / 2;
                    (x0, x0 + w)
                }
            })
            .collect(),
    };
    if plain {
        f.render_widget(
            Paragraph::new(Line::styled(
                "no synced lyrics — plain text",
                Style::new()
                    .fg(th.accent_fade(0.4))
                    .add_modifier(Modifier::ITALIC),
            ))
            .alignment(Alignment::Center),
            Rect {
                y: inner.y.saturating_sub(1),
                height: 1,
                ..inner
            },
        );
    }
    // karaoke: per text row of the active line, how many of its characters are lit
    let mut kar: std::collections::HashMap<usize, usize> = Default::default();
    if let (Some(fr), Some(c)) = (kar_progress, current) {
        let ids: Vec<usize> = (starts[c]..ends[c])
            .filter(|r| rows[*r].chord_x.is_none() && !sub_rows.contains(r))
            .collect();
        let counts: Vec<usize> = ids.iter().map(|r| non_space(&rows[*r].text)).collect();
        kar.extend(ids.into_iter().zip(spread_lit(&counts, fr)));
    }
    let kar_dim = Style::new().fg(th.fade(0.55)).add_modifier(Modifier::BOLD);
    for y in 0..inner.height {
        let idx = top + y as i64;
        if idx < 0 || idx as usize >= rows.len() {
            continue;
        }
        let Row { li, text, chord_x, intro, marker, gap } = &rows[idx as usize];
        if text.is_empty() {
            continue;
        }
        // (is the break happening now, seconds left in it)
        let break_state = |m: Mk| -> (bool, f64) {
            let t = pos + app.offset;
            let last = stamps.len().saturating_sub(1);
            match m {
                Mk::Lead => (current.is_none(), stamps[0] - t),
                Mk::After(i) => (
                    current == Some(i) && t >= stamps[i] + est_sung(&lines[i]),
                    stamps[i] + (stamps[i + 1] - stamps[i]) - t,
                ),
                Mk::Tail => (
                    current == Some(last) && t >= stamps[last] + est_sung(&lines[last]),
                    0.0,
                ),
            }
        };
        if let (Some(m), Some(x)) = (gap, chord_x) {
            // play-along chords of a solo / instrumental break, only while it lasts
            if !break_state(*m).0 {
                continue;
            }
            let w = crate::util::width(text.as_str()) as u16;
            let xs = (inner.x + *x as u16).min(inner.right().saturating_sub(w)).max(inner.x);
            f.render_widget(
                Paragraph::new(Line::styled(
                    text.as_str(),
                    Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
                )),
                Rect { x: xs, y: inner.y + y, width: w.min(inner.right() - xs), height: 1 },
            );
            continue;
        }
        if let Some(m) = marker {
            // Shown only while the break is actually happening, with a countdown.
            let (active, remain) = break_state(*m);
            if !active {
                continue;
            }
            let label = if remain > 0.5 && *m != Mk::Tail {
                format!("{text}  · {}s", remain.ceil() as u32)
            } else {
                text.clone()
            };
            f.render_widget(
                Paragraph::new(Line::styled(
                    label,
                    Style::new().fg(th.primary()).add_modifier(Modifier::ITALIC),
                ))
                .alignment(Alignment::Center),
                Rect { y: inner.y + y, height: 1, ..inner },
            );
            continue;
        }
        if let Some(x) = chord_x {
            let cur_i = current.map_or(-1, |c| c as i64);
            let d = *li as i64 - cur_i;
            // Default context (99) = every line; a smaller number limits chords to the
            // active line and the next N.
            let ctx = app.args.chords_context as i64;
            if !*intro && ctx < 99 && (d < 0 || d > ctx) {
                continue;
            }
            let active = if *intro { current.is_none() } else { d == 0 && current.is_some() };
            let style = if active {
                Style::new().fg(th.primary()).add_modifier(Modifier::BOLD)
            } else if d < 0 && !*intro {
                Style::new().fg(th.accent_fade(0.55))
            } else {
                Style::new().fg(th.accent_fade(0.35))
            };
            let w = crate::util::width(text.as_str()) as u16;
            let mut xs = inner.x + *x as u16;
            if xs + w > inner.right() {
                xs = inner.right().saturating_sub(w).max(inner.x);
            }
            f.render_widget(
                Paragraph::new(Line::styled(text.as_str(), style)),
                Rect {
                    x: xs,
                    y: inner.y + y,
                    width: w.min(inner.right() - xs),
                    height: 1,
                },
            );
            continue;
        }
        let style = match current {
            Some(c) if *li == c => Style::new().fg(th.primary()).add_modifier(Modifier::BOLD),
            Some(c) => fade_style(&th, li.abs_diff(c), *li < c),
            None if plain => Style::new().fg(th.fade(0.35)),
            None => Style::new().fg(th.fade(0.55)),
        };
        // the translation under an original line is italic and not bold
        let style = if sub_rows.contains(&(idx as usize)) {
            style.remove_modifier(Modifier::BOLD).add_modifier(Modifier::ITALIC)
        } else {
            style
        };
        // `[Jimin]` tag before the first text row of the active line and its neighbours
        let tag = current
            .filter(|c| li.abs_diff(*c) <= 2)
            .and_then(|_| app.members.get(*li))
            .filter(|m| !m.is_empty() && (starts[*li]..idx as usize).all(|j| rows[j].chord_x.is_some()))
            .map(|m| member_tag(m, width, crate::util::width(text.as_str())))
            .filter(|t| !t.is_empty());
        if let Some(tag) = tag {
            // the tag is part of the row: widen its clickable span to cover it
            let tag_w: u16 = tag.iter().map(|s| s.width() as u16).sum();
            let w = (crate::util::width(text.as_str()) as u16 + tag_w).min(inner.width);
            let x0 = inner.x + (inner.width - w) / 2;
            app.lyric_hits.spans[idx as usize] = (x0, x0 + w);
            let mut spans: Vec<Span> = tag;
            match kar.get(&(idx as usize)) {
                Some(k) => {
                    let (a, b) = split_lit(text.as_str(), *k);
                    spans.extend(lit_spans(a, b, style, kar_dim));
                }
                None => spans.push(Span::styled(text.as_str(), style)),
            }
            f.render_widget(
                Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
                Rect { y: inner.y + y, height: 1, ..inner },
            );
            continue;
        }
        let line = match kar.get(&(idx as usize)) {
            Some(k) => {
                let (a, b) = split_lit(text.as_str(), *k);
                Line::from(lit_spans(a, b, style, kar_dim))
            }
            None => Line::styled(text.as_str(), style),
        };
        f.render_widget(
            Paragraph::new(line).alignment(Alignment::Center),
            Rect {
                y: inner.y + y,
                height: 1,
                ..inner
            },
        );
    }
}
