//! Keyboard: the window uses the terminal UI's own key table (`app::BINDS`, plus the `[keys]`
//! section of the config file), so a key does the same thing in both. egui events are turned
//! into the table's key codes here (letters come from the typed text, so `L` and `l` are told
//! apart by what was typed and not by which physical key); the actions are run on the model.
//! The help window is generated from the same table.
use super::model::Model;
use crate::app::{Action, BINDS, GROUPS};
use crossterm::event::KeyCode;
use egui::Event;
use music_display::cli::TranslateMode;
use music_display::layout::PanelId;
use music_display::spotify;

/// What running an action asks of the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    Stay,
    Quit,
}

/// The key code of one egui event, if it is a key the table knows. Typed text gives the
/// letters, digits and symbols (shift already applied: `L` is `Char('L')`); the keys that type
/// nothing come from key events. Anything with ctrl / alt / the command key is not ours.
pub fn code_of(ev: &Event) -> Option<KeyCode> {
    match ev {
        Event::Text(s) => {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if !c.is_control() => Some(KeyCode::Char(c)),
                _ => None,
            }
        }
        Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
            use egui::Key as K;
            Some(match key {
                K::ArrowLeft => KeyCode::Left,
                K::ArrowRight => KeyCode::Right,
                K::ArrowUp => KeyCode::Up,
                K::ArrowDown => KeyCode::Down,
                K::PageUp => KeyCode::PageUp,
                K::PageDown => KeyCode::PageDown,
                K::End => KeyCode::End,
                K::Escape => KeyCode::Esc,
                K::Enter => KeyCode::Enter,
                K::Tab => KeyCode::Tab,
                _ => return None,
            })
        }
        _ => None,
    }
}

impl Model {
    /// Run one action of the key table.
    pub fn run_action(&mut self, a: Action) -> Flow {
        match a {
            Action::Quit => {
                // Esc / q close what is open first
                if self.ux.ai_ask {
                    // the question needs an answer
                } else if self.ux.help {
                    self.ux.help = false;
                } else if self.ux.s.open {
                    self.ux.s.open = false;
                } else if self.spot.devices.is_some() {
                    self.spot.devices = None;
                } else if self.spot.open {
                    self.spot.open = false;
                } else {
                    return Flow::Quit;
                }
            }
            Action::PlayPause => self.play_pause(),
            Action::Next => self.next(),
            Action::Prev => self.prev(),
            Action::Shuffle => self.toggle_shuffle(),
            Action::Repeat => self.cycle_repeat(),
            Action::SeekBack => {
                let p = self.position();
                self.seek((p - 5.0).max(0.0));
            }
            Action::SeekFwd => {
                let p = self.position();
                self.seek(p + 5.0);
            }
            Action::SyncEarlier => self.nudge(-0.1),
            Action::SyncLater => self.nudge(0.1),
            Action::VolUp => self.volume_step(1),
            Action::VolDown => self.volume_step(-1),
            Action::Mute => self.toggle_mute(),
            Action::ToggleLyrics => {
                let on = !self.layout.panels.shown(PanelId::Lyrics);
                self.layout.panels.set(PanelId::Lyrics, on);
            }
            Action::ToggleBig => {
                self.args.big_lyrics = !self.args.big_lyrics;
                self.say(format!("big active line {}", on_off(self.args.big_lyrics)));
            }
            Action::Karaoke => {
                self.args.karaoke = !self.args.karaoke;
                self.say(format!("word-by-word highlight {}", on_off(self.args.karaoke)));
            }
            Action::NextLyrics => self.next_lyrics(),
            Action::BigScale => {
                self.args.big_scale = self.args.big_scale % 3 + 1;
                self.say(format!("big line size {}", self.args.big_scale));
            }
            Action::LineSpacing => {
                self.args.lyric_spacing = (self.args.lyric_spacing + 1) % 4;
                self.say(format!("line spacing {}", self.args.lyric_spacing));
            }
            Action::LyricPad => {
                let pad = self.args.lyric_pad + 2;
                self.args.lyric_pad = if pad > 10 { 0 } else { pad };
                self.say(format!("side padding {}", self.args.lyric_pad));
            }
            Action::ToggleCava => {
                self.toggle_cava();
            }
            Action::ToggleChords => self.toggle_chords(),
            Action::TransposeUp => self.transpose = (self.transpose + 1).rem_euclid(12),
            Action::TransposeDown => self.transpose = (self.transpose - 1).rem_euclid(12),
            Action::CapoDown => self.change_capo(-1),
            Action::CapoUp => self.change_capo(1),
            Action::CapoReset => self.set_capo_override(None),
            Action::Diagrams => {
                self.show_diagrams = !self.show_diagrams;
                let on = self.show_diagrams;
                self.layout.panels.set(PanelId::Chords, on);
            }
            Action::Annotations => {
                let on = !self.annotations_shown();
                self.layout.panels.set(PanelId::Annotations, on);
                self.args.annotations = on;
                self.ensure_annotations();
            }
            Action::NotesDown => self.ux.scroll_notes(2.0),
            Action::NotesUp => self.ux.scroll_notes(-2.0),
            Action::Minimal => {
                let on = !self.x.minimal;
                self.set_minimal(on);
            }
            Action::Translate => {
                let mode = if self.translate_mode == TranslateMode::Off { TranslateMode::Translated } else { TranslateMode::Off };
                self.show_translation(mode);
            }
            Action::TranslateBoth => {
                let mode = if self.translate_mode == TranslateMode::Both { TranslateMode::Off } else { TranslateMode::Both };
                self.show_translation(mode);
            }
            Action::Settings => self.ux.s.open = !self.ux.s.open,
            Action::Help => self.ux.help = !self.ux.help,
            Action::Library => {
                if self.spot.on() {
                    super::library::toggle(self);
                } else {
                    self.say("Spotify is off (settings → Spotify)");
                }
            }
            Action::Like => {
                if self.spot.on() {
                    let uri = self.playing_uri();
                    self.spot.toggle_like(uri);
                    self.take_spot_note();
                } else {
                    self.say("Spotify is off (settings → Spotify)");
                }
            }
            Action::PlayHere => {
                if self.spot.on() {
                    self.spot.cmd(spotify::Cmd::Transfer);
                    self.take_spot_note();
                } else {
                    self.say("Spotify is off (settings → Spotify)");
                }
            }
            Action::Devices => {
                if self.spot.on() {
                    self.spot.open_devices();
                } else {
                    self.say("Spotify is off (settings → Spotify)");
                }
            }
            Action::BrowseUp => self.ux.browse_by(-1.0),
            Action::BrowseDown => self.ux.browse_by(1.0),
            Action::BrowsePageUp => self.ux.browse_by(-8.0),
            Action::BrowsePageDown => self.ux.browse_by(8.0),
            Action::BrowseEnd => self.ux.browse_end(),
        }
        Flow::Stay
    }

    /// `K` / `k`: a translation mode, with the lyrics panel on.
    fn show_translation(&mut self, mode: TranslateMode) {
        if mode != TranslateMode::Off {
            self.layout.panels.set(PanelId::Lyrics, true);
        }
        self.set_translate_mode(mode);
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

impl super::Gui {
    /// Shortcuts (not while typing in a field). The library window takes its own keys first.
    pub(super) fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let codes: Vec<KeyCode> = ctx.input(|i| i.events.iter().filter_map(code_of).collect());
        for code in codes {
            if super::library::key(&mut self.m, code) {
                continue;
            }
            if let Some(a) = self.m.ux.keys.action_for(code) {
                if self.m.run_action(a) == Flow::Quit {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

impl super::Gui {
    /// Every window that floats over the panels.
    pub(super) fn popups(&mut self, ctx: &egui::Context) {
        super::library::windows(ctx, &mut self.m);
        if self.m.ux.s.open {
            self.settings_window(ctx);
        }
        help_window(ctx, &mut self.m);
        super::privacy::window(ctx, &mut self.m);
    }
}

// ── the help window ─────────────────────────────────────────────────────────

/// Window-only gestures (the key table is above them in the window).
const MOUSE: &[(&str, &str)] = &[
    ("click the bar", "seek there"),
    ("click a lyric line", "seek to that line"),
    ("wheel over the volume", "volume up / down"),
    ("click the volume %", "mute / restore"),
    ("click Key / BPM / Capo / Tuning", "open the chord diagrams"),
    ("right-click a metadata field", "Hide, Move up, Move down"),
    ("drag a column border", "resize the columns (saved)"),
];

const LIBRARY: &[(&str, &str)] = &[
    ("↑ ↓ PgUp PgDn", "move"),
    ("Enter", "play"),
    ("→ / ←", "open / back"),
    ("Tab", "next tab"),
    ("Esc", "close the library"),
];

pub fn help_window(ctx: &egui::Context, m: &mut Model) {
    if !m.ux.help {
        return;
    }
    let mut open = true;
    egui::Window::new("Keys").open(&mut open).default_width(420.0).default_height(520.0).show(ctx, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(egui::RichText::new("The same keys as the terminal UI (remap them in [keys] of config.toml). Not while typing in a field.").small().weak());
            for g in GROUPS {
                ui.add_space(4.0);
                ui.heading(g);
                egui::Grid::new(("help", g)).num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                    for b in BINDS.iter().filter(|b| b.group == g) {
                        ui.monospace(m.ux.keys.label(b.action));
                        ui.label(b.desc);
                        ui.end_row();
                    }
                });
            }
            ui.add_space(4.0);
            ui.heading("Mouse");
            egui::Grid::new("help-mouse").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                for (k, d) in MOUSE {
                    ui.label(*k);
                    ui.label(*d);
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.heading("Spotify library window");
            egui::Grid::new("help-lib").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                for (k, d) in LIBRARY {
                    ui.monospace(*k);
                    ui.label(*d);
                    ui.end_row();
                }
            });
        });
    });
    if !open {
        m.ux.help = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use music_display::cli::Args;
    use music_display::layout::Layout;
    use music_display::AppEvent;

    fn text(s: &str) -> Event {
        Event::Text(s.into())
    }

    fn key(k: egui::Key, shift: bool) -> Event {
        Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: egui::Modifiers { shift, ..Default::default() } }
    }

    fn act(ev: Event) -> Option<Action> {
        crate::keymap::Keymap::default().action_for(code_of(&ev)?)
    }

    #[test]
    fn shift_is_told_apart_by_the_typed_text() {
        // l toggles the lyrics, L steps through the versions; the same physical key
        assert_eq!(act(text("l")), Some(Action::ToggleLyrics));
        assert_eq!(act(text("L")), Some(Action::NextLyrics));
        assert_eq!(act(text("t")), Some(Action::TransposeUp));
        assert_eq!(act(text("T")), Some(Action::TransposeDown));
        assert_eq!(act(text("d")), Some(Action::Devices));
        assert_eq!(act(text("D")), Some(Action::Diagrams));
        assert_eq!(act(text("n")), Some(Action::Next));
        assert_eq!(act(text("N")), Some(Action::Annotations));
        assert_eq!(act(text("m")), Some(Action::Mute));
        assert_eq!(act(text("M")), Some(Action::Minimal));
        assert_eq!(act(text("c")), Some(Action::ToggleCava));
        assert_eq!(act(text("g")), Some(Action::ToggleChords));
        assert_eq!(act(text("k")), Some(Action::TranslateBoth));
        assert_eq!(act(text("K")), Some(Action::Translate));
        assert_eq!(act(text("e")), Some(Action::NotesDown));
        assert_eq!(act(text("E")), Some(Action::NotesUp));
        // a letter key event alone is not a key: the text event carries the case
        assert_eq!(act(key(egui::Key::L, true)), None);
        assert_eq!(act(key(egui::Key::L, false)), None);
    }

    #[test]
    fn the_rest_of_the_table_maps_like_the_terminal() {
        assert_eq!(act(text(" ")), Some(Action::PlayPause));
        assert_eq!(act(text("+")), Some(Action::VolUp));
        assert_eq!(act(text("=")), Some(Action::VolUp));
        assert_eq!(act(text("-")), Some(Action::VolDown));
        assert_eq!(act(text(",")), Some(Action::SyncEarlier));
        assert_eq!(act(text(".")), Some(Action::SyncLater));
        assert_eq!(act(text("]")), Some(Action::CapoUp));
        assert_eq!(act(text("[")), Some(Action::CapoDown));
        assert_eq!(act(text("0")), Some(Action::CapoReset));
        assert_eq!(act(text("?")), Some(Action::Help));
        assert_eq!(act(text("s")), Some(Action::Settings));
        assert_eq!(act(text("S")), Some(Action::Settings));
        assert_eq!(act(text("o")), Some(Action::Library));
        assert_eq!(act(text("f")), Some(Action::Like));
        assert_eq!(act(text("H")), Some(Action::PlayHere));
        assert_eq!(act(text("q")), Some(Action::Quit));
        assert_eq!(act(key(egui::Key::Escape, false)), Some(Action::Quit));
        assert_eq!(act(key(egui::Key::ArrowLeft, false)), Some(Action::SeekBack));
        assert_eq!(act(key(egui::Key::ArrowRight, false)), Some(Action::SeekFwd));
        assert_eq!(act(key(egui::Key::ArrowUp, false)), Some(Action::BrowseUp));
        assert_eq!(act(key(egui::Key::PageDown, false)), Some(Action::BrowsePageDown));
        assert_eq!(act(key(egui::Key::End, false)), Some(Action::BrowseEnd));
    }

    #[test]
    fn ctrl_chords_and_control_text_are_not_keys() {
        let ctrl = Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { ctrl: true, command: true, ..Default::default() },
        };
        assert_eq!(code_of(&ctrl), None);
        assert_eq!(code_of(&text("\r")), None);
        assert_eq!(code_of(&text("ab")), None);
        let up = Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() };
        assert_eq!(code_of(&up), None);
    }

    fn model() -> (Model, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        music_display::util::enter_test_mode();
        let args = Args::parse_from(["music_display", "--genre-lookup", "false", "--no-cache-translations"]);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Model::new(args, Layout::default(), std::env::temp_dir().join("md-gui-keys.toml"), tx), rx)
    }

    #[tokio::test]
    async fn esc_closes_windows_before_it_quits() {
        let (mut m, _rx) = model();
        m.ux.help = true;
        m.ux.s.open = true;
        assert_eq!(m.run_action(Action::Quit), Flow::Stay);
        assert!(!m.ux.help && m.ux.s.open);
        assert_eq!(m.run_action(Action::Quit), Flow::Stay);
        assert!(!m.ux.s.open);
        assert_eq!(m.run_action(Action::Quit), Flow::Quit);
        // the AI question is not dismissed by Esc
        m.ux.ai_ask = true;
        assert_eq!(m.run_action(Action::Quit), Flow::Stay);
        assert!(m.ux.ai_ask);
    }

    #[tokio::test]
    async fn view_keys_change_what_the_terminal_keys_change() {
        let (mut m, _rx) = model();
        assert!(m.layout.panels.shown(PanelId::Lyrics));
        m.run_action(Action::ToggleLyrics);
        assert!(!m.layout.panels.shown(PanelId::Lyrics));
        m.run_action(Action::BigScale);
        assert_eq!(m.args.big_scale, 2);
        m.run_action(Action::BigScale);
        m.run_action(Action::BigScale);
        assert_eq!(m.args.big_scale, 1);
        for _ in 0..4 {
            m.run_action(Action::LineSpacing);
        }
        assert_eq!(m.args.lyric_spacing, 1, "four steps wrap round to the default 1");
        for _ in 0..6 {
            m.run_action(Action::LyricPad);
        }
        assert_eq!(m.args.lyric_pad, 0, "2, 4, 6, 8, 10, then round to 0");
        m.run_action(Action::TransposeDown);
        assert_eq!((m.transpose, super::super::ux::signed_transpose(m.transpose)), (11, -1));
        m.run_action(Action::TranslateBoth);
        assert_eq!(m.translate_mode, TranslateMode::Both);
        m.run_action(Action::TranslateBoth);
        assert_eq!(m.translate_mode, TranslateMode::Off);
        m.run_action(Action::Settings);
        assert!(m.ux.s.open);
        m.run_action(Action::Help);
        assert!(m.ux.help);
        m.run_action(Action::BrowseDown);
        assert!(m.ux.browsing());
        m.run_action(Action::BrowseEnd);
        assert!(!m.ux.browsing());
    }

    #[tokio::test]
    async fn spotify_keys_say_so_when_spotify_is_off() {
        let (mut m, _rx) = model();
        m.run_action(Action::Library);
        assert!(m.note.as_ref().is_some_and(|(n, _)| n.contains("Spotify is off")));
        assert!(!m.spot.open);
    }
}
