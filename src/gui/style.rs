//! How the window looks: the `[gui]` settings (see `music_display::guiconf`) turned into egui
//! styles, fonts, the background, panel cards and transport icons, plus the Appearance tab of
//! the settings window. The settings themselves are plain data in `guiconf`, so they are tested
//! without a window; nothing in this file has been checked visually on a real compositor.
use super::backdrop;
use super::model::Model;
use egui::{Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use music_display::color::{self, Rgb};
use music_display::guiconf::{Accent, Align as LAlign, Animation, Background, GuiConf, Preset, Theme, Weight};

/// The family the active line uses when `active_weight` is medium or bold.
const BOLD: &str = "md-bold";

/// What the window has applied so far, so the (costly) restyle only runs when something changed.
#[derive(Default)]
pub struct Look {
    applied: Option<(GuiConf, Option<Rgb>)>,
    fonts_for: Option<(String, Weight)>,
    /// The font collection is built on a background thread (scanning the system fonts takes
    /// a while); its answer arrives here.
    fonts_rx: Option<std::sync::mpsc::Receiver<egui::FontDefinitions>>,
    /// Accent taken from the current cover.
    accent: Option<Rgb>,
    /// A tiny copy of the cover, stretched smooth, for `background = "cover-blur"`.
    blur: Option<egui::TextureHandle>,
    /// The picture of `background = "image"`, with the path it was loaded from (and whether that failed).
    picture: Option<(String, Option<egui::TextureHandle>)>,
    families: Option<Vec<String>>,
    filter: String,
    /// The picture path being typed (written to the settings when the field loses focus).
    image_edit: Option<String>,
    /// A change not written to the file yet (written when the pointer is released).
    dirty: bool,
    /// "Reset appearance" was pressed once and asks again.
    pub confirm_reset: bool,
}

/// The window itself: transparent (so `opacity` can work live), with the chosen decorations.
pub fn viewport(g: &GuiConf) -> egui::ViewportBuilder {
    let v = egui::ViewportBuilder::default()
        .with_inner_size([1000.0, 640.0])
        .with_min_inner_size([480.0, 320.0])
        .with_title("music_display")
        .with_app_id("music_display")
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

/// The window colour for a theme (a named look can have its own dark one).
fn base(g: &GuiConf, dark: bool) -> Rgb {
    match g.theme_preset.base() {
        Some(c) if dark => c,
        _ => if dark { (27, 27, 31) } else { (243, 243, 246) },
    }
}

/// The colour the moving backgrounds are drawn in.
fn effect_color(g: &GuiConf, cover: Option<Rgb>) -> Color32 {
    match (g.accent, cover) {
        (Accent::Fixed(c), _) | (Accent::Cover, Some(c)) => rgb32(c),
        (Accent::Cover, None) => Color32::from_rgb(170, 130, 230),
    }
}

impl Look {
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

    /// Is something on the background moving (so the window should redraw about 30 times a second)?
    pub fn animated(g: &GuiConf) -> bool {
        matches!(g.background, Background::Particles | Background::Fog | Background::Bats)
    }

    /// Bring the egui styles, fonts and window flags in line with `g` (only when it changed).
    pub fn apply(&mut self, ctx: &egui::Context, g: &GuiConf) {
        if let Some(rx) = &self.fonts_rx {
            if let Ok(defs) = rx.try_recv() {
                ctx.set_fonts(defs);
                self.fonts_rx = None;
            }
        }
        let key = (g.clone(), self.accent);
        if self.applied.as_ref() == Some(&key) {
            return;
        }
        let first = self.applied.is_none();
        let prev = self.applied.replace(key).map(|(p, _)| p);
        ctx.set_theme(match g.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Dark => egui::ThemePreference::Dark,
            Theme::Light => egui::ThemePreference::Light,
        });
        if self.fonts_for.as_ref() != Some(&(g.font.clone(), g.active_weight)) {
            // the bold family must exist at once (the active line asks for it); the real fonts
            // replace this when the scan is done
            if self.fonts_for.is_none() {
                ctx.set_fonts(base_fonts());
            }
            let (family, weight, c) = (g.font.clone(), g.active_weight, ctx.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            self.fonts_rx = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(fonts(&family, weight));
                c.request_repaint();
            });
            self.fonts_for = Some((g.font.clone(), g.active_weight));
        }
        for th in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.set_style_of(th, style(g, self.accent, th == egui::Theme::Dark));
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
        let b = rgb32(base(g, dark));
        let p = ui.painter();
        let time = ui.input(|i| i.time);
        let fx = effect_color(g, self.accent);
        match (g.background, &self.blur, self.accent) {
            (Background::Image, _, _) => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
                if let Some((_, Some(t))) = &self.picture {
                    p.image(t.id(), r, cover_uv(r, t.size_vec2()), with_alpha(Color32::WHITE, a * 0.8));
                    p.rect_filled(r, 0.0, with_alpha(b, a * 0.35));
                }
            }
            (Background::Particles, _, _) => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
                backdrop::embers(p, r, time, fx, a);
            }
            (Background::Fog, _, _) => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
                backdrop::fog(p, r, time, fx, a);
            }
            (Background::Bats, _, _) => {
                p.rect_filled(r, 0.0, with_alpha(b, a));
                backdrop::fog(p, r, time, Color32::from_rgb(110, 50, 170), a);
                backdrop::embers(p, r, time, fx, a * 0.7);
                backdrop::bats(p, r, time, Color32::from_rgb(8, 4, 12), a);
            }
            (Background::CoverBlur, Some(t), _) => {
                let uv = cover_uv(r, Vec2::splat(1.0));
                p.rect_filled(r, 0.0, with_alpha(b, a));
                p.image(t.id(), r, uv, with_alpha(Color32::WHITE, a * 0.55));
                p.rect_filled(r, 0.0, with_alpha(b, a * 0.45));
            }
            (Background::Gradient, _, acc) => {
                let top = match acc {
                    Some(c) => rgb32(color::mix(base(g, dark), c, 0.28)),
                    None => rgb32(color::mix(base(g, dark), if dark { (255, 255, 255) } else { (0, 0, 0) }, 0.10)),
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
fn style(g: &GuiConf, cover: Option<Rgb>, dark: bool) -> egui::Style {
    let mut s = egui::Style { visuals: if dark { egui::Visuals::dark() } else { egui::Visuals::light() }, ..Default::default() };
    let acc = match g.accent {
        Accent::Fixed(c) => Some(c),
        Accent::Cover => cover.map(|c| color::readable(c, Some(base(g, dark)))),
    };
    let v = &mut s.visuals;
    v.panel_fill = Color32::TRANSPARENT;
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

fn fonts(family: &str, weight: Weight) -> egui::FontDefinitions {
    use egui::{FontData, FontDefinitions};
    use fontique::{Attributes, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, QueryFamily, QueryStatus, SourceCache, SourceCacheOptions};
    let mut defs = FontDefinitions::default();
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
    let normal = if family.is_empty() { None } else { pick(family, FontWeight::NORMAL) };
    let bold = pick(family, if weight == Weight::Medium { FontWeight::MEDIUM } else { FontWeight::BOLD });
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
    let dark = ui.visuals().dark_mode;
    let fill = match g.theme_preset {
        Preset::Halloween if dark => Color32::from_rgb(38, 20, 50),
        _ if dark => Color32::from_rgb(40, 40, 47),
        _ => Color32::WHITE,
    };
    let alpha = g.panel_opacity as f32 / 100.0 * (g.opacity as f32 / 100.0).max(0.0);
    let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let margin = g.panel_spacing.min(127) as i8;
    let h = ui.available_height();
    Frame::new()
        .fill(with_alpha(fill, alpha))
        .corner_radius(CornerRadius::same(g.corner_radius))
        .stroke(if g.panel_borders { Stroke::new(1.0, line) } else { Stroke::NONE })
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
}

/// A round transport button with a drawn icon (no dependence on the font having the glyphs);
/// lights up under the pointer, and `main` ones (play) are filled with the accent.
pub fn icon_button(ui: &mut Ui, icon: Icon, main: bool) -> Response {
    let side = if main { 40.0 } else { 30.0 };
    let (r, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    let accent = ui.visuals().hyperlink_color;
    let ink = if main { if color::luminance((accent.r(), accent.g(), accent.b())) > 0.6 { Color32::BLACK } else { Color32::WHITE } } else { ui.visuals().strong_text_color() };
    let p = ui.painter();
    if main {
        let k = if resp.is_pointer_button_down_on() { 0.8 } else if resp.hovered() { 1.0 } else { 0.85 };
        p.circle_filled(r.center(), side / 2.0, accent.gamma_multiply(k));
    } else if resp.hovered() {
        p.circle_filled(r.center(), side / 2.0, with_alpha(ui.visuals().widgets.hovered.fg_stroke.color, if resp.is_pointer_button_down_on() { 0.22 } else { 0.12 }));
    }
    let c = r.center();
    let u = side * 0.2;
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
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    let g = &mut m.gui;
    ui.label(egui::RichText::new("Changes show at once and are saved to [gui] in config.toml.").small().weak());
    ui.heading("Look");
    let mut preset = g.theme_preset;
    if choice(ui, "Look", &mut preset, &[(Preset::None, "none"), (Preset::Halloween, "Halloween")]) {
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
            (Background::Particles, "embers"),
            (Background::Fog, "fog"),
            (Background::Bats, "fog and bats"),
        ],
    );
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
    changed |= ui.checkbox(&mut g.remember_window, "Remember window size and position (next start)").changed();

    ui.separator();
    ui.heading("Colour");
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
