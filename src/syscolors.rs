//! Colours of the system for the window (`[gui] colors`): a colour file written by a wallpaper
//! tool (the JSON files the terminal UI reads too, or `colors_file`), or the GTK theme of the
//! desktop. Nothing here is on unless the setting asks for it, and nothing is specific to one
//! setup: the places that are looked at are the usual ones, and a source that has nothing
//! simply gives nothing. Reading is plain files plus an optional `gsettings` call, done off the
//! UI thread by the window, which asks again when a watched file changes.
use crate::color::{Rgb, luminance, parse_hex};
use music_display::guiconf::ColorsFrom;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a source says about the look of the desktop. Either colour can be missing.
#[derive(Clone, Debug, PartialEq)]
pub struct Colors {
    pub surface: Option<Rgb>,
    pub primary: Option<Rgb>,
    pub text: Option<Rgb>,
    /// Is the desktop dark? `None` when nothing says.
    pub dark: Option<bool>,
    /// Where it came from, for the settings window.
    pub source: String,
}

/// The answer, and the files it was read from (with their modification times) so that the
/// window can tell when to ask again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub colors: Option<Colors>,
    pub watch: Vec<(PathBuf, Option<SystemTime>)>,
}

/// Do any of the watched files differ from when they were read?
pub fn changed(watch: &[(PathBuf, Option<SystemTime>)]) -> bool {
    watch.iter().any(|(p, was)| std::fs::metadata(p).and_then(|m| m.modified()).ok() != *was)
}

fn stamp(p: &Path) -> (PathBuf, Option<SystemTime>) {
    (p.to_path_buf(), std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Ask the sources the mode names (`auto`: the colour file first, then GTK).
pub fn resolve(mode: ColorsFrom, file: &str) -> Found {
    let mut found = Found::default();
    if matches!(mode, ColorsFrom::File | ColorsFrom::Auto) {
        let path = expand(file);
        found.watch.extend(crate::theme::watch_list(path.as_deref()));
        if let Some(((surface, primary, text), at)) = crate::theme::file_colors(path.as_deref()) {
            found.colors = Some(Colors {
                surface,
                primary,
                text,
                dark: surface.map(|s| luminance(s) < 0.5),
                source: crate::util::shorten_home(&at),
            });
        }
    }
    if found.colors.is_none() && matches!(mode, ColorsFrom::Gtk | ColorsFrom::Auto) {
        let home = dirs::home_dir().unwrap_or_default();
        let g = gtk(&home, &[PathBuf::from("/usr/local/share/themes"), PathBuf::from("/usr/share/themes")], &gsettings);
        found.colors = g.colors;
        found.watch.extend(g.watch);
    }
    found
}

/// `~/…` and the empty string (= "the usual places").
fn expand(file: &str) -> Option<PathBuf> {
    let f = file.trim();
    if f.is_empty() {
        return None;
    }
    Some(match f.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(f),
    })
}

// ── GTK ─────────────────────────────────────────────────────────────────────

/// A value of `org.gnome.desktop.interface` as the plain text (`'prefer-dark'` → `prefer-dark`).
fn gsettings(key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", key])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string())
}

/// GNOME's accent colour names and their colours.
fn accent_named(name: &str) -> Option<Rgb> {
    Some(match name {
        "blue" => (0x35, 0x84, 0xe4),
        "teal" => (0x21, 0x90, 0xa4),
        "green" => (0x3a, 0x94, 0x4a),
        "yellow" => (0xc8, 0x88, 0x00),
        "orange" => (0xed, 0x5b, 0x00),
        "red" => (0xe6, 0x2d, 0x42),
        "pink" => (0xd5, 0x61, 0x99),
        "purple" => (0x91, 0x41, 0xac),
        "slate" => (0x6f, 0x83, 0x96),
        _ => return None,
    })
}

const SURFACE: &[&str] = &["window_bg_color", "theme_bg_color", "view_bg_color", "bg_color"];
const PRIMARY: &[&str] = &["accent_bg_color", "theme_selected_bg_color", "accent_color", "selected_bg_color"];
const TEXT: &[&str] = &["window_fg_color", "theme_fg_color", "fg_color"];

/// `@define-color name value;` lines of a GTK style sheet, and (one level deep) the sheets it
/// `@import`s, whose definitions the sheet's own override. `defs` holds the raw values.
fn load_css(defs: &mut HashMap<String, String>, path: &Path, watch: &mut Vec<(PathBuf, Option<SystemTime>)>, depth: u8) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    watch.push(stamp(path));
    // (comments go first, then the statements are the text between semicolons)
    let mut plain = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("/*") {
        plain.push_str(&rest[..i]);
        rest = rest[i..].find("*/").map_or("", |e| &rest[i + e + 2..]);
    }
    plain.push_str(rest);
    let statements: Vec<&str> = plain.split(';').map(str::trim).collect();
    if depth == 0 {
        for s in &statements {
            if let Some(arg) = s.strip_prefix("@import") {
                let name = arg.trim().trim_start_matches("url").trim_matches(|c: char| c == '(' || c == ')' || c == '"' || c == '\'' || c.is_whitespace());
                if !name.is_empty() && !name.contains("://") {
                    load_css(defs, &path.parent().unwrap_or(Path::new("")).join(name), watch, depth + 1);
                }
            }
        }
    }
    for s in statements {
        if let Some(def) = s.strip_prefix("@define-color") {
            if let Some((name, value)) = def.trim().split_once(char::is_whitespace) {
                defs.insert(name.to_string(), value.trim().to_string());
            }
        }
    }
}

/// A colour value of a style sheet: `#rrggbb`, `rgb(…)`, `rgba(…)`, or `@other_name`.
fn css_color(defs: &HashMap<String, String>, value: &str, depth: u8) -> Option<Rgb> {
    let v = value.trim();
    if let Some(name) = v.strip_prefix('@') {
        return (depth < 5).then(|| defs.get(name.trim())).flatten().and_then(|n| css_color(defs, n, depth + 1));
    }
    if v.starts_with('#') {
        return parse_hex(v);
    }
    let inner = v.strip_prefix("rgba").or_else(|| v.strip_prefix("rgb"))?.trim().strip_prefix('(')?.strip_suffix(')')?;
    let n: Vec<f32> = inner.split(',').filter_map(|p| p.trim().trim_end_matches('%').parse().ok()).collect();
    (n.len() >= 3).then(|| (n[0].clamp(0.0, 255.0) as u8, n[1].clamp(0.0, 255.0) as u8, n[2].clamp(0.0, 255.0) as u8))
}

fn pick(defs: &HashMap<String, String>, names: &[&str]) -> Option<Rgb> {
    names.iter().find_map(|n| defs.get(*n).and_then(|v| css_color(defs, v, 0)))
}

/// `gtk-theme-name` and `gtk-application-prefer-dark-theme` of a `settings.ini`.
fn settings_ini(path: &Path, watch: &mut Vec<(PathBuf, Option<SystemTime>)>) -> (Option<String>, Option<bool>) {
    let Ok(text) = std::fs::read_to_string(path) else { return (None, None) };
    watch.push(stamp(path));
    let value = |key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key)?.trim().strip_prefix('=').map(|v| v.trim().to_string()));
    let dark = value("gtk-application-prefer-dark-theme").map(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    (value("gtk-theme-name").filter(|n| !n.is_empty()), dark)
}

/// The GTK theme: the user's own style sheets first, then the sheet of the named theme, and
/// the desktop's accent colour for what is still missing. `theme_dirs` are the system theme
/// folders; `gs` reads `org.gnome.desktop.interface`.
pub fn gtk(home: &Path, theme_dirs: &[PathBuf], gs: &dyn Fn(&str) -> Option<String>) -> Found {
    let mut watch = Vec::new();
    let mut defs = HashMap::new();
    let (mut name, mut prefer_dark) = (None, None);
    // gtk-3.0 first, so that gtk-4.0 (the one that wins) is read last
    for v in ["gtk-3.0", "gtk-4.0"] {
        let dir = home.join(".config").join(v);
        load_css(&mut defs, &dir.join("gtk.css"), &mut watch, 0);
        let (n, d) = settings_ini(&dir.join("settings.ini"), &mut watch);
        name = n.or(name);
        prefer_dark = d.or(prefer_dark);
    }
    let name = name.or_else(|| gs("gtk-theme").filter(|n| !n.is_empty()));
    if pick(&defs, SURFACE).is_none() {
        if let Some(n) = &name {
            let mut dirs = vec![home.join(".themes"), home.join(".local/share/themes")];
            dirs.extend(theme_dirs.iter().cloned());
            let sheet = dirs.iter().flat_map(|d| ["gtk-4.0", "gtk-3.0"].map(|v| d.join(n).join(v).join("gtk.css"))).find(|p| p.exists());
            if let Some(p) = sheet {
                let mut theme_defs = HashMap::new();
                load_css(&mut theme_defs, &p, &mut watch, 0);
                // (the user's own definitions still win over the theme's)
                theme_defs.extend(defs);
                defs = theme_defs;
            }
        }
    }
    let surface = pick(&defs, SURFACE);
    let primary = pick(&defs, PRIMARY).or_else(|| gs("accent-color").and_then(|a| accent_named(&a)));
    let text = pick(&defs, TEXT);
    let dark = match gs("color-scheme").as_deref() {
        Some("prefer-dark") => Some(true),
        Some("prefer-light") => Some(false),
        _ => surface.map(|s| luminance(s) < 0.5).or(prefer_dark),
    };
    let colors = (surface.is_some() || primary.is_some()).then(|| Colors {
        surface,
        primary,
        text,
        dark,
        source: match &name {
            Some(n) => format!("GTK ({n})"),
            None => "GTK".to_string(),
        },
    });
    Found { colors, watch }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("md-syscolors-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn style_sheet_colours_resolve_references_and_formats() {
        let d = dir("css");
        let css = d.join("gtk.css");
        std::fs::write(
            &css,
            "/* a comment; with a semicolon */\n@define-color accent_color #3584e4;\n@define-color accent_bg_color @accent_color;\n@define-color window_bg_color rgb(30, 30, 46);\n@define-color window_fg_color rgba(205, 214, 244, 0.9);\n@define-color broken mix(@a, @b, 0.5);\n",
        )
        .unwrap();
        let (mut defs, mut watch) = (HashMap::new(), Vec::new());
        load_css(&mut defs, &css, &mut watch, 0);
        assert_eq!(pick(&defs, SURFACE), Some((30, 30, 46)));
        assert_eq!(pick(&defs, PRIMARY), Some((0x35, 0x84, 0xe4)));
        assert_eq!(pick(&defs, TEXT), Some((205, 214, 244)));
        assert_eq!(css_color(&defs, "mix(@a, @b, 0.5)", 0), None);
        assert_eq!(css_color(&defs, "@loop", 0), None);
        assert_eq!(watch.len(), 1);
    }

    #[test]
    fn an_imported_sheet_is_read_and_the_importing_one_wins() {
        let d = dir("import");
        std::fs::write(d.join("colors.css"), "@define-color window_bg_color #101010;\n@define-color accent_bg_color #aa0000;\n").unwrap();
        std::fs::write(d.join("gtk.css"), "@import url(\"colors.css\");\n@define-color accent_bg_color #00aa00;\n").unwrap();
        let (mut defs, mut watch) = (HashMap::new(), Vec::new());
        load_css(&mut defs, &d.join("gtk.css"), &mut watch, 0);
        assert_eq!(pick(&defs, SURFACE), Some((16, 16, 16)));
        assert_eq!(pick(&defs, PRIMARY), Some((0, 170, 0)));
        assert_eq!(watch.len(), 2, "both files are watched");
    }

    #[test]
    fn the_gtk_theme_comes_from_the_users_sheet_then_the_named_theme_then_the_accent_name() {
        let home = dir("home");
        // nothing at all
        assert_eq!(gtk(&home, &[], &none).colors, None);
        // only the desktop's accent and dark preference
        let gs = |k: &str| match k {
            "accent-color" => Some("orange".to_string()),
            "color-scheme" => Some("prefer-dark".to_string()),
            _ => None,
        };
        let c = gtk(&home, &[], &gs).colors.unwrap();
        assert_eq!((c.surface, c.primary, c.dark), (None, Some((0xed, 0x5b, 0x00)), Some(true)));
        // a named theme in a system folder
        let themes = dir("themes");
        std::fs::create_dir_all(themes.join("Nice/gtk-4.0")).unwrap();
        std::fs::write(themes.join("Nice/gtk-4.0/gtk.css"), "@define-color theme_bg_color #fafafa;\n@define-color theme_selected_bg_color #112233;\n").unwrap();
        std::fs::create_dir_all(home.join(".config/gtk-3.0")).unwrap();
        std::fs::write(home.join(".config/gtk-3.0/settings.ini"), "[Settings]\ngtk-theme-name=Nice\ngtk-application-prefer-dark-theme=0\n").unwrap();
        let c = gtk(&home, &[themes.clone()], &none).colors.unwrap();
        assert_eq!((c.surface, c.primary, c.dark), (Some((250, 250, 250)), Some((0x11, 0x22, 0x33)), Some(false)));
        assert_eq!(c.source, "GTK (Nice)");
        // the user's own sheet wins over the theme's colours
        std::fs::create_dir_all(home.join(".config/gtk-4.0")).unwrap();
        std::fs::write(home.join(".config/gtk-4.0/gtk.css"), "@define-color window_bg_color #202020;\n").unwrap();
        let c = gtk(&home, &[themes], &none).colors.unwrap();
        assert_eq!((c.surface, c.dark), (Some((32, 32, 32)), Some(true)));
    }

    #[test]
    fn watched_files_notice_a_change() {
        let d = dir("watch");
        let f = d.join("gtk.css");
        std::fs::write(&f, "@define-color window_bg_color #101010;\n").unwrap();
        let (mut defs, mut watch) = (HashMap::new(), Vec::new());
        load_css(&mut defs, &f, &mut watch, 0);
        assert!(!changed(&watch));
        std::fs::remove_file(&f).unwrap();
        assert!(changed(&watch), "a file that went away counts");
    }

    #[test]
    fn the_colour_file_source_and_the_mode_off() {
        let d = dir("file");
        let f = d.join("colors.json");
        std::fs::write(&f, r##"{"mSurface":"#14161c","mPrimary":"#8ab4f8"}"##).unwrap();
        let found = resolve(ColorsFrom::File, f.to_str().unwrap());
        let c = found.colors.unwrap();
        assert_eq!((c.surface, c.primary, c.dark), (Some((0x14, 0x16, 0x1c)), Some((0x8a, 0xb4, 0xf8)), Some(true)));
        assert_eq!(found.watch.len(), 1);
        // a file that is not there gives nothing (and is still watched, it may appear)
        let found = resolve(ColorsFrom::File, d.join("nope.json").to_str().unwrap());
        assert!(found.colors.is_none() && found.watch.len() == 1);
        // off asks nobody
        assert_eq!(resolve(ColorsFrom::Off, f.to_str().unwrap()), Found::default());
    }
}
