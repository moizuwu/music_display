//! `--gui`: a window with the same panels, config file and layout as the terminal front end.
//! Built only with `--features gui`. The terminal-only parts (the big block font, graphics
//! protocols, key remapping) are not in it; see the parity table in the README.
mod backdrop;
mod browse;
mod controls;
mod extras;
mod floathook;
mod floating;
mod footer;
mod keys;
mod library;
mod lyrics_view;
mod model;
mod popout;
mod privacy;
mod settings;
mod sizewatch;
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
                    colors_rx: None,
                    colors_watch: Vec::new(),
                    colors_asked: None,
                    colors_polled: std::time::Instant::now(),
                    size_watch: Default::default(),
                    col_epoch: 0,
                    col_total: 0.0,
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
    /// The colours of the system (`colors`): the answer being fetched off the UI thread, the
    /// files the last answer came from, and when they were asked and last looked at.
    colors_rx: Option<std::sync::mpsc::Receiver<crate::syscolors::Found>>,
    colors_watch: Vec<(PathBuf, Option<std::time::SystemTime>)>,
    colors_asked: Option<(std::time::Instant, music_display::guiconf::ColorsFrom, String)>,
    colors_polled: std::time::Instant,
    /// Which panels were dragged to which size.
    size_watch: sizewatch::SizeWatch,
    /// The columns' size in the window is taken from their share again each time the window
    /// itself changes size; this counts those times (it is part of the columns' ids).
    col_epoch: u32,
    col_total: f32,
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

    /// Keep the system colours current: ask when the setting changes, look at the files the last
    /// answer came from every few seconds (a wallpaper tool rewrites them), and ask again when one
    /// changed or every 20 seconds (the desktop settings are not files).
    fn refresh_colors(&mut self, ctx: &egui::Context) {
        use music_display::guiconf::ColorsFrom;
        use std::time::{Duration, Instant};
        if let Some(rx) = &self.colors_rx {
            if let Ok(found) = rx.try_recv() {
                self.colors_rx = None;
                self.colors_watch = found.watch;
                self.look.set_system(found.colors);
            }
        }
        let (mode, file) = (self.m.gui.colors, self.m.gui.colors_file.clone());
        if mode == ColorsFrom::Off {
            if self.look.system_source().is_some() {
                self.look.set_system(None);
            }
            self.colors_asked = None;
            return;
        }
        if self.colors_rx.is_some() {
            return;
        }
        let due = match &self.colors_asked {
            None => true,
            Some((_, m, f)) if *m != mode || *f != file => true,
            Some((at, _, _)) if self.colors_polled.elapsed() >= Duration::from_secs(3) => {
                self.colors_polled = Instant::now();
                at.elapsed() >= Duration::from_secs(20) || crate::syscolors::changed(&self.colors_watch)
            }
            _ => false,
        };
        if due {
            let (tx, rx) = std::sync::mpsc::channel();
            self.colors_rx = Some(rx);
            self.colors_asked = Some((Instant::now(), mode, file.clone()));
            self.colors_polled = Instant::now();
            let c = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(crate::syscolors::resolve(mode, &file));
                c.request_repaint();
            });
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

    /// A panel inside a column starts at the size it was dragged to last time.
    fn inner_size(&self, key: &str, default: f32) -> f32 {
        self.m.gui.inner_sizes.get(key).map_or(default, |v| *v as f32)
    }

    /// A panel was dragged to `extent` and the pointer was let go: that size, once. Only a size
    /// that changed while the pointer was down counts, so the sizes a panel gets from the
    /// layout (the first frames, a window that is resized or maximised) are never taken for
    /// something the user chose.
    fn dragged_size(&mut self, ui: &egui::Ui, key: &str, extent: f32) -> Option<f32> {
        let (down, released) = ui.input(|i| (i.pointer.any_down(), i.pointer.any_released()));
        self.size_watch.step(key, extent, down, released)
    }

    /// Remember a dragged size of a panel inside a column in `[gui] inner_sizes`.
    fn remember_size(&mut self, ui: &egui::Ui, key: &str, extent: f32) {
        if let Some(px) = self.dragged_size(ui, key, extent) {
            self.m.gui.inner_sizes.insert(key.to_string(), px.round().clamp(40.0, 4000.0) as u16);
            let t = self.m.gui.write();
            self.m.save_settings(&[("gui", toml::Value::Table(t))]);
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
                    let r = egui::Panel::bottom("cava_under")
                        .default_size(self.inner_size("cava", 120.0))
                        .resizable(!self.m.gui.lock_layout)
                        .show(ui, |ui| extras::cava_panel(ui, &self.m));
                    self.remember_size(ui, "cava", r.response.rect.height());
                }
                // with the annotations column open the diagrams live in it (above the notes)
                if self.m.show_diagrams && !self.m.annotations_shown() && !self.is_popped(PanelId::Chords) {
                    let r = egui::Panel::right("diagrams")
                        .default_size(self.inner_size("diagrams", 280.0))
                        .resizable(!self.m.gui.lock_layout)
                        .show(ui, |ui| view::diagrams(ui, &mut self.m));
                    self.remember_size(ui, "diagrams", r.response.rect.width());
                }
                lyrics_view::lyrics(ui, &mut self.m);
            }
            PanelId::Cava => extras::cava_panel(ui, &self.m),
            PanelId::Annotations => {
                if self.m.show_diagrams && !self.is_popped(PanelId::Chords) {
                    // one column, two panels: the diagrams take the top half, the notes the rest
                    let half = ui.available_height() / 2.0;
                    let r = egui::Panel::top("diagrams_stack")
                        .default_size(self.inner_size("stack", half))
                        .resizable(!self.m.gui.lock_layout)
                        .show(ui, |ui| view::diagrams(ui, &mut self.m));
                    self.remember_size(ui, "stack", r.response.rect.height());
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
        self.refresh_colors(ctx);
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
        self.m.ux.dark_now = ui.visuals().dark_mode;
        self.look.paint_background(ui, &g);
        self.look.paint_effects(ui, &g);
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
                    if ui.button(if self.m.ux.dark_now { "light" } else { "dark" }).on_hover_text("dark / light theme (v)").clicked() {
                        self.m.toggle_theme();
                    }
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
        // The columns start at their share of the window. egui remembers a panel's size in
        // pixels, so when the window changes size (the first frame is often not the size it ends
        // up with, a tiling manager maximises it) they would keep the pixels of the old size:
        // new ids make them start again from their share, and a drag is what changes the share.
        if (total - self.col_total).abs() > 2.0 && !ui.input(|i| i.pointer.any_down()) {
            self.col_total = total;
            self.col_epoch += 1;
        }
        for (i, id) in rest.iter().enumerate() {
            let pct = self.m.layout.size_of(*id).map(f32::from).unwrap_or(if *id == PanelId::Metadata { 34.0 } else { 25.0 });
            let pid = format!("col{i}-{}", self.col_epoch);
            let panel = if vertical { egui::Panel::top(pid) } else { egui::Panel::left(pid) };
            let panel = panel.frame(style::gutter(&g)).resizable(!g.lock_layout).default_size(total * pct / 100.0).min_size(if vertical { 80.0 } else { 160.0 });
            let resp = panel.show(ui, |ui| {
                style::card(ui, &g, |ui| self.panel(ui, *id));
            });
            // dragging the border stores the new share
            let extent = if vertical { resp.response.rect.height() } else { resp.response.rect.width() };
            if let Some(px) = self.dragged_size(ui, &format!("col:{}", id.id()), extent) {
                self.m.layout.set_size(*id, (px / total.max(1.0) * 100.0).round() as u16);
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
