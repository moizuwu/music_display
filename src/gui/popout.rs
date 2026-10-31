//! Panels in windows of their own. A popped-out panel leaves its column and is drawn in an OS
//! window (an "immediate" egui viewport, so it can use the same model as the main window);
//! closing that window docks the panel again. The panels are listed in `[gui] popped`.
//! Nothing here has been tried on a real compositor.
use super::{style, Gui};
use egui::{ViewportBuilder, ViewportClass, ViewportId};
use music_display::layout::{Item, PanelId};

/// The panels that can leave the window (the now-playing column counts as one).
pub const PANELS: [(PanelId, &str); 5] = [
    (PanelId::Metadata, "Now playing"),
    (PanelId::Lyrics, "Lyrics"),
    (PanelId::Cava, "Cava"),
    (PanelId::Annotations, "Annotations"),
    (PanelId::Chords, "Chord diagrams"),
];

fn size_of(id: PanelId) -> [f32; 2] {
    match id {
        PanelId::Lyrics => [520.0, 640.0],
        PanelId::Cava => [480.0, 220.0],
        PanelId::Chords => [360.0, 480.0],
        _ => [380.0, 560.0],
    }
}

impl Gui {
    pub(super) fn is_popped(&self, id: PanelId) -> bool {
        self.m.gui.popped.iter().any(|p| p == id.id())
    }

    /// Pop a panel out, or dock it again.
    pub(super) fn toggle_popout(&mut self, id: PanelId) {
        if self.is_popped(id) {
            self.m.gui.popped.retain(|p| p != id.id());
        } else {
            self.m.gui.popped.push(id.id().to_string());
            // a panel that is hidden or off would open empty: switch it on
            match id {
                PanelId::Lyrics | PanelId::Cava => self.m.layout.panels.set(id, true),
                PanelId::Chords => {
                    self.m.show_diagrams = true;
                    self.m.layout.panels.set(PanelId::Chords, true);
                }
                _ => {}
            }
        }
        let t = self.m.gui.write();
        let mut keys = vec![("gui", toml::Value::Table(t))];
        if self.is_popped(id) {
            // what was switched on to fill the window stays on at the next start
            self.m.save_layout();
            if id == PanelId::Cava {
                keys.push(("cava", true.into()));
            }
        }
        self.m.save_settings(&keys);
    }

    /// Draw every popped-out panel in its window; a window closed by its button docks the panel.
    pub(super) fn popouts(&mut self, ctx: &egui::Context) {
        let g = self.m.gui.clone();
        let mut docked: Vec<PanelId> = Vec::new();
        for (id, name) in PANELS {
            let title = format!("music_display - {name}");
            if !self.is_popped(id) {
                // (docked from the menu: the next time it is popped out it is a new window)
                super::floathook::forget(ctx, &title);
                continue;
            }
            let mut b = ViewportBuilder::default()
                .with_title(title.clone())
                .with_app_id("music_display.popout")
                .with_inner_size(size_of(id))
                .with_min_inner_size([200.0, 120.0])
                .with_transparent(true)
                .with_decorations(g.decorations);
            if g.always_on_top {
                b = b.with_always_on_top();
            }
            let vid = ViewportId::from_hash_of(("popout", id.id()));
            ctx.show_viewport_immediate(vid, b, |ui, class| {
                if class == ViewportClass::EmbeddedWindow {
                    // no second window possible: the panel stays in the main window's own frame
                    style::card(ui, &g, |ui| self.panel(ui, id));
                } else {
                    if g.float_windows && super::floathook::first_frame(ui.ctx(), &title) {
                        super::floathook::float_once(&title, size_of(id));
                    }
                    self.look.paint_background(ui, &g);
                    self.look.paint_effects(ui, &g);
                    egui::CentralPanel::default().frame(style::gutter(&g)).show(ui, |ui| style::card(ui, &g, |ui| self.panel(ui, id)));
                }
                if ui.input(|i| i.viewport().close_requested()) {
                    docked.push(id);
                    super::floathook::forget(ui.ctx(), &title);
                }
            });
        }
        for id in docked {
            self.toggle_popout(id);
        }
    }
}
