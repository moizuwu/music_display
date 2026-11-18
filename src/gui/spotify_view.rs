//! The experimental Spotify layout (`spotify_layout` in `[gui]`), arranged like the Spotify app:
//! a strip along the top (back / forward, home, search), your library on the left, the page in the
//! middle (home, search, album, playlist, artist, Liked Songs, queue or the lyrics), the song on
//! the right and a player bar along the bottom. The panels themselves are the window's own.
use super::sp_nav::Page;
use super::sp_side;
use super::style::{self, Icon};
use super::{controls, library, view};
use egui::{RichText, Sense, Ui};
use music_display::guiconf::GuiConf;
use music_display::layout::{Item, MetaField};

/// The height of the player bar.
const BAR_H: f32 = 84.0;
/// The height of the strip along the top.
const TOP_H: f32 = 44.0;

impl super::Gui {
    pub(super) fn spotify_view(&mut self, ui: &mut egui::Ui, g: &GuiConf) {
        // "Go to album" / "Go to artist" from a row menu
        if let Some(p) = self.m.spot.pending_nav.take().and_then(|u| Page::from_uri(&u)) {
            self.m.spot.pg.go(p);
        }
        // the side columns have a fixed share (the lists in them would otherwise take all the width)
        let side_w = (ui.available_width() * 0.26).clamp(220.0, 420.0);
        egui::Panel::bottom("sp-bar").frame(style::gutter(g)).resizable(false).exact_size(BAR_H).show(ui, |ui| {
            style::card(ui, g, |ui| {
                ui.columns(3, |c| {
                    self.bar_left(&mut c[0], g);
                    controls::bar_center(&mut c[1], &mut self.m);
                    self.bar_right(&mut c[2]);
                });
            });
        });
        egui::Panel::top("sp-top").frame(style::gutter(g)).resizable(false).exact_size(TOP_H).show(ui, |ui| self.top_strip(ui));
        egui::Panel::left("sp-library").frame(style::gutter(g)).resizable(false).exact_size(side_w).show(ui, |ui| {
            style::card_at(ui, g, style::Split { after: true, ..style::Split::NONE }, |ui| {
                sp_side::library_column(ui, &mut self.m);
            });
        });
        if g.spotify_now_playing {
            egui::Panel::right("sp-song").frame(style::gutter(g)).resizable(false).exact_size(side_w).show(ui, |ui| {
                style::card_at(ui, g, style::Split { before: true, ..style::Split::NONE }, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        view::cover(ui, self.cover.as_ref(), g.corner_radius, super::fun::vinyl_angle(ui.ctx(), &self.m));
                        ui.add_space(g.panel_spacing as f32);
                        view::metadata(ui, &mut self.m);
                    });
                });
            });
        }
        egui::CentralPanel::default().frame(style::gutter(g)).show(ui, |ui| {
            let split = style::Split { before: true, after: g.spotify_now_playing, vertical: false };
            style::card_at(ui, g, split, |ui| self.sp_middle(ui));
        });
    }

    /// The strip along the top: back and forward, home, and the search field.
    fn top_strip(&mut self, ui: &mut Ui) {
        let pg = &mut self.m.spot.pg;
        let (can_back, can_fwd) = (pg.nav.can_back(), pg.nav.can_forward());
        let at_home = *pg.nav.current() == Page::Home;
        let field_w = (ui.available_width() * 0.4).clamp(180.0, 440.0);
        let gap = ui.spacing().item_spacing.x;
        let side = style::ICON_SIDE;
        let left_w = 2.0 * (side + gap);
        let center_w = side + gap + field_w;
        let mut search = false;
        ui.horizontal_centered(|ui| {
            ui.add_enabled_ui(can_back, |ui| {
                if style::tool(ui, side, Icon::Back, false, "Back") {
                    pg.back();
                }
            });
            ui.add_enabled_ui(can_fwd, |ui| {
                if style::tool(ui, side, Icon::Open, false, "Forward") {
                    pg.forward();
                }
            });
            ui.add_space(((ui.available_width() - center_w) / 2.0 - left_w).max(8.0));
            if style::tool(ui, side, Icon::Home, at_home, "Home") {
                pg.go(Page::Home);
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
    fn bar_left(&mut self, ui: &mut egui::Ui, g: &GuiConf) {
        let snap = self.m.snap.clone();
        let mut open: Option<MetaField> = None;
        ui.horizontal_centered(|ui| {
            if g.spotify_bar_cover {
                if let Some(tex) = &self.cover {
                    let spin = super::fun::vinyl_angle(ui.ctx(), &self.m);
                    super::fun::cover_image(ui, tex, 56.0, egui::CornerRadius::same(4), spin);
                }
            }
            if let Some(s) = &snap {
                let lines = bar_lines(g, |f| view::field_value(&self.m, s, f));
                // (room for the heart after the text)
                let room = (ui.available_width() - if self.m.spot.on() { 40.0 } else { 0.0 }).max(40.0);
                ui.vertical(|ui| {
                    ui.set_max_width(room);
                    for (i, (f, v)) in lines.iter().enumerate() {
                        let text = if i == 0 { RichText::new(v).strong().size(15.0) } else { RichText::new(v).small().weak() };
                        let link = self.m.spot.on() && bar_link(*f).is_some();
                        let r = ui.add(egui::Label::new(text).truncate().sense(if link { Sense::click() } else { Sense::hover() }));
                        if link && r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open = bar_link(*f);
                        }
                    }
                });
            }
            library::heart_button(ui, &mut self.m);
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
            if self.m.spot.on() && style::tool(ui, style::ICON_SIDE, Icon::Queue, cur == Page::Queue, "Queue") {
                self.toggle_page(Page::Queue);
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
