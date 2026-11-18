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
    use super::super::sp_nav::Page;

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
    /// `out-spotify.png` with the experimental Spotify layout and `out-spotify-preset.png` with the
    /// built-in Spotify preset.
    #[tokio::test(flavor = "multi_thread")]
    async fn render_main_view() {
        let Ok(out) = std::env::var("MDIS_RENDER") else { return };
        music_display::util::enter_test_mode();
        let (m, rx) = fake_model(false);
        render(m, rx, &out);
        // the Spotify layout: its home page, and a page of each kind
        for (name, page) in [
            ("", Page::Home),
            ("-album", Page::Album("spotify:album:night".into())),
            ("-artist", Page::Artist("spotify:artist:band".into())),
            ("-playlist", Page::Playlist("spotify:playlist:road".into())),
            ("-search", Page::Search("night".into())),
            ("-queue", Page::Queue),
            ("-lyrics", Page::Lyrics),
        ] {
            let (mut m, rx) = fake_model(true);
            m.spot.pg.go(page);
            render(m, rx, &out.replace(".png", &format!("-spotify{name}.png")));
        }
        // the built-in Spotify preset, applied the way the Presets section applies it
        let (mut m, rx) = fake_model(false);
        let style = music_display::presets::built_in("Spotify").expect("built-in preset");
        (m.gui, m.layout) = (style.gui, style.layout);
        render(m, rx, &out.replace(".png", "-spotify-preset.png"));
    }

    /// A picture made up for a fake URL: a diagonal gradient of its own colour.
    fn fake_art(seed: u32) -> image::RgbaImage {
        let hue = (seed as f32 * 0.17).fract();
        let rgb = |h: f32, k: f32| {
            let f = |o: f32| ((((h + o).fract() * std::f32::consts::TAU).cos() * 0.5 + 0.5) * 160.0 * k + 40.0) as u8;
            image::Rgba([f(0.0), f(0.33), f(0.66), 255])
        };
        let mut img = image::RgbaImage::new(160, 160);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let k = 0.5 + 0.5 * ((x + y) as f32 / 320.0);
            *p = rgb(hue, k);
        }
        img
    }

    /// Fake pages and library rows for the Spotify layout (no network: everything is in place
    /// and fresh, so nothing is asked for).
    fn fake_spotify(m: &mut super::super::model::Model) {
        use music_display::spotify::{Head, Item, PageData, Section};
        let now = std::time::Instant::now();
        for n in 0..12u32 {
            m.spot.pg.images.insert(&format!("fake:{n}"), fake_art(n));
        }
        let img = |n: u32| format!("fake:{n}");
        let tr = |n: u32, name: &str, artist: &str, album: &str, secs: f64, art: u32| Item {
            kind: "track".into(),
            uri: if n == 0 { "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into() } else { format!("spotify:track:{n}") },
            name: name.into(),
            sub: artist.into(),
            duration: secs,
            image: img(art),
            album: album.into(),
            album_uri: "spotify:album:night".into(),
            artist_uri: "spotify:artist:band".into(),
            ..Default::default()
        };
        let card = |kind: &str, name: &str, sub: &str, art: u32| Item {
            kind: kind.into(),
            uri: format!("spotify:{kind}:{name}"),
            name: name.into(),
            sub: sub.into(),
            openable: true,
            image: img(art),
            ..Default::default()
        };
        let songs = |album: &str| {
            vec![
                tr(0, "City Lights", "The Example Band", album, 241.0, 1),
                tr(1, "Morning Drive", "The Example Band, Lena Park", album, 198.0, 1),
                tr(2, "Radio Low", "The Example Band", album, 214.0, 1),
                tr(3, "Windows Down", "The Example Band", album, 187.0, 1),
                tr(4, "Hold On to the Night", "The Example Band", album, 262.0, 1),
                tr(5, "Every Sign Is Pointing Home", "The Example Band", album, 233.0, 1),
                tr(6, "Nothing Left to Say", "The Example Band", album, 176.0, 1),
                tr(7, "Streetlights", "The Example Band", album, 305.0, 1),
            ]
        };
        let page = |m: &mut super::super::model::Model, key: &str, d: PageData| m.spot.pg.on_page(key, Ok(Box::new(d)));
        // the album
        let mut d = PageData {
            head: Head { kind: "Album".into(), title: "Night Drive".into(), by: "The Example Band".into(), by_uri: "spotify:artist:band".into(), image: img(1), year: "2021".into(), count: 8, ..Default::default() },
            tracks: songs("Night Drive"),
            ..Default::default()
        };
        page(m, "spotify:album:night", d.clone());
        // a playlist
        d.head = Head { kind: "Playlist".into(), title: "Road trip".into(), by: "Lena Park".into(), image: img(4), count: 8, text: "Windows down, volume up".into(), ..Default::default() };
        for t in &mut d.tracks {
            t.album = ["Night Drive", "Neon Roads", "Blue Hour"][t.name.len() % 3].into();
        }
        page(m, "spotify:playlist:road", d);
        // the artist
        let albums: Vec<Item> = ["Night Drive", "Neon Roads", "Blue Hour", "Static", "First Light"].iter().enumerate().map(|(i, n)| card("album", n, &format!("{}", 2021 - i), 1 + i as u32)).collect();
        let singles: Vec<Item> = ["City Lights", "Radio Low", "Windows Down"].iter().enumerate().map(|(i, n)| card("album", n, &format!("{}", 2022 - i), 6 + i as u32)).collect();
        page(
            m,
            "spotify:artist:band",
            PageData {
                head: Head { kind: "Artist".into(), title: "The Example Band".into(), image: img(1), count: 1_234_567, text: "synthpop, indie pop".into(), ..Default::default() },
                tracks: songs("Night Drive"),
                sections: vec![
                    Section { title: "Albums".into(), items: albums.clone(), error: String::new() },
                    Section { title: "Singles and EPs".into(), items: singles, error: String::new() },
                ],
                ..Default::default()
            },
        );
        // search
        page(
            m,
            "search:night",
            PageData {
                tracks: songs("Night Drive"),
                sections: vec![
                    Section { title: "Artists".into(), items: vec![card("artist", "The Example Band", "Artist", 1), card("artist", "Night Owls", "Artist", 7), card("artist", "Lena Park", "Artist", 2)], error: String::new() },
                    Section { title: "Albums".into(), items: albums, error: String::new() },
                    Section { title: "Playlists".into(), items: vec![card("playlist", "Night Drive", "by Spotify", 3), card("playlist", "Late night", "by Lena", 5)], error: String::new() },
                ],
                ..Default::default()
            },
        );
        // home: top artists and songs
        page(
            m,
            "home",
            PageData {
                tracks: songs("Night Drive"),
                sections: vec![Section {
                    title: "Your top artists".into(),
                    items: vec![card("artist", "The Example Band", "Artist", 1), card("artist", "Lena Park", "Artist", 2), card("artist", "Night Owls", "Artist", 7), card("artist", "Static", "Artist", 8), card("artist", "Blue Hour", "Artist", 9)],
                    error: String::new(),
                }],
                ..Default::default()
            },
        );
        let fresh = |t: &mut crate::browser::Level, items: Vec<Item>| {
            t.items = items;
            t.loaded = true;
            t.fetched = Some(now);
        };
        let tabs = &mut m.spot.lib.tabs;
        fresh(&mut tabs[0], vec![card("album", "Night Drive", "Album", 1), card("playlist", "Road trip", "Playlist", 4), card("artist", "Lena Park", "Artist", 2), card("album", "Neon Roads", "Album", 2), card("playlist", "Late night", "Playlist", 5), card("artist", "Night Owls", "Artist", 7)]);
        fresh(&mut tabs[1], vec![card("playlist", "Road trip", "42 songs", 4), card("playlist", "Late night", "by Lena", 5), card("playlist", "Focus", "128 songs", 3), card("playlist", "Sunday morning", "17 songs", 6), card("playlist", "Workout", "63 songs", 8)]);
        fresh(&mut tabs[3], vec![card("album", "Night Drive", "The Example Band", 1), card("album", "Neon Roads", "Lena Park", 2), card("album", "Blue Hour", "Night Owls", 9)]);
        fresh(&mut tabs[4], vec![card("artist", "The Example Band", "Artist", 1), card("artist", "Lena Park", "Artist", 2), card("artist", "Night Owls", "Artist", 7)]);
        // the queue: the song playing and what is next
        let q = &mut tabs[crate::browser::TAB_QUEUE];
        let mut items = songs("Night Drive");
        items.truncate(6);
        fresh(q, items);
        q.heads = vec![(0, "Now playing"), (1, "Next up")];
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
        let mk = |kind: &str, name: &str, sub: &str| music_display::spotify::Item { kind: kind.into(), uri: format!("spotify:{kind}:{name}"), name: name.into(), sub: sub.into(), openable: kind != "track", duration: if kind == "track" { 214.0 } else { 0.0 }, ..Default::default() };
        let tab = &mut m.spot.lib.tabs[0];
        tab.items = vec![mk("track", "City Lights", "The Example Band"), mk("track", "Morning Drive", "Lena Park"), mk("album", "Night Drive", "The Example Band"), mk("playlist", "Road trip", "Playlist"), mk("artist", "Lena Park", "Artist")];
        tab.loaded = true;
        tab.fetched = Some(std::time::Instant::now());
        if spotify_layout {
            fake_spotify(&mut m);
        }
        (m, rx)
    }

    /// Run the window for a few frames at 1100 × 680 points, 2 pixels per point, and save it.
    fn render(m: super::super::model::Model, rx: tokio::sync::mpsc::UnboundedReceiver<music_display::AppEvent>, out: &str) {
        render_with(m, rx, out, 6, &|_, _, _| {});
    }

    /// [`render`] for `frames` frames; `script(frame, ctx, input)` may add events to a frame (a
    /// click) or set up state before it (a spinning record).
    fn render_with(
        m: super::super::model::Model,
        rx: tokio::sync::mpsc::UnboundedReceiver<music_display::AppEvent>,
        out: &str,
        frames: usize,
        script: &dyn Fn(usize, &egui::Context, &mut egui::RawInput),
    ) {
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
        for i in 0..frames {
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pw, ph))),
                time: Some(i as f64 * 0.5),
                ..Default::default()
            };
            script(i, &ctx, &mut raw);
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

    /// The extras of the Spotify layout, each in its own picture next to the main one: the device
    /// popup (`-devices.png`), a spinning record with the glow and confetti (`-fun.png`), and the
    /// right-click menu of a library row (`-menu.png`).
    #[tokio::test(flavor = "multi_thread")]
    async fn render_main_view_extras() {
        let Ok(out) = std::env::var("MDIS_RENDER") else { return };
        music_display::util::enter_test_mode();
        let name = |suffix: &str| out.replace(".png", &format!("-{suffix}.png"));
        // the device popup, open above the player bar
        let (mut m, rx) = fake_model(true);
        let dev = |id: &str, name: &str, kind: &str, active: bool, local: bool| music_display::spotify::Device { id: id.into(), name: name.into(), kind: kind.into(), active, restricted: false, local };
        m.spot.devices = Some(crate::browser::DevicePicker {
            items: vec![dev("a", "Living room", "Speaker", false, false), dev("b", "Studio PC", "Computer", true, true), dev("c", "Pixel", "Smartphone", false, false)],
            ..Default::default()
        });
        render_with(m, rx, &name("devices"), 6, &|_, _, _| {});
        // the record, the glow and the confetti
        let (mut m, rx) = fake_model(true);
        m.gui.vinyl = true;
        m.gui.confetti = true;
        m.gui.beat_pulse = true;
        m.x.cava_bars = vec![700, 640, 520, 380, 300, 220, 160, 120];
        render_with(m, rx, &name("fun"), 6, &|i, ctx, _| {
            super::super::fun::set_angle(ctx, 0.8);
            super::super::fun::set_pulse(ctx, 0.8);
            if i == 5 {
                super::super::fun::burst(ctx, egui::pos2(300.0, 600.0));
            }
        });
        // the right-click menu of the first row of the library
        let (m, rx) = fake_model(true);
        render_with(m, rx, &name("menu"), 6, &|i, _, raw| {
            let at = egui::pos2(165.0, 245.0);
            if i >= 2 {
                raw.events.push(egui::Event::PointerMoved(at));
            }
            if i == 3 {
                raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
            }
            if i == 4 {
                raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
            }
        });
    }
}
