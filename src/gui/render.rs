//! A real render of the window for reviewing the look without a display: the program's own
//! drawing code runs on a fake song, egui tessellates it and a small software rasteriser turns
//! the triangles into a PNG. Only built for tests; `scripts/render.sh` runs it.
use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive};
use egui::{Color32, TextureId};
use std::collections::HashMap;

/// The textures egui uploaded (the font atlas, the cover), as RGBA pixels.
#[derive(Default)]
pub struct Textures(HashMap<TextureId, (usize, usize, Vec<Color32>)>);

impl Textures {
    pub fn apply<'a>(&mut self, set: impl IntoIterator<Item = (&'a TextureId, &'a ImageDelta)>) {
        for (id, delta) in set {
            let egui::ImageData::Color(img) = &delta.image;
            let (w, h) = (img.size[0], img.size[1]);
            match delta.pos {
                None => {
                    self.0.insert(*id, (w, h, img.pixels.clone()));
                }
                Some([x0, y0]) => {
                    if let Some((tw, _, px)) = self.0.get_mut(id) {
                        for y in 0..h {
                            for x in 0..w {
                                px[(y0 + y) * *tw + x0 + x] = img.pixels[y * w + x];
                            }
                        }
                    }
                }
            }
        }
    }

    fn sample(&self, id: TextureId, u: f32, v: f32) -> [f32; 4] {
        let Some((w, h, px)) = self.0.get(&id) else { return [1.0; 4] };
        let x = ((u * *w as f32) as usize).min(w - 1);
        let y = ((v * *h as f32) as usize).min(h - 1);
        let c = px[y * w + x];
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0]
    }
}

/// Fill the triangles into a `w` × `h` picture (premultiplied colours, blended in gamma space
/// as egui's own painter does); `ppp` is pixels per point.
pub fn rasterize(prims: &[ClippedPrimitive], tex: &Textures, w: usize, h: usize, ppp: f32, bg: [f32; 3]) -> Vec<[f32; 3]> {
    let mut img = vec![bg; w * h];
    for cp in prims {
        let Primitive::Mesh(mesh) = &cp.primitive else { continue };
        let clip = cp.clip_rect;
        let (cx0, cy0) = ((clip.min.x * ppp).max(0.0) as usize, (clip.min.y * ppp).max(0.0) as usize);
        let (cx1, cy1) = (((clip.max.x * ppp).ceil() as usize).min(w), ((clip.max.y * ppp).ceil() as usize).min(h));
        for t in mesh.indices.chunks_exact(3) {
            let v = [mesh.vertices[t[0] as usize], mesh.vertices[t[1] as usize], mesh.vertices[t[2] as usize]];
            let p: Vec<(f32, f32)> = v.iter().map(|q| (q.pos.x * ppp, q.pos.y * ppp)).collect();
            let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
            if area.abs() < 1e-9 {
                continue;
            }
            let xs = p.iter().map(|q| q.0);
            let ys = p.iter().map(|q| q.1);
            let x0 = (xs.clone().fold(f32::MAX, f32::min).floor().max(0.0) as usize).max(cx0);
            let x1 = (xs.fold(f32::MIN, f32::max).ceil() as usize).min(cx1);
            let y0 = (ys.clone().fold(f32::MAX, f32::min).floor().max(0.0) as usize).max(cy0);
            let y1 = (ys.fold(f32::MIN, f32::max).ceil() as usize).min(cy1);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[1].1 - py) * (p[2].0 - px)) / area;
                    let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[2].1 - py) * (p[0].0 - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let mix = |f: &dyn Fn(&egui::epaint::Vertex) -> f32| f(&v[0]) * w0 + f(&v[1]) * w1 + f(&v[2]) * w2;
                    let (u, vv) = (mix(&|q| q.uv.x), mix(&|q| q.uv.y));
                    let t = tex.sample(mesh.texture_id, u, vv);
                    let c = [
                        mix(&|q| q.color.r() as f32) / 255.0 * t[0],
                        mix(&|q| q.color.g() as f32) / 255.0 * t[1],
                        mix(&|q| q.color.b() as f32) / 255.0 * t[2],
                        mix(&|q| q.color.a() as f32) / 255.0 * t[3],
                    ];
                    let d = &mut img[y * w + x];
                    *d = [c[0] + d[0] * (1.0 - c[3]), c[1] + d[1] * (1.0 - c[3]), c[2] + d[2] * (1.0 - c[3])];
                }
            }
        }
    }
    img
}

/// Write the picture as a PNG.
pub fn save_png(img: &[[f32; 3]], w: usize, h: usize, path: &std::path::Path) -> std::io::Result<()> {
    let mut rgb = image::RgbImage::new(w as u32, h as u32);
    for (i, p) in img.iter().enumerate() {
        let px = image::Rgb([0, 1, 2].map(|k| (p[k].clamp(0.0, 1.0) * 255.0).round() as u8));
        rgb.put_pixel((i % w) as u32, (i / w) as u32, px);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    rgb.save(path).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use eframe::App;
    use music_display::cli::Args;
    use music_display::layout::{Layout, PanelId};
    use music_display::lyrics::Lyrics;

    /// A cover made up on the spot (no network, no files): a diagonal gradient with a ring.
    fn fake_cover() -> image::DynamicImage {
        let mut img = image::RgbImage::new(300, 300);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let (fx, fy) = (x as f32 / 300.0, y as f32 / 300.0);
            let d = ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt();
            let ring = if (0.24..0.30).contains(&d) { 70.0 } else { 0.0 };
            *p = image::Rgb([(40.0 + 120.0 * fx + ring) as u8, (30.0 + 60.0 * fy + ring) as u8, (90.0 + 110.0 * (1.0 - fx) + ring) as u8]);
        }
        image::DynamicImage::ImageRgb8(img)
    }

    /// `MDIS_RENDER=out.png cargo test --features gui render_main_view` (or `scripts/render.sh`):
    /// the main window as the program draws it, with the default look and a fake song; also
    /// `out-spotify.png` with the experimental Spotify layout.
    #[tokio::test(flavor = "multi_thread")]
    async fn render_main_view() {
        let Ok(out) = std::env::var("MDIS_RENDER") else { return };
        music_display::util::enter_test_mode();
        let (m, rx) = fake_model(false);
        render(m, rx, &out);
        let (m, rx) = fake_model(true);
        render(m, rx, &out.replace(".png", "-spotify.png"));
    }

    fn fake_model(spotify_layout: bool) -> (super::super::model::Model, tokio::sync::mpsc::UnboundedReceiver<music_display::AppEvent>) {
        let args = Args::parse_from(["mdis", "--gui", "--spotify"]);
        let mut layout = Layout::default();
        layout.panels.set(PanelId::Annotations, true);
        let cfg = std::path::PathBuf::from("target/render/no-config.toml");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut m = super::super::model::Model::new(args, layout, cfg, tx);
        m.gui.spotify_layout = spotify_layout;
        let line = "Playing\x1f95000000\x1fCity Lights\x1fThe Example Band\x1fNight Drive\x1f241000000\x1f\x1f2\x1f1\x1f2021-05-14\x1f0.7\x1f\x1f/com/spotify/track/4uLU6hMCjMI75M1A2tKUQC\x1fspotify\x1fSynthpop\x1fThe Example Band";
        let mut snap = music_display::player::parse(line).expect("snapshot");
        snap.shuffle = Some(false);
        snap.repeat = Some(music_display::player::Loop::Playlist);
        m.snap = Some(snap);
        m.set_position(95.0);
        let words = ["Streetlights hum a quiet tune", "We drive until the morning", "Every sign is pointing home", "City lights, city lights", "Hold on to the night", "Radio low, windows down", "Nothing left to say", "City lights, city lights"];
        m.lyrics = Some(Lyrics::Synced(words.iter().enumerate().map(|(i, s)| (80.0 + i as f64 * 4.0, s.to_string())).collect()));
        m.year = Some("2021".into());
        m.cover = Some(fake_cover());
        m.cover_fresh = true;
        // a few rows in the library, as if Recent had loaded
        let mk = |kind: &str, name: &str, sub: &str| music_display::spotify::Item { kind: kind.into(), uri: format!("spotify:{kind}:{name}"), name: name.into(), sub: sub.into(), openable: kind != "track", duration: if kind == "track" { 214.0 } else { 0.0 } };
        let tab = &mut m.spot.lib.tabs[0];
        tab.items = vec![mk("track", "City Lights", "The Example Band"), mk("track", "Morning Drive", "Lena Park"), mk("album", "Night Drive", "The Example Band"), mk("playlist", "Road trip", "Playlist"), mk("artist", "Lena Park", "Artist")];
        tab.loaded = true;
        tab.fetched = Some(std::time::Instant::now());
        (m, rx)
    }

    /// Run the window for a few frames at 1100 × 680 points, 2 pixels per point, and save it.
    fn render(m: super::super::model::Model, rx: tokio::sync::mpsc::UnboundedReceiver<music_display::AppEvent>, out: &str) {
        let ctx = egui::Context::default();
        let ppp = 2.0;
        ctx.set_pixels_per_point(ppp);
        let mut app = super::super::Gui {
            m,
            rx,
            cover: None,
            look: super::super::style::Look::default(),
            repaint: ctx.clone(),
            title_sent: None,
            zoom_seen: 1.0,
            colors_rx: None,
            colors_watch: Vec::new(),
            colors_asked: None,
            colors_polled: std::time::Instant::now(),
            size_watch: Default::default(),
            col_epoch: 0,
            col_total: 0.0,
        };
        let mut frame = eframe::Frame::_new_kittest();
        let (pw, ph) = (1100.0f32, 680.0f32);
        let mut tex = Textures::default();
        let mut last = None;
        for i in 0..6 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pw, ph))),
                time: Some(i as f64 * 0.5),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                let c = ui.ctx().clone();
                app.logic(&c, &mut frame);
                // (egui's debug-build hints, such as "Unaligned", are not part of the look)
                c.all_styles_mut(|st| st.debug = Default::default());
                c.all_styles_mut(|st| st.debug.show_unaligned = false);
                app.ui(ui, &mut frame);
            });
            tex.apply(out.textures_delta.set.iter().flat_map(|(id, ds)| ds.iter().map(move |d| (id, d))));
            out.textures_delta.clear();
            last = Some(std::mem::take(&mut out.shapes));
        }
        let prims = ctx.tessellate(last.unwrap_or_default(), ppp);
        let (w, h) = ((pw * ppp) as usize, (ph * ppp) as usize);
        let bg = ctx.style_of(ctx.theme()).visuals.window_fill;
        let bg = [bg.r(), bg.g(), bg.b()].map(|c| c as f32 / 255.0);
        let img = rasterize(&prims, &tex, w, h, ppp, bg);
        save_png(&img, w, h, std::path::Path::new(out)).expect("write the PNG");
        eprintln!("rendered {out}");
    }
}
