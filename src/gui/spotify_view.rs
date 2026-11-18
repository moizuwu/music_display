//! The experimental Spotify layout (`spotify_layout` in `[gui]`): the library on the left, the
//! lyrics in the middle, the song on the right and a player bar along the bottom, the way the
//! Spotify app arranges them. The panels themselves are the window's own.
use super::{controls, library, style, view};
use music_display::guiconf::GuiConf;
use music_display::layout::PanelId;

/// The height of the player bar.
const BAR_H: f32 = 84.0;

impl super::Gui {
    pub(super) fn spotify_view(&mut self, ui: &mut egui::Ui, g: &GuiConf) {
        // the side columns have a fixed share (the lists in them would otherwise take all the width)
        let side_w = (ui.available_width() * 0.26).clamp(220.0, 420.0);
        egui::Panel::bottom("sp-bar").frame(style::gutter(g)).resizable(false).exact_size(BAR_H).show(ui, |ui| {
            style::card(ui, g, |ui| {
                ui.columns(3, |c| {
                    self.bar_left(&mut c[0]);
                    controls::bar_center(&mut c[1], &mut self.m);
                    controls::bar_right(&mut c[2], &mut self.m);
                });
            });
        });
        egui::Panel::left("sp-library").frame(style::gutter(g)).resizable(false).exact_size(side_w).show(ui, |ui| {
            style::card(ui, g, |ui| {
                style::heading(ui, "Your Library");
                if self.m.spot.on() {
                    let tab = self.m.spot.lib.tab;
                    self.m.spot.load_tab(tab, false);
                    library::library_body(ui, &mut self.m);
                } else {
                    style::note(ui, "Spotify is off. Turn it on in the settings (Spotify tab) to see your library here.");
                }
            });
        });
        egui::Panel::right("sp-song").frame(style::gutter(g)).resizable(false).exact_size(side_w).show(ui, |ui| {
            style::card(ui, g, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    view::cover(ui, self.cover.as_ref(), g.corner_radius);
                    ui.add_space(g.panel_spacing as f32);
                    view::metadata(ui, &mut self.m);
                });
            });
        });
        egui::CentralPanel::default().frame(style::gutter(g)).show(ui, |ui| {
            style::card(ui, g, |ui| self.panel(ui, PanelId::Lyrics));
        });
    }

    /// The player bar, left part: the cover small, the title and artist, and the heart.
    fn bar_left(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            if let Some(tex) = &self.cover {
                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(56.0, 56.0)).corner_radius(4.0));
            }
            if let Some(s) = self.m.snap.clone() {
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(egui::RichText::new(&s.title).strong()).truncate());
                    ui.add(egui::Label::new(egui::RichText::new(&s.artist).weak()).truncate());
                });
            }
            library::heart_button(ui, &mut self.m);
        });
    }
}
