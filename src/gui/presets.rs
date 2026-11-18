//! The "Presets" section at the top of the Appearance tab: pick a saved style (look and layout)
//! to apply it, save the current one under a name, update, rename or delete. The data and the
//! file are `music_display::presets`; nothing here is asked of the OS, the confirmations are
//! drawn in the window.
use super::model::Model;
use super::style::{self, Icon};
use egui::Ui;
use music_display::presets::{self, Presets};

/// What a pressed button waits for the second press of.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Ask {
    #[default]
    Nothing,
    Overwrite,
    Delete,
    Rename,
}

#[derive(Default)]
pub struct State {
    /// Read from `presets.toml` the first time the section is drawn.
    list: Option<Presets>,
    selected: String,
    /// The name typed for a new preset, and for a rename.
    new_name: String,
    rename_to: String,
    ask: Ask,
    /// The last problem (a bad name, a file that could not be written).
    error: Option<String>,
}

impl State {
    fn list<'a>(list: &'a mut Option<Presets>, m: &Model) -> &'a mut Presets {
        list.get_or_insert_with(|| Presets::load(&presets::path_for(&m.cfg), &mut |w| eprintln!("music_display: {w}")))
    }

    fn write(&mut self, m: &Model) {
        let path = presets::path_for(&m.cfg);
        if let Some(Err(e)) = self.list.as_ref().map(|l| l.save(&path)) {
            self.error = Some(format!("couldn't save {}: {e}", path.display()));
        }
    }
}

/// Everything of the window that a preset holds is replaced at once and written to
/// `config.toml` like the Appearance tab and the layout editor do.
fn apply(m: &mut Model, name: &str, list: &Presets) {
    let Some(style) = list.get(name) else { return };
    m.gui = style.gui;
    m.gui.clamp();
    m.layout = style.layout;
    m.args.thirdpanel = !m.layout.cava_under_lyrics;
    let t = m.gui.write();
    m.save_settings(&[("gui", toml::Value::Table(t))]);
    m.save_layout();
    m.say(format!("Applied the preset \"{name}\""));
}

/// The section: a list to pick from and the buttons for the picked one.
pub fn section(ui: &mut Ui, m: &mut Model, st: &mut State) {
    style::heading(ui, "Presets");
    style::note(ui, "A preset is the whole look and the panel layout under a name. Applying one changes everything at once; save the current one first to get back to it.");
    let names = State::list(&mut st.list, m).names();
    if !names.contains(&st.selected) {
        st.selected = names[0].clone();
        st.ask = Ask::Nothing;
    }
    let built_in = presets::BUILT_IN.iter().any(|b| b.eq_ignore_ascii_case(&st.selected));
    ui.horizontal(|ui| {
        ui.label("Preset");
        egui::ComboBox::from_id_salt("style-preset").selected_text(st.selected.clone()).width(200.0).show_ui(ui, |ui| {
            for n in &names {
                if ui.selectable_label(*n == st.selected, n).clicked() && *n != st.selected {
                    st.selected = n.clone();
                    st.ask = Ask::Nothing;
                    st.error = None;
                }
            }
        });
        if ui.button("Apply").on_hover_text("Replace the look and the layout with this preset").clicked() {
            let list = State::list(&mut st.list, m).clone();
            apply(m, &st.selected, &list);
        }
    });
    if built_in {
        style::note(ui, "Built in: it can't be changed, renamed or deleted.");
    } else {
        manage(ui, m, st);
    }
    save_new(ui, m, st);
    if let Some(e) = &st.error {
        style::error(ui, e);
    }
}

/// Update, rename and delete of a saved preset, each asking again where it destroys something.
fn manage(ui: &mut Ui, m: &mut Model, st: &mut State) {
    let name = st.selected.clone();
    match st.ask {
        Ask::Overwrite => {
            let go = confirm(ui, &format!("Overwrite \"{name}\" with the current look and layout?"), "Yes, overwrite");
            if go == Some(true) {
                let result = State::list(&mut st.list, m).set(&name, &m.gui, &m.layout);
                done(st, m, result);
            } else if go == Some(false) {
                st.ask = Ask::Nothing;
            }
        }
        Ask::Delete => {
            let go = confirm(ui, &format!("Delete \"{name}\"?"), "Yes, delete");
            if go == Some(true) {
                let result = State::list(&mut st.list, m).remove(&name);
                done(st, m, result);
            } else if go == Some(false) {
                st.ask = Ask::Nothing;
            }
        }
        Ask::Rename => {
            let mut go = None;
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut st.rename_to).desired_width(180.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Rename").clicked() || enter {
                    go = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    go = Some(false);
                }
            });
            if go == Some(true) {
                let to = st.rename_to.trim().to_string();
                let result = State::list(&mut st.list, m).rename(&name, &to);
                if result.is_ok() {
                    st.selected = to;
                }
                done(st, m, result);
            } else if go == Some(false) {
                st.ask = Ask::Nothing;
            }
        }
        Ask::Nothing => {
            ui.horizontal(|ui| {
                if style::tool(ui, style::ROW_SIDE, Icon::Refresh, false, "Update: overwrite this preset with the current look and layout") {
                    st.ask = Ask::Overwrite;
                }
                if ui.button("Rename…").clicked() {
                    st.rename_to = name.clone();
                    st.ask = Ask::Rename;
                }
                if style::tool(ui, style::ROW_SIDE, Icon::Close, false, "Delete this preset") {
                    st.ask = Ask::Delete;
                }
            });
        }
    }
}

/// A question with a button for yes and one for no: `Some(answer)` once one is pressed.
fn confirm(ui: &mut Ui, question: &str, yes: &str) -> Option<bool> {
    let mut answer = None;
    ui.horizontal(|ui| {
        ui.label(question);
        if ui.button(yes).clicked() {
            answer = Some(true);
        }
        if ui.button("Cancel").clicked() {
            answer = Some(false);
        }
    });
    answer
}

/// The end of an update, rename or delete: the file is written, or the problem shown.
fn done(st: &mut State, m: &Model, result: Result<(), String>) {
    st.ask = Ask::Nothing;
    match result {
        Ok(()) => {
            st.error = None;
            st.write(m);
        }
        Err(e) => st.error = Some(e),
    }
}

/// "Save current as…": a name and a button. A name that exists asks before it overwrites.
fn save_new(ui: &mut Ui, m: &mut Model, st: &mut State) {
    ui.horizontal(|ui| {
        ui.label("Save current as…");
        let r = ui.add(egui::TextEdit::singleline(&mut st.new_name).hint_text("name").desired_width(180.0));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let plus = style::tool(ui, style::ROW_SIDE, Icon::Plus, false, "Save the current look and layout under this name");
        if (plus || enter) && !st.new_name.trim().is_empty() {
            let name = st.new_name.trim().to_string();
            let list = State::list(&mut st.list, m);
            if list.names().contains(&name) && !list.is_built_in(&name) {
                // an existing name goes through the same question as Update
                st.selected = name;
                st.ask = Ask::Overwrite;
                return;
            }
            let result = list.set(&name, &m.gui, &m.layout);
            if result.is_ok() {
                st.selected = name;
                st.new_name.clear();
            }
            done(st, m, result);
        }
    });
}
