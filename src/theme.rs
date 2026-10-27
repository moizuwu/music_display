use crate::cli::Args;
use crate::util::shorten_home;
use ratatui::style::Color;
use serde_json::Value;
use std::path::PathBuf;

pub use crate::color::{Rgb, luminance, mix, parse_hex};

/// Palette used by the whole UI.
///
/// Every slot is optional. `None` means "inherit from the terminal": the default
/// background/foreground, and the ANSI palette for the accent. The app never invents a
/// hex colour, so with `--terminal-colors` (or a missing/empty/invalid colour file) it
/// matches whatever theme the terminal emulator is running.
#[derive(Clone, Debug)]
pub struct Theme {
    pub surface: Option<Rgb>,
    pub primary: Option<Rgb>,
    pub on_surface: Option<Rgb>,
    /// Footer text describing where the colours came from.
    pub source: String,
}

impl Theme {
    pub fn c(rgb: Rgb) -> Color {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    }
    /// Background. `Color::Reset` = the terminal's own (possibly transparent) background.
    pub fn surface(&self) -> Color {
        self.surface.map_or(Color::Reset, Self::c)
    }
    /// Accent. Falls back to the terminal palette's cyan.
    pub fn primary(&self) -> Color {
        self.primary.map_or(Color::Cyan, Self::c)
    }
    /// Foreground. `Color::Reset` = the terminal's default foreground.
    pub fn text(&self) -> Color {
        self.on_surface.map_or(Color::Reset, Self::c)
    }
    /// Text blended `t` of the way toward the background (0 = full text, 1 = invisible).
    /// Without known RGB values this steps through the terminal's own grays.
    pub fn fade(&self, t: f32) -> Color {
        match (self.on_surface, self.surface) {
            (Some(o), Some(s)) => Self::c(mix(o, s, t)),
            _ if t < 0.3 => Color::Reset,
            _ if t < 0.6 => Color::Gray,
            _ => Color::DarkGray,
        }
    }
    /// Accent blended toward the background.
    pub fn accent_fade(&self, t: f32) -> Color {
        match (self.primary, self.surface) {
            (Some(p), Some(s)) => Self::c(mix(p, s, t)),
            // unknown background: blend toward mid-gray, which reads on dark and light
            (Some(p), None) => Self::c(mix(p, (128, 128, 128), t * 0.8)),
            (None, _) if t >= 0.5 => Color::DarkGray,
            (None, _) if t >= 0.25 => Color::Cyan,
            (None, _) => Color::LightCyan,
        }
    }
}

/// Candidate files, in priority order.
fn candidates() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    [
        ".config/noctalia/colors.json",
        ".cache/noctalia/colors.json",
        ".config/matugen/colors.json",
        ".cache/matugen/colors.json",
        ".config/noctalia/theme.json",
    ]
    .iter()
    .map(|p| home.join(p))
    .collect()
}

/// A colour value can be a plain string or a nested object
/// (`{"dark": "#.."}`, `{"dark": {"color": "#.."}}`, `{"default": {"hex": ..}}` …).
fn color_of(v: &Value) -> Option<Rgb> {
    match v {
        Value::String(s) => parse_hex(s),
        Value::Object(m) => ["dark", "default", "color", "hex", "value"]
            .iter()
            .filter_map(|k| m.get(*k))
            .find_map(color_of),
        _ => None,
    }
}

fn at<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |v, k| v.get(*k))
}

/// Depth-first search for the first key named `name` that holds a colour.
fn deep_find(v: &Value, name: &str) -> Option<Rgb> {
    match v {
        Value::Object(m) => {
            if let Some(c) = m.get(name).and_then(color_of) {
                return Some(c);
            }
            // prefer "dark" subtrees over "light" ones
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort_by_key(|k| (k.contains("light"), !k.contains("dark")));
            keys.into_iter().find_map(|k| deep_find(&m[k], name))
        }
        Value::Array(a) => a.iter().find_map(|x| deep_find(x, name)),
        _ => None,
    }
}

/// The `jq`-equivalent: try the known schemas explicitly, then fall back to a deep search.
fn extract(root: &Value, names: &[&str]) -> Option<Rgb> {
    for name in names {
        let paths: [&[&str]; 6] = [
            &[name],                   // Noctalia flat: .mSurface
            &["colors", "dark", name], // matugen: .colors.dark.surface
            &["colors", name],         // matugen: .colors.surface(.dark)
            &["dark", name],           // noctalia theme.json: .dark.mSurface
            &["colors", "default", name],
            &["theme", name],
        ];
        for p in paths {
            if let Some(c) = at(root, p).and_then(color_of) {
                return Some(c);
            }
        }
    }
    names.iter().find_map(|n| deep_find(root, n))
}

const SURFACE: &[&str] = &["mSurface", "surface", "background", "mBackground", "base"];
const PRIMARY: &[&str] = &["mPrimary", "primary", "accent", "mAccent"];
const ON_SURFACE: &[&str] = &[
    "mOnSurface",
    "on_surface",
    "onSurface",
    "foreground",
    "text",
];

type FileColors = (Option<Rgb>, Option<Rgb>, Option<Rgb>);

/// `None` for a missing, unreadable, invalid, empty (`{}`) or colour-less file.
fn from_file(path: &PathBuf) -> Option<FileColors> {
    let txt = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    let s = extract(&v, SURFACE);
    let p = extract(&v, PRIMARY);
    if s.is_none() && p.is_none() {
        return None;
    }
    Some((s, p, extract(&v, ON_SURFACE)))
}

pub fn load(args: &Args) -> Theme {
    let mut surface = None;
    let mut primary = None;
    let mut on_surface = None;

    // `from_file` = where the colours came from, if a file supplied any.
    let mut from_file: Option<String> = None;
    let mut fallback = "terminal colors".to_string();
    if !args.terminal_colors {
        let files: Vec<PathBuf> = match &args.colors {
            Some(p) => vec![p.clone()],
            None => candidates(),
        };
        match files
            .iter()
            .find_map(|f| self::from_file(f).map(|c| (f, c)))
        {
            Some((f, (s, p, o))) => {
                surface = s;
                primary = p;
                on_surface = o;
                from_file = Some(shorten_home(f));
            }
            // Nothing usable (missing, unreadable, invalid or `{}`): inherit the
            // terminal's colours instead of inventing any.
            None => {
                fallback = match &args.colors {
                    Some(p) => format!("terminal colors ({} unusable)", shorten_home(p)),
                    None => "terminal colors (no usable colour file)".to_string(),
                }
            }
        }
    }

    let mut overridden = false;
    for (flag, slot) in [(&args.surface, &mut surface), (&args.primary, &mut primary)] {
        if let Some(hex) = flag {
            match parse_hex(hex) {
                Some(c) => {
                    *slot = Some(c);
                    overridden = true;
                }
                None => eprintln!("music_display: ignoring invalid colour {hex:?}"),
            }
        }
    }
    let source = match (from_file, overridden) {
        (Some(f), false) => f,
        (Some(f), true) => format!("{f} + CLI overrides"),
        (None, false) => fallback,
        (None, true) if surface.is_some() && primary.is_some() => "CLI overrides".to_string(),
        (None, true) if args.terminal_colors => "terminal colors + CLI overrides".to_string(),
        (None, true) => "CLI overrides + terminal colors".to_string(),
    };

    // Text colour is only meaningful against a known surface; otherwise it stays the
    // terminal's default foreground.
    let on_surface = surface.map(|s| {
        let derived = if luminance(s) < 0.5 {
            mix(s, (255, 255, 255), 0.88)
        } else {
            mix(s, (0, 0, 0), 0.85)
        };
        match on_surface {
            Some(o) if (luminance(o) - luminance(s)).abs() > 0.35 => o,
            _ => derived,
        }
    });

    Theme {
        surface,
        primary,
        on_surface,
        source,
    }
}
