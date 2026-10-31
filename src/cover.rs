use crate::color::{Rgb, mix, readable};
use crate::util::{fnv1a, tmp_dir};
use image::DynamicImage;
use image::imageops::FilterType;

/// Largest cover download accepted.
const MAX_COVER: u64 = 20 * 1024 * 1024;

async fn bytes(c: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    if let Some(p) = crate::util::path_from_url(url) {
        return tokio::fs::read(p).await.ok();
    }
    if url.starts_with("http") {
        let path = tmp_dir().join(format!("cover-{:016x}", fnv1a(url)));
        if let Ok(b) = tokio::fs::read(&path).await {
            return Some(b);
        }
        let mut r = c.get(url).send().await.ok()?;
        if !r.status().is_success() || r.content_length().is_some_and(|n| n > MAX_COVER) {
            return None;
        }
        let mut b = Vec::new();
        while let Some(chunk) = r.chunk().await.ok()? {
            b.extend_from_slice(&chunk);
            if b.len() as u64 > MAX_COVER {
                return None;
            }
        }
        // Only a real image goes into the permanent cache (not a captive portal's HTML).
        image::guess_format(&b).ok()?;
        let _ = crate::util::write_safely(&path, &b);
        return Some(b);
    }
    // bare path
    tokio::fs::read(url).await.ok()
}

/// Luma → lerp(surface, primary), blended with the original by `strength`.
fn tint(img: DynamicImage, surface: Rgb, primary: Rgb, strength: f32) -> DynamicImage {
    let mut rgb = img.to_rgb8();
    for p in rgb.pixels_mut() {
        let [r, g, b] = p.0;
        let l = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
        let duo = mix(surface, primary, l);
        let out = mix((r, g, b), duo, strength);
        p.0 = [out.0, out.1, out.2];
    }
    DynamicImage::ImageRgb8(rgb)
}

/// (hue 0..360, saturation 0..1, value 0..1)
fn hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// The cover's dominant *vivid* colour: pixels vote into 12 hue buckets weighted by
/// saturation × brightness, so a big dark/gray background doesn't win over the artwork's
/// real accent. Grayscale covers fall back to the average of their lit pixels.
pub fn dominant_accent(img: &DynamicImage) -> Option<Rgb> {
    let small = img.thumbnail(48, 48).to_rgb8();
    let n = (small.width() * small.height()) as f32;
    // per bucket: weight, Σr·w, Σg·w, Σb·w
    let mut buckets = [[0f32; 4]; 12];
    let mut lit = [0f32; 4]; // count, Σr, Σg, Σb of non-black pixels
    for p in small.pixels() {
        let [r, g, b] = p.0;
        let (h, s, v) = hsv(r, g, b);
        if v >= 0.2 {
            lit[0] += 1.0;
            lit[1] += r as f32;
            lit[2] += g as f32;
            lit[3] += b as f32;
        }
        if v < 0.2 || s < 0.2 {
            continue;
        }
        let w = s * v;
        let bucket = &mut buckets[((h / 30.0) as usize) % 12];
        bucket[0] += w;
        bucket[1] += r as f32 * w;
        bucket[2] += g as f32 * w;
        bucket[3] += b as f32 * w;
    }
    let best = buckets.iter().max_by(|a, b| a[0].total_cmp(&b[0]))?;
    let (w, r, g, b) = if best[0] >= 0.02 * n {
        (best[0], best[1], best[2], best[3])
    } else if lit[0] > 0.0 {
        (lit[0], lit[1], lit[2], lit[3])
    } else {
        return None;
    };
    Some((
        (r / w).round() as u8,
        (g / w).round() as u8,
        (b / w).round() as u8,
    ))
}

/// How the loader should process the decoded cover.
#[derive(Clone, Copy)]
pub struct Look {
    pub surface: Option<Rgb>,
    pub primary: Option<Rgb>,
    /// 0 = leave the artwork untouched.
    pub strength: f32,
    /// Extract an accent from the artwork (also becomes the duotone's highlight).
    pub cover_theme: bool,
}

pub struct Loaded {
    pub image: DynamicImage,
    /// Readable accent derived from the artwork (only with `--cover-theme`).
    pub accent: Option<Rgb>,
}

pub async fn load(c: reqwest::Client, url: String, look: Look) -> Option<Loaded> {
    let data = bytes(&c, &url).await?;
    // Decoding/resizing is CPU work — keep it off the async workers.
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&data).ok()?;
        let img = if img.width() > 640 || img.height() > 640 {
            img.thumbnail(640, 640)
        } else {
            img
        };
        // Always measured on the untouched artwork, never on the tinted copy.
        let accent = look
            .cover_theme
            .then(|| dominant_accent(&img))
            .flatten()
            .map(|a| readable(a, look.surface));
        // A duotone needs both ends; with terminal colours (unknown RGB) it is skipped.
        let image = match (look.surface, accent.or(look.primary)) {
            (Some(s), Some(p)) if look.strength > 0.0 => {
                tint(img, s, p, look.strength.clamp(0.0, 1.0))
            }
            _ => img,
        };
        Some(Loaded { image, accent })
    })
    .await
    .ok()
    .flatten()
}

// ── ASCII rendering ───────────────────────────────────────────────────────
/// Dark → light density ramp.
const RAMP: &[u8] = b" .:-=+*#%@";

/// One coloured character per terminal cell.
pub struct AsciiArt {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<(char, Rgb)>,
}

/// Keep dark pixels' glyphs visible: scale their colour up to at least ~110 brightness.
fn lift(c: Rgb) -> Rgb {
    let m = c.0.max(c.1).max(c.2);
    match m {
        0 => (110, 110, 110),
        110..=255 => c,
        _ => {
            let k = 110.0 / m as f32;
            let f = |x: u8| (x as f32 * k).min(255.0) as u8;
            (f(c.0), f(c.1), f(c.2))
        }
    }
}

/// Resample to exactly `cols`×`rows` cells (the caller picks dimensions that already
/// account for the ~1:2 cell aspect) and map brightness to a character, colour to its fg.
pub fn ascii_art(img: &DynamicImage, cols: u16, rows: u16) -> AsciiArt {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let small = img
        .resize_exact(cols as u32, rows as u32, FilterType::Triangle)
        .to_rgb8();
    let lum = |p: &image::Rgb<u8>| {
        (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
    };
    let (mut lo, mut hi) = (1.0f32, 0.0f32);
    for p in small.pixels() {
        let l = lum(p);
        lo = lo.min(l);
        hi = hi.max(l);
    }
    let span = (hi - lo).max(0.05); // stretch contrast so flat covers still show detail
    let cells = small
        .pixels()
        .map(|p| {
            let l = ((lum(p) - lo) / span).clamp(0.0, 1.0);
            let ch = RAMP[(l * (RAMP.len() - 1) as f32).round() as usize] as char;
            (ch, lift((p[0], p[1], p[2])))
        })
        .collect();
    AsciiArt { cols, rows, cells }
}

#[cfg(test)]
mod sniff_tests {
    #[test]
    fn html_is_not_an_image() {
        assert!(image::guess_format(b"<html>sign in to the wifi</html>").is_err());
        assert!(image::guess_format(b"\x89PNG\r\n\x1a\n").is_ok());
    }
}
