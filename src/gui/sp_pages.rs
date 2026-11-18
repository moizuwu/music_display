//! The middle of the Spotify layout: the page on screen (Home, Search, album, playlist, artist,
//! Liked Songs, queue; the lyrics page is the window's own lyrics panel). Pages show what
//! `sp_nav` fetched: a big header with a picture, a round play button, a table of songs, rows of
//! cards. Clicks are collected while drawing and applied afterwards.
use super::model::Model;
use super::sp_img;
use super::sp_nav::Page;
use super::style::{self, Icon};
use super::view::accent;
use crate::browser::{ListRow, TAB_QUEUE};
use egui::{Color32, FontId, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};
use music_display::layout::PanelId;
use music_display::spotify::{Head, Item, PageData};
use std::sync::Arc;

/// What a click asked for (applied once the page is drawn).
enum A {
    /// Play a song of a table (its page's context, the song).
    Play(String, Item),
    /// Play a whole card (an album, playlist, artist, or one song).
    Card(Item),
    Open(Page),
    /// The big play (or shuffle) button of a page.
    Whole(Page, bool),
    Reload,
}

impl super::Gui {
    /// The page on screen, in the middle of the window.
    pub(super) fn sp_middle(&mut self, ui: &mut Ui) {
        let page = self.m.spot.pg.nav.current().clone();
        if page == Page::Lyrics {
            self.panel(ui, PanelId::Lyrics);
            return;
        }
        let m = &mut self.m;
        let mut acts = Vec::new();
        if m.spot.on() {
            m.spot.ensure_page();
            let tabs: &[usize] = match page {
                Page::Home => &[0, 1],
                Page::Queue => &[TAB_QUEUE],
                _ => &[],
            };
            for t in tabs {
                if m.spot.lib.tabs[*t].error.is_none() {
                    m.spot.load_tab(*t, false);
                }
            }
        }
        egui::ScrollArea::vertical().id_salt(("sp-page", page.key())).auto_shrink([false, false]).show(ui, |ui| {
            if !m.spot.on() {
                style::note(ui, "Spotify is off. Turn it on in the settings (Spotify tab) to see your pages here.");
                return;
            }
            if let Some(e) = m.spot.limited() {
                style::error(ui, &e);
            }
            match &page {
                Page::Home => home(ui, m, &mut acts),
                Page::Search(q) => search(ui, m, &page, q, &mut acts),
                Page::Queue => queue(ui, m, &mut acts),
                Page::Album(_) | Page::Playlist(_) | Page::Artist(_) | Page::Liked => list_page(ui, m, &page, &mut acts),
                Page::Lyrics => {}
            }
            ui.add_space(24.0);
        });
        apply(m, acts);
    }
}

fn apply(m: &mut Model, acts: Vec<A>) {
    for a in acts {
        match a {
            A::Play(ctx, it) => m.spot.play_in(&ctx, &it),
            A::Card(it) => m.spot.play_in("", &it),
            A::Open(p) => m.spot.pg.go(p),
            A::Whole(p, shuffle) => m.spot.play_page(&p, shuffle),
            A::Reload => m.spot.reload_page(),
        }
    }
    m.take_spot_note();
}

// ── text (pure, tested) ─────────────────────────────────────────────────

/// "1 hr 5 min", "3 min 20 sec", "40 sec".
pub fn fmt_total(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{} hr {} min", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{} min {} sec", s / 60, s % 60)
    } else {
        format!("{s} sec")
    }
}

/// 1234567 → "1,234,567".
pub fn thousands(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn plural(n: u64, one: &str) -> String {
    format!("{} {one}{}", thousands(n), if n == 1 { "" } else { "s" })
}

/// The grey line of a header after the owner: "2021 · 12 songs, 45 min 3 sec" or "1,234 followers".
pub fn counts(h: &Head, shown: usize, secs: f64) -> String {
    if h.kind == "Artist" {
        return if h.count > 0 { plural(h.count, "follower") } else { String::new() };
    }
    let n = if h.count > 0 { h.count } else { shown as u64 };
    let mut parts = Vec::new();
    if !h.year.is_empty() {
        parts.push(h.year.clone());
    }
    let mut size = if n > 0 { plural(n, "song") } else { String::new() };
    if secs > 0.0 {
        size = if size.is_empty() { fmt_total(secs) } else { format!("{size}, {}", fmt_total(secs)) };
    }
    if !size.is_empty() {
        parts.push(size);
    }
    parts.join(" · ")
}

/// How big the title of a header is for its length (a long name must not fill the screen).
pub fn title_size(title: &str) -> f32 {
    match title.chars().count() {
        0..=14 => 54.0,
        15..=24 => 42.0,
        25..=40 => 30.0,
        _ => 22.0,
    }
}

/// The result to show as the "top result": the first artist, else the first song, else the first
/// album or playlist.
pub fn top_result(d: &PageData) -> Option<Item> {
    let first = |title: &str| d.sections.iter().find(|s| s.title == title).and_then(|s| s.items.first()).cloned();
    first("Artists").or_else(|| d.tracks.first().cloned()).or_else(|| first("Albums")).or_else(|| first("Playlists"))
}

/// How many cards of `want` points (and `gap` between them) fit a row `avail` wide, and how wide each
/// then is.
pub fn card_grid(avail: f32, want: f32, gap: f32) -> (usize, f32) {
    let n = (((avail + gap) / (want + gap)).floor() as usize).max(1);
    (n, ((avail - gap * (n - 1) as f32) / n as f32).max(60.0))
}

// ── small pieces ────────────────────────────────────────────────────────

fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(text).strong().size(20.0));
    ui.add_space(6.0);
}

fn on_accent(c: Color32) -> Color32 {
    if (c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114) / 1000 > 150 { Color32::BLACK } else { Color32::WHITE }
}

/// What the page has so far, with its error or "Loading…" above it.
fn status(ui: &mut Ui, m: &Model, page: &Page) -> Option<Arc<PageData>> {
    let e = m.spot.pg.entry(page);
    if let Some(err) = e.and_then(|e| e.error.as_ref()) {
        style::error(ui, err);
    }
    let data = e.and_then(|e| e.data.clone());
    if data.is_none() && e.is_none_or(|e| e.loading) {
        style::note(ui, "Loading…");
    }
    data
}

/// The big header: the picture, the kind, the title, who and how long.
fn head(ui: &mut Ui, m: &mut Model, d: &PageData, round: bool, acts: &mut Vec<A>) {
    let h = &d.head;
    let side = (ui.available_width() * 0.26).clamp(110.0, 232.0);
    let bg = ui.painter().add(egui::Shape::Noop);
    let (left, top) = (ui.max_rect().left(), ui.cursor().top());
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        sp_img::art(ui, &mut m.spot, &h.image, Vec2::splat(side), round, Sense::hover());
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.add_space((side - 140.0).max(0.0));
            ui.label(RichText::new(&h.kind).small().strong());
            ui.add(egui::Label::new(RichText::new(&h.title).size(title_size(&h.title)).strong()).wrap());
            if !h.text.is_empty() && h.kind != "Artist" {
                ui.add(egui::Label::new(RichText::new(&h.text).weak()).truncate());
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if !h.by.is_empty() {
                    let link = h.by_uri.starts_with("spotify:artist:");
                    let r = ui.add(egui::Label::new(RichText::new(&h.by).strong()).sense(if link { Sense::click() } else { Sense::hover() }));
                    if link && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        acts.push(A::Open(Page::Artist(h.by_uri.clone())));
                    }
                }
                let rest = counts(h, d.tracks.len(), d.seconds());
                let sep = if h.by.is_empty() || rest.is_empty() { "" } else { " · " };
                ui.label(RichText::new(format!("{sep}{rest}")).weak());
                if h.kind == "Artist" && !h.text.is_empty() {
                    ui.label(RichText::new(format!(" · {}", h.text)).weak());
                }
            });
        });
    });
    // the accent colour fades out behind the header, the way the app tints it from the cover
    let c = accent(ui);
    let rect = Rect::from_min_max(pos2(left, top), pos2(ui.max_rect().right(), ui.cursor().top() + 96.0));
    let mut mesh = egui::Mesh::default();
    let (c0, c1) = (Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 80), Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 0));
    for (p, col) in [(rect.left_top(), c0), (rect.right_top(), c0), (rect.right_bottom(), c1), (rect.left_bottom(), c1)] {
        mesh.colored_vertex(p, col);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().set(bg, egui::Shape::mesh(mesh));
}

/// The round play button, shuffle and reload under a header.
fn actions(ui: &mut Ui, page: &Page, acts: &mut Vec<A>) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if page.play_target().is_some() {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::click());
            let c = accent(ui);
            ui.painter().circle_filled(r.center(), if resp.hovered() { 29.0 } else { 28.0 }, c);
            style::icon_at(ui.painter(), r.center(), 56.0, Icon::Play, on_accent(c));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Play").clicked() {
                acts.push(A::Whole(page.clone(), false));
            }
            if style::toggle_button(ui, Icon::Shuffle, false).on_hover_text("Shuffle play").clicked() {
                acts.push(A::Whole(page.clone(), true));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if style::tool(ui, style::ICON_SIDE, Icon::Refresh, false, "Reload this page") {
                acts.push(A::Reload);
            }
        });
    });
    ui.add_space(6.0);
}

// ── the table of songs ──────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Cols {
    /// A small cover beside each title.
    art: bool,
    /// The album column (only when there is room, and not on the album's own page).
    album: bool,
}

struct Geom {
    num: Rect,
    title: Rect,
    album: Option<Rect>,
    dur: Rect,
}

fn geom(rect: Rect, album: bool) -> Geom {
    let num = Rect::from_min_max(pos2(rect.left() + 8.0, rect.top()), pos2(rect.left() + 48.0, rect.bottom()));
    let dur = Rect::from_min_max(pos2(rect.right() - 64.0, rect.top()), pos2(rect.right() - 10.0, rect.bottom()));
    let (l, r) = (num.right() + 4.0, dur.left() - 8.0);
    if album {
        let split = l + (r - l) * 0.56;
        Geom { num, title: Rect::from_min_max(pos2(l, rect.top()), pos2(split - 8.0, rect.bottom())), album: Some(Rect::from_min_max(pos2(split + 8.0, rect.top()), pos2(r, rect.bottom()))), dur }
    } else {
        Geom { num, title: Rect::from_min_max(pos2(l, rect.top()), pos2(r, rect.bottom())), album: None, dur }
    }
}

fn mmss(secs: f64) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn table(ui: &mut Ui, m: &mut Model, tracks: &[Item], ctx: &str, cols: Cols, acts: &mut Vec<A>) {
    let album = cols.album && ui.available_width() > 560.0;
    table_head(ui, album);
    let playing = m.playing_uri();
    for (i, it) in tracks.iter().enumerate() {
        track_row(ui, m, it, i, ctx, Cols { album, ..cols }, playing.as_deref(), acts);
    }
}

fn table_head(ui: &mut Ui, album: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let g = geom(rect, album);
    let ink = ui.visuals().weak_text_color();
    let f = FontId::proportional(12.0);
    let p = ui.painter();
    p.text(g.num.center(), egui::Align2::CENTER_CENTER, "#", f.clone(), ink);
    p.text(g.title.left_center(), egui::Align2::LEFT_CENTER, "Title", f.clone(), ink);
    if let Some(a) = g.album {
        p.text(a.left_center(), egui::Align2::LEFT_CENTER, "Album", f, ink);
    }
    // a clock for the length
    let c = pos2(g.dur.right() - 8.0, g.dur.center().y);
    p.circle_stroke(c, 6.0, egui::Stroke::new(1.2, ink));
    p.line_segment([c, c + vec2(0.0, -3.5)], egui::Stroke::new(1.2, ink));
    p.line_segment([c, c + vec2(2.5, 1.0)], egui::Stroke::new(1.2, ink));
    let y = rect.bottom() - 0.5;
    p.line_segment([pos2(rect.left() + 4.0, y), pos2(rect.right() - 4.0, y)], egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color));
}

#[allow(clippy::too_many_arguments)]
fn track_row(ui: &mut Ui, m: &mut Model, it: &Item, i: usize, ctx: &str, cols: Cols, playing: Option<&str>, acts: &mut Vec<A>) {
    let h = if cols.art { 56.0 } else { 48.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    // the songs of a page play in its context; a playlist's menu can also remove them from it
    super::sp_actions::item_menu(&resp, m, it, (!ctx.is_empty()).then_some(ctx));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let hov = ui.rect_contains_pointer(rect);
    if hov {
        ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.fg_stroke.color.gamma_multiply(0.10));
    }
    let g = geom(rect, cols.album);
    let now = playing == Some(it.uri.as_str());
    let (ink, weak, acc) = (ui.visuals().text_color(), ui.visuals().weak_text_color(), accent(ui));
    // the number; the play sign under the pointer, an accent one for the song playing
    let num = ui.interact(g.num, resp.id.with("n"), Sense::click());
    if hov {
        style::icon_at(ui.painter(), g.num.center(), 22.0, Icon::PlayRow, ink);
        num.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    } else if now {
        style::icon_at(ui.painter(), g.num.center(), 22.0, Icon::PlayRow, acc);
    } else {
        ui.painter().text(g.num.center(), egui::Align2::CENTER_CENTER, (i + 1).to_string(), FontId::proportional(14.0), weak);
    }
    // the title and the artist under it
    let mut text = g.title;
    if cols.art {
        let a = Rect::from_min_size(pos2(text.left(), rect.center().y - 20.0), Vec2::splat(40.0));
        sp_img::paint_art(ui, &mut m.spot, &it.image, a, false);
        text.min.x = a.right() + 10.0;
    }
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::top_down(egui::Align::Min)));
    c.add_space(((h - 36.0) / 2.0).max(0.0));
    let name = RichText::new(&it.name).size(15.0);
    c.add(egui::Label::new(if now { name.color(acc) } else { name }).truncate());
    let by = c.add(egui::Label::new(RichText::new(&it.sub).small().weak()).truncate().sense(if it.artist_uri.is_empty() { Sense::hover() } else { Sense::click() }));
    if !it.artist_uri.is_empty() && by.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        acts.push(A::Open(Page::Artist(it.artist_uri.clone())));
    }
    if let Some(a) = g.album {
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(a).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let r = c.add(egui::Label::new(RichText::new(&it.album).weak()).truncate().sense(if it.album_uri.is_empty() { Sense::hover() } else { Sense::click() }));
        if !it.album_uri.is_empty() && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            acts.push(A::Open(Page::Album(it.album_uri.clone())));
        }
    }
    if it.duration > 0.0 {
        ui.painter().text(pos2(g.dur.right(), g.dur.center().y), egui::Align2::RIGHT_CENTER, mmss(it.duration), FontId::proportional(14.0), weak);
    }
    if num.clicked() || resp.double_clicked() {
        acts.push(A::Play(ctx.to_string(), it.clone()));
    }
}

// ── cards ───────────────────────────────────────────────────────────────

/// Cards in rows as wide as the window allows; `one_row` shows only what fits the first row.
fn cards(ui: &mut Ui, m: &mut Model, items: &[Item], one_row: bool, acts: &mut Vec<A>) {
    let gap = 14.0;
    let (n, w) = card_grid(ui.available_width(), 140.0, gap);
    let shown = if one_row { &items[..items.len().min(n)] } else { items };
    for row in shown.chunks(n) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for it in row {
                card(ui, m, it, w, acts);
            }
        });
        ui.add_space(gap);
    }
}

fn card(ui: &mut Ui, m: &mut Model, it: &Item, w: f32, acts: &mut Vec<A>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, w + 50.0), Sense::click());
    super::sp_actions::item_menu(&resp, m, it, None);
    let hov = ui.rect_contains_pointer(rect);
    let base = ui.visuals().widgets.hovered.fg_stroke.color;
    ui.painter().rect_filled(rect, 8.0, base.gamma_multiply(if hov { 0.12 } else { 0.05 }));
    let art = Rect::from_min_size(rect.min + vec2(10.0, 10.0), Vec2::splat(w - 20.0));
    sp_img::paint_art(ui, &mut m.spot, &it.image, art, it.kind == "artist");
    let btn = Rect::from_center_size(art.right_bottom() - vec2(26.0, 26.0), Vec2::splat(44.0));
    let play = ui.interact(btn, resp.id.with("play"), Sense::click());
    if hov {
        let c = accent(ui);
        ui.painter().circle_filled(btn.center(), if play.hovered() { 22.0 } else { 20.0 }, c);
        style::icon_at(ui.painter(), btn.center(), 44.0, Icon::PlayRow, on_accent(c));
        play.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    let text = Rect::from_min_max(pos2(rect.left() + 10.0, art.bottom() + 6.0), pos2(rect.right() - 10.0, rect.bottom()));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::top_down(egui::Align::Min)));
    c.add(egui::Label::new(RichText::new(&it.name).strong()).truncate());
    c.add(egui::Label::new(RichText::new(&it.sub).small().weak()).truncate());
    if play.clicked() {
        acts.push(A::Card(it.clone()));
    } else if resp.clicked() {
        acts.push(match Page::of(it) {
            Some(p) => A::Open(p),
            None => A::Card(it.clone()),
        });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
}

// ── the pages ───────────────────────────────────────────────────────────

fn list_page(ui: &mut Ui, m: &mut Model, page: &Page, acts: &mut Vec<A>) {
    let Some(d) = status(ui, m, page) else { return };
    let artist = matches!(page, Page::Artist(_));
    head(ui, m, &d, artist, acts);
    actions(ui, page, acts);
    for n in &d.notes {
        style::note(ui, n);
    }
    if artist {
        if !d.tracks.is_empty() {
            section_title(ui, "Popular");
            let id = ui.scope_id().with(("sp-more", page.key()));
            let more = ui.data(|x| x.get_temp::<bool>(id)).unwrap_or(false);
            let n = if more { d.tracks.len() } else { d.tracks.len().min(5) };
            table(ui, m, &d.tracks[..n], "", Cols { art: true, album: false }, acts);
            if d.tracks.len() > 5 && ui.button(if more { "Show less" } else { "Show more" }).clicked() {
                ui.data_mut(|x| x.insert_temp(id, !more));
            }
        }
        for s in &d.sections {
            section_title(ui, &s.title);
            if !s.error.is_empty() {
                style::note(ui, &s.error);
            }
            cards(ui, m, &s.items, false, acts);
        }
    } else if d.tracks.is_empty() {
        if d.notes.is_empty() {
            style::note(ui, "No songs here.");
        }
    } else {
        let album = matches!(page, Page::Album(_));
        table(ui, m, &d.tracks, &page.context(), Cols { art: !album, album: !album }, acts);
    }
}

fn home(ui: &mut Ui, m: &mut Model, acts: &mut Vec<A>) {
    ui.add_space(8.0);
    ui.label(RichText::new("Home").strong().size(30.0));
    let data = status(ui, m, &Page::Home);
    if let Some(d) = &data {
        for n in &d.notes {
            style::note(ui, n);
        }
    }
    let recent: Vec<Item> = m.spot.lib.tabs[0].items.clone();
    if let Some(e) = &m.spot.lib.tabs[0].error {
        style::note(ui, &format!("Recently played: {e}"));
    }
    if !recent.is_empty() {
        section_title(ui, "Recently played");
        cards(ui, m, &recent, true, acts);
    }
    if let Some(d) = &data {
        for s in &d.sections {
            section_title(ui, &s.title);
            if !s.error.is_empty() {
                style::note(ui, &s.error);
            }
            cards(ui, m, &s.items, true, acts);
        }
        if !d.tracks.is_empty() {
            section_title(ui, "Your top songs");
            cards(ui, m, &d.tracks, true, acts);
        }
    }
    let lists: Vec<Item> = m.spot.lib.tabs[1].items.clone();
    if !lists.is_empty() {
        section_title(ui, "Your playlists");
        cards(ui, m, &lists, true, acts);
    }
}

fn search(ui: &mut Ui, m: &mut Model, page: &Page, q: &str, acts: &mut Vec<A>) {
    ui.add_space(8.0);
    ui.label(RichText::new(format!("Results for “{q}”")).strong().size(24.0));
    let Some(d) = status(ui, m, page) else { return };
    if d.tracks.is_empty() && d.sections.is_empty() {
        style::note(ui, "Nothing found.");
        return;
    }
    let avail = ui.available_width();
    let top = top_result(&d);
    let songs: Vec<Item> = d.tracks.iter().take(5).cloned().collect();
    if avail > 640.0 {
        let w1 = (avail * 0.36).min(340.0);
        let w2 = avail - w1 - 20.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.vertical(|ui| {
                ui.set_width(w1);
                section_title(ui, "Top result");
                if let Some(t) = &top {
                    top_card(ui, m, t, w1, acts);
                }
            });
            ui.vertical(|ui| {
                ui.set_width(w2);
                section_title(ui, "Songs");
                table(ui, m, &songs, "", Cols { art: true, album: false }, acts);
            });
        });
    } else {
        section_title(ui, "Top result");
        if let Some(t) = &top {
            top_card(ui, m, t, avail.min(340.0), acts);
        }
        section_title(ui, "Songs");
        table(ui, m, &songs, "", Cols { art: true, album: false }, acts);
    }
    for title in ["Artists", "Albums", "Playlists"] {
        if let Some(s) = d.sections.iter().find(|s| s.title == title) {
            section_title(ui, title);
            cards(ui, m, &s.items, true, acts);
        }
    }
}

/// The big card of the best match.
fn top_card(ui: &mut Ui, m: &mut Model, it: &Item, w: f32, acts: &mut Vec<A>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 190.0), Sense::click());
    super::sp_actions::item_menu(&resp, m, it, None);
    let hov = ui.rect_contains_pointer(rect);
    let base = ui.visuals().widgets.hovered.fg_stroke.color;
    ui.painter().rect_filled(rect, 8.0, base.gamma_multiply(if hov { 0.12 } else { 0.05 }));
    let art = Rect::from_min_size(rect.min + vec2(16.0, 16.0), Vec2::splat(92.0));
    sp_img::paint_art(ui, &mut m.spot, &it.image, art, it.kind == "artist");
    let text = Rect::from_min_max(pos2(rect.left() + 16.0, art.bottom() + 12.0), pos2(rect.right() - 16.0, rect.bottom()));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::top_down(egui::Align::Min)));
    c.add(egui::Label::new(RichText::new(&it.name).strong().size(26.0)).truncate());
    let kind = match it.kind.as_str() {
        "track" => format!("Song · {}", it.sub),
        "artist" => "Artist".to_string(),
        "album" => format!("Album · {}", it.sub),
        _ => format!("Playlist · {}", it.sub),
    };
    c.add(egui::Label::new(RichText::new(kind).weak()).truncate());
    let btn = Rect::from_center_size(rect.right_bottom() - vec2(38.0, 38.0), Vec2::splat(44.0));
    let play = ui.interact(btn, resp.id.with("play"), Sense::click());
    if hov {
        let col = accent(ui);
        ui.painter().circle_filled(btn.center(), 22.0, col);
        style::icon_at(ui.painter(), btn.center(), 44.0, Icon::PlayRow, on_accent(col));
    }
    if play.clicked() {
        acts.push(A::Card(it.clone()));
    } else if resp.clicked() {
        acts.push(match Page::of(it) {
            Some(p) => A::Open(p),
            None => A::Card(it.clone()),
        });
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
}

fn queue(ui: &mut Ui, m: &mut Model, acts: &mut Vec<A>) {
    ui.add_space(8.0);
    ui.label(RichText::new("Queue").strong().size(30.0));
    let l = &m.spot.lib.tabs[TAB_QUEUE];
    if let Some(e) = &l.error {
        style::error(ui, e);
    }
    if l.loading && l.items.is_empty() {
        style::note(ui, "Loading…");
    }
    let rows = l.rows();
    let items: Vec<Item> = l.items.clone();
    let playing = m.playing_uri();
    let cols = Cols { art: true, album: true };
    table_head(ui, ui.available_width() > 560.0);
    for r in rows {
        match r {
            ListRow::Head(h) => section_title(ui, h),
            ListRow::Item(i) => {
                let album = ui.available_width() > 560.0;
                track_row(ui, m, &items[i], i, "", Cols { album, ..cols }, playing.as_deref(), acts);
            }
        }
    }
    if items.is_empty() && !m.spot.lib.tabs[TAB_QUEUE].loading {
        style::note(ui, "Nothing in the queue.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use music_display::spotify::Section;

    #[test]
    fn lengths_read_like_the_apps() {
        assert_eq!(fmt_total(40.0), "40 sec");
        assert_eq!(fmt_total(200.0), "3 min 20 sec");
        assert_eq!(fmt_total(3900.0), "1 hr 5 min");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn the_header_line_counts_what_the_page_has() {
        let mut h = Head { kind: "Album".into(), year: "2021".into(), count: 12, ..Default::default() };
        assert_eq!(counts(&h, 12, 2703.0), "2021 · 12 songs, 45 min 3 sec");
        h.count = 1;
        h.year.clear();
        assert_eq!(counts(&h, 1, 0.0), "1 song");
        // the API's count is missing: what was listed counts
        h.count = 0;
        assert_eq!(counts(&h, 3, 90.0), "3 songs, 1 min 30 sec");
        assert_eq!(counts(&h, 0, 0.0), "");
        let a = Head { kind: "Artist".into(), count: 1234, ..Default::default() };
        assert_eq!(counts(&a, 0, 0.0), "1,234 followers");
    }

    #[test]
    fn long_titles_get_smaller() {
        assert!(title_size("Short") > title_size("A title that is rather long!"));
        assert!(title_size("x".repeat(80).as_str()) < title_size("A title that is rather long!"));
    }

    #[test]
    fn the_top_result_is_an_artist_before_a_song() {
        let it = |kind: &str, n: &str| Item { kind: kind.into(), name: n.into(), uri: n.into(), ..Default::default() };
        let mut d = PageData { tracks: vec![it("track", "t")], ..Default::default() };
        d.sections.push(Section { title: "Albums".into(), items: vec![it("album", "al")], error: String::new() });
        assert_eq!(top_result(&d).unwrap().name, "t");
        d.sections.push(Section { title: "Artists".into(), items: vec![it("artist", "ar")], error: String::new() });
        assert_eq!(top_result(&d).unwrap().name, "ar");
        d.tracks.clear();
        d.sections.remove(1);
        assert_eq!(top_result(&d).unwrap().name, "al");
        assert!(top_result(&PageData::default()).is_none());
    }

    #[test]
    fn the_card_grid_fills_the_width() {
        let (n, w) = card_grid(1000.0, 168.0, 14.0);
        assert_eq!(n, 5);
        assert!((n as f32 * w + 14.0 * (n - 1) as f32 - 1000.0).abs() < 0.01);
        assert_eq!(card_grid(100.0, 168.0, 14.0).0, 1, "at least one card");
        assert!(card_grid(10.0, 168.0, 14.0).1 >= 60.0);
    }

    #[test]
    fn columns_share_the_row() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 48.0));
        let g = geom(r, true);
        let a = g.album.unwrap();
        assert!(g.title.right() < a.left() && a.right() < g.dur.left());
        assert!(geom(r, false).album.is_none());
        assert!(geom(r, false).title.width() > g.title.width());
    }
}
