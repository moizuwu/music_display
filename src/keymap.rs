//! The key table (`app::BINDS`) with the `[keys]` section of config.toml applied on top.
//!
//! `[keys]` maps an action name to one key or a list of keys; an entry REPLACES that action's
//! default keys. Nothing in here can fail the program: every problem becomes a warning with the
//! line it is on, and the action keeps its default keys. `quit` and `help` never lose their
//! default keys (an override only adds more), and Ctrl+C always quits (handled by the input loop).
//! The library, settings and help overlays have their own fixed keys and are not remappable.

use crate::app::{Action, BINDS};
use crossterm::event::KeyCode;
use std::path::Path;

impl Action {
    /// The stable snake_case name used in config.toml and `--print-config`.
    pub fn name(self) -> &'static str {
        match self {
            Action::Quit => "quit",
            Action::PlayPause => "play_pause",
            Action::Next => "next",
            Action::Prev => "prev",
            Action::Shuffle => "shuffle",
            Action::Repeat => "repeat",
            Action::SeekBack => "seek_back",
            Action::SeekFwd => "seek_fwd",
            Action::SyncEarlier => "sync_earlier",
            Action::SyncLater => "sync_later",
            Action::VolUp => "vol_up",
            Action::VolDown => "vol_down",
            Action::Mute => "mute",
            Action::ToggleLyrics => "toggle_lyrics",
            Action::ToggleBig => "toggle_big",
            Action::Karaoke => "karaoke",
            Action::NextLyrics => "next_lyrics",
            Action::BigScale => "big_scale",
            Action::LineSpacing => "line_spacing",
            Action::LyricPad => "lyric_pad",
            Action::ToggleCava => "toggle_cava",
            Action::ToggleChords => "toggle_chords",
            Action::TransposeUp => "transpose_up",
            Action::TransposeDown => "transpose_down",
            Action::CapoDown => "capo_down",
            Action::CapoUp => "capo_up",
            Action::CapoReset => "capo_reset",
            Action::Diagrams => "diagrams",
            Action::Minimal => "minimal",
            Action::Translate => "translate",
            Action::TranslateBoth => "translate_both",
            Action::Settings => "settings",
            Action::Library => "library",
            Action::Like => "like",
            Action::PlayHere => "play_here",
            Action::Devices => "devices",
            Action::BrowseUp => "browse_up",
            Action::BrowseDown => "browse_down",
            Action::BrowsePageUp => "browse_page_up",
            Action::BrowsePageDown => "browse_page_down",
            Action::BrowseEnd => "browse_end",
            Action::Help => "help",
        }
    }

    /// The action called `name` in config.toml (only actions that have a key in the table).
    pub fn from_name(name: &str) -> Option<Action> {
        BINDS.iter().map(|b| b.action).find(|a| a.name() == name)
    }

    /// `quit` and `help` always keep their default keys.
    fn keeps_defaults(self) -> bool {
        matches!(self, Action::Quit | Action::Help)
    }
}

const KEY_HELP: &str =
    "use one character or a name (space, enter, esc, tab, backspace, left, right, up, down, pageup, pagedown, home, end, f1-f12)";

/// One key as written in config.toml: a single character (case-sensitive) or a name.
pub fn parse_key(s: &str) -> Result<KeyCode, String> {
    if s.is_empty() {
        return Err(format!("an empty key: {KEY_HELP}"));
    }
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(KeyCode::Char(c));
    }
    let lower = s.to_lowercase();
    if let Some((head, _)) = lower.split_once(['+', '-']) {
        if ["ctrl", "control", "alt", "shift", "meta", "super", "cmd"].contains(&head) {
            return Err(format!(
                "'{s}': modifiers (ctrl, alt, shift) are not supported; use the plain key (\"T\" for a capital)"
            ));
        }
    }
    Ok(match lower.as_str() {
        "space" => KeyCode::Char(' '),
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        f if f.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n)) => {
            KeyCode::F(f[1..].parse().unwrap_or(1))
        }
        _ => return Err(format!("'{s}' is not a key: {KEY_HELP}")),
    })
}

/// A key as `[keys]` writes it (what `parse_key` reads back).
pub fn key_name(c: KeyCode) -> String {
    match c {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::F(n) => format!("f{n}"),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// A key as the help overlay and the footer show it.
pub fn key_label(c: KeyCode) -> String {
    match c {
        KeyCode::Left => "←".into(),
        KeyCode::Right => "→".into(),
        KeyCode::Up => "↑".into(),
        KeyCode::Down => "↓".into(),
        KeyCode::PageUp => "PgUp".into(),
        KeyCode::PageDown => "PgDn".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::F(n) => format!("F{n}"),
        other => key_name(other),
    }
}

#[derive(Clone, Debug)]
struct Entry {
    action: Action,
    codes: Vec<KeyCode>,
    /// Set by a `[keys]` entry (the help shows the keys as configured).
    custom: bool,
}

/// The effective key table: BINDS plus the overrides that were valid.
#[derive(Clone, Debug)]
pub struct Keymap {
    entries: Vec<Entry>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            entries: BINDS
                .iter()
                .map(|b| Entry { action: b.action, codes: b.codes.to_vec(), custom: false })
                .collect(),
        }
    }
}

fn default_codes(a: Action) -> &'static [KeyCode] {
    BINDS.iter().find(|b| b.action == a).map_or(&[], |b| b.codes)
}

impl Keymap {
    pub fn action_for(&self, code: KeyCode) -> Option<Action> {
        self.entries.iter().find(|e| e.codes.contains(&code)).map(|e| e.action)
    }

    pub fn codes(&self, action: Action) -> &[KeyCode] {
        self.entries.iter().find(|e| e.action == action).map_or(&[], |e| &e.codes)
    }

    /// How the help overlay writes the keys of `action`: the table's own text for the defaults,
    /// the configured keys joined with ` / ` otherwise.
    pub fn label(&self, action: Action) -> String {
        match self.entries.iter().find(|e| e.action == action) {
            Some(e) if e.custom => e.codes.iter().map(|c| key_label(*c)).collect::<Vec<_>>().join(" / "),
            _ => BINDS.iter().find(|b| b.action == action).map_or_else(String::new, |b| b.keys.to_string()),
        }
    }

    /// The first key of `action`, for the one-line hints in the footer.
    pub fn first(&self, action: Action) -> String {
        self.codes(action).first().map_or_else(|| "?".into(), |c| key_label(*c))
    }

    /// The actions in table order (the rows of the key editor in the help overlay).
    pub fn actions(&self) -> Vec<Action> {
        self.entries.iter().map(|e| e.action).collect()
    }

    /// Give `action` the key `code` (replacing its keys; `quit` and `help` keep their defaults
    /// too). Fails, changing nothing, when another action already has that key.
    pub fn rebind(&mut self, action: Action, code: KeyCode) -> Result<(), String> {
        if let Some(other) = self.entries.iter().find(|e| e.action != action && e.codes.contains(&code)) {
            let what = BINDS.iter().find(|b| b.action == other.action).map_or("another action", |b| b.desc);
            return Err(format!("{} is already used for: {what}", key_label(code)));
        }
        let mut codes = vec![code];
        if action.keeps_defaults() {
            codes.extend(default_codes(action).iter().filter(|d| **d != code));
        }
        self.set_codes(action, codes);
        Ok(())
    }

    /// Back to the built-in keys of `action`.
    pub fn reset(&mut self, action: Action) {
        self.set_codes(action, default_codes(action).to_vec());
    }

    fn set_codes(&mut self, action: Action, codes: Vec<KeyCode>) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.action == action) {
            e.custom = codes != default_codes(action);
            e.codes = codes;
        }
    }

    /// What `[keys]` holds for `action`: its keys, or `None` when it has the built-in ones.
    pub fn config_value(&self, action: Action) -> Option<toml::Value> {
        let e = self.entries.iter().find(|e| e.action == action && e.custom)?;
        Some(toml::Value::Array(e.codes.iter().map(|c| toml::Value::String(key_name(*c))).collect()))
    }

    /// The effective bindings as a TOML `[keys]` block (what `--print-config` shows); it parses
    /// back to the same keymap.
    pub fn render(&self) -> String {
        let mut o = String::from("\n# effective key bindings (the [keys] section; one key or a list per action)\n[keys]\n");
        for e in &self.entries {
            let keys: Vec<toml::Value> = e.codes.iter().map(|c| toml::Value::String(key_name(*c))).collect();
            o.push_str(&format!("{} = {}\n", e.action.name(), toml::Value::Array(keys)));
        }
        o
    }

    /// Apply the `[keys]` table. `raw` / `path` only locate warnings. Returns the keymap and the
    /// problems as `(line, message)`, in file order.
    pub fn from_table(keys: &toml::Value, raw: &str, path: &Path) -> (Keymap, Vec<String>) {
        let mut problems: Vec<(usize, String)> = Vec::new();
        let Some(table) = keys.as_table() else {
            let line = line_in(raw, None, "keys");
            let msg = format!("{}:{line}: [keys] must be a table of action = \"key\" lines (ignored)", path.display());
            return (Keymap::default(), vec![msg]);
        };

        struct Cand {
            line: usize,
            action: Action,
            codes: Vec<KeyCode>,
        }
        let mut cands: Vec<Cand> = Vec::new();
        for (name, val) in table {
            let line = line_in(raw, Some("keys"), name);
            let mut bad = |why: String| problems.push((line, format!("{name}: {why}; keeping the default")));
            let Some(action) = Action::from_name(name) else {
                bad("unknown action (the README lists them)".into());
                continue;
            };
            let strings: Vec<&str> = match val {
                toml::Value::String(s) => vec![s.as_str()],
                toml::Value::Array(a) if a.iter().all(|v| v.is_str()) => a.iter().filter_map(|v| v.as_str()).collect(),
                _ => {
                    bad("expected a key or a list of keys, as strings".into());
                    continue;
                }
            };
            if strings.is_empty() {
                bad("no key given".into());
                continue;
            }
            let mut codes: Vec<KeyCode> = Vec::new();
            let mut failed = false;
            for s in strings {
                match parse_key(s) {
                    Ok(c) if codes.contains(&c) => {
                        bad(format!("the same key twice ('{s}')"));
                        failed = true;
                    }
                    Ok(c) => codes.push(c),
                    Err(why) => {
                        bad(why);
                        failed = true;
                    }
                }
            }
            if !failed {
                cands.push(Cand { line, action, codes });
            }
        }
        // file order: a later line gives way to an earlier one when they clash
        cands.sort_by_key(|c| if c.line == 0 { usize::MAX } else { c.line });

        let mut map = Keymap::default();
        let mut order: Vec<(Action, usize)> = Vec::new(); // action -> candidate line
        for c in &cands {
            let e = map.entries.iter_mut().find(|e| e.action == c.action).expect("every action has a bind");
            e.codes = c.codes.clone();
            if c.action.keeps_defaults() {
                for d in default_codes(c.action) {
                    if !e.codes.contains(d) {
                        e.codes.push(*d);
                    }
                }
            }
            e.custom = true;
            order.push((c.action, c.line));
        }

        // A key held by two actions: the override that brought it in gives way (the later line
        // first); defaults can't be taken, so quit / help keep theirs.
        loop {
            let mut clash: Option<(KeyCode, usize, usize)> = None;
            'find: for (i, a) in map.entries.iter().enumerate() {
                for (j, b) in map.entries.iter().enumerate().skip(i + 1) {
                    if let Some(k) = a.codes.iter().find(|k| b.codes.contains(k)) {
                        clash = Some((*k, i, j));
                        break 'find;
                    }
                }
            }
            let Some((key, i, j)) = clash else { break };
            let brought = |idx: usize| {
                let e = &map.entries[idx];
                e.custom && !default_codes(e.action).contains(&key)
            };
            let line_of = |idx: usize| order.iter().find(|(a, _)| *a == map.entries[idx].action).map_or(0, |(_, l)| *l);
            let loser = match (brought(i), brought(j)) {
                (true, true) => if line_of(j) >= line_of(i) { j } else { i },
                (true, false) => i,
                (false, true) => j,
                (false, false) => break, // two defaults: the table itself is tested to have none
            };
            let other = if loser == i { j } else { i };
            let (action, other_name) = (map.entries[loser].action, map.entries[other].action.name());
            problems.push((
                line_of(loser),
                format!(
                    "{}: '{}' is already bound to {other_name}; keeping the default",
                    action.name(),
                    key_name(key)
                ),
            ));
            map.entries[loser] = Entry { action, codes: default_codes(action).to_vec(), custom: false };
        }

        problems.sort();
        let msgs = problems
            .into_iter()
            .map(|(line, m)| format!("{}:{line}: [keys] {m}", path.display()))
            .collect();
        (map, msgs)
    }
}

/// 1-based line of `key = …` inside the `[section]` of `raw` (0 when it isn't there; for an
/// inline table `keys = { … }` the line of `keys` itself).
fn line_in(raw: &str, section: Option<&str>, key: &str) -> usize {
    let is_key = |l: &str| {
        let l = l.trim_start();
        let l = l.strip_prefix('"').unwrap_or(l);
        l.strip_prefix(key).is_some_and(|rest| rest.trim_start_matches('"').trim_start().starts_with('='))
    };
    let Some(sec) = section else {
        return raw.lines().position(is_key).map_or(0, |i| i + 1);
    };
    let mut inside = false;
    for (i, l) in raw.lines().enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            inside = t == format!("[{sec}]");
            continue;
        }
        if inside && is_key(l) {
            return i + 1;
        }
    }
    line_in(raw, None, sec)
}

/// The keymap for the config file at `path`: the defaults when there is no file, no `[keys]`
/// section or the file is not valid TOML (the settings loader already warned about that).
pub fn load(path: &Path, warn: &mut dyn FnMut(String)) -> Keymap {
    let Ok(raw) = std::fs::read_to_string(path) else { return Keymap::default() };
    let Ok(table) = raw.parse::<toml::Table>() else { return Keymap::default() };
    let Some(keys) = table.get("keys") else { return Keymap::default() };
    let (map, problems) = Keymap::from_table(keys, &raw, path);
    for p in problems {
        warn(p);
    }
    map
}
