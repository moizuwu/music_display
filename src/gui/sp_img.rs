//! Pictures for the Spotify layout (covers of albums and playlists, artist photos): downloaded in
//! the background with the Spotify client, decoded off the UI thread, and turned into textures
//! the first time they are drawn. Nothing blocks the window; what has not arrived is a quiet
//! placeholder.
use super::library::Spot;
use egui::{CornerRadius, Rect, Response, Sense, TextureHandle, Ui, Vec2};
use music_display::spotify::Reply;
use music_display::AppEvent;
use std::collections::{HashMap, VecDeque};

/// How many pictures are kept (the oldest go first).
const KEEP: usize = 400;

enum Slot {
    /// Asked for, not here yet.
    Wanted,
    /// Could not be had (not asked again).
    Failed,
    /// Downloaded and decoded, no texture yet.
    Raw(Box<image::RgbaImage>),
    Tex(TextureHandle),
}

/// Pictures by URL. URLs that start with `fake:` are never downloaded (the render test fills them).
#[derive(Default)]
pub struct Images {
    map: HashMap<String, Slot>,
    order: VecDeque<String>,
    /// The main colour of each picture (for the Spotify look's page headers), found on arrival.
    colors: HashMap<String, egui::Color32>,
}

impl Images {
    pub fn on_image(&mut self, url: &str, img: Option<Box<image::RgbaImage>>) {
        self.put(url, img.map_or(Slot::Failed, Slot::Raw));
    }

    /// Hand over a picture that is already here (the render test's made-up covers).
    #[cfg(test)]
    pub fn insert(&mut self, url: &str, img: image::RgbaImage) {
        self.put(url, Slot::Raw(Box::new(img)));
    }

    /// The main colour of a picture that is here.
    pub fn color(&self, url: &str) -> Option<egui::Color32> {
        self.colors.get(url).copied()
    }

    fn put(&mut self, url: &str, slot: Slot) {
        if let Slot::Raw(img) = &slot {
            self.colors.insert(url.to_string(), dominant(img));
        }
        if self.map.insert(url.to_string(), slot).is_none() {
            self.order.push_back(url.to_string());
        }
        while self.order.len() > KEEP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
                self.colors.remove(&old);
            }
        }
    }

    /// The picture as a texture; `Err(true)` when it must be downloaded first (asked once).
    fn texture(&mut self, ctx: &egui::Context, url: &str) -> Result<Option<TextureHandle>, bool> {
        match self.map.get_mut(url) {
            None => {
                self.put(url, Slot::Wanted);
                Err(true)
            }
            Some(Slot::Tex(t)) => Ok(Some(t.clone())),
            Some(Slot::Raw(img)) => {
                let size = [img.width() as usize, img.height() as usize];
                let tex = ctx.load_texture(format!("sp:{url}"), egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR);
                self.map.insert(url.to_string(), Slot::Tex(tex.clone()));
                Ok(Some(tex))
            }
            Some(_) => Ok(None),
        }
    }
}

/// The main colour of a picture, the way Spotify tints a page header or the lyrics with it: the
/// average of its colourful pixels (grey ones count little), made dark enough for white text.
pub fn dominant(img: &image::RgbaImage) -> egui::Color32 {
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    let step = ((img.width() * img.height()) as usize / 4096).max(1);
    for p in img.pixels().step_by(step) {
        let [r, g, b] = [p[0], p[1], p[2]].map(|c| c as f32 / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        // saturation, and not too dark or too light
        let w = 0.05 + (max - min) * (1.0 - (2.0 * (max + min) / 2.0 - 1.0).abs());
        sum = [sum[0] + r * w, sum[1] + g * w, sum[2] + b * w];
        weight += w;
    }
    let avg = sum.map(|c| c / weight.max(1e-6));
    // at most this bright, so white text on it stays readable
    let lum = 0.2126 * avg[0] + 0.7152 * avg[1] + 0.0722 * avg[2];
    let k = if lum > 0.32 { 0.32 / lum } else { 1.0 };
    let [r, g, b] = avg.map(|c| ((c * k).clamp(0.0, 1.0) * 255.0).round() as u8);
    egui::Color32::from_rgb(r, g, b)
}

impl Spot {
    /// The texture of a picture, when it is here; the first call starts the download.
    pub fn picture(&mut self, ctx: &egui::Context, url: &str) -> Option<TextureHandle> {
        if url.is_empty() {
            return None;
        }
        match self.pg.images.texture(ctx, url) {
            Ok(t) => t,
            Err(_) => {
                // covers need no login: the plain client fetches them (Soloist's queue without the Web API)
                if !url.starts_with("fake:") {
                    let (tx, url, http) = (self.tx.clone(), url.to_string(), self.client.clone());
                    tokio::spawn(async move {
                        let img = music_display::spotify::fetch_image(&http, &url).await.map(Box::new);
                        let _ = tx.send(AppEvent::Spotify(Reply::Image { url, img }));
                    });
                }
                None
            }
        }
    }
}

/// A picture of `size` (square, rounded; `round` makes it a circle), or a placeholder of the
/// card's colour with a note while it is not here. Returns the response of its rectangle.
pub fn art(ui: &mut Ui, spot: &mut Spot, url: &str, size: Vec2, round: bool, sense: Sense) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    paint_art(ui, spot, url, rect, round);
    resp
}

/// [`art`] into a rectangle that is already allocated.
pub fn paint_art(ui: &Ui, spot: &mut Spot, url: &str, rect: Rect, round: bool) {
    let radius = if round { CornerRadius::same((rect.width().min(rect.height()) / 2.0) as u8) } else { CornerRadius::same(4) };
    match spot.picture(ui.ctx(), url) {
        Some(tex) => {
            egui::Image::new(&tex).fit_to_exact_size(rect.size()).corner_radius(radius).paint_at(ui, rect);
        }
        None => {
            let base = ui.visuals().widgets.noninteractive.bg_stroke.color;
            ui.painter().rect_filled(rect, radius, base.gamma_multiply(0.6));
            let ink = ui.visuals().weak_text_color();
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "♪", egui::FontId::proportional((rect.height() * 0.4).clamp(10.0, 40.0)), ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_asked_for_once_and_the_oldest_are_dropped() {
        let ctx = egui::Context::default();
        let mut imgs = Images::default();
        assert!(matches!(imgs.texture(&ctx, "u1"), Err(true)), "the first look asks for the download");
        assert!(matches!(imgs.texture(&ctx, "u1"), Ok(None)), "then it is waited for");
        imgs.on_image("u1", None);
        assert!(matches!(imgs.texture(&ctx, "u1"), Ok(None)), "a failure is not asked for again");
        imgs.on_image("u2", Some(Box::new(image::RgbaImage::new(4, 4))));
        assert!(matches!(imgs.texture(&ctx, "u2"), Ok(Some(_))));
        assert!(matches!(imgs.texture(&ctx, "u2"), Ok(Some(_))), "and stays a texture");
        for i in 0..KEEP + 10 {
            imgs.insert(&format!("x{i}"), image::RgbaImage::new(1, 1));
        }
        assert!(imgs.map.len() <= KEEP);
        assert!(!imgs.map.contains_key("u1"), "the oldest went");
    }
}

#[cfg(test)]
mod dominant_tests {
    use super::*;

    #[test]
    fn the_colourful_part_wins_and_it_is_dark_enough_for_white_text() {
        // mostly grey with a red block: red comes out
        let mut img = image::RgbaImage::from_pixel(64, 64, image::Rgba([128, 128, 128, 255]));
        for y in 0..24 {
            for x in 0..64 {
                img.put_pixel(x, y, image::Rgba([220, 30, 30, 255]));
            }
        }
        let c = dominant(&img);
        assert!(c.r() > c.g() * 2 && c.r() > c.b() * 2, "{c:?}");
        // a bright yellow cover is darkened
        let y = dominant(&image::RgbaImage::from_pixel(8, 8, image::Rgba([250, 230, 40, 255])));
        let lum = 0.2126 * y.r() as f32 + 0.7152 * y.g() as f32 + 0.0722 * y.b() as f32;
        assert!(lum <= 0.33 * 255.0, "{y:?}");
    }
}
