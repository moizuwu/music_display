//! The `[gui]` table of `config.toml`: how the `--gui` window looks. Plain data with no egui in
//! it, so it can be read, written, printed (`--print-config`) and tested without a window; the
//! drawing side is `src/gui/style.rs`. Every value is clamped to a sane range on the way in.
use crate::color::{self, Rgb};
use std::path::Path;

/// Defines a small string-valued choice with `NAMES`, `parse` and `name`.
macro_rules! choice {
    ($(#[$m:meta])* $t:ident { $($v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum $t { $($v),+ }
        impl $t {
            pub const NAMES: &'static [&'static str] = &[$($s),+];
            pub fn parse(s: &str) -> Option<Self> {
                match s.trim().to_ascii_lowercase().as_str() { $($s => Some(Self::$v),)+ _ => None }
            }
            pub fn name(self) -> &'static str {
                match self { $(Self::$v => $s),+ }
            }
        }
    };
}

choice!(
    /// Light or dark; `System` follows the desktop.
    Theme { System = "system", Dark = "dark", Light = "light" }
);
choice!(Background { Solid = "solid", CoverBlur = "cover-blur", Gradient = "gradient" });
choice!(Align { Left = "left", Center = "center" });
choice!(Animation { Smooth = "smooth", Instant = "instant" });
choice!(Weight { Regular = "regular", Medium = "medium", Bold = "bold" });

/// The accent colour: taken from the cover art, or a fixed one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    Cover,
    Fixed(Rgb),
}

impl Accent {
    pub fn name(self) -> String {
        match self {
            Accent::Cover => "cover".into(),
            Accent::Fixed((r, g, b)) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        if s.trim().eq_ignore_ascii_case("cover") {
            return Some(Accent::Cover);
        }
        color::parse_hex(s).map(Accent::Fixed)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct GuiConf {
    /// Window background, percent. Below 100 the window is see-through (needs a compositor).
    pub opacity: u8,
    /// The panel cards on top of the background, percent.
    pub panel_opacity: u8,
    pub theme: Theme,
    pub accent: Accent,
    pub background: Background,
    /// A system font family; empty = the built-in one.
    pub font: String,
    /// Sizes in points.
    pub lyrics_size: u16,
    pub active_size: u16,
    pub meta_size: u16,
    pub active_weight: Weight,
    pub corner_radius: u8,
    pub panel_spacing: u8,
    pub panel_borders: bool,
    pub lyrics_align: Align,
    /// How much lines fade the further they are from the active one, percent.
    pub inactive_fade: u8,
    pub lyrics_animation: Animation,
    /// Reopen at the size and place the window was closed at.
    pub remember_window: bool,
    pub always_on_top: bool,
    pub decorations: bool,
}

impl Default for GuiConf {
    fn default() -> Self {
        GuiConf {
            opacity: 100,
            panel_opacity: 100,
            theme: Theme::System,
            accent: Accent::Cover,
            background: Background::Solid,
            font: String::new(),
            lyrics_size: 18,
            active_size: 24,
            meta_size: 14,
            active_weight: Weight::Regular,
            corner_radius: 8,
            panel_spacing: 8,
            panel_borders: true,
            lyrics_align: Align::Left,
            inactive_fade: 40,
            lyrics_animation: Animation::Smooth,
            remember_window: true,
            always_on_top: false,
            decorations: true,
        }
    }
}

const KEYS: &str = "opacity, panel_opacity, theme, accent, background, font, lyrics_size, active_size, meta_size, \
active_weight, corner_radius, panel_spacing, panel_borders, lyrics_align, inactive_fade, lyrics_animation, \
remember_window, always_on_top, decorations";

impl GuiConf {
    /// Pull every value back into its range (the sliders and the file both go through this).
    pub fn clamp(&mut self) {
        self.opacity = self.opacity.min(100);
        self.panel_opacity = self.panel_opacity.min(100);
        self.lyrics_size = self.lyrics_size.clamp(10, 64);
        self.active_size = self.active_size.clamp(14, 72);
        self.meta_size = self.meta_size.clamp(8, 40);
        self.corner_radius = self.corner_radius.min(32);
        self.panel_spacing = self.panel_spacing.min(32);
        self.inactive_fade = self.inactive_fade.min(100);
        self.font = self.font.trim().to_string();
    }

    /// Read the `[gui]` table of a parsed config file onto `self`. Problems go to `warn`; a bad
    /// value keeps the current one and an out-of-range number is clamped.
    pub fn read(&mut self, root: &toml::Table, warn: &mut dyn FnMut(String)) {
        let Some(v) = root.get("gui") else { return };
        let Some(t) = v.as_table() else {
            warn("gui: expected a table ([gui]); ignored".into());
            return;
        };
        for (k, v) in t {
            let mut int = |lo: i64, hi: i64, slot: &mut i64| match v.as_integer() {
                Some(n) => {
                    if !(lo..=hi).contains(&n) {
                        warn(format!("[gui] {k}: {n} is outside {lo}..{hi}; using {}", n.clamp(lo, hi)));
                    }
                    *slot = n.clamp(lo, hi);
                }
                None => warn(format!("[gui] {k}: expected a whole number; using {slot}")),
            };
            match k.as_str() {
                "opacity" => self.opacity = num(&mut int, 0, 100, self.opacity as i64) as u8,
                "panel_opacity" => self.panel_opacity = num(&mut int, 0, 100, self.panel_opacity as i64) as u8,
                "lyrics_size" => self.lyrics_size = num(&mut int, 10, 64, self.lyrics_size as i64) as u16,
                "active_size" => self.active_size = num(&mut int, 14, 72, self.active_size as i64) as u16,
                "meta_size" => self.meta_size = num(&mut int, 8, 40, self.meta_size as i64) as u16,
                "corner_radius" => self.corner_radius = num(&mut int, 0, 32, self.corner_radius as i64) as u8,
                "panel_spacing" => self.panel_spacing = num(&mut int, 0, 32, self.panel_spacing as i64) as u8,
                "inactive_fade" => self.inactive_fade = num(&mut int, 0, 100, self.inactive_fade as i64) as u8,
                "panel_borders" | "remember_window" | "always_on_top" | "decorations" => {
                    let Some(b) = v.as_bool() else {
                        warn(format!("[gui] {k}: expected true or false; using the default"));
                        continue;
                    };
                    match k.as_str() {
                        "panel_borders" => self.panel_borders = b,
                        "remember_window" => self.remember_window = b,
                        "always_on_top" => self.always_on_top = b,
                        _ => self.decorations = b,
                    }
                }
                "theme" | "accent" | "background" | "font" | "active_weight" | "lyrics_align" | "lyrics_animation" => {
                    let Some(s) = v.as_str() else {
                        warn(format!("[gui] {k}: expected a string; using the default"));
                        continue;
                    };
                    let ok = match k.as_str() {
                        "theme" => Theme::parse(s).map(|x| self.theme = x).is_some(),
                        "accent" => Accent::parse(s).map(|x| self.accent = x).is_some(),
                        "background" => Background::parse(s).map(|x| self.background = x).is_some(),
                        "active_weight" => Weight::parse(s).map(|x| self.active_weight = x).is_some(),
                        "lyrics_align" => Align::parse(s).map(|x| self.lyrics_align = x).is_some(),
                        "lyrics_animation" => Animation::parse(s).map(|x| self.lyrics_animation = x).is_some(),
                        _ => {
                            self.font = s.trim().to_string();
                            true
                        }
                    };
                    if !ok {
                        warn(format!("[gui] {k}: unknown value '{s}'{}; using the default", expected(k)));
                    }
                }
                _ => warn(format!("[gui] {k}: unknown key (ignored); keys are {KEYS}")),
            }
        }
        self.clamp();
    }

    /// The `[gui]` table to write back.
    pub fn write(&self) -> toml::Table {
        let mut t = toml::Table::new();
        let mut put = |k: &str, v: toml::Value| {
            t.insert(k.to_string(), v);
        };
        put("opacity", (self.opacity as i64).into());
        put("panel_opacity", (self.panel_opacity as i64).into());
        put("theme", self.theme.name().into());
        put("accent", self.accent.name().into());
        put("background", self.background.name().into());
        put("font", self.font.clone().into());
        put("lyrics_size", (self.lyrics_size as i64).into());
        put("active_size", (self.active_size as i64).into());
        put("meta_size", (self.meta_size as i64).into());
        put("active_weight", self.active_weight.name().into());
        put("corner_radius", (self.corner_radius as i64).into());
        put("panel_spacing", (self.panel_spacing as i64).into());
        put("panel_borders", self.panel_borders.into());
        put("lyrics_align", self.lyrics_align.name().into());
        put("inactive_fade", (self.inactive_fade as i64).into());
        put("lyrics_animation", self.lyrics_animation.name().into());
        put("remember_window", self.remember_window.into());
        put("always_on_top", self.always_on_top.into());
        put("decorations", self.decorations.into());
        t
    }

    /// The `[gui]` block `--print-config` shows (valid config to paste back).
    pub fn render(&self) -> String {
        format!("\n[gui]\n{}", toml::to_string(&self.write()).unwrap_or_default())
    }
}

/// Run a range-checked integer read and hand back the (possibly unchanged) value.
fn num(int: &mut impl FnMut(i64, i64, &mut i64), lo: i64, hi: i64, current: i64) -> i64 {
    let mut v = current;
    int(lo, hi, &mut v);
    v
}

fn expected(key: &str) -> String {
    let list = match key {
        "theme" => Theme::NAMES.join(", "),
        "background" => Background::NAMES.join(", "),
        "active_weight" => Weight::NAMES.join(", "),
        "lyrics_align" => Align::NAMES.join(", "),
        "lyrics_animation" => Animation::NAMES.join(", "),
        _ => "cover or a hex colour like #3a7bd5".into(),
    };
    format!(" (expected {list})")
}

/// The `[gui]` settings of the file at `path` (defaults when it is missing or broken).
pub fn load(path: &Path, warn: &mut dyn FnMut(String)) -> GuiConf {
    let mut g = GuiConf::default();
    if let Ok(raw) = std::fs::read_to_string(path) {
        if let Ok(root) = raw.parse::<toml::Table>() {
            g.read(&root, &mut |m| warn(format!("{}: {m}", path.display())));
        }
    }
    g
}

/// Store `conf` as the `[gui]` table, keeping every other key.
pub fn save(path: &Path, conf: &GuiConf) -> std::io::Result<()> {
    crate::config::save(path, &[("gui", toml::Value::Table(conf.write()))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> (GuiConf, Vec<String>) {
        let root: toml::Table = src.parse().unwrap();
        let mut g = GuiConf::default();
        let mut w = Vec::new();
        g.read(&root, &mut |m| w.push(m));
        (g, w)
    }

    #[test]
    fn defaults_when_missing() {
        let (g, w) = parse("player = \"x\"\n");
        assert_eq!(g, GuiConf::default());
        assert!(w.is_empty());
    }

    #[test]
    fn reads_every_key() {
        let (g, w) = parse(
            "[gui]\nopacity = 80\npanel_opacity = 55\ntheme = \"dark\"\naccent = \"#ff8800\"\nbackground = \"cover-blur\"\n\
             font = \"Noto Sans\"\nlyrics_size = 20\nactive_size = 30\nmeta_size = 12\nactive_weight = \"bold\"\n\
             corner_radius = 4\npanel_spacing = 12\npanel_borders = false\nlyrics_align = \"center\"\ninactive_fade = 70\n\
             lyrics_animation = \"instant\"\nremember_window = false\nalways_on_top = true\ndecorations = false\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((g.opacity, g.panel_opacity, g.theme, g.background), (80, 55, Theme::Dark, Background::CoverBlur));
        assert_eq!(g.accent, Accent::Fixed((255, 136, 0)));
        assert_eq!(g.font, "Noto Sans");
        assert_eq!((g.lyrics_size, g.active_size, g.meta_size, g.active_weight), (20, 30, 12, Weight::Bold));
        assert_eq!((g.corner_radius, g.panel_spacing, g.panel_borders), (4, 12, false));
        assert_eq!((g.lyrics_align, g.inactive_fade, g.lyrics_animation), (Align::Center, 70, Animation::Instant));
        assert_eq!((g.remember_window, g.always_on_top, g.decorations), (false, true, false));
    }

    #[test]
    fn clamps_and_warns() {
        let (g, w) = parse("[gui]\nopacity = 250\nlyrics_size = 1\nactive_size = 9999\ncorner_radius = -3\ninactive_fade = 101\n");
        assert_eq!((g.opacity, g.lyrics_size, g.active_size, g.corner_radius, g.inactive_fade), (100, 10, 72, 0, 100));
        assert_eq!(w.len(), 5, "{w:?}");
    }

    #[test]
    fn bad_values_keep_the_default() {
        let (g, w) = parse(
            "[gui]\ntheme = \"neon\"\naccent = \"not-a-colour\"\nopacity = \"high\"\npanel_borders = 1\nwat = 1\nbackground = 3\n",
        );
        assert_eq!(g, GuiConf::default());
        assert_eq!(w.len(), 6, "{w:?}");
        assert!(w.iter().any(|m| m.contains("wat") && m.contains("unknown key")));
        let (_, w) = parse("gui = 3\n");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn accent_names() {
        assert_eq!(Accent::parse("Cover"), Some(Accent::Cover));
        assert_eq!(Accent::parse("#fff"), Some(Accent::Fixed((255, 255, 255))));
        assert_eq!(Accent::Fixed((1, 2, 255)).name(), "#0102ff");
        assert_eq!(Accent::parse("blue"), None);
    }

    #[test]
    fn write_read_round_trip() {
        let g = GuiConf {
            opacity: 63,
            theme: Theme::Light,
            accent: Accent::Fixed((10, 200, 30)),
            background: Background::Gradient,
            font: "DejaVu Sans".into(),
            active_weight: Weight::Bold,
            panel_borders: false,
            lyrics_align: Align::Center,
            lyrics_animation: Animation::Instant,
            always_on_top: true,
            ..GuiConf::default()
        };
        let mut root = toml::Table::new();
        root.insert("gui".into(), toml::Value::Table(g.write()));
        let mut back = GuiConf::default();
        let mut w = Vec::new();
        back.read(&root, &mut |m| w.push(m));
        assert!(w.is_empty());
        assert_eq!(back, g);
        // --print-config output is valid config too
        let (printed, w) = parse(&g.render());
        assert!(w.is_empty());
        assert_eq!(printed, g);
    }

    #[test]
    fn save_keeps_other_keys_and_loads_back() {
        let dir = std::env::temp_dir().join(format!("md-guiconf-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "player = \"mpd\"\n[layout]\ndirection = \"row\"\n").unwrap();
        let g = GuiConf { opacity: 40, theme: Theme::Dark, ..GuiConf::default() };
        save(&path, &g).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("player") && raw.contains("[layout]") && raw.contains("[gui]"));
        assert_eq!(load(&path, &mut |_| {}), g);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn main_config_ignores_the_gui_table() {
        // `[gui]` must not trigger "unknown key" in the shared reader
        use clap::{CommandFactory, FromArgMatches};
        let m = crate::cli::Args::command().get_matches_from(["music_display"]);
        let mut args = crate::cli::Args::from_arg_matches(&m).unwrap();
        let path = std::env::temp_dir().join(format!("md-guiconf-apply-{}.toml", std::process::id()));
        std::fs::write(&path, "[gui]\nopacity = 50\n").unwrap();
        let mut w = Vec::new();
        crate::config::apply(&mut args, &m, &path, true, &mut |s| w.push(s));
        std::fs::remove_file(&path).ok();
        assert!(w.is_empty(), "{w:?}");
    }
}
