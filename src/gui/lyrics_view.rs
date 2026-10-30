//! The lyrics panel. Lines are laid out one wrapped row at a time so a chord sits exactly above
//! the character it belongs to (placed by the text's own glyph positions), long silences get the
//! `[ Instrumental / Solo ]` countdown and the break's chords, and the view follows the playing
//! line until the user scrolls (then it waits, see `browse.rs`). The decisions that do not
//! need a window (which break is active, the size easing) are plain functions with tests.
use super::model::Model;
use super::style;
use crate::ui::{est_sung, non_space, remap_chords, spread_lit, split_lit};
use egui::text::{CCursor, LayoutJob};
use egui::{Align, Color32, FontId, RichText, Sense, TextFormat, Ui, Vec2};
use music_display::chords;
use music_display::cli::TranslateMode;
use music_display::guiconf::{Align as LAlign, Animation};
use music_display::lyrics::Lyrics;
use std::time::{Duration, Instant};

// ── breaks ──────────────────────────────────────────────────────────────────

/// A long silence: before the first line, after line `i`, or after the last line.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mk {
    Lead,
    After(usize),
    Tail,
}

/// Is there a break marker before the first line?
pub fn has_lead(l: &[(f64, String)], threshold: f64) -> bool {
    l.first().is_some_and(|x| x.0 > threshold)
}

/// The break marker after line `i`, if the silence is longer than `threshold` seconds.
pub fn marker_after(l: &[(f64, String)], i: usize, length: f64, threshold: f64) -> Option<Mk> {
    if l[i].1.trim() == "♪" {
        return None;
    }
    match l.get(i + 1) {
        Some(n) if n.0 - l[i].0 > threshold => Some(Mk::After(i)),
        None if length > 0.0 && length - l[i].0 > threshold + est_sung(&l[i].1) => Some(Mk::Tail),
        _ => None,
    }
}

/// (is the break happening now, seconds left in it) at time `t`; `cur` is the active line.
pub fn break_state(m: Mk, t: f64, cur: Option<usize>, l: &[(f64, String)]) -> (bool, f64) {
    let last = l.len().saturating_sub(1);
    match m {
        Mk::Lead => (cur.is_none(), l.first().map_or(0.0, |x| x.0) - t),
        Mk::After(i) => (cur == Some(i) && t >= l[i].0 + est_sung(&l[i].1), l.get(i + 1).map_or(0.0, |n| n.0) - t),
        Mk::Tail => (cur == Some(last) && t >= l[last].0 + est_sung(&l[last].1), 0.0),
    }
}

/// The marker's text, with the seconds left while there is a known end.
pub fn marker_label(m: Mk, remain: f64) -> String {
    if remain > 0.5 && m != Mk::Tail {
        format!("[ Instrumental / Solo ]  · {}s", remain.ceil() as u32)
    } else {
        "[ Instrumental / Solo ]".into()
    }
}

/// Is the chord row of a line drawn? `chords_context` (99 = every line) limits it to the active
/// line and the next N; `d` is the line's distance after the active one (-1 before the start).
pub fn chords_in_context(ctx: u16, cur: Option<usize>, i: usize) -> bool {
    let d = i as i64 - cur.map_or(-1, |c| c as i64);
    !(ctx < 99 && (d < 0 || d > ctx as i64))
}

// ── size easing ─────────────────────────────────────────────────────────────

/// Eases a line between the plain and the active size without egui's per-id animation memory
/// (which would keep an entry for every line of every song played).
pub struct LineAnim {
    cur: Option<usize>,
    prev: Option<usize>,
    since: Instant,
}

const EASE: f32 = 0.18;

impl Default for LineAnim {
    fn default() -> Self {
        LineAnim { cur: None, prev: None, since: Instant::now() - Duration::from_secs(5) }
    }
}

impl LineAnim {
    pub fn update(&mut self, cur: Option<usize>) {
        if cur != self.cur {
            self.prev = self.cur;
            self.cur = cur;
            self.since = Instant::now();
        }
    }

    /// 0 (plain size) to 1 (active size) for line `i`.
    pub fn weight_at(&self, i: usize, smooth: bool, now: Instant) -> f32 {
        if !smooth {
            return (self.cur == Some(i)) as u8 as f32;
        }
        let t = (now.saturating_duration_since(self.since).as_secs_f32() / EASE).min(1.0);
        if self.cur == Some(i) {
            t
        } else if self.prev == Some(i) {
            1.0 - t
        } else {
            0.0
        }
    }

    pub fn animating(&self) -> bool {
        self.since.elapsed().as_secs_f32() < EASE + 0.05
    }
}

// ── one line, with its chords ───────────────────────────────────────────────

struct Spec<'a> {
    text: &'a str,
    /// (column in the single-spaced text, shifted chord name)
    chords: &'a [(usize, String)],
    /// Colour of the chords; `None` = the room is kept but nothing is drawn.
    chord_color: Option<Color32>,
    font: FontId,
    color: Color32,
    chord_font: FontId,
    /// Karaoke: share lit, and the colour of the lit part.
    lit: Option<(f64, Color32)>,
    center: bool,
}

/// Draw a lyric line, wrapped to the width, each chord above its own character.
fn draw_line(ui: &mut Ui, s: &Spec) -> egui::Response {
    let norm = s.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = norm.chars().collect();
    let avail = ui.available_width().max(40.0);
    let painter = ui.painter().clone();
    let wrapped = painter.layout(norm.clone(), s.font.clone(), s.color, avail);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut at = 0;
    for r in &wrapped.rows {
        let n = r.row.glyphs.len();
        ranges.push((at, (at + n).min(chars.len())));
        at += n;
    }
    if ranges.is_empty() {
        ranges.push((0, 0));
    }
    let row_text: Vec<String> = ranges.iter().map(|(a, b)| chars[*a..*b].iter().collect::<String>().trim_end().to_string()).collect();
    let lit: Vec<usize> = match s.lit {
        Some((f, _)) => spread_lit(&row_text.iter().map(|t| non_space(t)).collect::<Vec<_>>(), f),
        None => Vec::new(),
    };
    let band = s.chord_font.size * 1.3;
    let last = ranges.len() - 1;
    struct Row {
        g: std::sync::Arc<egui::Galley>,
        chords: Vec<(f32, std::sync::Arc<egui::Galley>)>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for (k, ((a, _), t)) in ranges.iter().zip(&row_text).enumerate() {
        let g = match s.lit {
            Some((_, lit_color)) => {
                let (x, y) = split_lit(t, lit[k]);
                let mut job = LayoutJob::default();
                job.append(x, 0.0, TextFormat { font_id: s.font.clone(), color: lit_color, ..Default::default() });
                job.append(y, 0.0, TextFormat { font_id: s.font.clone(), color: s.color, ..Default::default() });
                painter.layout_job(job)
            }
            None => painter.layout_no_wrap(t.clone(), s.font.clone(), s.color),
        };
        let b = ranges[k].1;
        let n = t.chars().count();
        let mut ch = Vec::new();
        let mut min_x = 0.0f32;
        for (c, name) in s.chords.iter().filter(|(c, _)| *c >= *a && (*c < b || k == last)) {
            let x = g.pos_from_cursor(CCursor::new((c - a).min(n))).min.x.max(min_x);
            let cg = painter.layout_no_wrap(name.clone(), s.chord_font.clone(), s.chord_color.unwrap_or(Color32::TRANSPARENT));
            min_x = x + cg.size().x + 6.0;
            ch.push((x, cg));
        }
        rows.push(Row { g, chords: ch });
    }
    let total: f32 = rows.iter().map(|r| r.g.size().y + if r.chords.is_empty() { 0.0 } else { band }).sum();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail, total), Sense::click());
    let mut y = rect.top();
    for r in &rows {
        let w = r.g.size().x;
        let x0 = if s.center { rect.left() + (avail - w).max(0.0) / 2.0 } else { rect.left() };
        if !r.chords.is_empty() {
            if s.chord_color.is_some() {
                for (cx, cg) in &r.chords {
                    painter.galley(egui::pos2(x0 + cx, y), cg.clone(), Color32::TRANSPARENT);
                }
            }
            y += band;
        }
        painter.galley(egui::pos2(x0, y), r.g.clone(), s.color);
        y += r.g.size().y;
    }
    resp
}

// ── the panel ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct Act {
    seek_to: Option<f64>,
}

pub fn lyrics(ui: &mut Ui, m: &mut Model) {
    m.browse.expire();
    let browsing = m.browse.active();
    let shown = m.translation().is_some_and(|t| m.lines().is_some_and(|l| l.len() == t.len()));
    let title = super::translation::title(&super::translation::TitleIn {
        browsing,
        mode: m.translate_mode,
        shown,
        state: &m.tr.state,
        backend: &m.translate_backend,
        ai_model: &m.args.ai_model,
        lang: &m.args.translate_lang,
    });
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("lyrics 0.1 s later").clicked() {
                m.nudge(0.1);
            }
            ui.label(format!("{:+.1}s", m.offset));
            if ui.small_button("−").on_hover_text("lyrics 0.1 s earlier").clicked() {
                m.nudge(-0.1);
            }
        });
    });
    if m.snap.is_none() {
        return;
    }
    let weak = |ui: &Ui, t: &str| RichText::new(t.to_string()).color(ui.visuals().weak_text_color());
    match &m.lyrics {
        None => return drop(ui.label(weak(ui, "fetching lyrics…"))),
        Some(Lyrics::NotFound) => return drop(ui.label(weak(ui, "no synced lyrics found"))),
        Some(Lyrics::Instrumental) => return drop(ui.label(RichText::new("♪  instrumental  ♪").color(style_accent(ui)))),
        Some(Lyrics::Error(e)) => return drop(ui.label(weak(ui, e))),
        _ => {}
    }
    let cur = m.active_line();
    m.line_anim.update(cur);
    // the user moved the view by hand (wheel, scrollbar, drag): stop following for a while
    let area = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(area) {
        let moved = ui.input(|i| i.smooth_scroll_delta.y != 0.0 || i.pointer.is_decidedly_dragging());
        if moved {
            m.browse.touch();
        }
    }
    if ui.input(|i| i.pointer.primary_down() && i.pointer.interact_pos().is_some_and(|p| area.contains(p) && p.x > area.right() - 14.0)) {
        m.browse.touch();
    }
    let rows_by_key = m.browse.take_rows();
    let following = !m.browse.active();
    if let Some(rem) = m.browse.remaining() {
        ui.ctx().request_repaint_after(rem);
    }
    if m.line_anim.animating() {
        ui.ctx().request_repaint();
    }
    let plain = matches!(m.lyrics, Some(Lyrics::Plain(_)));
    let length = m.snap.as_ref().map_or(0.0, |s| s.length);
    let mut area_ui = egui::ScrollArea::vertical().id_salt(m.track_key).auto_shrink([false, false]);
    if following {
        if plain && length > 0.0 {
            let (content, view) = m.lyric_dims;
            area_ui = area_ui.vertical_scroll_offset((m.position() / length).clamp(0.0, 1.0) as f32 * (content - view).max(0.0));
        } else if cur.is_none() {
            area_ui = area_ui.vertical_scroll_offset(0.0);
        }
    }
    let mut act = Act::default();
    let line_h = m.gui.lyrics_size as f32 * 1.6;
    let out = {
        let mm: &Model = m;
        area_ui.show(ui, |ui| {
            if rows_by_key != 0.0 {
                ui.scroll_with_delta(Vec2::new(0.0, -rows_by_key * line_h));
            }
            // `lyric_pad` 0-10: side padding
            let pad = mm.args.lyric_pad as f32 * 4.0;
            egui::Frame::new().inner_margin(egui::Margin::symmetric(pad.min(120.0) as i8, 0)).show(ui, |ui| {
                draw_all(ui, mm, cur, following, &mut act)
            })
        })
    };
    m.lyric_dims = (out.content_size.y, out.inner_rect.height());
    if let Some(t) = act.seek_to {
        m.follow();
        m.seek(t);
    }
}

fn style_accent(ui: &Ui) -> Color32 {
    ui.visuals().hyperlink_color
}

fn draw_all(ui: &mut Ui, m: &Model, cur: Option<usize>, following: bool, act: &mut Act) {
    let g = &m.gui;
    let accent = style_accent(ui);
    let synced = m.synced();
    let plain = synced.is_none();
    let n = match &m.lyrics {
        Some(Lyrics::Synced(l)) => l.len(),
        Some(Lyrics::Plain(l)) => l.len(),
        _ => 0,
    };
    let tr = m.translation().filter(|t| t.len() == n);
    let both = m.translate_mode == TranslateMode::Both;
    let center = g.lyrics_align == LAlign::Center;
    let smooth = g.lyrics_animation == Animation::Smooth;
    let karaoke = m.args.karaoke && m.playing();
    let threshold = m.args.interlude_secs;
    let length = m.snap.as_ref().map_or(0.0, |s| s.length);
    let t = m.position() + m.offset;
    let al = m.aligned.as_ref().filter(|_| m.chords_on && !plain).filter(|a| a.usable_at(m.args.chords_match));
    let (shift, flats) = if al.is_some() { (m.chord_shift(), m.use_flats()) } else { (0, false) };
    let ctx = m.args.chords_context;
    let chord_font = FontId::proportional((g.lyrics_size as f32 * 0.7).max(11.0));
    let label_h = g.meta_size as f32 * 1.4;
    let now = Instant::now();
    let strong = ui.visuals().strong_text_color();
    let base = ui.visuals().text_color();
    let weak_c = ui.visuals().weak_text_color();
    let mut hit_active: Option<egui::Response> = None;

    if plain {
        ui.vertical_centered(|ui| ui.label(RichText::new("no synced lyrics — plain text").italics().small().color(weak_c)));
    }
    let names = |v: &[String]| v.iter().map(|c| chords::transpose(c, shift, flats)).collect::<Vec<_>>().join("   ");
    let centered = |ui: &mut Ui, text: RichText| {
        if center {
            ui.vertical_centered(|ui| ui.label(text));
        } else {
            ui.label(text);
        }
    };
    let marker = |ui: &mut Ui, mk: Mk| {
        let Some(l) = synced else { return };
        let (active, remain) = break_state(mk, t, cur, l);
        if active {
            centered(ui, RichText::new(marker_label(mk, remain)).italics().color(accent));
        } else {
            ui.add_space(label_h);
        }
    };
    let gap_chords = |ui: &mut Ui, mk: Mk, i: usize| {
        let (Some(l), Some(a)) = (synced, al) else { return };
        if !break_state(mk, t, cur, l).0 {
            return;
        }
        for row in a.gaps.get(i).into_iter().flatten() {
            centered(ui, RichText::new(names(row)).strong().color(accent));
        }
    };
    if let Some(l) = synced {
        if has_lead(l, threshold) {
            marker(ui, Mk::Lead);
        }
    }
    if let Some(a) = al {
        for row in &a.intro {
            centered(ui, RichText::new(names(row)).strong().color(if cur.is_none() { accent } else { accent.gamma_multiply(0.45) }));
        }
    }
    for i in 0..n {
        let orig = m.line_at(i).unwrap_or("");
        let trans = tr.map(|t| t[i].as_str()).filter(|t| !t.trim().is_empty());
        let text = match trans {
            Some(t) if !both => t,
            _ => orig,
        };
        // `lyric_spacing` 0-3: extra air between lines
        if i > 0 && m.args.lyric_spacing > 0 {
            ui.add_space(m.args.lyric_spacing as f32 * 4.0);
        }
        let w = m.line_anim.weight_at(i, smooth, now);
        let active = cur == Some(i);
        // `big_lyrics` / `big_scale`: the active line grows by a third per step
        let active_size = g.active_size as f32 * if m.args.big_lyrics { 1.0 + m.args.big_scale as f32 / 3.0 } else { 1.0 };
        let size = g.lyrics_size as f32 + (active_size - g.lyrics_size as f32) * w;
        let font = if w > 0.5 { style::active_font(g, size) } else { FontId::proportional(size) };
        let color = if active {
            strong
        } else {
            match cur {
                Some(c) => base.gamma_multiply(1.0 - style::fade(g, i.abs_diff(c))),
                None if !plain => base.gamma_multiply(0.5),
                None => base.gamma_multiply(0.85),
            }
        };
        let line_chords: Vec<(usize, String)> = match al.and_then(|a| a.per_line.get(i)) {
            Some(cs) if !cs.is_empty() => {
                let cs: Vec<(usize, String)> = if text.as_ptr() != orig.as_ptr() { remap_chords(cs, orig, text) } else { cs.clone() };
                cs.into_iter().map(|(c, nme)| (c, chords::transpose(&nme, shift, flats))).collect()
            }
            _ => Vec::new(),
        };
        let chord_color = chords_in_context(ctx, cur, i).then(|| match cur {
            Some(c) if i == c => accent,
            Some(c) if i < c => accent.gamma_multiply(0.5),
            _ => accent.gamma_multiply(0.7),
        });
        let lit = (active && karaoke && text.trim() != "♪").then(|| (m.line_fraction(), accent));
        // the translation and the member tag of the active line light up with the line itself
        let part_color = if active { strong } else { weak_c };
        let has_tr = both && trans.is_some_and(|t| t.trim() != orig.trim());
        let member_tag = cur
            .filter(|c| i.abs_diff(*c) <= 2)
            .and_then(|_| m.members.get(i))
            .filter(|x| !x.is_empty())
            .map(|mem| {
                let tag = RichText::new(format!("[{}]", mem.join(" · "))).color(part_color);
                if g.member_size > 0 { tag.size(g.member_size as f32) } else { tag.small() }
            });
        let (parts, inline) = line_parts(m.args.translation_position, m.args.member_position, has_tr, member_tag.is_some(), center);
        let mut line_resp = None;
        // the halo goes behind all parts of the active line: its slot is reserved first, the
        // rectangle is what the parts took
        let halo = (active && g.lyrics_glow > 0).then(|| ui.painter().add(egui::Shape::Noop));
        let mut group: Option<egui::Rect> = None;
        let mut grow = |r: &egui::Response| group = Some(group.map_or(r.rect, |g| g.union(r.rect)));
        for part in parts {
            match part {
                Part::Member => {
                    if let Some(tag) = member_tag.clone() {
                        let r = if center { ui.vertical_centered(|ui| ui.label(tag)).inner } else { ui.label(tag) };
                        grow(&r);
                    }
                }
                Part::Translation => {
                    if let Some(tr_text) = trans {
                        let txt = RichText::new(tr_text).italics().color(part_color);
                        let txt = if g.translation_size > 0 { txt.size(g.translation_size as f32) } else { txt };
                        let r = if center {
                            ui.vertical_centered(|ui| ui.add(egui::Label::new(txt).wrap())).inner
                        } else {
                            ui.add(egui::Label::new(txt).wrap())
                        };
                        grow(&r);
                    }
                }
                Part::Line => {
                    let spec = Spec { text, chords: &line_chords, chord_color, font: font.clone(), color, chord_font: chord_font.clone(), lit, center };
                    let resp = match (inline, member_tag.clone()) {
                        // the tag in front of the line, on its first row; the line keeps its own width
                        (true, Some(tag)) => ui.horizontal_top(|ui| {
                            ui.label(tag);
                            ui.vertical(|ui| draw_line(ui, &spec)).inner
                        })
                        .inner,
                        _ => draw_line(ui, &spec),
                    };
                    grow(&resp);
                    if synced.is_some() && resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        ui.painter().rect_filled(resp.rect.expand(3.0), g.corner_radius.min(8), base.gamma_multiply(0.07));
                    }
                    line_resp = Some(resp);
                }
            }
        }
        let Some(resp) = line_resp else { continue };
        if let (Some(slot), Some(rect)) = (halo, group) {
            let k = g.lyrics_glow as f32 / 100.0;
            let rings = (1..=6u8)
                .rev()
                .map(|r| {
                    let spread = r as f32 * 4.0;
                    egui::Shape::rect_filled(rect.expand(spread), egui::CornerRadius::same((g.corner_radius as f32 + spread) as u8), accent.gamma_multiply(k * 0.07))
                })
                .collect();
            ui.painter().set(slot, egui::Shape::Vec(rings));
        }
        if let Some(l) = synced {
            if resp.clicked() {
                act.seek_to = Some((l[i].0 - m.offset).max(0.0));
            }
            if let Some(mk) = marker_after(l, i, length, threshold) {
                marker(ui, mk);
                gap_chords(ui, mk, i);
            }
        }
        if active {
            hit_active = Some(resp);
        }
    }
    if following {
        if let Some(r) = hit_active {
            r.scroll_to_me(Some(Align::Center));
        }
    }
}

/// One thing drawn for a lyric line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Member,
    Translation,
    Line,
}

/// What is drawn for one line, top to bottom, and whether the member tag shares the line's row.
/// `auto` puts the tag under the line; `inline` needs a left-aligned line, so a centred one
/// gets the tag above instead.
fn line_parts(
    translation: music_display::cli::TranslationPosition,
    member: music_display::cli::MemberPosition,
    has_translation: bool,
    has_member: bool,
    center: bool,
) -> (Vec<Part>, bool) {
    use music_display::cli::{MemberPosition as MP, TranslationPosition as TP};
    let member = match (member, center) {
        (MP::Auto, _) => MP::Below,
        (MP::Inline, true) => MP::Above,
        (m, _) => m,
    };
    let mut parts = Vec::new();
    if has_member && member == MP::Above {
        parts.push(Part::Member);
    }
    if has_translation && translation == TP::Above {
        parts.push(Part::Translation);
    }
    parts.push(Part::Line);
    if has_translation && translation == TP::Below {
        parts.push(Part::Translation);
    }
    if has_member && member == MP::Below {
        parts.push(Part::Member);
    }
    (parts, has_member && member == MP::Inline)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Vec<(f64, String)> {
        vec![(10.0, "first line here".into()), (14.0, "second".into()), (60.0, "after a solo".into())]
    }

    #[test]
    fn the_translation_and_the_member_tag_follow_their_positions() {
        use music_display::cli::{MemberPosition as MP, TranslationPosition as TP};
        use Part::*;
        let p = |tp, mp, c| line_parts(tp, mp, true, true, c);
        assert_eq!(p(TP::Below, MP::Auto, false), (vec![Line, Translation, Member], false));
        assert_eq!(p(TP::Above, MP::Above, false), (vec![Member, Translation, Line], false));
        assert_eq!(p(TP::Above, MP::Below, false), (vec![Translation, Line, Member], false));
        assert_eq!(p(TP::Below, MP::Inline, false), (vec![Line, Translation], true));
        // inline needs a left-aligned line
        assert_eq!(p(TP::Below, MP::Inline, true), (vec![Member, Line, Translation], false));
        // nothing to show, nothing drawn
        assert_eq!(line_parts(TP::Above, MP::Above, false, false, false), (vec![Line], false));
    }

    #[test]
    fn a_lead_in_marker_needs_a_long_enough_wait() {
        assert!(has_lead(&song(), 5.0));
        assert!(!has_lead(&song(), 12.0));
        assert!(!has_lead(&[], 5.0));
    }

    #[test]
    fn long_gaps_get_a_marker_and_short_ones_do_not() {
        let l = song();
        assert_eq!(marker_after(&l, 0, 100.0, 5.0), None);
        assert_eq!(marker_after(&l, 1, 100.0, 5.0), Some(Mk::After(1)));
        // after the last line: only when the song goes on for long enough
        assert_eq!(marker_after(&l, 2, 100.0, 5.0), Some(Mk::Tail));
        assert_eq!(marker_after(&l, 2, 63.0, 5.0), None);
        assert_eq!(marker_after(&l, 2, 0.0, 5.0), None, "unknown length");
        let rest = vec![(0.0, "♪".to_string()), (30.0, "x".to_string())];
        assert_eq!(marker_after(&rest, 0, 40.0, 5.0), None, "a ♪ line is the break itself");
    }

    #[test]
    fn a_break_is_active_after_the_line_is_sung_with_a_countdown() {
        let l = song();
        // before the first line
        assert_eq!(break_state(Mk::Lead, 4.0, None, &l), (true, 6.0));
        assert!(!break_state(Mk::Lead, 11.0, Some(0), &l).0);
        // line 1 starts at 14 and takes about est_sung seconds; the solo then lasts to 60
        assert!(!break_state(Mk::After(1), 14.5, Some(1), &l).0);
        let (on, remain) = break_state(Mk::After(1), 30.0, Some(1), &l);
        assert!(on && (remain - 30.0).abs() < 1e-9);
        assert!(!break_state(Mk::After(1), 61.0, Some(2), &l).0);
        assert!(break_state(Mk::Tail, 90.0, Some(2), &l).0);
        assert_eq!(marker_label(Mk::After(1), 29.2), "[ Instrumental / Solo ]  · 30s");
        assert_eq!(marker_label(Mk::Tail, 0.0), "[ Instrumental / Solo ]");
    }

    #[test]
    fn chords_context_limits_the_rows_around_the_active_line() {
        assert!(chords_in_context(99, Some(5), 0));
        assert!(!chords_in_context(2, Some(5), 4), "already sung");
        assert!(chords_in_context(2, Some(5), 7));
        assert!(!chords_in_context(2, Some(5), 8));
        assert!(chords_in_context(2, None, 1), "before the start the first lines count");
    }

    #[test]
    fn the_active_size_eases_in_and_the_old_line_eases_out() {
        let mut a = LineAnim::default();
        a.update(Some(3));
        let t0 = a.since;
        assert_eq!(a.weight_at(3, true, t0), 0.0);
        assert!((a.weight_at(3, true, t0 + Duration::from_millis(90)) - 0.5).abs() < 0.01);
        assert_eq!(a.weight_at(3, true, t0 + Duration::from_secs(1)), 1.0);
        a.update(Some(4));
        let t1 = a.since;
        assert_eq!(a.weight_at(3, true, t1), 1.0);
        assert_eq!(a.weight_at(3, true, t1 + Duration::from_secs(1)), 0.0);
        assert_eq!(a.weight_at(4, false, t1), 1.0, "instant mode has no easing");
        assert_eq!(a.weight_at(9, true, t1), 0.0);
    }
}
