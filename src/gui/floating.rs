//! The windows that float over the panels (settings, keys, library, devices, the AI question).
//! Each is a window of its own that can be dragged out of the main window, with the same
//! background, opacity and title-bar settings as the rest; where the display server cannot
//! open a second window it stays inside the main one. Nothing here has been tried on a real
//! compositor.
use super::style::{self, Look};
use egui::{Ui, ViewportBuilder, ViewportClass, ViewportId};
use music_display::guiconf::GuiConf;

/// Show `add` in the window `id`. `open` goes false when the window's close button is pressed.
#[allow(clippy::too_many_arguments)]
pub fn window(
    ctx: &egui::Context,
    look: &mut Look,
    g: &GuiConf,
    id: &str,
    title: &str,
    size: [f32; 2],
    open: &mut bool,
    mut add: impl FnMut(&mut Ui, &mut Look),
) {
    let mut b = ViewportBuilder::default()
        .with_title(format!("music_display - {title}"))
        .with_app_id("music_display")
        .with_inner_size(size)
        .with_min_inner_size([240.0, 140.0])
        .with_transparent(true)
        .with_decorations(g.decorations);
    if g.always_on_top {
        b = b.with_always_on_top();
    }
    ctx.show_viewport_immediate(ViewportId::from_hash_of(("floating", id)), b, |ui, class| {
        if class == ViewportClass::EmbeddedWindow {
            // the backend already put this in a window of the main one
            add(ui, look);
        } else {
            look.paint_background(ui, g);
            look.paint_effects(ui, g);
            egui::CentralPanel::default().frame(style::gutter(g)).show(ui, |ui| style::card(ui, g, |ui| add(ui, look)));
        }
        if ui.input(|i| i.viewport().close_requested()) {
            *open = false;
        }
    });
}
