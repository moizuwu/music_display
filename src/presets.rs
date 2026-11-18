//! Named style presets of the `--gui` window: a snapshot of the whole look (`[gui]`) and the
//! panel layout (`[layout]`), kept in `presets.toml` next to `config.toml`. Plain data and file
//! handling, no egui, so it is tested without a window; the buttons are `src/gui/presets.rs`.
//!
//! The file has one table per preset, each holding the same `gui` and `layout` tables as the
//! config file (so the keys are read and written by `GuiConf` and `Layout` themselves):
//! `[preset."My look".gui]` and `[preset."My look".layout]`. Two presets are built in, cannot
//! be changed or deleted and are not in the file.
use crate::guiconf::{Accent, Background, Effects, GuiConf, Theme, Weight};
use crate::layout::Layout;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The built-in presets, in the order they are listed.
pub const BUILT_IN: &[&str] = &["Default", "Spotify"];

/// A look and a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub gui: GuiConf,
    pub layout: Layout,
}

/// The built-in preset of this name (case-insensitive), if there is one.
pub fn built_in(name: &str) -> Option<Style> {
    let name = name.trim();
    if name.eq_ignore_ascii_case("default") {
        return Some(Style { gui: GuiConf::default(), layout: Layout::default() });
    }
    name.eq_ignore_ascii_case("spotify").then(spotify)
}

/// Spotify's own look: black window, dark grey panels, white and grey text, its green as the
/// accent, no effects, arranged like the Spotify app. The font is left to the built-in sans one so it works everywhere.
fn spotify() -> Style {
    let gui = GuiConf {
        theme: Theme::Dark,
        accent: Accent::Fixed((0x1e, 0xd7, 0x60)),
        background: Background::Solid,
        effects: Effects::None,
        window_color: Some((0x00, 0x00, 0x00)),
        card_color: Some((0x12, 0x12, 0x12)),
        text_color: Some((0xff, 0xff, 0xff)),
        muted_color: Some((0xb3, 0xb3, 0xb3)),
        panel_opacity: 100,
        corner_radius: 8,
        panel_borders: false,
        spotify_layout: true,
        active_weight: Weight::Bold,
        inactive_fade: 50,
        ..GuiConf::default()
    };
    Style { gui, layout: Layout::default() }
}

/// Where the presets of the config file at `cfg` live.
pub fn path_for(cfg: &Path) -> PathBuf {
    cfg.with_file_name("presets.toml")
}

/// The presets the user saved, by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presets {
    user: BTreeMap<String, Style>,
}

impl Presets {
    /// Read `path`; a missing file is no presets, and every problem goes to `warn`.
    pub fn load(path: &Path, warn: &mut dyn FnMut(String)) -> Self {
        let mut p = Presets::default();
        let Ok(raw) = std::fs::read_to_string(path) else { return p };
        let root: toml::Table = match raw.parse() {
            Ok(r) => r,
            Err(e) => {
                warn(format!("{}: {e}; no presets read", path.display()));
                return p;
            }
        };
        let mut warn = |m: String| warn(format!("{}: {m}", path.display()));
        for k in root.keys().filter(|k| *k != "preset") {
            warn(format!("{k}: unknown key (ignored); the file only has [preset.\"name\"] tables"));
        }
        match root.get("preset").map(|v| v.as_table().ok_or(())) {
            Some(Ok(t)) => {
                for (name, v) in t {
                    match v.as_table() {
                        Some(t) => p.read_one(name, t, &mut warn),
                        None => warn(format!("preset \"{name}\": expected a table; ignored")),
                    }
                }
            }
            Some(Err(())) => warn("preset: expected tables; ignored".into()),
            None => {}
        }
        p
    }

    fn read_one(&mut self, name: &str, t: &toml::Table, warn: &mut dyn FnMut(String)) {
        if let Err(e) = check_name(name) {
            warn(format!("preset \"{name}\": {e}; ignored"));
            return;
        }
        if built_in(name).is_some() {
            warn(format!("preset \"{name}\": that name is built in; ignored"));
            return;
        }
        for k in t.keys().filter(|k| !["gui", "layout"].contains(&k.as_str())) {
            warn(format!("preset \"{name}\": {k}: unknown key (ignored); keys are gui, layout"));
        }
        // the tables are read where `GuiConf` and `Layout` look for them: under `gui` / `layout`
        let mut here = |m: String| warn(format!("preset \"{name}\": {m}"));
        let (mut gui, mut layout) = (GuiConf::default(), Layout::default());
        gui.read(t, &mut here);
        layout.read(t, &mut here);
        self.user.insert(name.trim().to_string(), Style { gui, layout });
    }

    /// Write the presets to `path` (the folder is made when needed).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut all = toml::Table::new();
        for (name, s) in &self.user {
            let mut t = toml::Table::new();
            t.insert("gui".into(), toml::Value::Table(s.gui.write()));
            t.insert("layout".into(), toml::Value::Table(s.layout.write()));
            all.insert(name.clone(), toml::Value::Table(t));
        }
        let mut root = toml::Table::new();
        root.insert("preset".into(), toml::Value::Table(all));
        let text = toml::to_string(&root).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)
    }

    /// Every preset name: the built-in ones, then the saved ones alphabetically.
    pub fn names(&self) -> Vec<String> {
        BUILT_IN.iter().map(|s| s.to_string()).chain(self.user.keys().cloned()).collect()
    }

    pub fn is_built_in(&self, name: &str) -> bool {
        built_in(name).is_some()
    }

    /// The look and layout to apply for `name`.
    pub fn get(&self, name: &str) -> Option<Style> {
        built_in(name).or_else(|| self.user.get(name.trim()).cloned())
    }

    /// Save `gui` and `layout` under `name`: a new preset, or the one of that name overwritten.
    pub fn set(&mut self, name: &str, gui: &GuiConf, layout: &Layout) -> Result<(), String> {
        check_name(name)?;
        if built_in(name).is_some() {
            return Err(format!("\"{}\" is built in and can't be changed", name.trim()));
        }
        self.user.insert(name.trim().to_string(), Style { gui: gui.clone(), layout: layout.clone() });
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        if built_in(name).is_some() {
            return Err(format!("\"{}\" is built in and can't be deleted", name.trim()));
        }
        self.user.remove(name.trim()).map(|_| ()).ok_or_else(|| format!("no preset called \"{}\"", name.trim()))
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        check_name(to)?;
        if built_in(from).is_some() {
            return Err(format!("\"{}\" is built in and can't be renamed", from.trim()));
        }
        let to = to.trim();
        if to != from.trim() && (built_in(to).is_some() || self.user.contains_key(to)) {
            return Err(format!("there is a preset called \"{to}\" already"));
        }
        let s = self.user.remove(from.trim()).ok_or_else(|| format!("no preset called \"{}\"", from.trim()))?;
        self.user.insert(to.to_string(), s);
        Ok(())
    }
}

/// A name must have something in it and fit on one line.
fn check_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        Err("the name is empty".into())
    } else if n.chars().any(char::is_control) {
        Err("the name can't have control characters".into())
    } else if n.chars().count() > 60 {
        Err("the name is longer than 60 characters".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mdis-presets-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("presets.toml")
    }

    fn warnings(p: &Path) -> (Presets, Vec<String>) {
        let mut w = Vec::new();
        (Presets::load(p, &mut |m| w.push(m)), w)
    }

    fn changed() -> (GuiConf, Layout) {
        let mut g = GuiConf { corner_radius: 20, lyrics_size: 30, ..GuiConf::default() };
        g.accent = Accent::Fixed((1, 2, 3));
        g.popped = vec!["cava".into()];
        let l = Layout { direction: crate::layout::Split::Vertical, cava_under_lyrics: false, ..Layout::default() };
        (g, l)
    }

    #[test]
    fn built_ins_are_there_and_fixed() {
        let mut p = Presets::default();
        assert_eq!(p.names(), ["Default", "Spotify"]);
        assert_eq!(p.get("Default"), Some(Style { gui: GuiConf::default(), layout: Layout::default() }));
        assert!(p.is_built_in("spotify"));
        let (g, l) = changed();
        assert!(p.set("Default", &g, &l).is_err());
        assert!(p.set(" spotify ", &g, &l).is_err());
        assert!(p.remove("Spotify").is_err());
        assert!(p.rename("Default", "Mine").is_err());
        p.set("Mine", &g, &l).unwrap();
        assert!(p.rename("Mine", "Spotify").is_err());
        assert_eq!(p.names(), ["Default", "Spotify", "Mine"]);
    }

    #[test]
    fn spotify_has_its_colours() {
        let s = built_in("Spotify").unwrap().gui;
        assert_eq!(s.accent, Accent::Fixed((30, 215, 96)));
        assert_eq!((s.window_color, s.card_color), (Some((0, 0, 0)), Some((18, 18, 18))));
        assert_eq!((s.text_color, s.muted_color), (Some((255, 255, 255)), Some((179, 179, 179))));
        assert_eq!((s.theme, s.effects, s.spotify_layout), (Theme::Dark, Effects::None, true));
        // and it survives the file round trip of `[gui]`
        let mut back = GuiConf::default();
        back.read(&toml::Table::from_iter([("gui".to_string(), toml::Value::Table(s.write()))]), &mut |m| panic!("{m}"));
        assert_eq!(back, s);
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = tmp("round");
        let (g, l) = changed();
        let mut p = Presets::default();
        p.set("My look", &g, &l).unwrap();
        p.set("Other \"quoted\"", &GuiConf::default(), &l).unwrap();
        p.save(&path).unwrap();
        let (back, w) = warnings(&path);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(back, p);
        let s = back.get("My look").unwrap();
        assert_eq!((s.gui, s.layout), (g, l));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn overwrite_rename_and_delete() {
        let (g, l) = changed();
        let mut p = Presets::default();
        p.set("A", &GuiConf::default(), &Layout::default()).unwrap();
        p.set("A", &g, &l).unwrap();
        assert_eq!(p.get("A").unwrap().gui, g);
        p.set("B", &g, &l).unwrap();
        assert!(p.rename("A", "B").is_err());
        p.rename("A", "C").unwrap();
        assert!(p.get("A").is_none() && p.get("C").is_some());
        p.remove("C").unwrap();
        assert!(p.remove("C").is_err());
        assert!(p.set("  ", &g, &l).is_err());
    }

    #[test]
    fn a_missing_file_is_no_presets() {
        let (p, w) = warnings(&tmp("missing"));
        assert!(p.user.is_empty() && w.is_empty());
    }

    #[test]
    fn a_bad_file_warns_and_never_fails() {
        let path = tmp("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is = = not toml").unwrap();
        let (p, w) = warnings(&path);
        assert!(p.user.is_empty());
        assert_eq!(w.len(), 1);
        std::fs::write(
            &path,
            "stray = 1\n[preset.Ok]\nextra = 1\n[preset.Ok.gui]\nopacity = 70\nnope = 1\ncorner_radius = \"x\"\n\
             [preset.Ok.layout]\ndirection = \"sideways\"\n[preset.Spotify.gui]\nopacity = 1\n[preset.Str]\n",
        )
        .unwrap();
        let (p, w) = warnings(&path);
        let joined = w.join("\n");
        for needle in ["stray", "extra", "nope", "corner_radius", "layout.direction", "built in"] {
            assert!(joined.contains(needle), "{needle} not in {joined}");
        }
        assert_eq!(p.get("Ok").unwrap().gui.opacity, 70);
        assert!(p.get("Str").is_some());
        assert_eq!(p.get("Spotify"), built_in("Spotify"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_file_sits_next_to_the_config() {
        assert_eq!(path_for(Path::new("/a/b/config.toml")), PathBuf::from("/a/b/presets.toml"));
    }
}
