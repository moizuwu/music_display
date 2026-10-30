//! Backgrounds that move: rising embers, drifting fog and bats. Everything is drawn with the
//! painter from the clock, so there are no files and no state; `alpha` is the window opacity.
//! Nothing here has been looked at on a real compositor.
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

/// A stable pseudo-random number in 0..1 for `(i, k)`.
fn rnd(i: u32, k: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn tone(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Small glowing specks that rise and fade out near the top.
pub fn embers(p: &Painter, r: Rect, t: f64, color: Color32, alpha: f32) {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    for i in 0..70u32 {
        let speed = 14.0 + rnd(i, 1) * 34.0;
        let rise = (t as f32 * speed + rnd(i, 2) * h) % h;
        let y = r.bottom() - rise;
        let x = r.left() + rnd(i, 3) * w + (t as f32 * 0.6 + i as f32).sin() * 14.0;
        let life = 1.0 - rise / h;
        let size = 0.8 + rnd(i, 4) * 2.4;
        let flicker = 0.6 + 0.4 * (t as f32 * (2.0 + rnd(i, 5) * 3.0) + i as f32).sin();
        p.circle_filled(Pos2::new(x, y), size * 2.6, tone(color, alpha * life * 0.12 * flicker));
        p.circle_filled(Pos2::new(x, y), size, tone(color, alpha * life * 0.75 * flicker));
    }
}

/// Large soft blobs that wander slowly; each is a few stacked discs, fainter toward the edge.
pub fn fog(p: &Painter, r: Rect, t: f64, color: Color32, alpha: f32) {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    let reach = w.max(h);
    for i in 0..7u32 {
        let (sx, sy) = (0.02 + rnd(i, 1) * 0.05, 0.02 + rnd(i, 2) * 0.05);
        let phase = rnd(i, 3) * std::f32::consts::TAU;
        let c = Pos2::new(
            r.left() + w * (0.5 + 0.55 * (t as f32 * sx + phase).sin()),
            r.top() + h * (0.5 + 0.45 * (t as f32 * sy + phase * 1.7).cos()),
        );
        let radius = reach * (0.22 + rnd(i, 4) * 0.2);
        for j in 0..6 {
            let f = 1.0 - j as f32 / 6.0;
            p.circle_filled(c, radius * f, tone(color, alpha * 0.05));
        }
    }
}

/// Bats crossing the window, wings flapping.
pub fn bats(p: &Painter, r: Rect, t: f64, color: Color32, alpha: f32) {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    for i in 0..6u32 {
        let size = 9.0 + rnd(i, 1) * 13.0;
        let speed = 38.0 + rnd(i, 2) * 55.0;
        let span = w + size * 8.0;
        let dir = if rnd(i, 3) > 0.5 { 1.0 } else { -1.0 };
        let run = (t as f32 * speed + rnd(i, 4) * span) % span;
        let x = if dir > 0.0 { r.left() - size * 4.0 + run } else { r.right() + size * 4.0 - run };
        let y = r.top() + h * (0.08 + rnd(i, 5) * 0.55) + (t as f32 * 1.3 + i as f32 * 2.0).sin() * 20.0;
        bat(p, Pos2::new(x, y), size, (t as f32 * 9.0 + i as f32 * 1.7).sin(), tone(color, alpha * 0.85));
    }
}

/// One bat: body, ears and two wings (`flap` -1..1 lifts or drops the wing tips).
fn bat(p: &Painter, c: Pos2, s: f32, flap: f32, col: Color32) {
    let tip = -s * 0.7 * flap;
    for side in [-1.0f32, 1.0] {
        let wing = vec![
            Pos2::new(c.x + side * s * 0.2, c.y - s * 0.1),
            Pos2::new(c.x + side * s * 1.7, c.y + tip - s * 0.25),
            Pos2::new(c.x + side * s * 1.0, c.y + tip * 0.4 + s * 0.35),
            Pos2::new(c.x + side * s * 0.2, c.y + s * 0.4),
        ];
        p.add(Shape::convex_polygon(wing, col, Stroke::NONE));
        let ear = vec![
            Pos2::new(c.x + side * s * 0.1, c.y - s * 0.4),
            Pos2::new(c.x + side * s * 0.3, c.y - s * 0.85),
            Pos2::new(c.x + side * s * 0.4, c.y - s * 0.35),
        ];
        p.add(Shape::convex_polygon(ear, col, Stroke::NONE));
    }
    p.circle_filled(c, s * 0.38, col);
}
