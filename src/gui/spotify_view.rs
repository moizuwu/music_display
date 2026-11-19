//! The experimental Spotify layout (`spotify_layout` in `[gui]`), arranged like the Spotify app:
//! a strip along the top (back / forward, home, search), your library on the left, the page in the
//! middle (home, search, album, playlist, artist, Liked Songs, queue or the lyrics), the song on
//! the right and a player bar along the bottom. The panels themselves are the window's own.
use super::sp_look::SpLook;
use super::sp_nav::Page;
use super::sp_side;
use super::style::{self, Icon};
use super::{controls, library, view};
use egui::{RichText, Sense, Ui};
use music_display::guiconf::GuiConf;
use music_display::layout::{Item, MetaField};

/// The height of the player bar.
const BAR_H: f32 = 84.0;

impl super::Gui {
    pub(super) fn spotify_view(&mut self, ui: &mut egui::Ui, g: &GuiConf) {
        // "Go to album" / "Go to artist" from a row menu
        if let Some(p) = self.m.spot.pending_nav.take().and_then(|u| Page::from_uri(&u)) {
            self.m.spot.pg.go(p);
        }
        let lk = SpLook::get(ui, g);
        // the Spotify look shows the queue as the panel on the right, never as a page
        if lk.on && *self.m.spot.pg.nav.current() == Page::Queue {
            set_queue_open(ui.ctx(), true);
            if self.m.spot.pg.nav.can_back() {
                self.m.spot.pg.back();
            } else {
                self.m.spot.pg.go(Page::Home);
            }
        }
        // the side columns have a fixed share (the lists in them would otherwise take all the width)
        let side_w = if lk.on { lk.side_w } else { (ui.available_width() * 0.26).clamp(220.0, 420.0) };
        let bar_h = if lk.on { lk.bar_h } else { BAR_H };
        egui::Panel::bottom("sp-bar").show_separator_line(false).frame(style::gutter(g)).resizable(false).exact_size(bar_h).show(ui, |ui| {
            let bar = |me: &mut Self, ui: &mut Ui| {
                ui.columns(3, |c| {
                    me.bar_left(&mut c[0], g, &lk);
                    controls::bar_center(&mut c[1], &mut me.m);
                    me.bar_right(&mut c[2]);
                });
            };
            // Spotify's player bar sits on the black window, not on a card
            if lk.on {
                bar(self, ui);
            } else {
                style::card(ui, g, |ui| bar(self, ui));
            }
        });
        egui::Panel::left("sp-library").show_separator_line(false).frame(style::gutter(g)).resizable(false).exact_size(side_w).show(ui, |ui| {
            style::card_at(ui, g, style::Split { after: true, ..style::Split::NONE }, |ui| {
                sp_side::library_column(ui, &mut self.m);
            });
        });
        let queue = lk.on && queue_open(ui.ctx()) && self.m.spot.on();
        if queue {
            self.m.spot.load_tab(crate::browser::TAB_QUEUE, false);
            self.m.spot.load_tab(0, false);
        }
        if g.spotify_now_playing || queue {
            egui::Panel::right("sp-song").show_separator_line(false).frame(style::gutter(g)).resizable(false).exact_size(side_w).show(ui, |ui| {
                style::card_at(ui, g, style::Split { before: true, ..style::Split::NONE }, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        if queue {
                            self.queue_panel(ui, &lk);
                        } else {
                            self.now_playing(ui, g, &lk);
                        }
                    });
                });
            });
        }
        egui::CentralPanel::default().frame(style::gutter(g)).show(ui, |ui| {
            let split = style::Split { before: true, after: g.spotify_now_playing || queue, vertical: false };
            style::card_at(ui, g, split, |ui| self.sp_middle(ui));
        });
    }

    /// The "Now playing" column, the way Spotify has it: the album at the top, the cover, the
    /// title big and bold with the artist under it and the saved mark, then quiet lines.
    fn now_playing(&mut self, ui: &mut Ui, g: &GuiConf, lk: &SpLook) {
        let Some(s) = self.m.snap.clone() else {
            style::note(ui, "Nothing is playing.");
            return;
        };
        if lk.on && !s.album.is_empty() {
            ui.label(RichText::new(&s.album).font(lk.bold(lk.body)).color(lk.text));
            ui.add_space(10.0);
        }
        view::cover(ui, self.cover.as_ref(), g.corner_radius, super::fun::vinyl_angle(ui.ctx(), &self.m));
        ui.add_space(if lk.on { 14.0 } else { g.panel_spacing as f32 });
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - style::ICON_SIDE - 8.0).max(60.0));
                let (t, a) = if lk.on {
                    (RichText::new(&s.title).font(lk.bold(lk.heading)).color(lk.text), RichText::new(&s.artist).font(lk.font(lk.body)).color(lk.sub))
                } else {
                    (RichText::new(&s.title).size(22.0).strong(), RichText::new(&s.artist).size(15.0).weak())
                };
                ui.add(egui::Label::new(t).wrap());
                ui.add(egui::Label::new(a).wrap());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.saved_button(ui, lk));
        });
        ui.add_space(8.0);
        for f in [MetaField::Release, MetaField::Genre] {
            if let Some(v) = view::field_value(&self.m, &s, f).filter(|v| !v.is_empty() && v != "…" && v != "...") {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(f.label()).size(lk.small).color(lk.sub));
                    ui.add(egui::Label::new(RichText::new(v).size(lk.small).color(lk.text)).truncate());
                });
            }
        }
    }

    /// Spotify's queue panel: Queue / Recently played tabs, the song playing (its title green), then
    /// "Next from" and what comes, each row a small cover with the title and the artists.
    fn queue_panel(&mut self, ui: &mut Ui, lk: &SpLook) {
        let id = egui::Id::unique("sp-queue-recent");
        let recent = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        ui.horizontal(|ui| {
            for (i, name) in ["Queue", "Recently played"].iter().enumerate() {
                let on = recent == (i == 1);
                let r = ui.add(egui::Label::new(RichText::new(*name).font(lk.bold(lk.body + 1.0)).color(if on { lk.text } else { lk.sub })).sense(Sense::click()));
                if on {
                    let y = r.rect.bottom() + 4.0;
                    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(2.0, lk.green));
                }
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    ui.data_mut(|d| d.insert_temp(id, i == 1));
                }
                ui.add_space(10.0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if style::icon_ink(ui, style::ROW_SIDE, Icon::Close, lk.sub, "Close") {
                    set_queue_open(ui.ctx(), false);
                }
            });
        });
        ui.add_space(14.0);
        let tab = if recent { 0 } else { crate::browser::TAB_QUEUE };
        let items: Vec<music_display::spotify::Item> = self.m.spot.lib.tabs[tab].items.clone();
        if items.is_empty() {
            style::note(ui, if self.m.spot.lib.tabs[tab].loading { "Loading…" } else { "Nothing here." });
            return;
        }
        let playing = self.m.playing_uri();
        let mut rest = items.as_slice();
        if !recent {
            if let Some((first, more)) = items.split_first().filter(|(f, _)| Some(&f.uri) == playing.as_ref()) {
                ui.label(RichText::new("Now playing").font(lk.bold(lk.body + 2.0)).color(lk.text));
                ui.add_space(6.0);
                self.queue_row(ui, lk, first, true);
                ui.add_space(14.0);
                rest = more;
            }
            if !rest.is_empty() {
                let from = self.m.snap.as_ref().map(|s| s.album.clone()).filter(|a| !a.is_empty());
                let head = from.map_or("Next up".to_string(), |a| format!("Next from: {a}"));
                ui.label(RichText::new(head).font(lk.bold(lk.body + 2.0)).color(lk.text));
                ui.add_space(6.0);
            }
        }
        for it in rest {
            self.queue_row(ui, lk, it, false);
        }
    }

    /// One row of the queue panel.
    fn queue_row(&mut self, ui: &mut Ui, lk: &SpLook, it: &music_display::spotify::Item, now: bool) {
        let h = 50.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::click());
        super::sp_actions::item_menu(&resp, &mut self.m, it, None);
        if resp.hovered() || now {
            ui.painter().rect_filled(rect, 6.0, egui::Color32::from_white_alpha(if resp.hovered() { 16 } else { 10 }));
        }
        let art = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 5.0), egui::Vec2::splat(40.0));
        super::sp_img::paint_art(ui, &mut self.m.spot, &it.image, art, false);
        let text = egui::Rect::from_min_max(egui::pos2(art.right() + 10.0, rect.top() + 6.0), rect.right_bottom());
        let mut c = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::top_down(egui::Align::Min)));
        c.add(egui::Label::new(RichText::new(&it.name).font(lk.font(lk.body)).color(if now { lk.green } else { lk.text })).truncate());
        c.add(egui::Label::new(RichText::new(&it.sub).font(lk.font(lk.small)).color(lk.sub)).truncate());
    }

    /// Like / unlike: the heart in the normal look; Spotify's green check (saved) or a plus.
    fn saved_button(&mut self, ui: &mut Ui, lk: &SpLook) {
        if !lk.on {
            library::heart_button(ui, &mut self.m);
            return;
        }
        let uri = self.m.playing_uri();
        if !self.m.spot.on() || uri.is_none() {
            return;
        }
        let on = self.m.spot.liked == Some(true);
        let (icon, ink, tip) = if on { (Icon::Check, lk.green, "Remove from Liked Songs") } else { (Icon::Plus, lk.sub, "Save to Liked Songs") };
        if style::icon_ink(ui, style::ROW_SIDE, icon, ink, tip) {
            self.m.spot.toggle_like(uri);
        }
    }

    /// Back, forward, home and the search field: in the window's top bar with this layout.
    pub(super) fn top_strip(&mut self, ui: &mut Ui) {
        let lk = SpLook::get(ui, &self.m.gui);
        let pg = &mut self.m.spot.pg;
        let (can_back, can_fwd) = (pg.nav.can_back(), pg.nav.can_forward());
        let at_home = *pg.nav.current() == Page::Home;
        let field_w = (ui.available_width() * 0.4).clamp(180.0, if lk.on { 480.0 } else { 440.0 });
        let gap = ui.spacing().item_spacing.x;
        let side = if lk.on { 44.0 } else { style::ICON_SIDE };
        let center_w = side + gap + field_w;
        let mut search = false;
        ui.horizontal_centered(|ui| {
            ui.add_enabled_ui(can_back, |ui| {
                if style::tool(ui, style::ICON_SIDE, Icon::Back, false, "Back") {
                    pg.back();
                }
            });
            ui.add_enabled_ui(can_fwd, |ui| {
                if style::tool(ui, style::ICON_SIDE, Icon::Open, false, "Forward") {
                    pg.forward();
                }
            });
            // home and the search field centred on the window, as in Spotify (not on what is left
            // of the bar), when there is room for that
            let mid = ui.ctx().content_rect().center().x;
            let want = mid - center_w / 2.0 - ui.cursor().min.x;
            let room = ui.available_width() - center_w - 8.0;
            ui.add_space(want.min(room).max(8.0));
            let home = if lk.on {
                // Spotify: a round dark button, the house white (green-free even when at home)
                style::round_button(ui, side, Icon::Home, lk.raised, if at_home { lk.text } else { lk.sub }, "Home").clicked()
            } else {
                style::tool(ui, side, Icon::Home, at_home, "Home")
            };
            if home {
                pg.go(Page::Home);
            }
            if lk.on {
                // a pill with the magnifier inside on the left, the text after it, ✕ to clear
                let (rect, _) = ui.allocate_exact_size(egui::vec2(field_w - side - gap, side), Sense::hover());
                ui.painter().rect_filled(rect, side / 2.0, lk.raised);
                let mag = egui::Rect::from_center_size(egui::pos2(rect.left() + side / 2.0 + 2.0, rect.center().y), egui::Vec2::splat(style::ICON_SIDE));
                style::paint_icon(ui.painter(), mag.center(), style::ICON_SIDE, Icon::Search, lk.sub);
                let clear_w = if pg.query.is_empty() { 0.0 } else { side };
                let text = egui::Rect::from_min_max(egui::pos2(mag.right() + 4.0, rect.top()), egui::pos2(rect.right() - clear_w - 8.0, rect.bottom()));
                let r = ui.put(
                    text,
                    egui::TextEdit::singleline(&mut pg.query)
                        .hint_text(RichText::new("What do you want to play?").color(lk.sub))
                        .frame(egui::Frame::NONE)
                        .font(lk.font(15.0))
                        .text_color(lk.text)
                        .align(egui::Align2::LEFT_CENTER),
                );
                if std::mem::take(&mut pg.focus_search) {
                    r.request_focus();
                }
                search = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !pg.query.trim().is_empty();
                if clear_w > 0.0 {
                    let x = egui::Rect::from_center_size(egui::pos2(rect.right() - side / 2.0 - 4.0, rect.center().y), egui::Vec2::splat(style::ROW_SIDE));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(x));
                    if style::icon_ink(&mut child, style::ROW_SIDE, Icon::Close, lk.sub, "Clear") {
                        pg.query.clear();
                    }
                }
                return;
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut pg.query)
                    .hint_text("What do you want to play?")
                    .desired_width(field_w - side - gap)
                    .margin(egui::vec2(10.0, 6.0))
                    .align(egui::Align2::LEFT_CENTER),
            );
            if std::mem::take(&mut pg.focus_search) {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let go = !pg.query.trim().is_empty() && style::tool(ui, side, Icon::Search, false, "Search");
            search = enter || go;
        });
        if search {
            self.m.spot.search_page();
        }
    }

    /// The player bar, left part: the small cover, the song's details as the settings list them
    /// (`spotify_bar_cover`, `spotify_bar_fields`), and the heart.
    fn bar_left(&mut self, ui: &mut egui::Ui, g: &GuiConf, lk: &SpLook) {
        let snap = self.m.snap.clone();
        let mut open: Option<MetaField> = None;
        ui.horizontal_centered(|ui| {
            if g.spotify_bar_cover {
                if let Some(tex) = &self.cover {
                    let spin = super::fun::vinyl_angle(ui.ctx(), &self.m);
                    super::fun::cover_image(ui, tex, if lk.on { 48.0 } else { 56.0 }, egui::CornerRadius::same(4), spin);
                }
            }
            if let Some(s) = &snap {
                let lines = bar_lines(g, |f| view::field_value(&self.m, s, f));
                // (room for the heart after the text)
                let room = (ui.available_width() - if self.m.spot.on() { 40.0 } else { 0.0 }).max(40.0);
                ui.vertical(|ui| {
                    ui.set_max_width(room);
                    for (i, (f, v)) in lines.iter().enumerate() {
                        let text = match (i, lk.on) {
                            (0, true) => RichText::new(v).font(lk.bold(lk.body)).color(lk.text),
                            (_, true) => RichText::new(v).size(11.0).color(lk.sub),
                            (0, false) => RichText::new(v).strong().size(15.0),
                            _ => RichText::new(v).small().weak(),
                        };
                        let link = self.m.spot.on() && bar_link(*f).is_some();
                        let r = ui.add(egui::Label::new(text).truncate().sense(if link { Sense::click() } else { Sense::hover() }));
                        if link && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open = bar_link(*f);
                        }
                    }
                });
            }
            self.saved_button(ui, lk);
        });
        if let (Some(f), Some(s)) = (open, &snap) {
            let genre = self.m.genre.clone().unwrap_or_default();
            self.m.spot.open_for(f, s, &genre);
        }
    }

    /// The player bar, right part: the queue and the lyrics, then devices, play here and volume.
    fn bar_right(&mut self, ui: &mut egui::Ui) {
        let cur = self.m.spot.pg.nav.current().clone();
        ui.horizontal_centered(|ui| {
            // (right to left would reverse the order of the buttons: space first, then the rest)
            let vol_w = controls::bar_right_width(ui, &self.m);
            let mine = style::ICON_SIDE * 2.0 + ui.spacing().item_spacing.x * 2.0;
            ui.add_space((ui.available_width() - vol_w - mine).max(0.0));
            // with the Spotify look the queue is a panel on the right (as in the app), else a page
            let look = super::sp_look::active(ui.ctx());
            let q_on = if look { queue_open(ui.ctx()) } else { cur == Page::Queue };
            if self.m.spot.on() && style::tool(ui, style::ICON_SIDE, Icon::Queue, q_on, "Queue") {
                if look {
                    set_queue_open(ui.ctx(), !q_on);
                } else {
                    self.toggle_page(Page::Queue);
                }
            }
            if style::tool(ui, style::ICON_SIDE, Icon::Lyrics, cur == Page::Lyrics, "Lyrics") {
                self.toggle_page(Page::Lyrics);
            }
            controls::bar_right(ui, &mut self.m);
        });
    }

    /// A page's button: open it, or go back to where it was opened from.
    fn toggle_page(&mut self, p: Page) {
        let pg = &mut self.m.spot.pg;
        if *pg.nav.current() == p {
            pg.back();
        } else {
            pg.go(p);
        }
    }
}

/// Is the queue panel open (the Spotify look's queue, in the right column)?
fn queue_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("sp-queue-open"))).unwrap_or(false)
}

fn set_queue_open(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("sp-queue-open"), on));
}

/// The player bar's lines: the shown fields' values in the order the setting lists them (fields
/// with nothing to show are left out).
pub fn bar_lines(g: &GuiConf, value: impl Fn(MetaField) -> Option<String>) -> Vec<(MetaField, String)> {
    g.spotify_bar_fields.iter().filter_map(|f| value(*f).filter(|v| !v.trim().is_empty()).map(|v| (*f, v))).take(4).collect()
}

/// The field a click on a bar line opens in the library: the song and the album open the album
/// page, the artists their artist page.
pub fn bar_link(f: MetaField) -> Option<MetaField> {
    match f {
        MetaField::Title | MetaField::Album => Some(MetaField::Album),
        MetaField::Artist | MetaField::AlbumArtist => Some(MetaField::Artist),
        _ => None,
    }
}

/// The Appearance tab's part for the player bar: the cover, the "Now playing" column and the
/// list of song details (tick to show, arrows to reorder). True when something changed.
pub fn bar_settings(ui: &mut Ui, g: &mut GuiConf) -> bool {
    let mut changed = false;
    ui.add_enabled_ui(g.spotify_layout, |ui| {
        ui.indent("sp-bar-settings", |ui| {
            changed |= ui.checkbox(&mut g.spotify_bar_cover, "Small cover in the player bar").changed();
            changed |= ui.checkbox(&mut g.spotify_now_playing, "\"Now playing\" column on the right").changed();
            ui.label(RichText::new("Song details in the player bar").small().weak());
            // the shown ones in their order, then the rest
            let shown = g.spotify_bar_fields.clone();
            let rest: Vec<MetaField> = MetaField::ALL.iter().copied().filter(|f| !shown.contains(f)).collect();
            for f in shown.iter().chain(rest.iter()) {
                let mut on = shown.contains(f);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, f.label()).changed() {
                        g.set_bar_field(*f, on);
                        changed = true;
                    }
                    if on {
                        if style::tool(ui, style::ROW_SIDE, Icon::Up, false, "Move up") && g.shift_bar_field(*f, -1) {
                            changed = true;
                        }
                        if style::tool(ui, style::ROW_SIDE, Icon::Down, false, "Move down") && g.shift_bar_field(*f, 1) {
                            changed = true;
                        }
                    }
                });
            }
        });
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_shows_the_fields_in_the_settings_order_and_skips_empty_ones() {
        let mut g = GuiConf::default();
        let v = |f: MetaField| match f {
            MetaField::Title => Some("City Lights".to_string()),
            MetaField::Artist => Some("The Band".to_string()),
            MetaField::Album => Some("Night Drive".to_string()),
            MetaField::Genre => Some("  ".to_string()),
            _ => None,
        };
        assert_eq!(bar_lines(&g, v).iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["City Lights", "The Band"]);
        g.spotify_bar_fields = vec![MetaField::Album, MetaField::Genre, MetaField::Key, MetaField::Title];
        assert_eq!(bar_lines(&g, v).iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["Night Drive", "City Lights"]);
        g.spotify_bar_fields = MetaField::ALL.to_vec();
        assert!(bar_lines(&g, |_| Some("x".into())).len() <= 4, "the bar has room for four lines");
        g.spotify_bar_fields.clear();
        assert!(bar_lines(&g, v).is_empty());
    }

    #[test]
    fn clicks_on_the_bar_open_the_album_or_the_artist() {
        assert_eq!(bar_link(MetaField::Title), Some(MetaField::Album));
        assert_eq!(bar_link(MetaField::AlbumArtist), Some(MetaField::Artist));
        assert_eq!(bar_link(MetaField::Genre), None);
    }
}
