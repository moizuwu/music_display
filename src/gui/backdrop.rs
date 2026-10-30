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

fn rot(c: Pos2, p: Pos2, a: f32) -> Pos2 {
    let (s, co) = a.sin_cos();
    let d = p - c;
    Pos2::new(c.x + d.x * co - d.y * s, c.y + d.x * s + d.y * co)
}

/// Falling snow in two depths (small and slow behind, larger and quicker in front).
pub fn snow(p: &Painter, r: Rect, t: f64, color: Color32, alpha: f32) {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    for i in 0..110u32 {
        let near = i % 3 == 0;
        let size = if near { 2.2 + rnd(i, 1) * 2.2 } else { 0.9 + rnd(i, 1) * 1.3 };
        let speed = if near { 38.0 + rnd(i, 2) * 30.0 } else { 14.0 + rnd(i, 2) * 14.0 };
        let fall = (t as f32 * speed + rnd(i, 3) * h) % (h + 8.0);
        let x = r.left() + rnd(i, 4) * w + (t as f32 * (0.4 + rnd(i, 5)) + i as f32).sin() * if near { 22.0 } else { 9.0 };
        let twinkle = 0.75 + 0.25 * (t as f32 * 2.0 + i as f32 * 1.3).sin();
        p.circle_filled(Pos2::new(x, r.top() - 4.0 + fall), size, tone(color, alpha * if near { 0.85 } else { 0.55 } * twinkle));
    }
}

/// A string of coloured lights sagging along the top edge.
pub fn lights(p: &Painter, r: Rect, t: f64, alpha: f32) {
    let w = r.width().max(1.0);
    let sag = 14.0;
    let wire = |x: f32| r.top() + 5.0 + sag * (std::f32::consts::PI * (x - r.left()) / w).sin();
    let pts: Vec<Pos2> = (0..=48).map(|k| {
        let x = r.left() + w * k as f32 / 48.0;
        Pos2::new(x, wire(x))
    }).collect();
    p.add(Shape::line(pts, Stroke::new(1.5, tone(Color32::from_rgb(20, 60, 40), alpha))));
    const COLORS: [Color32; 4] = [
        Color32::from_rgb(230, 60, 60),
        Color32::from_rgb(250, 205, 70),
        Color32::from_rgb(70, 190, 100),
        Color32::from_rgb(80, 150, 240),
    ];
    let n = (w / 46.0).max(4.0) as u32;
    for k in 0..=n {
        let x = r.left() + w * k as f32 / n as f32;
        let lit = 0.55 + 0.45 * ((t as f32 * 1.6 + k as f32 * 0.9).sin() * 0.5 + 0.5);
        let c = COLORS[k as usize % COLORS.len()];
        let at = Pos2::new(x, wire(x) + 6.0);
        p.circle_filled(at, 9.0, tone(c, alpha * 0.16 * lit));
        p.circle_filled(at, 3.8, tone(c, alpha * lit));
    }
}

const EGG_COLORS: [Color32; 6] = [
    Color32::from_rgb(246, 168, 196),
    Color32::from_rgb(168, 214, 240),
    Color32::from_rgb(252, 226, 140),
    Color32::from_rgb(190, 232, 176),
    Color32::from_rgb(208, 184, 238),
    Color32::from_rgb(255, 196, 150),
];

/// Painted eggs drifting slowly down, with a row of grass at the bottom.
pub fn eggs(p: &Painter, r: Rect, t: f64, alpha: f32) {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    for i in 0..9u32 {
        let s = 16.0 + rnd(i, 1) * 16.0;
        let span = h + s * 3.0;
        let y = r.top() - s * 1.5 + (t as f32 * (7.0 + rnd(i, 2) * 9.0) + rnd(i, 3) * span) % span;
        let x = r.left() + w * (0.05 + rnd(i, 4) * 0.9) + (t as f32 * 0.35 + i as f32 * 2.0).sin() * 18.0;
        let c = EGG_COLORS[i as usize % EGG_COLORS.len()];
        let radius = egui::vec2(s * 0.74, s);
        p.add(Shape::ellipse_filled(Pos2::new(x, y), radius, tone(c, alpha * 0.55)));
        // two zigzag bands and a few dots, kept inside the egg's width at their height
        let band = tone(Color32::WHITE, alpha * 0.7);
        for dy in [-0.25f32, 0.35] {
            let half = radius.x * (1.0 - dy * dy).max(0.0).sqrt() * 0.92;
            let pts: Vec<Pos2> = (0..=8).map(|k| {
                let fx = -half + 2.0 * half * k as f32 / 8.0;
                Pos2::new(x + fx, y + s * dy + if k % 2 == 0 { -s * 0.07 } else { s * 0.07 })
            }).collect();
            p.add(Shape::line(pts, Stroke::new(s * 0.1, band)));
        }
        p.circle_filled(Pos2::new(x - s * 0.18, y + s * 0.62), s * 0.06, band);
        p.circle_filled(Pos2::new(x + s * 0.2, y - s * 0.62), s * 0.06, band);
    }
    grass(p, r, t, Color32::from_rgb(120, 190, 110), alpha * 0.8);
}

/// Blades of grass along the bottom edge, swaying in the wind.
fn grass(p: &Painter, r: Rect, t: f64, color: Color32, alpha: f32) {
    let step = 9.0;
    let n = (r.width() / step) as u32 + 1;
    for k in 0..n {
        let x = r.left() + k as f32 * step + rnd(k, 7) * 4.0;
        let hgt = 12.0 + rnd(k, 8) * 20.0;
        let lean = (t as f32 * 1.4 + k as f32 * 0.35).sin() * 5.0;
        let tip = Pos2::new(x + lean, r.bottom() - hgt);
        let shade = 0.75 + 0.25 * rnd(k, 9);
        p.add(Shape::convex_polygon(
            vec![Pos2::new(x - 2.5, r.bottom()), Pos2::new(x + 2.5, r.bottom()), tip],
            tone(Color32::from_rgb((color.r() as f32 * shade) as u8, (color.g() as f32 * shade) as u8, (color.b() as f32 * shade) as u8), alpha),
            Stroke::NONE,
        ));
    }
}

/// A warm sky: a low sun glow, pollen drifting up and daisies and cornflowers in the grass.
pub fn meadow(p: &Painter, r: Rect, t: f64, dark: bool, alpha: f32) {
    let (w, h) = (r.width().max(1.0), r.height().max(1.0));
    // the sky: from a pale blue at the top to a warm haze at the horizon
    let (top, bottom) = if dark { (Color32::from_rgb(10, 22, 40), Color32::from_rgb(34, 40, 30)) } else { (Color32::from_rgb(190, 224, 246), Color32::from_rgb(252, 244, 200)) };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), tone(top, alpha));
    mesh.colored_vertex(r.right_top(), tone(top, alpha));
    mesh.colored_vertex(r.right_bottom(), tone(bottom, alpha));
    mesh.colored_vertex(r.left_bottom(), tone(bottom, alpha));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
    // the sun, low on the right, breathing slowly
    let sun = Pos2::new(r.left() + w * 0.82, r.top() + h * 0.78);
    let breathe = 1.0 + 0.04 * (t as f32 * 0.5).sin();
    for j in 0..8 {
        let f = 1.0 - j as f32 / 8.0;
        p.circle_filled(sun, w.min(h) * 0.5 * f * breathe, tone(Color32::from_rgb(255, 214, 90), alpha * 0.07));
    }
    // pollen
    for i in 0..48u32 {
        let rise = (t as f32 * (6.0 + rnd(i, 1) * 10.0) + rnd(i, 2) * h) % h;
        let at = Pos2::new(r.left() + rnd(i, 3) * w + (t as f32 * 0.5 + i as f32).sin() * 16.0, r.bottom() - rise);
        let glint = 0.5 + 0.5 * (t as f32 * 1.7 + i as f32 * 2.1).sin();
        p.circle_filled(at, 1.0 + rnd(i, 4) * 1.6, tone(Color32::from_rgb(255, 226, 120), alpha * 0.6 * glint));
    }
    // flowers on stems along the bottom
    let n = (w / 64.0) as u32 + 1;
    for k in 0..n {
        let x = r.left() + (k as f32 + rnd(k, 11) * 0.6) * 64.0;
        let stem = 34.0 + rnd(k, 12) * 46.0;
        let lean = (t as f32 * 1.1 + k as f32 * 0.8).sin() * 4.0;
        let head = Pos2::new(x + lean, r.bottom() - stem);
        p.add(Shape::line(vec![Pos2::new(x, r.bottom()), Pos2::new(x + lean * 0.4, r.bottom() - stem * 0.5), head], Stroke::new(1.6, tone(Color32::from_rgb(70, 150, 80), alpha))));
        let petals = if k % 3 == 1 { Color32::from_rgb(0, 106, 167) } else if k % 3 == 2 { Color32::from_rgb(254, 204, 0) } else { Color32::from_rgb(252, 252, 246) };
        let (count, reach, size) = if k % 3 == 1 { (6, 5.5, 4.2) } else { (9, 7.0, 3.6) };
        for q in 0..count {
            let a = q as f32 / count as f32 * std::f32::consts::TAU + k as f32;
            p.circle_filled(head + egui::vec2(a.cos(), a.sin()) * reach, size, tone(petals, alpha * 0.95));
        }
        let heart = if k % 3 == 1 { Color32::from_rgb(40, 40, 120) } else { Color32::from_rgb(240, 170, 20) };
        p.circle_filled(head, 3.6, tone(heart, alpha));
    }
}

/// A small crooked decoration in the bottom right corner, drawn on a layer above the panels
/// that ignores the pointer.
pub fn figure(p: &Painter, r: Rect, alpha: f32) {
    let s = (r.width().min(r.height()) * 0.16).clamp(44.0, 120.0);
    let c = Pos2::new(r.right() - s * 1.35, r.bottom() - s * 0.35);
    let a = -0.21;
    let at = |x: f32, y: f32| rot(c, c + egui::vec2(x * s, y * s), a);
    let skin = tone(Color32::from_rgb(246, 206, 176), alpha);
    let white = tone(Color32::from_rgb(250, 250, 250), alpha);
    let red = tone(Color32::from_rgb(200, 30, 40), alpha);
    p.circle_filled(at(0.0, 0.45), s * 0.78, white);
    p.circle_filled(at(-0.5, 0.2), s * 0.4, white);
    p.circle_filled(at(0.5, 0.2), s * 0.4, white);
    p.circle_filled(at(0.0, -0.1), s * 0.62, skin);
    p.circle_filled(at(-0.22, -0.2), s * 0.06, tone(Color32::from_rgb(30, 30, 40), alpha));
    p.circle_filled(at(0.22, -0.2), s * 0.06, tone(Color32::from_rgb(30, 30, 40), alpha));
    p.circle_filled(at(0.0, -0.02), s * 0.11, tone(Color32::from_rgb(232, 120, 120), alpha));
    p.circle_filled(at(-0.2, 0.14), s * 0.2, white);
    p.circle_filled(at(0.2, 0.14), s * 0.2, white);
    p.add(Shape::convex_polygon(vec![at(-0.62, -0.4), at(0.62, -0.4), at(0.55, -1.25)], red, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![at(-0.7, -0.5), at(0.7, -0.5), at(0.7, -0.28), at(-0.7, -0.28)], white, Stroke::NONE));
    p.circle_filled(at(0.55, -1.3), s * 0.17, white);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `draw` in a headless pass and return how many shapes it painted.
    fn painted(draw: impl Fn(&Painter, Rect)) -> usize {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let r = Rect::from_min_size(Pos2::ZERO, egui::vec2(640.0, 400.0));
            draw(&ui.painter().clone(), r);
        });
        out.textures_delta.clear();
        out.shapes.len()
    }

    #[test]
    fn every_drawing_paints_something_at_any_time() {
        for t in [0.0, 1.3, 250.0] {
            assert!(painted(|p, r| embers(p, r, t, Color32::GOLD, 1.0)) > 10);
            assert!(painted(|p, r| fog(p, r, t, Color32::GRAY, 1.0)) > 10);
            assert!(painted(|p, r| bats(p, r, t, Color32::BLACK, 1.0)) > 5);
            assert!(painted(|p, r| snow(p, r, t, Color32::WHITE, 1.0)) > 50);
            assert!(painted(|p, r| lights(p, r, t, 1.0)) > 5);
            assert!(painted(|p, r| eggs(p, r, t, 1.0)) > 20);
            assert!(painted(|p, r| meadow(p, r, t, false, 1.0)) > 20);
            assert!(painted(|p, r| meadow(p, r, t, true, 1.0)) > 20);
            assert!(painted(|p, r| figure(p, r, 1.0)) > 5);
        }
        // a degenerate window must not panic either
        let _ = painted(|p, _| snow(p, Rect::from_min_size(Pos2::ZERO, egui::vec2(0.0, 0.0)), 1.0, Color32::WHITE, 1.0));
    }
}
