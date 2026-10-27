//! Plain RGB colour maths, free of any terminal types so every front end can use it.

pub type Rgb = (u8, u8, u8);

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

pub fn luminance(c: Rgb) -> f32 {
    (0.2126 * c.0 as f32 + 0.7152 * c.1 as f32 + 0.0722 * c.2 as f32) / 255.0
}

/// Nudge an accent colour until it is readable: against `surface` when known, otherwise
/// into a mid-brightness band that works on both dark and light terminals.
pub fn readable(mut c: Rgb, surface: Option<Rgb>) -> Rgb {
    const WHITE: Rgb = (255, 255, 255);
    const BLACK: Rgb = (0, 0, 0);
    match surface {
        Some(s) => {
            let toward = if luminance(s) < 0.5 { WHITE } else { BLACK };
            for _ in 0..10 {
                if (luminance(c) - luminance(s)).abs() >= 0.4 {
                    break;
                }
                c = mix(c, toward, 0.15);
            }
        }
        None => {
            for _ in 0..10 {
                if luminance(c) >= 0.4 {
                    break;
                }
                c = mix(c, WHITE, 0.12);
            }
            for _ in 0..10 {
                if luminance(c) <= 0.8 {
                    break;
                }
                c = mix(c, BLACK, 0.12);
            }
        }
    }
    c
}

/// Parse `#rrggbb`, `rrggbb`, `#rgb`, or `#rrggbbaa` (alpha dropped).
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    let h = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h.to_string(),
        8 => h[..6].to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}
