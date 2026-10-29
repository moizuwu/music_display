//! `--gui`: a window with the same panels, config file and layout as the terminal front end.
//! Built only with `--features gui`. The terminal-only parts (the big block font, graphics
//! protocols) are not in it; see the parity table in the README.
mod extras;
mod keys;
mod library;
mod model;
mod privacy;
mod settings;
mod style;
mod ux;
mod view;

use anyhow::Result;
use model::Model;
use music_display::cli::Args;
use music_display::layout::{Item, Layout, PanelId};
use music_display::{AppEvent, config};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub fn run(args: Args, cfg_path: PathBuf, layout: Layout) -> Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut model = Model::new(args, layout, cfg_path.clone(), tx.clone());
    // the look of the window: `[gui]` of the config file
    model.gui = music_display::guiconf::load(&cfg_path, &mut |w| eprintln!("music_display: {w}"));
    settings::main_flags_into_gui(&mut model.gui, model.args.transparent, settings::gui_sets(&cfg_path, "opacity"));
    // the terminal UI's key table, `[keys]` included
    model.ux.keys = crate::keymap::load(&cfg_path, &mut |_| {});
    if model.args.use_spotifyd {
        model.ux.spotifyd = music_display::spotifyd::start();
    }
    // the same pollers as the terminal front end
    tokio::spawn(model.ctl.clone().poll_loop(tx.clone()));
    tokio::spawn(model.ctl.clone().follow_loop(tx));
    let native = eframe::NativeOptions {
        viewport: style::viewport(&model.gui),
        persist_window: model.gui.remember_window,
        ..Default::default()
    };
    // eframe owns this thread until the window closes; the runtime keeps working on its others
    tokio::task::block_in_place(|| {
        eframe::run_native(
            "music_display",
            native,
            Box::new(move |cc| {
                Ok(Box::new(Gui { m: model, rx, cover: None, look: style::Look::default(), repaint: cc.egui_ctx.clone() }) as Box<dyn eframe::App>)
            }),
        )
    })
    .map_err(|e| anyhow::anyhow!("the window failed: {e}"))
}

struct Gui {
    m: Model,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    cover: Option<egui::TextureHandle>,
    look: style::Look,
    repaint: egui::Context,
}

impl Gui {
    fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.m.apply(ev);
        }
        let cover_changed = self.m.cover_fresh;
        if self.m.cover_fresh {
            self.m.cover_fresh = false;
            self.cover = self.m.cover.as_ref().map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                self.repaint.load_texture("cover", egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
            });
        }
        if cover_changed {
            self.look.set_cover(&self.repaint, self.m.cover.as_ref());
        }
        if self.m.note.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() > 6) {
            self.m.note = None;
        }
    }

    fn panel(&mut self, ui: &mut egui::Ui, id: PanelId) {
        match id {
            PanelId::Metadata => {
                let l = self.m.layout.clone();
                // the now-playing column: cover, metadata and controls in the order listed
                for p in l.now_playing() {
                    match p {
                        PanelId::Cover => view::cover(ui, self.cover.as_ref(), self.m.gui.corner_radius),
                        PanelId::Metadata => view::metadata(ui, &mut self.m),
                        PanelId::Controls => view::controls(ui, &mut self.m),
                        _ => {}
                    }
                    ui.add_space(self.m.gui.panel_spacing as f32);
                }
            }
            PanelId::Lyrics => {
                // cava docked under the lyrics
                if self.m.cava_wanted() && self.m.layout.cava_under_lyrics {
                    egui::Panel::bottom("cava_under").default_size(120.0).resizable(true).show(ui, |ui| extras::cava_panel(ui, &self.m));
                }
                // with the annotations column open the diagrams live in it (above the notes)
                if self.m.show_diagrams && !self.m.annotations_shown() {
                    egui::Panel::right("diagrams").default_size(280.0).resizable(true).show(ui, |ui| view::diagrams(ui, &mut self.m));
                }
                view::lyrics(ui, &mut self.m);
            }
            PanelId::Cava => extras::cava_panel(ui, &self.m),
            PanelId::Annotations => {
                if self.m.show_diagrams {
                    // one column, two panels: the diagrams take the top half, the notes the rest
                    let half = ui.available_height() / 2.0;
                    egui::Panel::top("diagrams_stack").default_size(half).resizable(true).show(ui, |ui| view::diagrams(ui, &mut self.m));
                }
                view::annotations(ui, &mut self.m);
            }
            _ => {}
        }
    }
}

impl eframe::App for Gui {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();
        self.m.ux_tick();
        self.m.sync_cava();
        // the clock moves the lyrics: redraw about 30 times a second while playing
        let fast = self.m.playing() || self.m.cava_wanted();
        ctx.request_repaint_after(std::time::Duration::from_millis(if fast { 33 } else { 250 }));
    }

    // see-through: the background is painted by `style::Look::paint_background`
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    // the layout's own sizes decide the panel widths, not what egui remembered
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let g = self.m.gui.clone();
        self.look.apply(ui.ctx(), &g);
        self.look.paint_background(ui, &g);
        egui::Panel::top("top").frame(style::gutter(&g)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("music_display");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("?").on_hover_text("keys (?)").clicked() {
                        self.m.ux.help = !self.m.ux.help;
                    }
                    if ui.button("settings").on_hover_text("settings (s)").clicked() {
                        self.m.ux.s.open = !self.m.ux.s.open;
                    }
                    if let Some((n, _)) = &self.m.note {
                        ui.label(egui::RichText::new(n.as_str()).weak());
                    }
                });
            });
        });
        // the terminal UI's keys (not while typing in a field)
        self.handle_keys(ui.ctx());
        if self.m.x.minimal {
            egui::CentralPanel::default().show(ui, |ui| extras::teleprompter(ui, &mut self.m));
            self.popups(ui.ctx());
            return;
        }
        // columns in the layout's order; the first one gets a side panel, the last fills the rest
        let cols: Vec<PanelId> = self
            .m
            .layout
            .column_order()
            .into_iter()
            .filter(|p| match p {
                PanelId::Lyrics => self.m.layout.panels.shown(PanelId::Lyrics),
                PanelId::Cava => self.m.cava_wanted(),
                PanelId::Annotations => self.m.annotations_shown(),
                _ => true,
            })
            .collect();
        let (last, rest) = match cols.split_last() {
            Some((l, r)) => (*l, r.to_vec()),
            None => (PanelId::Metadata, Vec::new()),
        };
        let total = ui.available_width();
        for (i, id) in rest.iter().enumerate() {
            let pct = self.m.layout.size_of(*id).map(f32::from).unwrap_or(if *id == PanelId::Metadata { 34.0 } else { 25.0 });
            let panel = egui::Panel::left(format!("col{i}")).frame(style::gutter(&g)).resizable(true).default_size(total * pct / 100.0).min_size(160.0);
            let resp = panel.show(ui, |ui| {
                style::card(ui, &g, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.panel(ui, *id));
                });
            });
            // dragging the border stores the new share
            let new = (resp.response.rect.width() / total.max(1.0) * 100.0).round() as u16;
            if resp.response.drag_stopped() && self.m.layout.size_of(*id) != Some(new) {
                self.m.layout.set_size(*id, new);
                self.m.save_layout();
            }
        }
        egui::CentralPanel::default().frame(style::gutter(&g)).show(ui, |ui| style::card(ui, &g, |ui| self.panel(ui, last)));
        self.popups(ui.ctx());
    }
}

impl Gui {
    /// Panels, metadata fields and buttons: tick to show, arrows to reorder, saved at once.
    fn layout_editor(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        macro_rules! group {
            ($title:expr, $list:expr) => {{
                ui.label(egui::RichText::new($title).strong());
                let n = $list.items.len();
                for i in 0..n {
                    let (item, mut on) = $list.items[i];
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, item.id()).changed() {
                            $list.set(item, on);
                            changed = true;
                        }
                        if ui.small_button("↑").clicked() && $list.shift(item, -1) {
                            changed = true;
                        }
                        if ui.small_button("↓").clicked() && $list.shift(item, 1) {
                            changed = true;
                        }
                    });
                }
            }};
        }
        group!("Panels", self.m.layout.panels);
        group!("Metadata fields", self.m.layout.fields);
        group!("Control buttons", self.m.layout.buttons);
        if ui.button("Automatic sizes").clicked() {
            self.m.layout.clear_sizes();
            changed = true;
        }
        if ui.button("Reset layout").clicked() {
            self.m.layout = Layout::default();
            changed = true;
        }
        if changed {
            self.m.show_diagrams = self.m.layout.panels.shown(PanelId::Chords);
            self.m.ensure_annotations();
            self.m.save_layout();
        }
    }
}

#[allow(dead_code)]
fn _config_is_shared() -> PathBuf {
    config::default_path()
}

/// The window stops the spotifyd it started when it closes (`kill_spotifyd_on_exit`).
impl Drop for Gui {
    fn drop(&mut self) {
        if !self.m.args.no_kill_spotifyd {
            if let Some(mut c) = self.m.ux.spotifyd.take() {
                music_display::spotifyd::stop(&mut c);
            }
        }
    }
}
