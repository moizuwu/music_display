//! The panels, drawn with egui. Each takes the model and reads or changes it; the same panel ids
//! and layout config as the terminal front end decide what is shown and in which order.
use super::model::{Chords, Model, Notes};
use super::style::{self, Icon};
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Ui, Vec2};
use music_display::cli::TranslateMode;
use music_display::layout::{Button, MetaField};
use music_display::lyrics::Lyrics;
use music_display::player::Status;

pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn accent(ui: &Ui) -> Color32 {
    ui.visuals().hyperlink_color
}

// ── metadata, cover, controls ───────────────────────────────────────────────

pub fn metadata(ui: &mut Ui, m: &mut Model) {
    let Some(s) = m.snap.clone() else {
        ui.label(RichText::new(format!("waiting for {}…", m.args.player)).weak());
        return;
    };
    let mut clicked_diagrams = false;
    let mut hide: Option<MetaField> = None;
    let mut mv: Option<(MetaField, i32)> = None;
    let capo = m.effective_capo();
    let rows: Vec<(MetaField, String)> = m
        .layout
        .fields
        .visible()
        .filter_map(|f| {
            let v = match f {
                MetaField::Title => s.title.clone(),
                MetaField::Artist => s.artist.clone(),
                MetaField::Album => s.album.clone(),
                MetaField::TrackNo => format!(
                    "{:02}-{}",
                    s.disc_no.unwrap_or(1),
                    s.track_no.map(|t| format!("{t:02}")).unwrap_or_else(|| "--".into())
                ),
                MetaField::Release => {
                    music_display::year::full_date(&s.content_created).or_else(|| m.year.clone()).unwrap_or_else(|| "…".into())
                }
                MetaField::Genre => m.genre.clone()?,
                MetaField::Tuning => m.sheet.as_ref().and_then(|x| x.tuning.clone())?,
                MetaField::Capo => {
                    if !(m.chords_on && capo > 0) {
                        return None;
                    }
                    capo.to_string()
                }
                MetaField::Bpm => m.sheet.as_ref().and_then(|x| x.bpm)?.to_string(),
                MetaField::Key => {
                    let (k, _) = m.keys()?;
                    format!("{} / {}", k.long(), k.camelot())
                }
            };
            Some((f, if v.is_empty() { "—".into() } else { v }))
        })
        .collect();
    let meta = m.gui.meta_size as f32;
    let gap = 6.0 + m.gui.panel_spacing as f32 * 0.4;
    egui::Grid::new("meta").num_columns(2).spacing([12.0, gap]).show(ui, |ui| {
        for (f, v) in &rows {
            let label = ui.add(egui::Label::new(RichText::new(f.label()).color(accent(ui)).strong()).sense(Sense::click()));
            // the title is the headline of the card
            let text = if *f == MetaField::Title { RichText::new(v.as_str()).size(meta + 4.0).strong() } else { RichText::new(v.as_str()) };
            let value = ui.add(egui::Label::new(text).wrap().sense(Sense::click()));
            for r in [&label, &value] {
                // clicking Key, BPM, Capo or Tuning opens the chord diagrams
                if f.opens_diagrams() {
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        clicked_diagrams = true;
                    }
                }
                r.context_menu(|ui| {
                    if ui.button("Hide").clicked() {
                        hide = Some(*f);
                        ui.close();
                    }
                    if ui.button("Move up").clicked() {
                        mv = Some((*f, -1));
                        ui.close();
                    }
                    if ui.button("Move down").clicked() {
                        mv = Some((*f, 1));
                        ui.close();
                    }
                });
            }
            ui.end_row();
        }
    });
    if clicked_diagrams {
        m.show_diagrams = true;
        m.layout.panels.set(music_display::layout::PanelId::Chords, true);
    }
    if let Some(f) = hide {
        m.layout.fields.set(f, false);
        m.save_layout();
    }
    if let Some((f, d)) = mv {
        // past the next visible neighbour
        let items = &mut m.layout.fields.items;
        if let Some(at) = items.iter().position(|(x, _)| *x == f) {
            let mut to = at as i32;
            loop {
                to += d;
                match items.get(to.max(0) as usize) {
                    _ if to < 0 => break,
                    None => break,
                    Some((_, true)) => {
                        let e = items.remove(at);
                        items.insert(to as usize, e);
                        break;
                    }
                    Some(_) => {}
                }
            }
        }
        m.save_layout();
    }
}

pub fn cover(ui: &mut Ui, tex: Option<&egui::TextureHandle>, radius: u8) {
    // square, as wide as the column allows (but never taller than the column or 480 px), centred
    let side = ui.available_width().min(ui.available_height()).clamp(64.0, 480.0);
    let radius = egui::CornerRadius::same(radius.saturating_add(2));
    ui.vertical_centered(|ui| match tex {
        Some(t) => {
            ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(side)).corner_radius(radius));
        }
        None => {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(side.min(160.0)), Sense::hover());
            ui.painter().rect_stroke(r, radius, Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "♪", FontId::proportional(32.0), ui.visuals().weak_text_color());
        }
    });
}

pub fn controls(ui: &mut Ui, m: &mut Model) {
    let Some(s) = m.snap.clone() else { return };
    // progress bar: click or drag to seek
    let pos = m.position();
    ui.horizontal(|ui| {
        ui.monospace(fmt_time(pos));
        let w = (ui.available_width() - 48.0).max(40.0);
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::click_and_drag());
        let hot = resp.hovered() || resp.dragged();
        if hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let frac = if s.length > 0.0 { (pos / s.length).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let mid = r.center().y;
        let p = ui.painter();
        p.line_segment([egui::pos2(r.left(), mid), egui::pos2(r.right(), mid)], Stroke::new(if hot { 5.0 } else { 3.0 }, ui.visuals().widgets.inactive.bg_fill));
        let x = r.left() + r.width() * frac;
        p.line_segment([egui::pos2(r.left(), mid), egui::pos2(x, mid)], Stroke::new(if hot { 5.0 } else { 3.0 }, accent(ui)));
        p.circle_filled(egui::pos2(x, mid), if hot { 7.0 } else { 5.0 }, accent(ui));
        if (resp.clicked() || resp.dragged()) && s.length > 0.0 {
            if let Some(pt) = resp.interact_pointer_pos() {
                let f = ((pt.x - r.left()) / r.width()).clamp(0.0, 1.0) as f64;
                m.seek(f * s.length);
            }
        }
        ui.monospace(fmt_time(s.length));
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let shown = m.layout.buttons.clone();
        let b = |f: Button| shown.shown(f);
        if b(Button::Shuffle) {
            if let Some(on) = s.shuffle {
                if ui.selectable_label(on, "shuffle").clicked() {
                    m.toggle_shuffle();
                }
            }
        }
        if b(Button::Repeat) {
            if let Some(r) = s.repeat {
                let label = match r {
                    music_display::player::Loop::None => "repeat",
                    music_display::player::Loop::Track => "repeat 1",
                    music_display::player::Loop::Playlist => "repeat all",
                };
                if ui.selectable_label(r != music_display::player::Loop::None, label).clicked() {
                    m.cycle_repeat();
                }
            }
        }
        if b(Button::Prev) && style::icon_button(ui, Icon::Prev, false).on_hover_text("previous").clicked() {
            m.prev();
        }
        if b(Button::Play) {
            let (icon, tip) = if s.status == Status::Playing { (Icon::Pause, "pause") } else { (Icon::Play, "play") };
            if style::icon_button(ui, icon, true).on_hover_text(tip).clicked() {
                m.play_pause();
            }
        }
        if b(Button::Next) && style::icon_button(ui, Icon::Next, false).on_hover_text("next").clicked() {
            m.next();
        }
        if b(Button::Volume) {
            if let Some(v) = s.volume {
                let mut pct = (v * 100.0) as f32;
                if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").show_value(true)).changed() {
                    m.set_volume(pct as f64 / 100.0);
                }
            }
        }
        super::library::transport_buttons(ui, m);
    });
}

// ── lyrics ──────────────────────────────────────────────────────────────────

pub fn lyrics(ui: &mut Ui, m: &mut Model) {
    let title = match (m.translate_mode, m.translation()) {
        (TranslateMode::Off, _) | (_, None) => "Lyrics".to_string(),
        (_, Some(_)) if m.translate_backend.starts_with("AI ") => format!("Lyrics · translated by {}", m.args.ai_model),
        (_, Some(_)) if m.translate_backend.starts_with("fallback:") => {
            format!("Lyrics · translated ({})", m.translate_backend.trim_start_matches("fallback: "))
        }
        (TranslateMode::Both, _) => format!("Lyrics · original + translated ({})", m.args.translate_lang),
        _ => format!("Lyrics · translated ({})", m.args.translate_lang),
    };
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
    let (lines, stamps): (Vec<String>, Vec<f64>) = match &m.lyrics {
        None => return drop(ui.label(RichText::new("fetching lyrics…").weak())),
        Some(Lyrics::NotFound) => return drop(ui.label(RichText::new("no synced lyrics found").weak())),
        Some(Lyrics::Instrumental) => return drop(ui.label(RichText::new("♪  instrumental  ♪").color(accent(ui)))),
        Some(Lyrics::Error(e)) => return drop(ui.label(RichText::new(e.as_str()).weak())),
        Some(Lyrics::Synced(l)) => (l.iter().map(|x| x.1.clone()).collect(), l.iter().map(|x| x.0).collect()),
        Some(Lyrics::Plain(l)) => (l.clone(), Vec::new()),
    };
    let g = m.gui.clone();
    let cur = m.active_line();
    let frac = m.line_fraction() as f32;
    let tr = m.translation().cloned().filter(|t| t.len() == lines.len());
    let both = m.translate_mode == TranslateMode::Both;
    let karaoke = m.args.karaoke;
    let mut seek_to: Option<f64> = None;
    let mut want_scroll = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, line) in lines.iter().enumerate() {
            let shown = match (&tr, both) {
                (Some(t), false) if !t[i].trim().is_empty() => t[i].clone(),
                _ => line.clone(),
            };
            let active = cur == Some(i);
            // chords sit above the words
            if m.chords_on {
                let ch = m.chords_of(i);
                if !ch.is_empty() {
                    let mut row = String::new();
                    for (col, name) in ch {
                        let at = col.max(row.chars().count() + usize::from(!row.is_empty()));
                        row.push_str(&" ".repeat(at.saturating_sub(row.chars().count())));
                        row.push_str(&name);
                    }
                    ui.label(RichText::new(row).monospace().color(accent(ui)).strong());
                }
            }
            let size = style::line_size(ui, &g, ui.make_persistent_id(("line", m.track_key, i)), active);
            let font = if active { style::active_font(&g, size) } else { FontId::proportional(size) };
            // lines fade out the further they are from the active one
            let dist = cur.map_or(0, |c| i.abs_diff(c));
            let color = if active { ui.visuals().strong_text_color() } else { ui.visuals().text_color().gamma_multiply(1.0 - style::fade(&g, dist)) };
            let resp = if active && karaoke {
                // the sung part is lit, the rest dimmer
                let chars: Vec<char> = shown.chars().collect();
                let lit = ((chars.len() as f32) * frac).round() as usize;
                let (a, b): (String, String) = (chars[..lit.min(chars.len())].iter().collect(), chars[lit.min(chars.len())..].iter().collect());
                let mut job = egui::text::LayoutJob::default();
                job.append(&a, 0.0, egui::TextFormat { font_id: font.clone(), color: accent(ui), ..Default::default() });
                job.append(&b, 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
                style::aligned(ui, &g, |ui| ui.add(egui::Label::new(job).sense(Sense::click())))
            } else {
                style::aligned(ui, &g, |ui| ui.add(egui::Label::new(RichText::new(shown).font(font).color(color)).sense(Sense::click())))
            };
            if !stamps.is_empty() && resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                ui.painter().rect_filled(resp.rect.expand(3.0), g.corner_radius.min(8), ui.visuals().text_color().gamma_multiply(0.07));
            }
            if both {
                if let Some(t) = &tr {
                    ui.label(RichText::new(&t[i]).italics().weak());
                }
            }
            if let Some(mem) = m.members.get(i).filter(|x| !x.is_empty()) {
                ui.label(RichText::new(mem.join(" · ")).small().weak());
            }
            if active {
                resp.scroll_to_me(Some(egui::Align::Center));
                want_scroll = true;
            }
            if resp.clicked() && !stamps.is_empty() {
                seek_to = Some((stamps[i] - m.offset).max(0.0));
            }
        }
    });
    let _ = want_scroll;
    if let Some(t) = seek_to {
        m.seek(t);
    }
}

// ── chord diagrams ──────────────────────────────────────────────────────────

pub fn diagrams(ui: &mut Ui, m: &mut Model) {
    ui.heading("Chords");
    ui.horizontal(|ui| {
        if ui.small_button("t −").clicked() {
            m.transpose = (m.transpose - 1).rem_euclid(12);
        }
        ui.label(format!("transpose {:+}", m.transpose));
        if ui.small_button("t +").clicked() {
            m.transpose = (m.transpose + 1).rem_euclid(12);
        }
        ui.separator();
        if ui.small_button("capo −").clicked() {
            m.change_capo(-1);
        }
        ui.label(format!("capo {}", m.effective_capo()));
        if ui.small_button("capo +").clicked() {
            m.change_capo(1);
        }
    });
    match &m.chords_state {
        Chords::Idle if !m.chords_on => {
            ui.label(RichText::new("chords are off (settings → Chords)").weak());
        }
        Chords::Fetching => {
            ui.label(RichText::new("looking for a chord sheet…").weak());
        }
        Chords::Missing => {
            ui.label(RichText::new("no chord sheet found").weak());
        }
        Chords::Failed(e) => {
            ui.label(RichText::new(e.as_str()).weak());
        }
        _ => {}
    }
    let names = m.song_chords();
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for name in &names {
                ui.vertical(|ui| {
                    ui.set_width(84.0);
                    ui.label(RichText::new(name).strong());
                    match music_display::diagram::lookup(name) {
                        Some(shape) => draw_shape(ui, &shape, accent(ui)),
                        None => {
                            ui.label(RichText::new("no diagram").weak());
                        }
                    }
                });
            }
        });
    });
}

/// A six-string diagram: muted / open markers, nut, four frets, finger dots.
pub fn draw_shape(ui: &mut Ui, shape: &music_display::diagram::Shape, color: Color32) {
    let (w, h) = (72.0, 92.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let p = ui.painter();
    let ink = ui.visuals().text_color();
    let frets: Vec<i8> = shape.frets.to_vec();
    let low = frets.iter().copied().filter(|f| *f > 0).min().unwrap_or(1);
    let top = if frets.iter().copied().max().unwrap_or(0) <= 4 { 1 } else { low.max(1) };
    let (x0, y0, dx, dy) = (r.left() + 8.0, r.top() + 14.0, 11.0, 17.0);
    for (s, &fret) in frets.iter().enumerate() {
        let x = x0 + dx * s as f32;
        p.line_segment([egui::pos2(x, y0), egui::pos2(x, y0 + dy * 4.0)], Stroke::new(1.0, ink));
        match fret {
            -1 => {
                p.text(egui::pos2(x, r.top() + 5.0), Align2::CENTER_CENTER, "×", FontId::monospace(11.0), ink);
            }
            0 => {
                p.circle_stroke(egui::pos2(x, r.top() + 5.0), 3.0, Stroke::new(1.0, ink));
            }
            f => {
                let row = (f - top) as f32;
                p.circle_filled(egui::pos2(x, y0 + dy * (row + 0.5)), 4.5, color);
            }
        }
    }
    for i in 0..=4 {
        let thick = if i == 0 && top == 1 { 3.0 } else { 1.0 };
        p.line_segment([egui::pos2(x0, y0 + dy * i as f32), egui::pos2(x0 + dx * 5.0, y0 + dy * i as f32)], Stroke::new(thick, ink));
    }
    if top > 1 {
        p.text(egui::pos2(x0 + dx * 5.0 + 6.0, y0 + dy * 0.5), Align2::LEFT_CENTER, format!("{top}fr"), FontId::monospace(9.0), ink);
    }
}

// ── annotations ─────────────────────────────────────────────────────────────

pub fn annotations(ui: &mut Ui, m: &mut Model) {
    let mode = m.args.annotations_mode;
    ui.heading(format!("Annotations · {}", mode.label()));
    ui.label(RichText::new("experimental").small().weak());
    match &m.notes {
        Notes::Idle => return drop(ui.label(RichText::new("waiting for lyrics…").weak())),
        Notes::Loading => return drop(ui.label(RichText::new("looking up annotations…").weak())),
        Notes::Failed(e) => return drop(ui.add(egui::Label::new(RichText::new(e.as_str()).weak()).wrap())),
        Notes::Ready => {}
    }
    let Some(i) = m.active_line() else {
        return drop(ui.label(RichText::new("annotations follow the synced lyrics").weak()));
    };
    let notes = m.notes_for(i);
    let max = m.args.annotation_max_chars;
    // a long note scrolls; back to the top when the line changes
    egui::ScrollArea::vertical().id_salt(("notes", m.track_key, i)).auto_shrink([false, true]).show(ui, |ui| {
        if let Some(e) = &m.genius_err {
            ui.add(egui::Label::new(RichText::new(format!("Genius: {e}")).small().weak()).wrap());
        }
        if notes.is_empty() {
            ui.label(RichText::new("no annotation for this line").weak());
        }
        for (k, n) in notes.iter().enumerate() {
            if k > 0 {
                ui.add_space(4.0);
                ui.separator();
                ui.label(RichText::new("AI extra").small().weak());
            }
            if !n.fragment.is_empty() {
                ui.label(RichText::new(format!("“{}”", n.fragment)).italics().color(accent(ui)));
            }
            let text = RichText::new(music_display::annotations::shown(&n.text, max));
            ui.add(egui::Label::new(if k > 0 { text.italics() } else { text }).wrap());
            ui.label(RichText::new(format!("— {}", n.source)).weak());
        }
        if mode == music_display::cli::AnnotationMode::GeniusAi {
            match &m.ai_state {
                Notes::Loading => drop(ui.label(RichText::new("AI extras: looking up…").small().weak())),
                Notes::Failed(e) => drop(ui.add(egui::Label::new(RichText::new(format!("AI extras: {e}")).small().weak()).wrap())),
                _ => {}
            }
        }
    });
}
