//! The panels, drawn with egui. Each takes the model and reads or changes it; the same panel ids
//! and layout config as the terminal front end decide what is shown and in which order.
use super::model::{Chords, Model, Notes};
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Ui, Vec2};
use music_display::layout::MetaField;

pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn accent(ui: &Ui) -> Color32 {
    ui.visuals().hyperlink_color
}

// ── metadata, cover, controls ───────────────────────────────────────────────

pub fn metadata(ui: &mut Ui, m: &mut Model) {
    let Some(s) = m.snap.clone() else {
        ui.label(RichText::new(format!("waiting for {}…", m.args.player)).weak());
        return;
    };
    let mut clicked_diagrams = false;
    let mut to_library: Option<MetaField> = None;
    let spotify_on = m.spot.on();
    let mut hide: Option<MetaField> = None;
    let mut mv: Option<(MetaField, i32)> = None;
    let capo = m.effective_capo();
    let rows: Vec<(MetaField, String)> = m
        .layout
        .fields
        .visible()
        .filter_map(|f| {
            let v = match f {
                MetaField::Title => match m.spot.liked {
                    Some(on) if m.spot.on() => format!("{} {}", s.title, if on { "♥" } else { "♡" }),
                    _ => s.title.clone(),
                },
                MetaField::Artist => s.artist.clone(),
                MetaField::Album => s.album.clone(),
                MetaField::TrackNo => format!(
                    "{:02}-{}",
                    s.disc_no.unwrap_or(1),
                    s.track_no.map(|t| format!("{t:02}")).unwrap_or_else(|| "--".into())
                ),
                MetaField::Release => {
                    music_display::year::full_date(&s.content_created)
                        .or_else(|| m.year.clone())
                        .unwrap_or_else(|| if m.year_done { "N/A".into() } else { "...".into() })
                }
                MetaField::Genre => m.genre.clone()?,
                MetaField::Tuning => {
                    if !(m.chords_on || m.show_diagrams) {
                        return None;
                    }
                    m.sheet.as_ref().and_then(|x| x.tuning.clone())?
                }
                MetaField::Capo => {
                    if m.chords_on && capo > 0 {
                        let mut v = capo.to_string();
                        if let Some((_, shapes)) = m.keys() {
                            v.push_str(&format!(" ({})", shapes.short()));
                        }
                        let sheet_capo = m.sheet.as_ref().map_or(0, |x| x.capo);
                        if capo != sheet_capo {
                            v.push_str(&format!(" [sheet {sheet_capo}]"));
                        }
                        v
                    } else if m.chords_on && m.sheet.as_ref().is_some_and(|x| x.capo > 0) {
                        "none".into()
                    } else {
                        return None;
                    }
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
                // with the Spotify library on, Track, Artist, Album and Genre open it there
                if f.opens_library() && spotify_on {
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        to_library = Some(*f);
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
    if let Some(f) = to_library {
        let genre = m.genre.clone().unwrap_or_default();
        m.spot.open_for(f, &s, &genre);
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
    // square, as wide as the column allows but at most 40% of the window's height (so the
    // metadata and the controls stay on screen), centred
    let side = super::controls::cover_side(ui.available_width(), ui.ctx().content_rect().height(), 0.4);
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

// ── chord diagrams ──────────────────────────────────────────────────────────

pub fn diagrams(ui: &mut Ui, m: &mut Model) {
    ui.horizontal(|ui| {
        ui.heading("Chords");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("close the chord diagrams (d)").clicked() {
                m.show_diagrams = false;
                m.layout.panels.set(music_display::layout::PanelId::Chords, false);
            }
        });
    });
    ui.horizontal(|ui| {
        if ui.small_button("t −").clicked() {
            m.transpose = (m.transpose - 1).rem_euclid(12);
        }
        ui.label(format!("transpose {:+}", super::footer::signed_transpose(m.transpose)));
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
            ui.label(RichText::new("chords are off: press g, or tick them in settings").weak());
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
    ui.heading(if m.args.panel_sources { format!("Annotations · {}", mode.label()) } else { "Annotations".to_string() });
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
    // e / E scroll by hand; `annotation_scroll = "auto"` follows the song through the line instead
    let auto = m.args.annotation_scroll == music_display::cli::AnnotationScroll::Auto;
    let by = m.ux.take_ann_rows();
    let progress = m.note_line_progress(i);
    let line = ui.text_style_height(&egui::TextStyle::Body);
    // (the overflow of the last frame is what auto scroll maps the progress onto)
    let over_id = ui.make_persistent_id(("notes-over", m.track_key, i));
    let over: f32 = ui.data(|d| d.get_temp(over_id)).unwrap_or(0.0);
    let mut area = egui::ScrollArea::vertical().id_salt(("notes", m.track_key, i)).auto_shrink([false, true]);
    if auto {
        area = area.vertical_scroll_offset(progress as f32 * over);
    }
    let out = area.show(ui, |ui| {
        if by != 0.0 && !auto {
            ui.scroll_with_delta(Vec2::new(0.0, -by * line));
        }
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
    ui.data_mut(|d| d.insert_temp(over_id, (out.content_size.y - out.inner_rect.height()).max(0.0)));
}
