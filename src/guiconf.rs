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
choice!(
    /// What is painted behind the panels. `Image` is the file named by `background_image`; `Sky` is
    /// a sky with a low sun, `Meadow` the sky with flowers and `Lawn` blades of grass. Things that
    /// fall or drift on top of it are the `effects`.
    Background {
        Solid = "solid",
        CoverBlur = "cover-blur",
        Gradient = "gradient",
        Image = "image",
        Sky = "sky",
        Meadow = "meadow",
        Lawn = "lawn",
    }
);
choice!(
    /// What moves over the background (drawn live, no files). `Auto` is the effect of the look:
    /// Halloween fog, embers and bats; Christmas snow and a string of lights; Easter dandelion
    /// seeds and eggs; Midsummer pollen and wind; no look, nothing.
    Effects {
        Auto = "auto",
        None = "none",
        Snow = "snow",
        Winter = "winter",
        Leaves = "leaves",
        Seeds = "seeds",
        Wind = "wind",
        Embers = "embers",
        Fog = "fog",
        Bats = "bats",
        Haunt = "haunt",
        Pollen = "pollen",
        Eggs = "eggs",
    }
);
choice!(
    /// A named look: picking one in the Appearance tab sets the matching values below, which can
    /// then be changed one by one. The name stays so the look's own touches (its window colour)
    /// keep applying.
    Preset {
        None = "none",
        Halloween = "halloween",
        Christmas = "christmas",
        Easter = "easter",
        Midsummer = "midsummer",
    }
);
choice!(
    /// Where the window takes its colours from besides the look: `File` a colour file of a
    /// wallpaper tool (matugen, noctalia…), `Gtk` the GTK theme, `Auto` the file, then GTK.
    ColorsFrom { Off = "off", File = "file", Gtk = "gtk", Auto = "auto" }
);
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
    /// What moves over the background, and how much (0-100 %: how many pieces, how strong).
    pub effects: Effects,
    pub effects_intensity: u8,
    /// Colours of the system (off by default): the window's base colour, text and accent
    /// follow them, whatever look is on. `colors_file` names the colour file (empty = the usual
    /// places the terminal UI looks at).
    pub colors: ColorsFrom,
    pub colors_file: String,
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
    /// Size of everything in the window, percent (the zoom keys change it too).
    pub ui_scale: u16,
    pub theme_preset: Preset,
    /// Picture for `background = "image"`; `~` is the home folder.
    pub background_image: String,
    /// A halo around the active lyric line, percent (0 = none).
    pub lyrics_glow: u8,
    /// The column borders cannot be dragged.
    pub lock_layout: bool,
    /// Size of the translated line in points; 0 = the text size of the metadata.
    pub translation_size: u16,
    /// Size of the member tag in points; 0 = small.
    pub member_size: u16,
    /// Sizes in pixels of the panels inside a column, remembered when they are dragged:
    /// `cava` (its height under the lyrics), `diagrams` (their width beside the lyrics) and
    /// `stack` (the height of the diagrams above the notes).
    pub inner_sizes: std::collections::BTreeMap<String, u16>,
    /// Panels shown in a window of their own (layout names like `lyrics`, `cava`).
    pub popped: Vec<String>,
}

impl Default for GuiConf {
    fn default() -> Self {
        GuiConf {
            opacity: 100,
            panel_opacity: 100,
            theme: Theme::System,
            accent: Accent::Cover,
            background: Background::Solid,
            effects: Effects::Auto,
            effects_intensity: 60,
            colors: ColorsFrom::Off,
            colors_file: String::new(),
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
            ui_scale: 100,
            theme_preset: Preset::None,
            background_image: String::new(),
            lyrics_glow: 0,
            lock_layout: false,
            translation_size: 0,
            member_size: 0,
            inner_sizes: Default::default(),
            popped: Vec::new(),
        }
    }
}

impl Preset {
    /// The window colour this look uses instead of the plain dark / light one.
    pub fn base(self, dark: bool) -> Option<Rgb> {
        match (self, dark) {
            (Preset::None, _) => None,
            (Preset::Halloween, true) => Some((17, 9, 24)),
            (Preset::Halloween, false) => Some((250, 241, 230)),
            (Preset::Christmas, true) => Some((9, 23, 18)),
            (Preset::Christmas, false) => Some((236, 244, 238)),
            (Preset::Easter, true) => Some((30, 26, 38)),
            (Preset::Easter, false) => Some((253, 248, 240)),
            (Preset::Midsummer, true) => Some((14, 28, 30)),
            (Preset::Midsummer, false) => Some((246, 250, 236)),
        }
    }

    /// The fill of the panel cards in this look, when it has its own.
    pub fn card(self, dark: bool) -> Option<Rgb> {
        match (self, dark) {
            (Preset::None, _) => None,
            (Preset::Halloween, true) => Some((38, 20, 50)),
            (Preset::Halloween, false) => Some((255, 249, 240)),
            (Preset::Christmas, true) => Some((22, 44, 36)),
            (Preset::Christmas, false) => Some((250, 253, 250)),
            (Preset::Easter, true) => Some((44, 38, 56)),
            (Preset::Easter, false) => Some((255, 252, 247)),
            (Preset::Midsummer, true) => Some((24, 44, 46)),
            (Preset::Midsummer, false) => Some((255, 255, 250)),
        }
    }
}

/// A background of an older version: the colour/scenery it is now, and the effect it had. The
/// meadow keeps its name and gets back its pollen.
fn legacy_background(s: &str) -> Option<(Background, Option<Effects>)> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "particles" => (Background::Solid, Some(Effects::Embers)),
        "fog" => (Background::Solid, Some(Effects::Fog)),
        "bats" => (Background::Solid, Some(Effects::Haunt)),
        "snow" => (Background::Solid, Some(Effects::Winter)),
        "eggs" => (Background::Lawn, Some(Effects::Eggs)),
        _ => return None,
    })
}

/// One thing that moves over the background.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fx {
    Snow,
    Lights,
    Leaves,
    Seeds,
    Wind,
    Embers,
    Fog,
    Bats,
    Pollen,
    Eggs,
}

impl GuiConf {
    /// The effects to draw now: what `effects` says, `auto` being the look's own. Empty when the
    /// intensity is zero.
    pub fn active_effects(&self) -> Vec<Fx> {
        if self.effects_intensity == 0 {
            return Vec::new();
        }
        match self.effects {
            Effects::None => vec![],
            Effects::Snow => vec![Fx::Snow],
            Effects::Winter => vec![Fx::Snow, Fx::Lights],
            Effects::Leaves => vec![Fx::Leaves],
            Effects::Seeds => vec![Fx::Seeds],
            Effects::Wind => vec![Fx::Wind],
            Effects::Embers => vec![Fx::Embers],
            Effects::Fog => vec![Fx::Fog],
            Effects::Bats => vec![Fx::Bats],
            Effects::Haunt => vec![Fx::Fog, Fx::Embers, Fx::Bats],
            Effects::Pollen => vec![Fx::Pollen],
            Effects::Eggs => vec![Fx::Eggs],
            Effects::Auto => match self.theme_preset {
                Preset::None => vec![],
                Preset::Halloween => vec![Fx::Fog, Fx::Embers, Fx::Bats],
                Preset::Christmas => vec![Fx::Snow, Fx::Lights],
                Preset::Easter => vec![Fx::Seeds, Fx::Eggs],
                Preset::Midsummer => vec![Fx::Pollen, Fx::Wind],
            },
        }
    }
}

const KEYS: &str = "opacity, panel_opacity, theme, accent, background, effects, effects_intensity, colors, colors_file, font, lyrics_size, active_size, meta_size, \
active_weight, corner_radius, panel_spacing, panel_borders, lyrics_align, inactive_fade, lyrics_animation, \
remember_window, always_on_top, decorations, ui_scale, theme_preset, background_image, lyrics_glow, lock_layout, translation_size, member_size, inner_sizes, popped";

impl GuiConf {
    /// Set the values the look is made of (everything else is left as the user has it, the
    /// dark / light theme included: every look has a palette for both).
    pub fn apply_preset(&mut self, p: Preset) {
        if self.theme_preset == Preset::Christmas && p != Preset::Christmas {
            self.font.clear();
        }
        self.theme_preset = p;
        match p {
            Preset::None => {}
            Preset::Halloween => {
                self.accent = Accent::Fixed((255, 122, 24));
                self.background = Background::Solid;
                self.effects = Effects::Auto;
                self.panel_opacity = 82;
                self.corner_radius = 14;
                self.panel_borders = true;
                self.active_weight = Weight::Bold;
                self.inactive_fade = 55;
                self.lyrics_glow = 60;
            }
            Preset::Christmas => {
                self.accent = Accent::Fixed((242, 201, 76));
                self.background = Background::Solid;
                self.effects = Effects::Auto;
                self.panel_opacity = 85;
                self.corner_radius = 12;
                self.panel_borders = true;
                self.active_weight = Weight::Bold;
                self.inactive_fade = 50;
                self.lyrics_glow = 40;
                self.font = "Mountains of Christmas".into();
            }
            Preset::Easter => {
                self.accent = Accent::Fixed((214, 92, 150));
                self.background = Background::Lawn;
                self.effects = Effects::Auto;
                self.panel_opacity = 88;
                self.corner_radius = 18;
                self.panel_borders = true;
                self.active_weight = Weight::Medium;
                self.inactive_fade = 35;
                self.lyrics_glow = 0;
            }
            Preset::Midsummer => {
                self.accent = Accent::Fixed((0, 106, 167));
                self.background = Background::Meadow;
                self.effects = Effects::Auto;
                self.panel_opacity = 85;
                self.corner_radius = 16;
                self.panel_borders = true;
                self.active_weight = Weight::Medium;
                self.inactive_fade = 35;
                self.lyrics_glow = 25;
            }
        }
        self.clamp();
    }

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
        self.ui_scale = self.ui_scale.clamp(50, 200);
        self.lyrics_glow = self.lyrics_glow.min(100);
        self.effects_intensity = self.effects_intensity.min(100);
        // 0 means "automatic"; anything else is a size in points
        if self.translation_size != 0 {
            self.translation_size = self.translation_size.clamp(8, 64);
        }
        if self.member_size != 0 {
            self.member_size = self.member_size.clamp(8, 40);
        }
        self.background_image = self.background_image.trim().to_string();
        self.colors_file = self.colors_file.trim().to_string();
        self.popped.dedup();
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
                "ui_scale" => self.ui_scale = num(&mut int, 50, 200, self.ui_scale as i64) as u16,
                "translation_size" => self.translation_size = num(&mut int, 0, 64, self.translation_size as i64) as u16,
                "member_size" => self.member_size = num(&mut int, 0, 40, self.member_size as i64) as u16,
                "effects_intensity" => self.effects_intensity = num(&mut int, 0, 100, self.effects_intensity as i64) as u8,
                "lyrics_glow" => self.lyrics_glow = num(&mut int, 0, 100, self.lyrics_glow as i64) as u8,
                "inner_sizes" => match v.as_table() {
                    Some(t) => {
                        self.inner_sizes = t
                            .iter()
                            .filter_map(|(k, v)| match v.as_integer() {
                                Some(n) => Some((k.clone(), n.clamp(40, 4000) as u16)),
                                None => {
                                    warn(format!("[gui] inner_sizes.{k}: expected a whole number; ignored"));
                                    None
                                }
                            })
                            .collect()
                    }
                    None => warn(format!("[gui] {k}: expected a table; ignored")),
                },
                "popped" => match v.as_array() {
                    Some(a) => self.popped = a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).collect(),
                    None => warn(format!("[gui] {k}: expected a list of panel names; ignored")),
                },
                "panel_borders" | "remember_window" | "always_on_top" | "decorations" | "lock_layout" => {
                    let Some(b) = v.as_bool() else {
                        warn(format!("[gui] {k}: expected true or false; using the default"));
                        continue;
                    };
                    match k.as_str() {
                        "panel_borders" => self.panel_borders = b,
                        "remember_window" => self.remember_window = b,
                        "always_on_top" => self.always_on_top = b,
                        "lock_layout" => self.lock_layout = b,
                        _ => self.decorations = b,
                    }
                }
                "theme" | "accent" | "background" | "font" | "active_weight" | "lyrics_align" | "lyrics_animation" | "theme_preset"
                | "background_image" | "effects" | "colors" | "colors_file" => {
                    let Some(s) = v.as_str() else {
                        warn(format!("[gui] {k}: expected a string; using the default"));
                        continue;
                    };
                    let ok = match k.as_str() {
                        "theme" => Theme::parse(s).map(|x| self.theme = x).is_some(),
                        "accent" => Accent::parse(s).map(|x| self.accent = x).is_some(),
                        "background" => match Background::parse(s).map(|b| (b, (b == Background::Meadow).then_some(Effects::Pollen))).or_else(|| legacy_background(s)) {
                            // (the old moving backgrounds: the effect they had is kept, unless
                            // the file names one of its own, which comes later in the table)
                            Some((b, fx)) => {
                                self.background = b;
                                if let Some(fx) = fx {
                                    self.effects = fx;
                                }
                                true
                            }
                            None => false,
                        },
                        "effects" => Effects::parse(s).map(|x| self.effects = x).is_some(),
                        "colors" => ColorsFrom::parse(s).map(|x| self.colors = x).is_some(),
                        "colors_file" => {
                            self.colors_file = s.trim().to_string();
                            true
                        }
                        "active_weight" => Weight::parse(s).map(|x| self.active_weight = x).is_some(),
                        "lyrics_align" => Align::parse(s).map(|x| self.lyrics_align = x).is_some(),
                        "lyrics_animation" => Animation::parse(s).map(|x| self.lyrics_animation = x).is_some(),
                        "theme_preset" => Preset::parse(s).map(|x| self.theme_preset = x).is_some(),
                        "background_image" => {
                            self.background_image = s.trim().to_string();
                            true
                        }
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
        put("effects", self.effects.name().into());
        put("colors", self.colors.name().into());
        put("colors_file", self.colors_file.clone().into());
        put("effects_intensity", (self.effects_intensity as i64).into());
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
        put("ui_scale", (self.ui_scale as i64).into());
        put("theme_preset", self.theme_preset.name().into());
        put("background_image", self.background_image.clone().into());
        put("lyrics_glow", (self.lyrics_glow as i64).into());
        put("lock_layout", self.lock_layout.into());
        put(
            "inner_sizes",
            toml::Value::Table(self.inner_sizes.iter().map(|(k, v)| (k.clone(), toml::Value::Integer(*v as i64))).collect()),
        );
        put("translation_size", (self.translation_size as i64).into());
        put("member_size", (self.member_size as i64).into());
        put("popped", toml::Value::Array(self.popped.iter().map(|s| s.clone().into()).collect()));
        t
    }

    /// The `[gui]` block `--print-config` shows (valid config to paste back).
    pub fn render(&self) -> String {
        // (as a table of its own, so the nested `inner_sizes` comes out as `[gui.inner_sizes]`)
        let mut root = toml::Table::new();
        root.insert("gui".into(), toml::Value::Table(self.write()));
        format!("\n{}", toml::to_string(&root).unwrap_or_default())
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
        "effects" => Effects::NAMES.join(", "),
        "colors" => ColorsFrom::NAMES.join(", "),
        "active_weight" => Weight::NAMES.join(", "),
        "lyrics_align" => Align::NAMES.join(", "),
        "lyrics_animation" => Animation::NAMES.join(", "),
        "theme_preset" => Preset::NAMES.join(", "),
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
             lyrics_animation = \"instant\"\nremember_window = false\nalways_on_top = true\ndecorations = false\n\
             ui_scale = 130\ntheme_preset = \"halloween\"\nbackground_image = \"~/a.png\"\nlyrics_glow = 60\nlock_layout = true\n\
             translation_size = 20\nmember_size = 11\npopped = [\"lyrics\", \"cava\"]\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((g.opacity, g.panel_opacity, g.theme, g.background), (80, 55, Theme::Dark, Background::CoverBlur));
        assert_eq!(g.accent, Accent::Fixed((255, 136, 0)));
        assert_eq!(g.font, "Noto Sans");
        assert_eq!((g.lyrics_size, g.active_size, g.meta_size, g.active_weight), (20, 30, 12, Weight::Bold));
        assert_eq!((g.corner_radius, g.panel_spacing, g.panel_borders), (4, 12, false));
        assert_eq!((g.lyrics_align, g.inactive_fade, g.lyrics_animation), (Align::Center, 70, Animation::Instant));
        assert_eq!((g.remember_window, g.always_on_top, g.decorations), (false, true, false));
        assert_eq!((g.ui_scale, g.theme_preset, g.lyrics_glow, g.lock_layout), (130, Preset::Halloween, 60, true));
        assert_eq!((g.translation_size, g.member_size), (20, 11));
        assert_eq!((g.background_image.as_str(), g.popped.as_slice()), ("~/a.png", ["lyrics".to_string(), "cava".to_string()].as_slice()));
    }

    #[test]
    fn clamps_and_warns() {
        let (g, w) = parse("[gui]\nopacity = 250\nlyrics_size = 1\nactive_size = 9999\ncorner_radius = -3\ninactive_fade = 101\nui_scale = 5\n");
        assert_eq!((g.opacity, g.lyrics_size, g.active_size, g.corner_radius, g.inactive_fade), (100, 10, 72, 0, 100));
        assert_eq!(g.ui_scale, 50);
        assert_eq!(w.len(), 6, "{w:?}");
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
    fn the_halloween_look_sets_its_values_and_keeps_the_rest() {
        let mut g = GuiConf { opacity: 70, font: "Serif".into(), ..GuiConf::default() };
        g.apply_preset(Preset::Halloween);
        assert_eq!((g.background, g.effects, g.theme_preset), (Background::Solid, Effects::Auto, Preset::Halloween));
        assert_eq!(g.accent, Accent::Fixed((255, 122, 24)));
        assert_eq!((g.opacity, g.font.as_str()), (70, "Serif"));
        assert!(Preset::Halloween.base(true).is_some() && Preset::None.base(true).is_none());
    }

    #[test]
    fn text_sizes_are_automatic_at_zero_and_clamped_otherwise() {
        let (g, w) = parse("[gui]\ntranslation_size = 3\nmember_size = 99\n");
        assert_eq!((g.translation_size, g.member_size), (8, 40));
        assert_eq!(w.len(), 1, "{w:?}"); // only member_size is outside what the file may say
        let (g, w) = parse("[gui]\ntranslation_size = 0\nmember_size = 0\n");
        assert_eq!((g.translation_size, g.member_size), (0, 0));
        assert!(w.is_empty());
    }

    #[test]
    fn every_look_has_its_own_values_and_colours() {
        for name in Preset::NAMES.iter().filter(|n| **n != "none") {
            let p = Preset::parse(name).unwrap();
            let mut g = GuiConf::default();
            g.apply_preset(p);
            assert_eq!(g.theme_preset, p);
            assert_ne!(g.accent, Accent::Cover, "{name}");
            assert!(!g.active_effects().is_empty(), "{name} moves");
            // a colour for the window and the cards in the dark theme and in the light one
            for dark in [true, false] {
                assert!(p.base(dark).is_some() && p.card(dark).is_some(), "{name} {dark}");
            }
            // dark is dark and light is light
            let lum = |c: (u8, u8, u8)| c.0 as u32 + c.1 as u32 + c.2 as u32;
            assert!(lum(p.base(true).unwrap()) < 200 && lum(p.base(false).unwrap()) > 600, "{name}");
            // the name survives the file
            let mut root = toml::Table::new();
            root.insert("gui".into(), toml::Value::Table(g.write()));
            let mut back = GuiConf::default();
            back.read(&root, &mut |m| panic!("{m}"));
            assert_eq!(back, g, "{name}");
        }
        // picking a look keeps the dark / light theme the window has
        let mut g = GuiConf { theme: Theme::Light, ..GuiConf::default() };
        g.apply_preset(Preset::Halloween);
        assert_eq!(g.theme, Theme::Light);
    }

    #[test]
    fn auto_effects_belong_to_the_look_and_intensity_zero_stops_them() {
        let fx = |p: Preset| {
            let mut g = GuiConf::default();
            g.apply_preset(p);
            g.active_effects()
        };
        assert!(GuiConf::default().active_effects().is_empty(), "no look, no effects");
        assert_eq!(fx(Preset::Christmas), [Fx::Snow, Fx::Lights]);
        assert_eq!(fx(Preset::Easter), [Fx::Seeds, Fx::Eggs]);
        assert_eq!(fx(Preset::Midsummer), [Fx::Pollen, Fx::Wind]);
        assert_eq!(fx(Preset::Halloween), [Fx::Fog, Fx::Embers, Fx::Bats]);
        // one chosen by hand works on any background and look
        let g = GuiConf { effects: Effects::Leaves, theme_preset: Preset::Easter, ..GuiConf::default() };
        assert_eq!(g.active_effects(), [Fx::Leaves]);
        let g = GuiConf { effects_intensity: 0, ..g };
        assert!(g.active_effects().is_empty());
    }

    #[test]
    fn the_old_moving_backgrounds_keep_working_as_a_background_and_an_effect() {
        let (g, w) = parse("[gui]\nbackground = \"snow\"\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((g.background, g.effects), (Background::Solid, Effects::Winter));
        let (g, _) = parse("[gui]\nbackground = \"eggs\"\n");
        assert_eq!((g.background, g.effects), (Background::Lawn, Effects::Eggs));
        let (g, _) = parse("[gui]\nbackground = \"bats\"\n");
        assert_eq!((g.background, g.effects), (Background::Solid, Effects::Haunt));
        let (g, _) = parse("[gui]\nbackground = \"particles\"\n");
        assert_eq!(g.effects, Effects::Embers);
        let (g, _) = parse("[gui]\nbackground = \"fog\"\n");
        assert_eq!(g.effects, Effects::Fog);
        // the meadow had pollen
        let (g, _) = parse("[gui]\nbackground = \"meadow\"\n");
        assert_eq!((g.background, g.effects), (Background::Meadow, Effects::Pollen));
        // an effect named in the file wins over what the old background implied
        let (g, w) = parse("[gui]\nbackground = \"snow\"\neffects = \"leaves\"\n");
        assert!(w.is_empty());
        assert_eq!((g.background, g.effects), (Background::Solid, Effects::Leaves));
        let (g, _) = parse("[gui]\nbackground = \"meadow\"\neffects = \"auto\"\n");
        assert_eq!(g.effects, Effects::Auto);
        // what is written is the new form, and reads back
        let mut root = toml::Table::new();
        root.insert("gui".into(), toml::Value::Table(g.write()));
        assert_eq!(root["gui"]["effects"].as_str(), Some("auto"));
    }

    #[test]
    fn leaving_a_look_resets_what_it_set() {
        let mut g = GuiConf::default();
        g.apply_preset(Preset::Christmas);
        assert!(!g.font.is_empty());
        g.apply_preset(Preset::Easter);
        assert!(g.font.is_empty());
        // a font chosen by hand survives a look that sets none
        g.font = "Noto Sans".into();
        g.apply_preset(Preset::Midsummer);
        assert_eq!(g.font, "Noto Sans");
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
            effects: Effects::Leaves,
            effects_intensity: 33,
            colors: ColorsFrom::Auto,
            colors_file: "~/x/colors.json".into(),
            font: "DejaVu Sans".into(),
            active_weight: Weight::Bold,
            panel_borders: false,
            lyrics_align: Align::Center,
            lyrics_animation: Animation::Instant,
            always_on_top: true,
            ui_scale: 150,
            theme_preset: Preset::Halloween,
            background_image: "/x/y.png".into(),
            lyrics_glow: 30,
            lock_layout: true,
            translation_size: 22,
            member_size: 12,
            inner_sizes: [("cava".to_string(), 150u16), ("stack".to_string(), 300)].into_iter().collect(),
            popped: vec!["cava".into()],
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
