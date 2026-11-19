//! How the window looks: the `[gui]` settings (see `music_display::guiconf`) turned into egui
//! styles, fonts, the background, panel cards and transport icons, plus the Appearance tab of
//! the settings window. The settings themselves are plain data in `guiconf`, so they are tested
//! without a window; nothing in this file has been checked visually on a real compositor.
use super::backdrop;
use crate::syscolors::Colors;
use super::model::Model;
use egui::{Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use music_display::color::{self, luminance, Rgb};
use music_display::guiconf::{ColorsFrom, Effects, Fx, Accent, Align as LAlign, Animation, Background, GuiConf, Preset, Theme, Weight};

/// The family the active line uses when `active_weight` is medium or bold.
pub const BOLD: &str = "md-bold";

/// What the window has applied so far, so the (costly) restyle only runs when something changed.
#[derive(Default)]
pub struct Look {
    applied: Option<(GuiConf, Option<Rgb>, Option<Colors>)>,
    fonts_for: Option<(String, Weight)>,
    /// The font collection is built on a background thread (scanning the system fonts takes
    /// a while); its answer arrives here.
    fonts_rx: Option<std::sync::mpsc::Receiver<egui::FontDefinitions>>,
    /// Accent taken from the current cover.
    accent: Option<Rgb>,
    /// The colours of the system, when `colors` asked for them and a source had some.
    sys: Option<Colors>,
    /// A tiny copy of the cover, stretched smooth, for `background = "cover-blur"`.
    blur: Option<egui::TextureHandle>,
    /// The picture of `background = "image"`, with the path it was loaded from (and whether that failed).
    picture: Option<(String, Option<egui::TextureHandle>)>,
    families: Option<Vec<String>>,
    filter: String,
    /// The picture path being typed (written to the settings when the field loses focus).
    image_edit: Option<String>,
    /// The colour file path being typed, likewise.
    colors_edit: Option<String>,
    /// A change not written to the file yet (written when the pointer is released).
    dirty: bool,
    /// "Reset appearance" was pressed once and asks again.
    pub confirm_reset: bool,
    pub presets: super::presets::State,
}

/// The logo as the window icon (the same one for every window of the program).
pub fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon/music_display-256.png")).unwrap_or_default()
}

/// The window itself: transparent (so `opacity` can work live), with the chosen decorations.
pub fn viewport(g: &GuiConf) -> egui::ViewportBuilder {
    let v = egui::ViewportBuilder::default()
        .with_inner_size([1000.0, 640.0])
        .with_min_inner_size([480.0, 320.0])
        .with_title("mdis")
        .with_app_id("music_display")
        .with_icon(icon())
        .with_transparent(true)
        .with_decorations(g.decorations);
    if g.always_on_top { v.with_always_on_top() } else { v }
}

fn rgb32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn sys_id() -> egui::Id {
    egui::Id::unique("system-colours")
}

/// The system colours the window was last given (kept in the context so that the helpers that
/// only have a `Ui` can reach them).
fn system(ctx: &egui::Context) -> Option<Colors> {
    ctx.data(|d| d.get_temp::<Option<Colors>>(sys_id())).flatten()
}

/// The system colours, for a window in the mode they were made for.
fn for_mode(sys: Option<&Colors>, dark: bool) -> Option<&Colors> {
    sys.filter(|c| c.dark.is_none_or(|d| d == dark))
}

/// The window colour for a theme: the system's, else the look's (a named look can have its own).
fn base(g: &GuiConf, sys: Option<&Colors>, dark: bool) -> Rgb {
    for_mode(sys, dark)
        .and_then(|c| c.surface)
        .or(g.window_color)
        .or_else(|| g.theme_preset.base(dark))
        .unwrap_or(if dark { (27, 27, 31) } else { (243, 243, 246) })
}

/// The accent: the system's when it has one, else the chosen one, else the cover's.
fn accent_of(g: &GuiConf, cover: Option<Rgb>, sys: Option<&Colors>) -> Option<Rgb> {
    sys.and_then(|c| c.primary).or(match g.accent {
        Accent::Fixed(c) => Some(c),
        Accent::Cover => cover,
    })
}

/// The colour the moving effects are drawn in.
fn effect_color(g: &GuiConf, cover: Option<Rgb>, sys: Option<&Colors>) -> Color32 {
    accent_of(g, cover, sys).map_or(Color32::from_rgb(170, 130, 230), rgb32)
}

impl Look {
    /// The colours the system gave (or `None`): the window restyles when they change.
    pub fn set_system(&mut self, c: Option<Colors>) {
        self.sys = c;
    }

    /// Where the system colours in use came from, for the settings window.
    pub fn system_source(&self) -> Option<&str> {
        self.sys.as_ref().map(|c| c.source.as_str())
    }

    /// A new cover: take its accent and a small blurred copy for the background.
    pub fn set_cover(&mut self, ctx: &egui::Context, img: Option<&image::DynamicImage>) {
        self.accent = img.and_then(music_display::cover::dominant_accent);
        self.blur = img.map(|i| {
            let small = i.resize_exact(24, 24, image::imageops::FilterType::Triangle).to_rgba8();
            ctx.load_texture("cover-blur", egui::ColorImage::from_rgba_unmultiplied([24, 24], small.as_raw()), egui::TextureOptions::LINEAR)
        });
    }

    /// Load the picture at `path` once (and again when the path changes); a bad path shows nothing.
    fn load_picture(&mut self, ctx: &egui::Context, path: &str) {
        if self.picture.as_ref().is_some_and(|(p, _)| p == path) {
            return;
        }
        let full = match path.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| path.into()),
            None => std::path::PathBuf::from(path),
        };
        let tex = (!path.is_empty())
            .then(|| image::open(&full).ok())
            .flatten()
            .map(|img| {
                let img = if img.width().max(img.height()) > 2048 { img.thumbnail(2048, 2048) } else { img };
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                ctx.load_texture("background-picture", egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
            });
        self.picture = Some((path.to_string(), tex));
    }

    /// Is something moving (so the window should redraw about 30 times a second): an effect, or
    /// the sun and the grass of a scenery background?
    pub fn animated(g: &GuiConf) -> bool {
        !g.active_effects().is_empty() || matches!(g.background, Background::Sky | Background::Meadow | Background::Lawn)
    }

    /// What moves over the background (`effects`): meant to be called right after
    /// `paint_background`, so the panels are drawn over it.
    pub fn paint_effects(&self, ui: &Ui, g: &GuiConf) {
        let fx = g.active_effects();
        if fx.is_empty() {
            return;
        }
        let r = ui.ctx().content_rect();
        let dark = ui.visuals().dark_mode;
        // the intensity: 60 % is the usual amount of pieces; the strength follows it up to that
        let scale = g.effects_intensity as f32 / 60.0;
        let a = g.opacity as f32 / 100.0 * (0.4 + 0.6 * scale.min(1.0));
        let time = ui.input(|i| i.time);
        let accent = effect_color(g, self.accent, self.sys.as_ref());
        let p = ui.painter();
        for f in fx {
            match f {
                Fx::Snow => backdrop::snow(p, r, time, if dark { Color32::from_rgb(236, 246, 255) } else { Color32::from_rgb(110, 145, 190) }, a, scale),
                Fx::Lights => backdrop::lights(p, r, time, a),
                Fx::Leaves => backdrop::leaves(p, r, time, a, scale),
                Fx::Seeds => backdrop::seeds(p, r, time, if dark { Color32::from_rgb(236, 236, 224) } else { Color32::from_rgb(120, 120, 108) }, a, scale),
                Fx::Wind => backdrop::wind(p, r, time, if dark { Color32::from_rgb(200, 222, 238) } else { Color32::from_rgb(90, 124, 156) }, a, scale),
                Fx::Embers => backdrop::embers(p, r, time, accent, a * 0.7, scale),
                Fx::Fog => backdrop::fog(p, r, time, if g.theme_preset == Preset::Halloween { Color32::from_rgb(110, 50, 170) } else { accent }, a, scale),
                Fx::Bats => backdrop::bats(p, r, time, if dark { Color32::from_rgb(8, 4, 12) } else { Color32::from_rgb(52, 30, 70) }, a, scale),
                Fx::Pollen => backdrop::pollen(p, r, time, a, scale),
                Fx::Eggs => backdrop::eggs(p, r, time, a, scale),
            }
        }
    }

    /// Bring the egui styles, fonts and window flags in line with `g` (only when it changed).
    pub fn apply(&mut self, ctx: &egui::Context, g: &GuiConf) {
        if let Some(rx) = &self.fonts_rx {
            if let Ok(defs) = rx.try_recv() {
                ctx.set_fonts(defs);
                self.fonts_rx = None;
            }
        }
        ctx.data_mut(|d| d.insert_temp(sys_id(), self.sys.clone()));
        let key = (g.clone(), self.accent, self.sys.clone());
        if self.applied.as_ref() == Some(&key) {
            return;
        }
        let first = self.applied.is_none();
        let prev = self.applied.replace(key).map(|(p, _, _)| p);
        ctx.set_theme(match (g.theme, self.sys.as_ref().and_then(|c| c.dark)) {
            // the desktop says it is dark or light: follow it
            (Theme::System, Some(true)) => egui::ThemePreference::Dark,
            (Theme::System, Some(false)) => egui::ThemePreference::Light,
            (Theme::System, None) => egui::ThemePreference::System,
            (Theme::Dark, _) => egui::ThemePreference::Dark,
            (Theme::Light, _) => egui::ThemePreference::Light,
        });
        let family_now = font_of(g).to_string();
        if self.fonts_for.as_ref() != Some(&(family_now.clone(), g.active_weight)) {
            // the bold family must exist at once (the active line asks for it); the real fonts
            // replace this when the scan is done
            // a family the program carries (Figtree for the Spotify look) is there at once: no scan
            if let Some(d) = carried_fonts(&family_now, g.active_weight) {
                ctx.set_fonts(d);
            } else if self.fonts_for.is_none() {
                ctx.set_fonts(base_fonts());
            }
            let (family, weight, c) = (family_now.clone(), g.active_weight, ctx.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.fonts_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(fonts(&family, weight));
                c.request_repaint();
            });
            self.fonts_for = Some((family_now, g.active_weight));
        }
        for th in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.set_style_of(th, style(g, self.accent, self.sys.as_ref(), th == egui::Theme::Dark));
        }
        // window flags that can change while it is open
        if first || prev.as_ref().is_some_and(|p| p.always_on_top != g.always_on_top) {
            let level = if g.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        if first || prev.as_ref().is_some_and(|p| p.ui_scale != g.ui_scale) {
            ctx.set_zoom_factor(g.ui_scale as f32 / 100.0);
        }
        if prev.as_ref().is_some_and(|p| p.decorations != g.decorations) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(g.decorations));
        }
    }

    /// The window background, under every panel: solid, a blurred cover, or a gradient.
    pub fn paint_background(&mut self, ui: &Ui, g: &GuiConf) {
        if g.background == Background::Image {
            self.load_picture(ui.ctx(), &g.background_image);
        }
        let r = ui.ctx().content_rect();
        let dark = ui.visuals().dark_mode;
        let a = g.opacity as f32 / 100.0;
        let sys = system(ui.ctx());
        let b = rgb32(base(g, sys.as_ref(), dark));
        let p = ui.painter();
        let time = ui.input(|i| i.time);
        match (g.background, &self.blur, self.accent) {
            (Background::Image, _, _) => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
                if let Some((_, Some(t))) = &self.picture {
                    p.image(t.id(), r, cover_uv(r, t.size_vec2()), with_alpha(Color32::WHITE, a * 0.8));
                    p.rect_filled(r, 0.0, with_alpha(b, a * 0.35));
                }
            }
            (Background::Sky, _, _) => backdrop::sky(p, r, time, dark, a),
            (Background::Meadow, _, _) => {
                backdrop::sky(p, r, time, dark, a);
                backdrop::flowers(p, r, time, a);
            }
            (Background::Lawn, _, _) => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
                backdrop::grass(p, r, time, Color32::from_rgb(120, 190, 110), a * 0.8);
            }
            (Background::CoverBlur, Some(t), _) => {
                let uv = cover_uv(r, Vec2::splat(1.0));
                p.rect_filled(r, 0.0, with_alpha(b, a));
                p.image(t.id(), r, uv, with_alpha(Color32::WHITE, a * 0.55));
                p.rect_filled(r, 0.0, with_alpha(b, a * 0.45));
            }
            (Background::Gradient, _, acc) => {
                let top = match acc {
                    Some(c) => rgb32(color::mix(base(g, sys.as_ref(), dark), c, 0.28)),
                    None => rgb32(color::mix(base(g, sys.as_ref(), dark), if dark { (255, 255, 255) } else { (0, 0, 0) }, 0.10)),
                };
                let mut mesh = egui::Mesh::default();
                let (ct, cb) = (with_alpha(top, a), with_alpha(b, a));
                mesh.colored_vertex(r.left_top(), ct);
                mesh.colored_vertex(r.right_top(), ct);
                mesh.colored_vertex(r.right_bottom(), cb);
                mesh.colored_vertex(r.left_bottom(), cb);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                p.add(egui::Shape::mesh(mesh));
            }
            _ => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
            }
        }
    }
}

/// The part of a picture (UV) that fills `r` without stretching it.
fn cover_uv(r: Rect, size: Vec2) -> Rect {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    let (pw, ph) = (size.x.max(1.0), size.y.max(1.0));
    let (kx, ky) = if w / h > pw / ph { (1.0, (h / w) * (pw / ph) / 1.0) } else { ((w / h) * (ph / pw), 1.0) };
    Rect::from_min_max(Pos2::new(0.5 - kx / 2.0, 0.5 - ky / 2.0), Pos2::new(0.5 + kx / 2.0, 0.5 + ky / 2.0))
}

/// The egui style for one theme: neutral egui defaults plus the user's accent, radius, spacing
/// and text sizes. Panels are see-through; the background and the cards are painted by us.
fn style(g: &GuiConf, cover: Option<Rgb>, sys: Option<&Colors>, dark: bool) -> egui::Style {
    let mut s = egui::Style { visuals: if dark { egui::Visuals::dark() } else { egui::Visuals::light() }, ..Default::default() };
    // (an accent from the cover or the system is made readable on the window colour)
    let acc = match (sys.and_then(|c| c.primary), g.accent) {
        (Some(c), _) => Some(color::readable(c, Some(base(g, sys, dark)))),
        (None, Accent::Fixed(c)) => Some(c),
        (None, Accent::Cover) => cover.map(|c| color::readable(c, Some(base(g, sys, dark)))),
    };
    // tooltips show at once (egui waits half a second by default)
    s.interaction.tooltip_delay = 0.0;
    s.interaction.show_tooltips_only_when_still = false;
    let v = &mut s.visuals;
    v.panel_fill = Color32::TRANSPARENT;
    // the system's text colour, when it is there and stands out from the window colour
    if let Some(text) = for_mode(sys, dark).and_then(|c| c.text) {
        if (luminance(text) - luminance(base(g, sys, dark))).abs() > 0.35 {
            v.override_text_color = Some(rgb32(text));
        }
    }
    // own text colours of the settings, when the system gave none
    if v.override_text_color.is_none() {
        v.override_text_color = g.text_color.map(rgb32);
    }
    v.weak_text_color = g.muted_color.map(rgb32);
    let r = CornerRadius::same(g.corner_radius);
    v.window_corner_radius = CornerRadius::same(g.corner_radius.saturating_add(4));
    v.menu_corner_radius = r;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    if let Some(c) = acc {
        let c = rgb32(c);
        v.hyperlink_color = c;
        v.selection.bg_fill = with_alpha(c, 0.35);
        v.selection.stroke = Stroke::new(1.0, c);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, with_alpha(c, 0.8));
        v.widgets.active.bg_stroke = Stroke::new(1.0, c);
    }
    // hover states: a slightly lifted fill so buttons answer the pointer
    let lift = if dark { 255 } else { 0 };
    v.widgets.hovered.weak_bg_fill = with_alpha(Color32::from_gray(lift), 0.14);
    v.widgets.hovered.expansion = 1.0;
    s.spacing.item_spacing = Vec2::new(8.0, 3.0 + g.panel_spacing as f32 * 0.35);
    s.spacing.button_padding = Vec2::new(8.0, 4.0);
    s.scroll_animation = if g.lyrics_animation == Animation::Smooth { egui::style::ScrollAnimation::default() } else { egui::style::ScrollAnimation::none() };
    let m = g.meta_size as f32;
    use egui::TextStyle::*;
    for (t, size) in [(Body, m), (Button, m), (Monospace, m), (Small, (m - 3.0).max(7.0)), (Heading, m + 5.0)] {
        if let Some(f) = s.text_styles.get_mut(&t) {
            f.size = size;
        }
    }
    s
}

/// egui's default fonts, with the chosen system family in front, and the bold family the
/// active line can use. An unknown family just leaves the defaults.
fn base_fonts() -> egui::FontDefinitions {
    use egui::{FontDefinitions, FontFamily};
    let mut defs = FontDefinitions::default();
    let prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    defs.families.insert(FontFamily::Name(BOLD.into()), prop);
    defs
}

/// A family the program carries itself: used when no installed font has the name.
pub const BUILT_IN_FAMILY: &str = "Mountains of Christmas";
static BUILT_IN_REGULAR: &[u8] = include_bytes!("../../assets/fonts/MountainsofChristmas-Regular.ttf");
static BUILT_IN_BOLD: &[u8] = include_bytes!("../../assets/fonts/MountainsofChristmas-Bold.ttf");
/// Figtree, the Spotify look's font (assets/fonts/Figtree-OFL.txt): used when it is not installed.
static FIGTREE_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Figtree-Regular.ttf");
static FIGTREE_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Figtree-SemiBold.ttf");
static FIGTREE_BOLD: &[u8] = include_bytes!("../../assets/fonts/Figtree-Bold.ttf");

/// The faces the program carries for a family name: (regular, medium, bold).
fn carried(family: &str) -> Option<(&'static [u8], &'static [u8], &'static [u8])> {
    if family.eq_ignore_ascii_case(BUILT_IN_FAMILY) {
        Some((BUILT_IN_REGULAR, BUILT_IN_BOLD, BUILT_IN_BOLD))
    } else if family.eq_ignore_ascii_case(music_display::guiconf::FIGTREE) {
        Some((FIGTREE_REGULAR, FIGTREE_SEMIBOLD, FIGTREE_BOLD))
    } else {
        None
    }
}

/// The font family in use: the chosen one, or that of the look when none is chosen (so a look
/// set in the config file by name has it too, not only one picked in the Appearance tab).
fn font_of(g: &GuiConf) -> &str {
    if g.font.is_empty() && g.theme_preset == Preset::Christmas { BUILT_IN_FAMILY } else { &g.font }
}

fn fonts(family: &str, weight: Weight) -> egui::FontDefinitions {
    use egui::{FontData, FontDefinitions};
    use fontique::{Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, QueryFamily, QueryStatus, SourceCache, SourceCacheOptions};
    let defs = FontDefinitions::default();
    let mut c = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    let mut pick = |name: &str, weight: FontWeight| {
        let mut q = c.query(&mut cache);
        if name.is_empty() {
            q.set_families([QueryFamily::Generic(fontique::GenericFamily::SansSerif)]);
        } else {
            q.set_families([QueryFamily::Named(name)]);
        }
        q.set_attributes(Attributes::new(FontWidth::NORMAL, FontStyle::Normal, weight));
        let mut found = None;
        q.matches_with(|f| {
            found = Some(f.clone());
            QueryStatus::Stop
        });
        found.map(|f| {
            let (blob, _) = f.blob.into_raw_parts();
            FontData::from_blob(blob, f.index)
        })
    };
    let own = carried(family);
    let normal = if family.is_empty() { None } else { pick(family, FontWeight::NORMAL) }
        .or_else(|| own.map(|(r, _, _)| FontData::from_static(r)));
    let bold = pick(family, if weight == Weight::Medium { FontWeight::MEDIUM } else { FontWeight::BOLD })
        .or_else(|| own.map(|(_, m, b)| FontData::from_static(if weight == Weight::Medium { m } else { b })));
    with_faces(defs, normal, bold)
}

/// The fonts of a family the program carries, without looking at the installed ones.
fn carried_fonts(family: &str, weight: Weight) -> Option<egui::FontDefinitions> {
    let (r, m, b) = carried(family)?;
    let d = egui::FontData::from_static;
    Some(with_faces(egui::FontDefinitions::default(), Some(d(r)), Some(d(if weight == Weight::Medium { m } else { b }))))
}

/// egui's fonts with `normal` in front of the proportional family and `bold` as the bold one.
fn with_faces(mut defs: egui::FontDefinitions, normal: Option<egui::FontData>, bold: Option<egui::FontData>) -> egui::FontDefinitions {
    let prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut bold_list = Vec::new();
    if let Some(d) = bold {
        defs.font_data.insert(BOLD.into(), d.into());
        bold_list.push(BOLD.to_string());
    }
    if let Some(d) = normal {
        defs.font_data.insert("md-user".into(), d.into());
        defs.families.entry(FontFamily::Proportional).or_default().insert(0, "md-user".into());
    }
    // the family follows the user's font too, so a missing bold face still looks consistent
    bold_list.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or(prop));
    defs.families.insert(FontFamily::Name(BOLD.into()), bold_list);
    defs
}

/// The font of the active lyric line.
pub fn active_font(g: &GuiConf, size: f32) -> FontId {
    match g.active_weight {
        Weight::Bold | Weight::Medium => FontId::new(size, FontFamily::Name(BOLD.into())),
        Weight::Regular => FontId::proportional(size),
    }
}

/// The frame around a column of panels: the gutter between cards and the window edge.
pub fn gutter(g: &GuiConf) -> Frame {
    Frame::new().inner_margin(Margin::same((g.panel_spacing / 2).min(127) as i8))
}

/// A rounded card behind a panel's content: `panel_opacity` fill, optional border.
pub fn card<R>(ui: &mut Ui, g: &GuiConf, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_at(ui, g, Split::NONE, add)
}

/// Which sides of a card touch a split (the draggable border between two columns, or between
/// two stacked rows): the corners on those sides are square, the outer ones stay rounded.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Split {
    /// A split before the card (left of it, or above it when `vertical`).
    pub before: bool,
    /// A split after it (right of it, or below it).
    pub after: bool,
    /// The columns are stacked top to bottom (`layout.direction` vertical).
    pub vertical: bool,
}

impl Split {
    pub const NONE: Split = Split { before: false, after: false, vertical: false };

    /// The corners of a card with radius `r`, square where a split is.
    pub fn corners(self, r: u8) -> CornerRadius {
        let (b, a) = (if self.before { 0 } else { r }, if self.after { 0 } else { r });
        if self.vertical {
            CornerRadius { nw: b, ne: b, sw: a, se: a }
        } else {
            CornerRadius { nw: b, sw: b, ne: a, se: a }
        }
    }
}

/// [`card`], with square corners on the sides in `split`.
pub fn card_at<R>(ui: &mut Ui, g: &GuiConf, split: Split, add: impl FnOnce(&mut Ui) -> R) -> R {
    let dark = ui.visuals().dark_mode;
    let sys = system(ui.ctx());
    let fill = match (for_mode(sys.as_ref(), dark).and_then(|c| c.surface), g.card_color.or_else(|| g.theme_preset.card(dark))) {
        // a little lighter (or darker) than the window colour the system gave
        (Some(s), _) => rgb32(color::mix(s, if dark { (255, 255, 255) } else { (0, 0, 0) }, if dark { 0.07 } else { 0.04 })),
        (None, Some(c)) => rgb32(c),
        (None, None) if dark => Color32::from_rgb(40, 40, 47),
        (None, None) => Color32::WHITE,
    };
    let alpha = g.panel_opacity as f32 / 100.0 * (g.opacity as f32 / 100.0).max(0.0);
    let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let margin = g.panel_spacing.min(127) as i8;
    let h = ui.available_height();
    Frame::new()
        .fill(with_alpha(fill, alpha))
        .corner_radius(split.corners(g.corner_radius))
        // a soft edge: the full line colour made every card look outlined in pencil
        .stroke(if g.panel_borders { Stroke::new(1.0, with_alpha(line, 0.35)) } else { Stroke::NONE })
        .inner_margin(Margin::same(margin))
        .show(ui, |ui| {
            ui.set_min_height((h - 2.0 * margin as f32 - 2.0).max(0.0));
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// How strongly a lyric line at `dist` lines from the active one is faded, 0 (none) to 1.
pub fn fade(g: &GuiConf, dist: usize) -> f32 {
    g.inactive_fade as f32 / 100.0 * (dist.min(5) as f32 / 5.0) * 0.85
}

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Pause,
    Prev,
    Next,
    Shuffle,
    Repeat,
    /// Repeat with the "1" of repeating the song.
    RepeatOne,
    /// The speaker; the number is how many sound waves it shows (0 = muted, crossed out).
    Volume(u8),
    Heart(bool),
    /// Spotify Connect: pick the device.
    Devices,
    /// Move playback to this computer.
    PlayHere,
    Gear,
    Library,
    Help,
    Sun,
    Moon,
    /// The pop-out menu: four little windows.
    Windows,
    Back,
    Open,
    Home,
    /// The lyrics: a speech bubble with lines of text.
    Lyrics,
    Refresh,
    Search,
    Close,
    Up,
    Down,
    Plus,
    Minus,
    /// A small play triangle (a row's play button).
    PlayRow,
    /// Add to the queue.
    Queue,
    /// Add to a playlist.
    AddList,
    // ── editing and devices (sp_actions.rs) ──
    /// A pencil: rename.
    Edit,
    Trash,
    Check,
    /// A device that is a speaker, a computer or a phone.
    Speaker,
    Computer,
    Phone,
}

/// The size of every transport button (previous, play / pause, next alike).
pub const ICON_SIDE: f32 = 30.0;

/// A round transport button with a drawn icon (no dependence on the font having the glyphs);
/// all of them look the same and light up under the pointer.
pub fn icon_button(ui: &mut Ui, icon: Icon) -> Response {
    let ink = ui.visuals().strong_text_color();
    paint_button(ui, icon, ink, false, ICON_SIDE)
}

/// A switch with a drawn icon, the way Spotify draws its shuffle and repeat: dim when off,
/// the accent colour with a dot under it when on, brighter under the pointer.
pub fn toggle_button(ui: &mut Ui, icon: Icon, on: bool) -> Response {
    let ink = if on { super::view::accent(ui) } else { ui.visuals().weak_text_color() };
    paint_button(ui, icon, ink, on, ICON_SIDE)
}

/// The one style of a title (a panel's, a window's section, the library's page).
pub fn heading(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).heading()).truncate());
}

/// A quiet status line ("Loading…", the library's note).
pub fn note(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// An error in the colour of warnings.
pub fn error(ui: &mut Ui, text: &str) {
    ui.colored_label(ui.visuals().warn_fg_color, text);
}

/// The size of the buttons in a row of a list.
pub const ROW_SIDE: f32 = 24.0;

/// A tool button (top bar, headers, list rows): a drawn icon with its tooltip, in the normal text
/// colour, the accent colour with a dot while `on`. True when clicked. `side` is `ICON_SIDE` or `ROW_SIDE`.
pub fn tool(ui: &mut Ui, side: f32, icon: Icon, on: bool, tip: &str) -> bool {
    tool_response(ui, side, icon, on, tip).clicked()
}

/// [`tool`], giving the response (for a menu that opens from the button).
pub fn tool_response(ui: &mut Ui, side: f32, icon: Icon, on: bool, tip: &str) -> Response {
    let ink = if on { super::view::accent(ui) } else { ui.visuals().text_color() };
    paint_button(ui, icon, ink, on, side).on_hover_text(tip)
}

/// Just the drawing of an icon, `side` big, centred on `c` (for icons inside other things, like
/// the magnifier in the search field).
pub fn paint_icon(p: &egui::Painter, c: Pos2, side: f32, icon: Icon, ink: Color32) {
    draw_icon(p, c, side * 0.2, icon, ink);
}

/// An icon in a colour of its own (no "on" dot): the Spotify look's green "saved" check.
pub fn icon_ink(ui: &mut Ui, side: f32, icon: Icon, ink: Color32, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    let ink = if resp.hovered() && ink == ui.visuals().weak_text_color() { ui.visuals().strong_text_color() } else { ink };
    draw_icon(ui.painter(), r.center(), side * 0.2, icon, ink);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

/// A round button filled with `fill` and its icon in `ink` (Spotify's white play button, its
/// round home button); a little bigger under the pointer.
pub fn round_button(ui: &mut Ui, side: f32, icon: Icon, fill: Color32, ink: Color32, tip: &str) -> Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    let grow = if resp.hovered() { 1.04 } else { 1.0 };
    ui.painter().circle_filled(r.center(), side / 2.0 * grow, fill);
    draw_icon(ui.painter(), r.center(), side * 0.2 * grow, icon, ink);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

fn paint_button(ui: &mut Ui, icon: Icon, mut ink: Color32, dot: bool, side: f32) -> Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    if resp.hovered() {
        let strong = ui.visuals().strong_text_color();
        p_hover(ui, r, side, resp.is_pointer_button_down_on());
        if !dot {
            ink = strong;
        }
    }
    let p = ui.painter();
    let c = r.center();
    draw_icon(p, c, side * 0.2, icon, ink);
    if dot {
        p.circle_filled(Pos2::new(c.x, r.bottom() - 2.5), 1.6, ink);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// An icon drawn on its own (no button around it), centred on `c`, in a box of `side`.
pub fn icon_at(p: &egui::Painter, c: Pos2, side: f32, icon: Icon, ink: Color32) {
    draw_icon(p, c, side * 0.2, icon, ink);
}

fn p_hover(ui: &Ui, r: Rect, side: f32, down: bool) {
    ui.painter().circle_filled(r.center(), side / 2.0, with_alpha(ui.visuals().widgets.hovered.fg_stroke.color, if down { 0.22 } else { 0.12 }));
}

/// The icons are drawn on a 16 × 16 grid like Spotify's own (thin 1.5 strokes, small arrow heads,
/// round joins), centred on `c`; `u` is a fifth of the button, so the grid unit is `u / 6`.
fn draw_icon(p: &egui::Painter, c: Pos2, u: f32, icon: Icon, ink: Color32) {
    let k = u / 6.0 * 1.1;
    let at = |x: f32, y: f32| Pos2::new(c.x + (x - 8.0) * k, c.y + (y - 8.0) * k);
    let w = 1.5 * k;
    // a smooth line with round ends
    let stroke = |pts: &[Pos2]| {
        p.add(egui::Shape::line(pts.to_vec(), Stroke::new(w, ink)));
        for q in [pts.first(), pts.last()].into_iter().flatten() {
            p.circle_filled(*q, w / 2.0, ink);
        }
    };
    // a closed outline; `sharp` are the points where it turns sharply (rounded off by a dot)
    let closed = |pts: &[Pos2], sharp: &[Pos2]| {
        p.add(egui::Shape::closed_line(pts.to_vec(), Stroke::new(w, ink)));
        for q in sharp {
            p.circle_filled(*q, w / 2.0, ink);
        }
    };
    let bez = |a: (f32, f32), b: (f32, f32), c2: (f32, f32), d: (f32, f32), t0: f32, t1: f32| -> Vec<Pos2> {
        (0..=24)
            .map(|i| {
                let t = t0 + (t1 - t0) * i as f32 / 24.0;
                let m = 1.0 - t;
                let f = |a: f32, b: f32, c: f32, d: f32| m * m * m * a + 3.0 * m * m * t * b + 3.0 * m * t * t * c + t * t * t * d;
                at(f(a.0, b.0, c2.0, d.0), f(a.1, b.1, c2.1, d.1))
            })
            .collect()
    };
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| -> Vec<Pos2> {
        let n = (((a1 - a0).abs() / 9.0).ceil() as usize).max(8);
        (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                at(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };
    // a small solid arrow head, tip at (x, y), pointing right (dir = 1) or left (dir = -1)
    let head = |x: f32, y: f32, dir: f32| {
        p.add(egui::Shape::convex_polygon(vec![at(x - dir * 3.2, y - 2.1), at(x - dir * 3.2, y + 2.1), at(x, y)], ink, Stroke::NONE));
    };
    let tri = |x: f32, dir: f32| vec![Pos2::new(x - dir * u * 0.8, c.y - u), Pos2::new(x - dir * u * 0.8, c.y + u), Pos2::new(x + dir * u * 0.9, c.y)];
    let bar = |x: f32| Rect::from_center_size(Pos2::new(x, c.y), Vec2::new(u * 0.32, u * 2.0));
    match icon {
        Icon::Play => {
            p.add(egui::Shape::convex_polygon(tri(c.x + u * 0.15, 1.0), ink, Stroke::NONE));
        }
        Icon::Pause => {
            for x in [c.x - u * 0.55, c.x + u * 0.55] {
                p.rect_filled(bar(x), 1.0, ink);
            }
        }
        Icon::Prev => {
            p.rect_filled(bar(c.x - u * 0.95), 1.0, ink);
            p.add(egui::Shape::convex_polygon(tri(c.x + u * 0.1, -1.0), ink, Stroke::NONE));
        }
        Icon::Next => {
            p.rect_filled(bar(c.x + u * 0.95), 1.0, ink);
            p.add(egui::Shape::convex_polygon(tri(c.x - u * 0.1, 1.0), ink, Stroke::NONE));
        }
        Icon::Shuffle => {
            // two arrows that cross; the one coming from below has a gap where it passes under
            stroke(&bez((1.0, 4.25), (6.5, 4.25), (6.5, 11.75), (12.0, 11.75), 0.0, 1.0));
            head(15.0, 11.75, 1.0);
            stroke(&bez((1.0, 11.75), (6.5, 11.75), (6.5, 4.25), (12.0, 4.25), 0.0, 0.40));
            stroke(&bez((1.0, 11.75), (6.5, 11.75), (6.5, 4.25), (12.0, 4.25), 0.60, 1.0));
            head(15.0, 4.25, 1.0);
        }
        Icon::Repeat | Icon::RepeatOne => {
            // a loop: along the top to the right, along the bottom to the left
            let mut top = vec![at(1.75, 8.5), at(1.75, 5.5)];
            top.extend(arc(3.75, 5.5, 2.0, 180.0, 270.0));
            top.push(at(11.5, 3.5));
            stroke(&top);
            head(15.0, 3.5, 1.0);
            let mut bottom = vec![at(14.25, 7.5), at(14.25, 10.5)];
            bottom.extend(arc(12.25, 10.5, 2.0, 0.0, 90.0));
            bottom.push(at(4.5, 12.5));
            stroke(&bottom);
            head(1.0, 12.5, -1.0);
            if icon == Icon::RepeatOne {
                p.add(egui::Shape::line(vec![at(6.9, 7.2), at(8.2, 6.2), at(8.2, 9.8)], Stroke::new(w * 0.85, ink)));
            }
        }
        Icon::Volume(waves) => {
            // the speaker, then its waves (or a cross when muted)
            p.add(egui::Shape::convex_polygon(vec![at(1.5, 5.75), at(4.4, 5.75), at(8.4, 2.4), at(8.4, 13.6), at(4.4, 10.25), at(1.5, 10.25)], ink, Stroke::NONE));
            if waves == 0 {
                stroke(&[at(10.6, 5.6), at(14.6, 10.4)]);
                stroke(&[at(14.6, 5.6), at(10.6, 10.4)]);
            } else {
                stroke(&arc(8.4, 8.0, 3.4, -48.0, 48.0));
                if waves >= 2 {
                    stroke(&arc(8.4, 8.0, 6.0, -48.0, 48.0));
                }
            }
        }
        Icon::Heart(full) => {
            let mut pts = bez((8.0, 13.9), (8.0, 13.9), (1.4, 10.2), (1.4, 5.1), 0.0, 1.0);
            pts.extend(bez((1.4, 5.1), (1.4, 3.0), (3.1, 1.6), (4.9, 1.6), 0.0, 1.0));
            pts.extend(bez((4.9, 1.6), (6.3, 1.6), (7.4, 2.4), (8.0, 3.7), 0.0, 1.0));
            pts.extend(bez((8.0, 3.7), (8.6, 2.4), (9.7, 1.6), (11.1, 1.6), 0.0, 1.0));
            pts.extend(bez((11.1, 1.6), (12.9, 1.6), (14.6, 3.0), (14.6, 5.1), 0.0, 1.0));
            pts.extend(bez((14.6, 5.1), (14.6, 10.2), (8.0, 13.9), (8.0, 13.9), 0.0, 1.0));
            if full {
                // star-shaped around the middle: a fan of triangles
                let mut mesh = egui::Mesh::default();
                let mid = mesh.vertices.len() as u32;
                mesh.colored_vertex(at(8.0, 7.0), ink);
                for q in &pts {
                    mesh.colored_vertex(*q, ink);
                }
                for i in 0..pts.len() as u32 {
                    mesh.add_triangle(mid, mid + 1 + i, mid + 1 + (i + 1) % pts.len() as u32);
                }
                p.add(egui::Shape::mesh(mesh));
            }
            closed(&pts, &[at(8.0, 3.7)]);
        }
        Icon::Devices => {
            // Spotify Connect: a screen, open at the corner where a small speaker sits
            let mut screen = vec![at(8.0, 11.0), at(3.0, 11.0)];
            screen.extend(arc(3.0, 9.5, 1.5, 90.0, 180.0));
            screen.push(at(1.5, 4.0));
            screen.extend(arc(3.0, 4.0, 1.5, 180.0, 270.0));
            screen.push(at(13.0, 2.5));
            screen.extend(arc(13.0, 4.0, 1.5, 270.0, 360.0));
            screen.push(at(14.5, 6.0));
            stroke(&screen);
            let mut spk = Vec::new();
            for (cx, cy, a0) in [(10.9, 9.7, 180.0), (12.9, 9.7, 270.0), (12.9, 12.7, 0.0), (10.9, 12.7, 90.0)] {
                spk.extend(arc(cx, cy, 1.0, a0, a0 + 90.0));
            }
            closed(&spk, &[]);
            p.circle_stroke(at(11.9, 11.9), 1.2 * k, Stroke::new(w * 0.8, ink));
            p.circle_filled(at(11.9, 9.5), 0.5 * k, ink);
        }
        Icon::PlayHere => {
            // a computer with a play sign on its screen
            let mut screen = arc(3.0, 4.0, 1.5, 180.0, 270.0);
            screen.extend(arc(13.0, 4.0, 1.5, 270.0, 360.0));
            screen.extend(arc(13.0, 9.5, 1.5, 0.0, 90.0));
            screen.extend(arc(3.0, 9.5, 1.5, 90.0, 180.0));
            closed(&screen, &[]);
            stroke(&[at(8.0, 11.0), at(8.0, 13.5)]);
            stroke(&[at(4.8, 13.6), at(11.2, 13.6)]);
            p.add(egui::Shape::convex_polygon(vec![at(6.5, 4.7), at(6.5, 8.9), at(10.2, 6.8)], ink, Stroke::NONE));
        }
        Icon::Gear => {
            p.circle_stroke(at(8.0, 8.0), 4.4 * k, Stroke::new(w, ink));
            p.circle_stroke(at(8.0, 8.0), 1.7 * k, Stroke::new(w, ink));
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                stroke(&[at(8.0 + 5.2 * a.cos(), 8.0 + 5.2 * a.sin()), at(8.0 + 6.9 * a.cos(), 8.0 + 6.9 * a.sin())]);
            }
        }
        Icon::Library => {
            stroke(&[at(2.5, 2.5), at(2.5, 13.5)]);
            stroke(&[at(6.0, 2.5), at(6.0, 13.5)]);
            stroke(&[at(9.3, 3.3), at(12.6, 13.2)]);
        }
        Icon::Help => {
            p.circle_stroke(at(8.0, 8.0), 6.2 * k, Stroke::new(w, ink));
            let mut q = arc(8.0, 6.4, 1.9, 180.0, 360.0 + 70.0);
            q.push(at(8.0, 8.6));
            q.push(at(8.0, 9.6));
            stroke(&q);
            p.circle_filled(at(8.0, 11.6), 0.8 * k, ink);
        }
        Icon::Sun => {
            p.circle_stroke(at(8.0, 8.0), 3.0 * k, Stroke::new(w, ink));
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                stroke(&[at(8.0 + 5.1 * a.cos(), 8.0 + 5.1 * a.sin()), at(8.0 + 6.7 * a.cos(), 8.0 + 6.7 * a.sin())]);
            }
        }
        Icon::Moon => {
            let mut m = arc(8.0, 8.0, 6.0, 55.0, 305.0);
            m.extend(bez((11.44, 3.09), (6.6, 5.4), (6.6, 10.6), (11.44, 12.91), 0.0, 1.0));
            closed(&m, &[]);
        }
        Icon::Windows => {
            for (x, y) in [(2.0, 2.0), (9.0, 2.0), (2.0, 9.0), (9.0, 9.0)] {
                p.rect_stroke(Rect::from_min_max(at(x, y), at(x + 5.0, y + 5.0)), 1.2 * k, Stroke::new(w, ink), egui::StrokeKind::Middle);
            }
        }
        Icon::Lyrics => {
            closed(&[at(2.0, 3.0), at(14.0, 3.0), at(14.0, 11.0), at(8.5, 11.0), at(4.5, 14.2), at(4.5, 11.0), at(2.0, 11.0)], &[at(2.0, 3.0), at(14.0, 3.0), at(14.0, 11.0), at(2.0, 11.0), at(4.5, 14.2)]);
            stroke(&[at(5.5, 5.0), at(10.5, 5.0)]);
            stroke(&[at(5.5, 7.5), at(9.0, 7.5)]);
        }
        Icon::Home => {
            stroke(&[at(1.5, 7.6), at(8.0, 2.0), at(14.5, 7.6)]);
            stroke(&[at(3.2, 6.6), at(3.2, 14.0), at(6.6, 14.0), at(6.6, 10.0), at(9.4, 10.0), at(9.4, 14.0), at(12.8, 14.0), at(12.8, 6.6)]);
        }
        Icon::Back => stroke(&[at(10.0, 2.5), at(4.5, 8.0), at(10.0, 13.5)]),
        Icon::Open => stroke(&[at(6.0, 2.5), at(11.5, 8.0), at(6.0, 13.5)]),
        Icon::Refresh => {
            stroke(&arc(8.0, 8.0, 5.2, 20.0, 320.0));
            p.add(egui::Shape::convex_polygon(vec![at(13.3, 3.0), at(13.5, 6.5), at(10.1, 5.7)], ink, Stroke::NONE));
        }
        Icon::Search => {
            p.circle_stroke(at(7.0, 7.0), 4.6 * k, Stroke::new(w, ink));
            stroke(&[at(10.5, 10.5), at(14.0, 14.0)]);
        }
        Icon::Close => {
            stroke(&[at(3.5, 3.5), at(12.5, 12.5)]);
            stroke(&[at(12.5, 3.5), at(3.5, 12.5)]);
        }
        Icon::Up => stroke(&[at(3.0, 10.0), at(8.0, 5.0), at(13.0, 10.0)]),
        Icon::Down => stroke(&[at(3.0, 6.0), at(8.0, 11.0), at(13.0, 6.0)]),
        Icon::Plus => {
            stroke(&[at(8.0, 3.0), at(8.0, 13.0)]);
            stroke(&[at(3.0, 8.0), at(13.0, 8.0)]);
        }
        Icon::Minus => stroke(&[at(3.0, 8.0), at(13.0, 8.0)]),
        Icon::PlayRow => {
            p.add(egui::Shape::convex_polygon(vec![at(4.8, 2.6), at(4.8, 13.4), at(13.4, 8.0)], ink, Stroke::NONE));
        }
        Icon::Queue => {
            stroke(&[at(2.0, 4.0), at(10.0, 4.0)]);
            stroke(&[at(2.0, 8.0), at(10.0, 8.0)]);
            stroke(&[at(2.0, 12.0), at(6.0, 12.0)]);
            p.add(egui::Shape::convex_polygon(vec![at(10.0, 9.8), at(10.0, 14.2), at(14.2, 12.0)], ink, Stroke::NONE));
        }
        Icon::AddList => {
            stroke(&[at(2.0, 4.0), at(11.0, 4.0)]);
            stroke(&[at(2.0, 8.0), at(11.0, 8.0)]);
            stroke(&[at(2.0, 12.0), at(6.0, 12.0)]);
            stroke(&[at(12.5, 9.5), at(12.5, 14.5)]);
            stroke(&[at(10.0, 12.0), at(15.0, 12.0)]);
        }
        // ── editing and devices ──
        Icon::Edit => {
            closed(&[at(10.6, 2.6), at(13.4, 5.4), at(5.6, 13.2), at(2.4, 13.6), at(2.8, 10.4)], &[at(10.6, 2.6), at(13.4, 5.4)]);
            stroke(&[at(9.0, 4.4), at(11.6, 7.0)]);
        }
        Icon::Trash => {
            stroke(&[at(2.5, 4.2), at(13.5, 4.2)]);
            stroke(&[at(6.0, 4.2), at(6.0, 2.5), at(10.0, 2.5), at(10.0, 4.2)]);
            stroke(&[at(4.2, 4.2), at(5.0, 13.5), at(11.0, 13.5), at(11.8, 4.2)]);
            stroke(&[at(6.6, 6.6), at(6.8, 11.2)]);
            stroke(&[at(9.4, 6.6), at(9.2, 11.2)]);
        }
        Icon::Check => stroke(&[at(2.8, 8.6), at(6.4, 12.2), at(13.2, 4.6)]),
        Icon::Speaker => {
            p.rect_stroke(Rect::from_min_max(at(4.0, 1.5), at(12.0, 14.5)), 1.6 * k, Stroke::new(w, ink), egui::StrokeKind::Middle);
            p.circle_stroke(at(8.0, 9.4), 2.3 * k, Stroke::new(w, ink));
            p.circle_filled(at(8.0, 4.6), 0.9 * k, ink);
        }
        Icon::Computer => {
            p.rect_stroke(Rect::from_min_max(at(2.0, 3.0), at(14.0, 10.8)), 1.4 * k, Stroke::new(w, ink), egui::StrokeKind::Middle);
            stroke(&[at(8.0, 10.8), at(8.0, 13.4)]);
            stroke(&[at(4.8, 13.5), at(11.2, 13.5)]);
        }
        Icon::Phone => {
            p.rect_stroke(Rect::from_min_max(at(4.5, 1.5), at(11.5, 14.5)), 1.6 * k, Stroke::new(w, ink), egui::StrokeKind::Middle);
            p.circle_filled(at(8.0, 12.3), 0.8 * k, ink);
        }
    }
}

// ── Appearance tab ──────────────────────────────────────────────────────────

fn families() -> Vec<String> {
    use fontique::{Collection, CollectionOptions};
    let mut c = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    let mut v: Vec<String> = c.family_names().map(str::to_string).collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup();
    v
}

fn choice<T: Copy + PartialEq>(ui: &mut Ui, label: &str, cur: &mut T, opts: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        for (v, name) in opts {
            changed |= ui.selectable_value(cur, *v, *name).changed();
        }
    });
    changed
}

fn slider<N: egui::emath::Numeric>(ui: &mut Ui, label: &str, v: &mut N, range: std::ops::RangeInclusive<N>, suffix: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(v, range).suffix(suffix)).changed()
    })
    .inner
}

/// The settings window's Appearance tab. Everything applies at once (the window is the preview)
/// and is written to `[gui]` once the pointer is released.
pub fn appearance(ui: &mut Ui, m: &mut Model, look: &mut Look) {
    let mut changed = false;
    super::presets::section(ui, m, &mut look.presets);
    ui.separator();
    let g = &mut m.gui;
    ui.label(egui::RichText::new("Changes show at once and are saved to [gui] in config.toml.").small().weak());
    ui.heading("Look");
    let mut preset = g.theme_preset;
    if choice(ui, "Look", &mut preset, &[
            (Preset::None, "none"),
            (Preset::Halloween, "Halloween"),
            (Preset::Christmas, "Christmas"),
            (Preset::Easter, "Easter"),
            (Preset::Midsummer, "Midsummer"),
        ]) {
        g.apply_preset(preset);
        changed = true;
    }
    ui.label(egui::RichText::new("Picking a look sets colours, background and glow below; change any of them afterwards.").small().weak());
    let mut v = g.ui_scale as u32;
    if slider(ui, "Interface size", &mut v, 50..=200, "%") {
        g.ui_scale = v as u16;
        changed = true;
    }
    ui.label(egui::RichText::new("Ctrl and + / - / 0 zoom as well; Ctrl+Shift+Up / Down change the window opacity.").small().weak());
    ui.separator();
    ui.heading("Window");
    let mut o = g.opacity as u32;
    if slider(ui, "Window opacity", &mut o, 0..=100, "%") {
        g.opacity = o as u8;
        changed = true;
    }
    if g.opacity < 100 {
        ui.label(egui::RichText::new("See-through windows need a compositing window manager.").small().weak());
    }
    let mut o = g.panel_opacity as u32;
    if slider(ui, "Panel opacity", &mut o, 0..=100, "%") {
        g.panel_opacity = o as u8;
        changed = true;
    }
    changed |= choice(ui, "Theme", &mut g.theme, &[(Theme::System, "system"), (Theme::Dark, "dark"), (Theme::Light, "light")]);
    changed |= choice(
        ui,
        "Background",
        &mut g.background,
        &[
            (Background::Solid, "solid"),
            (Background::CoverBlur, "cover blur"),
            (Background::Gradient, "gradient"),
            (Background::Image, "picture"),
            (Background::Sky, "sky"),
            (Background::Meadow, "meadow"),
            (Background::Lawn, "lawn"),
        ],
    );
    let fx_opts = [
        (Effects::Auto, "automatic (the look's own)"),
        (Effects::None, "none"),
        (Effects::Snow, "snow"),
        (Effects::Winter, "snow and lights"),
        (Effects::Leaves, "autumn leaves"),
        (Effects::Seeds, "dandelion seeds"),
        (Effects::Wind, "wind"),
        (Effects::Embers, "embers"),
        (Effects::Fog, "fog"),
        (Effects::Bats, "bats"),
        (Effects::Haunt, "fog, embers and bats"),
        (Effects::Pollen, "pollen"),
        (Effects::Eggs, "eggs"),
    ];
    changed |= choice(ui, "Effects", &mut g.effects, &fx_opts);
    let mut v = g.effects_intensity as u32;
    if slider(ui, "Effect intensity", &mut v, 0..=100, "%") {
        g.effects_intensity = v as u8;
        changed = true;
    }
    if g.background == Background::Image {
        ui.horizontal(|ui| {
            ui.label("Picture file");
            let buf = look.image_edit.get_or_insert_with(|| g.background_image.clone());
            let r = ui.add(egui::TextEdit::singleline(buf).hint_text("/path/to/picture.png or ~/…").desired_width(260.0));
            if r.lost_focus() && buf.trim() != g.background_image {
                g.background_image = buf.trim().to_string();
                changed = true;
            } else if !r.has_focus() && *buf != g.background_image {
                *buf = g.background_image.clone();
            }
        });
        if !g.background_image.is_empty() && look.picture.as_ref().is_some_and(|(p, t)| *p == g.background_image && t.is_none()) {
            ui.colored_label(ui.visuals().warn_fg_color, "That file can't be read as a picture.");
        }
    }
    if Look::animated(g) {
        ui.label(egui::RichText::new("Moving backgrounds redraw the window about 30 times a second while it has focus.").small().weak());
    }
    changed |= ui.checkbox(&mut g.decorations, "Window title bar and borders").changed();
    changed |= ui.checkbox(&mut g.always_on_top, "Keep the window on top").changed();
    changed |= ui.checkbox(&mut g.spotify_layout, "Spotify layout (experimental): library left, lyrics middle, song right, player bar at the bottom").changed();
    ui.add_enabled_ui(g.spotify_layout, |ui| {
        changed |= ui.checkbox(&mut g.spotify_look, "Spotify look as well: its sizes, Figtree, cover-coloured headers, queue panel, lyrics page").changed();
    });
    changed |= super::spotify_view::bar_settings(ui, g);
    changed |= ui.checkbox(&mut g.float_windows, "Open the extra windows floating (Hyprland, sway, niri)").changed();
    changed |= ui.checkbox(&mut g.remember_window, "Remember window size and position (next start)").changed();

    ui.separator();
    ui.heading("Fun");
    changed |= ui.checkbox(&mut g.vinyl, "Vinyl: the cover spins like a record while the song plays").changed();
    changed |= ui.checkbox(&mut g.confetti, "Confetti: liking a song bursts hearts from the heart button").changed();
    changed |= ui.checkbox(&mut g.beat_pulse, "Beat pulse: the play button glows with the music (starts cava)").changed();
    if g.vinyl || g.beat_pulse {
        ui.label(egui::RichText::new("These redraw the window about 30 times a second while they move.").small().weak());
    }

    ui.separator();
    ui.heading("Colour");
    changed |= choice(
        ui,
        "Colours from",
        &mut g.colors,
        &[
            (ColorsFrom::Off, "the look and the cover"),
            (ColorsFrom::File, "a colour file (matugen, noctalia…)"),
            (ColorsFrom::Gtk, "the GTK theme"),
            (ColorsFrom::Auto, "a colour file, then the GTK theme"),
        ],
    );
    if g.colors != ColorsFrom::Off {
        if matches!(g.colors, ColorsFrom::File | ColorsFrom::Auto) {
            ui.horizontal(|ui| {
                ui.label("Colour file");
                let buf = look.colors_edit.get_or_insert_with(|| g.colors_file.clone());
                let r = ui.add(egui::TextEdit::singleline(buf).hint_text("empty = the usual places").desired_width(260.0));
                if r.lost_focus() && buf.trim() != g.colors_file {
                    g.colors_file = buf.trim().to_string();
                    changed = true;
                } else if !r.has_focus() && *buf != g.colors_file {
                    *buf = g.colors_file.clone();
                }
            });
        }
        match look.system_source() {
            Some(s) => ui.label(egui::RichText::new(format!("in use: {s} (its window colour, text and accent replace the look's)")).small().weak()),
            None => ui.label(egui::RichText::new("nothing found yet; the look's colours are used").small().weak()),
        };
    }
    ui.horizontal(|ui| {
        ui.label("Accent");
        let mut cover = g.accent == Accent::Cover;
        if ui.selectable_label(cover, "from the cover").clicked() && !cover {
            g.accent = Accent::Cover;
            cover = true;
            changed = true;
        }
        if ui.selectable_label(!cover, "fixed colour").clicked() && cover {
            let c = look.accent.unwrap_or((90, 150, 230));
            g.accent = Accent::Fixed(c);
            changed = true;
        }
        if let Accent::Fixed((r, gr, b)) = g.accent {
            let mut c = [r, gr, b];
            if egui::color_picker::color_edit_button_srgb(ui, &mut c).changed() {
                g.accent = Accent::Fixed((c[0], c[1], c[2]));
                changed = true;
            }
        }
    });

    ui.separator();
    ui.heading("Text");
    ui.horizontal(|ui| {
        ui.label("Font");
        let shown = if g.font.is_empty() { "default".to_string() } else { g.font.clone() };
        egui::ComboBox::from_id_salt("gui-font").selected_text(shown).width(220.0).show_ui(ui, |ui| {
            let list = look.families.get_or_insert_with(families);
            ui.text_edit_singleline(&mut look.filter).on_hover_text("filter");
            if ui.selectable_label(g.font.is_empty(), "default").clicked() {
                g.font.clear();
                changed = true;
            }
            let f = look.filter.to_lowercase();
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for name in list.iter().filter(|n| f.is_empty() || n.to_lowercase().contains(&f)) {
                    if ui.selectable_label(&g.font == name, name).clicked() {
                        g.font = name.clone();
                        changed = true;
                    }
                }
            });
        });
    });
    let mut v = g.lyrics_size as u32;
    if slider(ui, "Lyrics size", &mut v, 10..=64, " pt") {
        g.lyrics_size = v as u16;
        changed = true;
    }
    let mut v = g.active_size as u32;
    if slider(ui, "Active line size", &mut v, 14..=72, " pt") {
        g.active_size = v as u16;
        changed = true;
    }
    let mut v = g.meta_size as u32;
    if slider(ui, "Text size (metadata, buttons)", &mut v, 8..=40, " pt") {
        g.meta_size = v as u16;
        changed = true;
    }
    let mut v = g.translation_size as u32;
    if slider(ui, "Translation size (0 = automatic)", &mut v, 0..=64, " pt") {
        g.translation_size = v as u16;
        changed = true;
    }
    let mut v = g.member_size as u32;
    if slider(ui, "Member parts size (0 = automatic)", &mut v, 0..=40, " pt") {
        g.member_size = v as u16;
        changed = true;
    }
    changed |= choice(ui, "Active line weight", &mut g.active_weight, &[(Weight::Regular, "regular"), (Weight::Medium, "medium"), (Weight::Bold, "bold")]);
    changed |= choice(ui, "Lyrics alignment", &mut g.lyrics_align, &[(LAlign::Left, "left"), (LAlign::Center, "center")]);
    let mut v = g.inactive_fade as u32;
    if slider(ui, "Fade of inactive lines", &mut v, 0..=100, "%") {
        g.inactive_fade = v as u8;
        changed = true;
    }
    let mut v = g.lyrics_glow as u32;
    if slider(ui, "Glow around the active line", &mut v, 0..=100, "%") {
        g.lyrics_glow = v as u8;
        changed = true;
    }
    changed |= choice(ui, "Lyrics movement", &mut g.lyrics_animation, &[(Animation::Smooth, "smooth"), (Animation::Instant, "instant")]);

    ui.separator();
    ui.heading("Panels");
    let mut v = g.corner_radius as u32;
    if slider(ui, "Corner radius", &mut v, 0..=32, " px") {
        g.corner_radius = v as u8;
        changed = true;
    }
    let mut v = g.panel_spacing as u32;
    if slider(ui, "Panel spacing", &mut v, 0..=32, " px") {
        g.panel_spacing = v as u8;
        changed = true;
    }
    changed |= ui.checkbox(&mut g.panel_borders, "Panel borders").changed();
    changed |= ui.checkbox(&mut g.lock_layout, "Lock the layout (column borders can't be dragged)").changed();

    ui.separator();
    // one press asks, the second one resets
    if look.confirm_reset {
        ui.horizontal(|ui| {
            ui.label("Reset every appearance setting?");
            if ui.button("Yes, reset").clicked() {
                *g = GuiConf::default();
                changed = true;
                look.confirm_reset = false;
            }
            if ui.button("Cancel").clicked() {
                look.confirm_reset = false;
            }
        });
    } else if ui.button("Reset appearance").clicked() {
        look.confirm_reset = true;
    }
    look.dirty |= changed;
    // a slider drag changes every frame: write once the pointer is up
    if look.dirty && !ui.input(|i| i.pointer.any_down()) {
        look.dirty = false;
        m.gui.clamp();
        let t = m.gui.write();
        m.save_settings(&[("gui", toml::Value::Table(t))]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_touching_a_split_are_square() {
        let r = 12;
        assert_eq!(Split::NONE.corners(r), CornerRadius::same(r));
        // a middle column: square on both sides
        assert_eq!(Split { before: true, after: true, vertical: false }.corners(r), CornerRadius::ZERO);
        // the first column: rounded left, square right
        assert_eq!(Split { before: false, after: true, vertical: false }.corners(r), CornerRadius { nw: r, sw: r, ne: 0, se: 0 });
        // stacked rows: the split is below the first one
        assert_eq!(Split { before: false, after: true, vertical: true }.corners(r), CornerRadius { nw: r, ne: r, sw: 0, se: 0 });
    }

    fn sys(dark: Option<bool>) -> Colors {
        Colors { surface: Some((20, 22, 30)), primary: Some((10, 200, 10)), text: Some((230, 230, 240)), dark, source: "test".into() }
    }

    #[test]
    fn system_colours_replace_the_look_in_their_own_mode_only() {
        let mut g = GuiConf::default();
        g.apply_preset(music_display::guiconf::Preset::Christmas);
        let c = sys(Some(true));
        assert_eq!(base(&g, Some(&c), true), (20, 22, 30));
        // the light mode keeps the look's own palette
        assert_eq!(Some(base(&g, Some(&c), false)), g.theme_preset.base(false));
        assert_eq!(Some(base(&g, None, true)), g.theme_preset.base(true));
        // a source that does not say whether it is dark serves both
        assert_eq!(base(&g, Some(&sys(None)), false), (20, 22, 30));
        // the system accent beats the look's fixed one and the cover's; without it they stay
        assert_eq!(accent_of(&g, Some((1, 2, 3)), Some(&c)), Some((10, 200, 10)));
        assert_eq!(accent_of(&g, Some((1, 2, 3)), None), Some((242, 201, 76)));
        let plain = GuiConf::default();
        assert_eq!(accent_of(&plain, Some((1, 2, 3)), None), Some((1, 2, 3)));
        assert_eq!(accent_of(&plain, None, None), None);
        // a colour source with a surface and no accent leaves the accent alone
        let only_surface = Colors { primary: None, ..sys(Some(true)) };
        assert_eq!(accent_of(&g, None, Some(&only_surface)), Some((242, 201, 76)));
    }

    #[test]
    fn own_colours_replace_the_looks_but_not_the_systems() {
        let g = GuiConf { window_color: Some((1, 2, 3)), theme_preset: Preset::Halloween, ..GuiConf::default() };
        assert_eq!(base(&g, None, true), (1, 2, 3));
        let c = sys(None);
        assert_eq!(Some(base(&g, Some(&c), true)), c.surface);
    }

    #[test]
    fn the_transport_buttons_are_one_size() {
        // previous, play / pause and next share one constant: none is drawn larger than the others
        assert_eq!(ICON_SIDE, 30.0);
        let src = include_str!("controls.rs");
        assert_eq!(src.matches("style::icon_button(ui, ").count(), 3);
    }

    #[test]
    fn a_look_named_in_the_file_gets_its_font_without_one_being_chosen() {
        let mut g = GuiConf { theme_preset: Preset::Christmas, ..GuiConf::default() };
        assert!(g.font.is_empty());
        assert_eq!(font_of(&g), BUILT_IN_FAMILY);
        // a font chosen by hand wins, and other looks have none of their own
        g.font = "Noto Sans".into();
        assert_eq!(font_of(&g), "Noto Sans");
        assert_eq!(font_of(&GuiConf { theme_preset: Preset::Easter, ..GuiConf::default() }), "");
        assert_eq!(font_of(&GuiConf::default()), "");
    }

    #[test]
    fn the_built_in_family_is_there_when_no_installed_font_has_the_name() {
        let defs = fonts(BUILT_IN_FAMILY, Weight::Bold);
        assert!(defs.font_data.contains_key("md-user"));
        assert!(defs.font_data.contains_key(BOLD));
        let first = defs.families.get(&FontFamily::Proportional).and_then(|f| f.first()).cloned();
        assert_eq!(first.as_deref(), Some("md-user"));
        // another unknown family still just leaves the defaults
        let defs = fonts("No Such Family Anywhere", Weight::Regular);
        assert!(!defs.font_data.contains_key("md-user"));
    }
}
