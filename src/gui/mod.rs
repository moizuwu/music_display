//! `--gui`: a window with the same panels, config file and layout as the terminal front end.
//! Built only with `--features gui`. The terminal-only parts (the big block font, graphics
//! protocols, key remapping) are not in it; see the parity table in the README.
mod backdrop;
mod browse;
mod controls;
mod extras;
mod footer;
mod keys;
mod library;
mod lyrics_view;
mod model;
mod popout;
mod privacy;
mod settings;
mod style;
mod ux;
mod translation;
mod view;
mod window;

use anyhow::Result;
use model::Model;
use music_display::cli::Args;
use music_display::layout::{Item, Layout, PanelId};
use music_display::AppEvent;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub fn run(args: Args, cfg_path: PathBuf, layout: Layout) -> Result<()> {
    // every event the background tasks send also wakes the window
    let (tx, rx, wake) = window::channel();
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
    // SIGTERM / SIGHUP close the window like its close button; spotifyd (use_spotifyd) is
    // stopped when `daemon` is dropped, whichever way the window ends
    window::close_on_signal(wake.clone());
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
                let _ = wake.set(cc.egui_ctx.clone());
                Ok(Box::new(Gui {
                    m: model,
                    rx,
                    cover: None,
                    look: style::Look::default(),
                    repaint: cc.egui_ctx.clone(),
                    title_sent: None,
                    zoom_seen: 1.0,
                }) as Box<dyn eframe::App>)
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
    /// The window title last sent (`window_title`).
    title_sent: Option<String>,
    /// The zoom factor last seen, to notice the zoom keys.
    zoom_seen: f32,
}

impl Gui {
    /// The zoom keys (egui's own Ctrl and + / - / 0) are kept in `ui_scale`, and Ctrl+Shift with
    /// Up / Down changes the window opacity by 5; both are written to `[gui]`.
    fn window_keys(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        let z = ctx.zoom_factor();
        if (z - self.zoom_seen).abs() > 0.001 {
            self.zoom_seen = z;
            let pct = ((z * 100.0).round() as u16).clamp(50, 200);
            if pct != self.m.gui.ui_scale {
                self.m.gui.ui_scale = pct;
                changed = true;
            }
        }
        let step = ctx.input(|i| {
            let both = i.modifiers.command && i.modifiers.shift;
            (both && i.key_pressed(egui::Key::ArrowUp)) as i16 - (both && i.key_pressed(egui::Key::ArrowDown)) as i16
        });
        if step != 0 {
            self.m.gui.opacity = (self.m.gui.opacity as i16 + step * 5).clamp(0, 100) as u8;
            changed = true;
        }
        if changed {
            let t = self.m.gui.write();
            self.m.save_settings(&[("gui", toml::Value::Table(t))]);
        }
    }

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
                // the now-playing column: cover, metadata and controls in the order listed; it
                // is the one column that scrolls (the others scroll their own content)
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for p in l.now_playing() {
                        match p {
                            PanelId::Cover => view::cover(ui, self.cover.as_ref(), self.m.gui.corner_radius),
                            PanelId::Metadata => view::metadata(ui, &mut self.m),
                            PanelId::Controls => controls::controls(ui, &mut self.m),
                            _ => {}
                        }
                        ui.add_space(self.m.gui.panel_spacing as f32);
                    }
                });
            }
            // the diagrams on their own, when no lyrics or notes column is there to hold them
            PanelId::Chords => view::diagrams(ui, &mut self.m),
            PanelId::Lyrics => {
                // cava docked under the lyrics
                if self.m.cava_wanted() && self.m.layout.cava_under_lyrics && !self.is_popped(PanelId::Cava) {
                    egui::Panel::bottom("cava_under").default_size(120.0).resizable(true).show(ui, |ui| extras::cava_panel(ui, &self.m));
                }
                // with the annotations column open the diagrams live in it (above the notes)
                if self.m.show_diagrams && !self.m.annotations_shown() && !self.is_popped(PanelId::Chords) {
                    egui::Panel::right("diagrams").default_size(280.0).resizable(true).show(ui, |ui| view::diagrams(ui, &mut self.m));
                }
                lyrics_view::lyrics(ui, &mut self.m);
            }
            PanelId::Cava => extras::cava_panel(ui, &self.m),
            PanelId::Annotations => {
                if self.m.show_diagrams && !self.is_popped(PanelId::Chords) {
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
        self.m.flush_volume(false);
        self.m.spot.preload_tick();
        // the window title follows the song when `window_title` says so
        let want = window::window_title(&self.m.args.window_title, self.m.snap.as_ref());
        if want != self.title_sent {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want.clone().unwrap_or_else(|| "music_display".into())));
            self.title_sent = want;
        }
        // the clock moves the lyrics: redraw about 30 times a second while playing
        self.window_keys(ctx);
        // a moving background only runs while the window has the focus
        let moving = style::Look::animated(&self.m.gui) && ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let fast = self.m.playing() || self.m.cava_wanted() || moving;
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
        if g.theme_preset == music_display::guiconf::Preset::Christmas {
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("look-extra"));
            backdrop::figure(&ui.ctx().layer_painter(layer), ui.ctx().content_rect(), g.opacity as f32 / 100.0);
        }
        egui::Panel::top("top").frame(style::gutter(&g)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("music_display");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button("windows", |ui| {
                        ui.label(egui::RichText::new("Show in a window of its own").small().weak());
                        for (id, name) in popout::PANELS {
                            let mut on = self.is_popped(id);
                            if ui.checkbox(&mut on, name).changed() {
                                self.toggle_popout(id);
                            }
                        }
                    });
                    if ui.button("?").on_hover_text("keys (?)").clicked() {
                        self.m.ux.help = !self.m.ux.help;
                    }
                    if ui.button("settings").on_hover_text("settings (s)").clicked() {
                        self.m.ux.s.open = !self.m.ux.s.open;
                    }
                });
            });
        });
        egui::Panel::bottom("footer").frame(style::gutter(&g)).show(ui, |ui| footer::footer(ui, &mut self.m));
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
        // the diagrams are docked beside the lyrics or the notes; with neither shown they get a column
        let mut cols = cols;
        if self.m.show_diagrams && !self.m.layout.panels.shown(PanelId::Lyrics) && !self.m.annotations_shown() {
            cols.push(PanelId::Chords);
        }
        // panels in a window of their own leave the columns
        cols.retain(|p| !self.is_popped(*p));
        let vertical = self.m.layout.direction == music_display::layout::Split::Vertical;
        let (last, rest) = match cols.split_last() {
            Some((l, r)) => (Some(*l), r.to_vec()),
            None if self.m.gui.popped.is_empty() => (Some(PanelId::Metadata), Vec::new()),
            None => (None, Vec::new()),
        };
        // layout.direction: columns side by side, or stacked top to bottom
        let total = if vertical { ui.available_height() } else { ui.available_width() };
        for (i, id) in rest.iter().enumerate() {
            let pct = self.m.layout.size_of(*id).map(f32::from).unwrap_or(if *id == PanelId::Metadata { 34.0 } else { 25.0 });
            let panel = if vertical { egui::Panel::top(format!("col{i}")) } else { egui::Panel::left(format!("col{i}")) };
            let panel = panel.frame(style::gutter(&g)).resizable(!g.lock_layout).default_size(total * pct / 100.0).min_size(if vertical { 80.0 } else { 160.0 });
            let resp = panel.show(ui, |ui| {
                style::card(ui, &g, |ui| self.panel(ui, *id));
            });
            // dragging the border stores the new share
            let extent = if vertical { resp.response.rect.height() } else { resp.response.rect.width() };
            let new = (extent / total.max(1.0) * 100.0).round() as u16;
            if resp.response.drag_stopped() && self.m.layout.size_of(*id) != Some(new) {
                self.m.layout.set_size(*id, new);
                self.m.save_layout();
            }
        }
        egui::CentralPanel::default().frame(style::gutter(&g)).show(ui, |ui| {
            style::card(ui, &g, |ui| match last {
                Some(id) => self.panel(ui, id),
                None => {
                    ui.weak("Every panel is in a window of its own. Dock one again from the \"windows\" menu.");
                }
            })
        });
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
