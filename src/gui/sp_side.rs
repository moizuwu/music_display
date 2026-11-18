//! The Spotify layout's library column, the way the Spotify app has it: a "Your Library" header,
//! filter chips (Playlists / Albums / Artists), Liked Songs pinned on top and rows with a square
//! (round for artists) picture, the name and a line with the type and owner.
use super::model::Model;
use super::sp_img;
use super::sp_nav::Page;
use super::style::{self, Icon};
use super::view::accent;
use crate::browser::{TAB_ALBUMS, TAB_ARTISTS, TAB_PLAYLISTS};
use egui::{Rect, RichText, Sense, Ui, Vec2};
use music_display::spotify::Item;

const ROW_H: f32 = 56.0;
const THUMB: f32 = 44.0;
/// The chips and the tabs of the library they show.
const CHIPS: [(&str, usize); 3] = [("Playlists", TAB_PLAYLISTS), ("Albums", TAB_ALBUMS), ("Artists", TAB_ARTISTS)];

/// The tabs the chips ask for: the ones switched on, or all of them when none is.
pub fn wanted_tabs(chips: &[bool; 3]) -> Vec<usize> {
    let on: Vec<usize> = CHIPS.iter().zip(chips).filter(|(_, on)| **on).map(|((_, t), _)| *t).collect();
    if on.is_empty() { CHIPS.iter().map(|(_, t)| *t).collect() } else { on }
}

/// The line under a name: "Playlist · 42 songs", "Album · The Band", "Artist".
pub fn sub_line(it: &Item) -> String {
    match it.kind.as_str() {
        "playlist" => {
            let by = it.sub.strip_prefix("by ").unwrap_or(&it.sub);
            if by.is_empty() { "Playlist".into() } else { format!("Playlist · {by}") }
        }
        "album" if !it.sub.is_empty() => format!("Album · {}", it.sub),
        "album" => "Album".into(),
        "artist" => "Artist".into(),
        _ => it.sub.clone(),
    }
}

/// Does this row stand for the page on screen?
fn is_page(it: &Item, cur: &Page) -> bool {
    Page::of(it).as_ref() == Some(cur)
}

enum Act {
    Open(Item),
    Play(Item),
}

pub fn library_column(ui: &mut Ui, m: &mut Model) {
    let mut act: Option<Act> = None;
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new("Your Library").strong().size(16.0)).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if m.spot.on() && style::tool(ui, style::ROW_SIDE, Icon::Refresh, false, "Reload the library") {
                for t in wanted_tabs(&[false; 3]) {
                    m.spot.load_tab(t, true);
                }
            }
        });
    });
    if !m.spot.on() {
        style::note(ui, "Spotify is off. Turn it on in the settings (Spotify tab) to see your library here.");
        return;
    }
    // new playlist (and rename / delete the one on screen), under the header like Spotify's "Create"
    super::sp_actions::playlist_tools(ui, m);
    {
    }
    ui.horizontal_wrapped(|ui| {
        for (i, (name, _)) in CHIPS.iter().enumerate() {
            let on = m.spot.pg.chips[i];
            if ui.add(egui::Button::new(*name).selected(on).corner_radius(16.0)).clicked() {
                m.spot.pg.chips[i] = !on;
            }
        }
    });
    if let Some(e) = m.spot.limited().or_else(|| m.spot.status.clone()) {
        ui.label(RichText::new(e).small().weak());
    }
    // the lists load when they are shown (not again after a failure: Reload asks)
    let tabs = wanted_tabs(&m.spot.pg.chips);
    for t in &tabs {
        if m.spot.lib.tabs[*t].error.is_none() {
            m.spot.load_tab(*t, false);
        }
    }
    let cur = m.spot.pg.nav.current().clone();
    // Liked Songs, pinned
    let liked = Item { kind: "liked".into(), uri: "liked".into(), name: "Liked Songs".into(), sub: "Playlist".into(), openable: true, ..Default::default() };
    row(ui, m, &liked, &cur, &mut act);
    let mut shown: Vec<Item> = Vec::new();
    for t in &tabs {
        shown.extend(m.spot.lib.tabs[*t].items.iter().cloned());
    }
    if tabs.iter().any(|t| m.spot.lib.tabs[*t].loading) && shown.is_empty() {
        style::note(ui, "Loading…");
    }
    for t in &tabs {
        if let Some(e) = &m.spot.lib.tabs[*t].error {
            style::error(ui, e);
        }
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for it in &shown {
            row(ui, m, it, &cur, &mut act);
        }
    });
    match act {
        Some(Act::Open(it)) => m.spot.open_page(&it),
        Some(Act::Play(it)) => m.spot.play_in("", &it),
        None => {}
    }
}

/// One row: picture, name, type and owner; a click opens its page, the play sign on the picture
/// (shown under the pointer) plays it.
fn row(ui: &mut Ui, m: &mut Model, it: &Item, cur: &Page, act: &mut Option<Act>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    super::sp_actions::item_menu(&resp, m, it, None);
    let hovered = ui.rect_contains_pointer(rect);
    if hovered || is_page(it, cur) {
        let a = if hovered { 0.10 } else { 0.06 };
        ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.fg_stroke.color.gamma_multiply(a));
    }
    let thumb = Rect::from_min_size(rect.left_top() + Vec2::new(6.0, (ROW_H - THUMB) / 2.0), Vec2::splat(THUMB));
    if it.kind == "liked" {
        let fill = accent(ui).gamma_multiply(0.85);
        ui.painter().rect_filled(thumb, 4.0, fill);
        style::icon_at(ui.painter(), thumb.center(), THUMB * 0.6, Icon::Heart(true), ui.visuals().strong_text_color());
    } else {
        sp_img::paint_art(ui, &mut m.spot, &it.image, thumb, it.kind == "artist");
    }
    let play = ui.interact(thumb, resp.id.with("play"), Sense::click());
    if hovered {
        ui.painter().rect_filled(thumb, if it.kind == "artist" { THUMB / 2.0 } else { 4.0 }, egui::Color32::from_black_alpha(130));
        style::icon_at(ui.painter(), thumb.center(), THUMB * 0.7, Icon::PlayRow, egui::Color32::WHITE);
        play.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    let text = Rect::from_min_max(thumb.right_top() + Vec2::new(10.0, 0.0), rect.right_bottom() - Vec2::new(4.0, 0.0));
    let mut c = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::top_down(egui::Align::Min)));
    c.add_space(5.0);
    let playing_here = is_page(it, cur);
    let name = RichText::new(&it.name);
    c.add(egui::Label::new(if playing_here { name.color(accent(&c)) } else { name }).truncate());
    c.add(egui::Label::new(RichText::new(sub_line(it)).small().weak()).truncate());
    if play.clicked() {
        *act = Some(Act::Play(it.clone()));
    } else if resp.clicked() {
        *act = Some(Act::Open(it.clone()));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(kind: &str, sub: &str) -> Item {
        Item { kind: kind.into(), sub: sub.into(), ..Default::default() }
    }

    #[test]
    fn chips_filter_and_none_means_all() {
        assert_eq!(wanted_tabs(&[false; 3]), [TAB_PLAYLISTS, TAB_ALBUMS, TAB_ARTISTS]);
        assert_eq!(wanted_tabs(&[false, true, false]), [TAB_ALBUMS]);
        assert_eq!(wanted_tabs(&[true, false, true]), [TAB_PLAYLISTS, TAB_ARTISTS]);
    }

    #[test]
    fn the_line_under_a_name_says_what_it_is() {
        assert_eq!(sub_line(&it("playlist", "42 songs")), "Playlist · 42 songs");
        assert_eq!(sub_line(&it("playlist", "by Lena")), "Playlist · Lena");
        assert_eq!(sub_line(&it("playlist", "")), "Playlist");
        assert_eq!(sub_line(&it("album", "The Band")), "Album · The Band");
        assert_eq!(sub_line(&it("album", "")), "Album");
        assert_eq!(sub_line(&it("artist", "Artist")), "Artist");
    }

    #[test]
    fn the_row_of_the_open_page_is_marked() {
        let a = Item { kind: "album".into(), uri: "spotify:album:1".into(), ..Default::default() };
        assert!(is_page(&a, &Page::Album("spotify:album:1".into())));
        assert!(!is_page(&a, &Page::Home));
        assert!(is_page(&Item { kind: "liked".into(), ..Default::default() }, &Page::Liked));
    }
}
