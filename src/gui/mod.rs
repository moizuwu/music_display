//! `--gui`: a window with the same panels, config file and layout as the terminal front end.
//! Built only with `--features gui`. The terminal-only parts (cava bars, the big block font,
//! graphics protocols, key remapping) are not in it; see the parity table in the README.
mod model;
mod view;

use anyhow::Result;
use model::Model;
use music_display::cli::{AnnotationMode, Args, TranslateEngine, TranslateMode};
use music_display::layout::{Item, Layout, PanelId};
use music_display::{AppEvent, config};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub fn run(args: Args, cfg_path: PathBuf, layout: Layout) -> Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
    let model = Model::new(args, layout, cfg_path, tx.clone());
    // the same pollers as the terminal front end
    tokio::spawn(model.ctl.clone().poll_loop(tx.clone()));
    tokio::spawn(model.ctl.clone().follow_loop(tx));
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 640.0]).with_min_inner_size([480.0, 320.0]).with_title("music_display"),
        ..Default::default()
    };
    // eframe owns this thread until the window closes; the runtime keeps working on its others
    tokio::task::block_in_place(|| {
        eframe::run_native(
            "music_display",
            native,
            Box::new(move |cc| {
                Ok(Box::new(Gui { m: model, rx, cover: None, settings: false, customize: false, repaint: cc.egui_ctx.clone() }) as Box<dyn eframe::App>)
            }),
        )
    })
    .map_err(|e| anyhow::anyhow!("the window failed: {e}"))
}

struct Gui {
    m: Model,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    cover: Option<egui::TextureHandle>,
    settings: bool,
    customize: bool,
    repaint: egui::Context,
}

impl Gui {
    fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.m.apply(ev);
        }
        if self.m.cover_fresh {
            self.m.cover_fresh = false;
            self.cover = self.m.cover.as_ref().map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                self.repaint.load_texture("cover", egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
            });
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
                        PanelId::Cover => view::cover(ui, self.cover.as_ref()),
                        PanelId::Metadata => view::metadata(ui, &mut self.m),
                        PanelId::Controls => view::controls(ui, &mut self.m),
                        _ => {}
                    }
                    ui.add_space(8.0);
                }
            }
            PanelId::Lyrics => {
                // with the annotations column open the diagrams live in it (above the notes)
                if self.m.show_diagrams && !self.m.annotations_shown() {
                    egui::Panel::right("diagrams").default_size(280.0).resizable(true).show(ui, |ui| view::diagrams(ui, &mut self.m));
                }
                view::lyrics(ui, &mut self.m);
            }
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
        // the clock moves the lyrics: redraw about 30 times a second while playing
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.m.playing() { 33 } else { 250 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("music_display");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("?").clicked() {
                        self.m.say("keys: space play/pause · ←/→ seek · , . lyrics offset · c chords · d diagrams · n notes");
                    }
                    if ui.button("settings").clicked() {
                        self.settings = !self.settings;
                    }
                    if let Some((n, _)) = &self.m.note {
                        ui.label(egui::RichText::new(n.as_str()).weak());
                    }
                });
            });
        });
        // shortcuts (not while typing in a field)
        if !ui.ctx().egui_wants_keyboard_input() {
            let (space, left, right, comma, dot, c, d, n) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Space),
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.key_pressed(egui::Key::Comma),
                    i.key_pressed(egui::Key::Period),
                    i.key_pressed(egui::Key::C),
                    i.key_pressed(egui::Key::D),
                    i.key_pressed(egui::Key::N),
                )
            });
            if space {
                self.m.play_pause();
            }
            if left {
                let p = self.m.position();
                self.m.seek((p - 5.0).max(0.0));
            }
            if right {
                let p = self.m.position();
                self.m.seek(p + 5.0);
            }
            if comma {
                self.m.nudge(-0.1);
            }
            if dot {
                self.m.nudge(0.1);
            }
            if c {
                self.m.toggle_chords();
            }
            if d {
                self.m.show_diagrams = !self.m.show_diagrams;
                let on = self.m.show_diagrams;
                self.m.layout.panels.set(PanelId::Chords, on);
            }
            if n {
                let on = !self.m.annotations_shown();
                self.m.layout.panels.set(PanelId::Annotations, on);
                self.m.args.annotations = on;
                self.m.ensure_annotations();
            }
        }
        // columns in the layout's order; the first one gets a side panel, the last fills the rest
        let cols: Vec<PanelId> = self
            .m
            .layout
            .column_order()
            .into_iter()
            .filter(|p| match p {
                PanelId::Lyrics => self.m.layout.panels.shown(PanelId::Lyrics),
                PanelId::Cava => false, // TUI only
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
            let panel = egui::Panel::left(format!("col{i}")).resizable(true).default_size(total * pct / 100.0).min_size(160.0);
            let resp = panel.show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.panel(ui, *id));
            });
            // dragging the border stores the new share
            let new = (resp.response.rect.width() / total.max(1.0) * 100.0).round() as u16;
            if resp.response.drag_stopped() && self.m.layout.size_of(*id) != Some(new) {
                self.m.layout.set_size(*id, new);
                self.m.save_layout();
            }
        }
        egui::CentralPanel::default().show(ui, |ui| self.panel(ui, last));
        if self.settings {
            self.settings_window(ui.ctx());
        }
    }
}

impl Gui {
    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut cfg: Vec<(&str, toml::Value)> = Vec::new();
        egui::Window::new("Settings").open(&mut open).default_width(380.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Lyrics");
                let mut mode = self.m.translate_mode;
                egui::ComboBox::from_label("Translation shown as")
                    .selected_text(mode.name())
                    .show_ui(ui, |ui| {
                        for name in TranslateMode::NAMES {
                            let v = TranslateMode::parse(name).unwrap_or_default();
                            ui.selectable_value(&mut mode, v, *name);
                        }
                    });
                if mode != self.m.translate_mode {
                    self.m.set_translate_mode(mode);
                    cfg.push(("translate_mode", mode.name().into()));
                    cfg.push(("translate", (mode != TranslateMode::Off).into()));
                }
                let mut karaoke = self.m.args.karaoke;
                if ui.checkbox(&mut karaoke, "Light up the line word by word").changed() {
                    self.m.args.karaoke = karaoke;
                    cfg.push(("karaoke", karaoke.into()));
                }
                let mut lang = self.m.args.translate_lang.clone();
                ui.horizontal(|ui| {
                    ui.label("Translate into");
                    if ui.text_edit_singleline(&mut lang).lost_focus() && lang != self.m.args.translate_lang && !lang.trim().is_empty() {
                        self.m.args.translate_lang = lang.trim().to_string();
                        cfg.push(("translate_lang", self.m.args.translate_lang.clone().into()));
                        self.m.ensure_translation();
                    }
                });
                let mut engine_ai = self.m.args.translate_engine == TranslateEngine::Ai;
                if ui.checkbox(&mut engine_ai, "Translate with an AI model (see README: Ollama is free)").changed() {
                    self.m.args.translate_engine = if engine_ai { TranslateEngine::Ai } else { TranslateEngine::Standard };
                    cfg.push(("translate_engine", self.m.args.translate_engine.name().into()));
                    self.m.translations.clear();
                    self.m.ensure_translation();
                }
                if engine_ai {
                    let mut model = self.m.args.ai_model.clone();
                    ui.horizontal(|ui| {
                        ui.label("AI model");
                        if ui.text_edit_singleline(&mut model).lost_focus() && model != self.m.args.ai_model {
                            self.m.args.ai_model = model.trim().to_string();
                            cfg.push(("ai.model", self.m.args.ai_model.clone().into()));
                        }
                    });
                    ui.label(
                        egui::RichText::new(format!("Lyrics and song info go to the AI provider ({}).", self.m.args.ai_provider))
                            .small()
                            .weak(),
                    );
                }
                ui.separator();
                ui.heading("Chords and notes");
                let mut chords = self.m.chords_on;
                if ui.checkbox(&mut chords, "Chords above the lyrics").changed() {
                    self.m.toggle_chords();
                    cfg.push(("chords", self.m.chords_on.into()));
                }
                let mut notes = self.m.annotations_shown();
                if ui.checkbox(&mut notes, "Annotations panel (experimental)").changed() {
                    self.m.layout.panels.set(PanelId::Annotations, notes);
                    self.m.args.annotations = notes;
                    self.m.ensure_annotations();
                    cfg.push(("annotations", notes.into()));
                }
                let mut ai_notes = self.m.args.annotations_mode == AnnotationMode::Ai;
                if ui.checkbox(&mut ai_notes, "…as AI-generated notes instead of Genius").changed() {
                    self.m.args.annotations_mode = if ai_notes { AnnotationMode::Ai } else { AnnotationMode::Genius };
                    cfg.push(("annotations_mode", self.m.args.annotations_mode.name().into()));
                }
                ui.separator();
                ui.heading("Layout");
                if ui.button(if self.customize { "Hide the layout editor" } else { "Customize layout…" }).clicked() {
                    self.customize = !self.customize;
                }
                if self.customize {
                    self.layout_editor(ui);
                }
            });
        });
        if !cfg.is_empty() {
            self.m.save_settings(&cfg);
        }
        self.settings = open;
    }

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
