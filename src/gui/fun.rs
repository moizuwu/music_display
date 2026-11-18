//! The silly extras, each behind its own `[gui]` switch and off by default: `vinyl` (the cover
//! spins like a record while the song plays), `confetti` (liking a song bursts hearts from the
//! button) and `beat_pulse` (the play button glows with the cava level). The motion is plain
//! functions of time, so it is tested; the window is only asked to redraw while something moves.
use super::model::Model;
use egui::{Color32, Context, Id, LayerId, Mesh, Order, Pos2, Rect, Shape, Stroke, TextureHandle, Ui, Vec2};
use music_display::player::Status;
use std::f32::consts::{PI, TAU};
use std::time::Duration;

/// How often an animation asks for the next frame.
const FRAME: Duration = Duration::from_millis(33);

// ── vinyl ───────────────────────────────────────────────────────────────────

/// Radians per second: a turn in about seven seconds, slow like a record on a quiet evening.
const SPIN: f32 = 0.9;

/// The angle after `dt` seconds: it moves while the song plays and stands still otherwise. A long
/// gap (the window was not drawn for a while) counts as one frame, so the record does not jump.
pub fn spun(angle: f32, dt: f32, playing: bool) -> f32 {
    if playing { (angle + dt.clamp(0.0, 0.1) * SPIN) % TAU } else { angle }
}

#[derive(Clone, Copy, Default)]
struct Spin {
    angle: f32,
    at: f64,
}

/// The angle of the record this frame, `None` with `vinyl` off. Asking twice in one frame (the big
/// cover and the one in the player bar) gives the same angle.
pub fn vinyl_angle(ctx: &Context, m: &Model) -> Option<f32> {
    if !m.gui.vinyl {
        return None;
    }
    let playing = m.snap.as_ref().is_some_and(|s| s.status == Status::Playing);
    let (now, id) = (ctx.input(|i| i.time), Id::unique("fun-spin"));
    let mut s: Spin = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if now > s.at {
        s.angle = spun(s.angle, if s.at == 0.0 { 0.0 } else { (now - s.at) as f32 }, playing);
        s.at = now;
        ctx.data_mut(|d| d.insert_temp(id, s));
    }
    if playing {
        ctx.request_repaint_after(FRAME);
    }
    Some(s.angle)
}

/// For the render check: turn the record to `angle`.
#[cfg(test)]
pub fn set_angle(ctx: &Context, angle: f32) {
    ctx.data_mut(|d| d.insert_temp(Id::unique("fun-spin"), Spin { angle, at: f64::MAX }));
}

/// The points of the rim of a record turned by `angle`: where on the circle each is, and where in
/// the cover picture it takes its colour from (the picture turns, the outline does not).
pub fn rim(n: usize, angle: f32) -> Vec<(Vec2, Vec2)> {
    (0..n)
        .map(|i| {
            let a = TAU * i as f32 / n as f32;
            (Vec2::angled(a), Vec2::new(0.5, 0.5) + 0.5 * Vec2::angled(a - angle))
        })
        .collect()
}

/// A square cover, or a round one that turns when `angle` is given: the picture clipped to a circle
/// with a small hole in the middle and a groove ring, like a record.
pub fn cover_image(ui: &mut Ui, tex: &TextureHandle, side: f32, radius: egui::CornerRadius, angle: Option<f32>) {
    let Some(angle) = angle else {
        ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(side)).corner_radius(radius));
        return;
    };
    let (r, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
    let (c, rad) = (r.center(), side / 2.0);
    let mut mesh = Mesh::with_texture(tex.id());
    mesh.vertices.push(egui::epaint::Vertex { pos: c, uv: Pos2::new(0.5, 0.5), color: Color32::WHITE });
    let ring = rim(96, angle);
    for (dir, uv) in &ring {
        mesh.vertices.push(egui::epaint::Vertex { pos: c + *dir * rad, uv: uv.to_pos2(), color: Color32::WHITE });
    }
    for i in 0..ring.len() as u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % ring.len() as u32);
    }
    let p = ui.painter();
    p.add(Shape::mesh(mesh));
    let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
    p.circle_stroke(c, rad, Stroke::new(1.0, line));
    p.circle_stroke(c, rad * 0.62, Stroke::new(0.8, Color32::from_black_alpha(46)));
    p.circle_stroke(c, rad * 0.8, Stroke::new(0.8, Color32::from_black_alpha(30)));
    let hole = (rad * 0.07).max(2.0);
    p.circle_filled(c, hole * 1.9, Color32::from_black_alpha(90));
    p.circle_filled(c, hole, ui.visuals().extreme_bg_color);
}

// ── confetti ────────────────────────────────────────────────────────────────

/// How long a burst lasts, in seconds.
pub const LIFE: f32 = 0.9;
/// Pieces in a burst.
const PIECES: usize = 26;
/// Pixels per second squared.
const GRAVITY: f32 = 320.0;
const COLORS: [Color32; 6] = [
    Color32::from_rgb(255, 99, 146),
    Color32::from_rgb(255, 196, 61),
    Color32::from_rgb(64, 201, 185),
    Color32::from_rgb(255, 140, 66),
    Color32::from_rgb(160, 120, 255),
    Color32::from_rgb(120, 200, 90),
];

fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// One piece of a burst `t` seconds after it started: where it is relative to the heart button,
/// how opaque (1 → 0 over [`LIFE`]), how big, and its colour. Every piece is a plain function of
/// its number and the time, so nothing needs to be stored per piece.
pub fn piece(i: usize, t: f32) -> (Vec2, f32, f32, Color32) {
    let n = i as f32;
    // fanned out over the upper half, never the same twice
    let dir = -PI / 2.0 + (frac(n * 0.618_034) - 0.5) * PI * 1.5;
    let speed = 90.0 + 130.0 * frac(n * 0.371);
    let pos = Vec2::angled(dir) * speed * t + Vec2::new(0.0, 0.5 * GRAVITY * t * t);
    let alpha = (1.0 - (t / LIFE).clamp(0.0, 1.0).powi(2)).max(0.0);
    (pos, alpha, 3.0 + 3.0 * frac(n * 0.73), COLORS[i % COLORS.len()])
}

#[derive(Clone, Copy)]
struct Burst {
    origin: Pos2,
    at: f64,
}

fn bursts_id() -> Id {
    Id::unique("fun-confetti")
}

/// Start a burst at `origin` (the heart button's centre).
pub fn burst(ctx: &Context, origin: Pos2) {
    let at = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Burst>>(bursts_id()).push(Burst { origin, at }));
    ctx.request_repaint();
}

/// Draw the bursts that are still going, over everything (a foreground layer), and ask for the
/// next frame only while there are some. Called once per frame; costs nothing with none.
pub fn paint_confetti(ctx: &Context) {
    let now = ctx.input(|i| i.time);
    let mut list: Vec<Burst> = ctx.data(|d| d.get_temp(bursts_id())).unwrap_or_default();
    if list.is_empty() {
        return;
    }
    list.retain(|b| (now - b.at) < LIFE as f64);
    ctx.data_mut(|d| d.insert_temp(bursts_id(), list.clone()));
    if list.is_empty() {
        return;
    }
    let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("fun-confetti-layer")));
    for b in &list {
        let t = (now - b.at) as f32;
        for i in 0..PIECES {
            let (off, alpha, size, color) = piece(i, t);
            let color = color.gamma_multiply(alpha);
            let at = b.origin + off;
            match i % 3 {
                0 => heart(&p, at, size * 1.6, color),
                1 => {
                    p.circle_filled(at, size * 0.5, color);
                }
                _ => {
                    // a small strip, turning as it falls
                    let a = t * (4.0 + i as f32 * 0.3) + i as f32;
                    let (u, v) = (Vec2::angled(a) * size, Vec2::angled(a + PI / 2.0) * size * 0.35);
                    p.add(Shape::convex_polygon(vec![at + u + v, at + u - v, at - u - v, at - u + v], color, Stroke::NONE));
                }
            }
        }
    }
    ctx.request_repaint_after(FRAME);
}

/// A small heart: two round lobes and a point.
fn heart(p: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    p.circle_filled(c + Vec2::new(-s * 0.25, -s * 0.2), s * 0.28, color);
    p.circle_filled(c + Vec2::new(s * 0.25, -s * 0.2), s * 0.28, color);
    p.add(Shape::convex_polygon(vec![c + Vec2::new(-s * 0.5, -s * 0.08), c + Vec2::new(s * 0.5, -s * 0.08), c + Vec2::new(0.0, s * 0.55)], color, Stroke::NONE));
}

// ── beat pulse ──────────────────────────────────────────────────────────────

/// How loud the music is right now, 0..=1, from cava's bars (the lower half, where the beat is,
/// boosted a little because cava rarely reaches its top).
pub fn level(bars: &[u16]) -> f32 {
    let low = &bars[..bars.len().div_ceil(2)];
    if low.is_empty() {
        return 0.0;
    }
    let mean = low.iter().map(|b| *b as f32).sum::<f32>() / low.len() as f32;
    (mean / music_display::cava::MAX as f32 * 1.8).clamp(0.0, 1.0)
}

/// The glow follows a rising level at once and falls back slowly, so it pulses instead of flickering.
pub fn smoothed(prev: f32, target: f32, dt: f32) -> f32 {
    let rate = if target > prev { 30.0 } else { 5.0 };
    (prev + (target - prev) * (1.0 - (-rate * dt.clamp(0.0, 0.2)).exp())).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Default)]
struct Pulse {
    level: f32,
    at: f64,
}

/// The glow strength this frame (0 with `beat_pulse` off, no cava bars, or nothing playing). Asks
/// for another frame only while it is lit.
pub fn pulse(ctx: &Context, m: &Model) -> f32 {
    if !m.gui.beat_pulse {
        return 0.0;
    }
    let playing = m.snap.as_ref().is_some_and(|s| s.status == Status::Playing);
    let target = if playing { level(&m.x.cava_bars) } else { 0.0 };
    let (now, id) = (ctx.input(|i| i.time), Id::unique("fun-pulse"));
    let mut s: Pulse = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if now > s.at {
        s.level = smoothed(s.level, target, if s.at == 0.0 { 0.0 } else { (now - s.at) as f32 });
        s.at = now;
        ctx.data_mut(|d| d.insert_temp(id, s));
    }
    if s.level > 0.01 {
        ctx.request_repaint_after(FRAME);
    }
    s.level
}

/// For the render check: light the glow at `level`.
#[cfg(test)]
pub fn set_pulse(ctx: &Context, level: f32) {
    ctx.data_mut(|d| d.insert_temp(Id::unique("fun-pulse"), Pulse { level, at: f64::MAX }));
}

/// Where a glow goes: reserve a slot in the painter before a button is drawn, then fill it with
/// [`Slot::fill`], so the glow sits behind the button.
pub struct Slot(egui::layers::ShapeIdx);

pub fn slot(ui: &Ui) -> Slot {
    Slot(ui.painter().add(Shape::Noop))
}

impl Slot {
    /// A soft disc in the accent colour around `button`, as strong as `level`.
    pub fn fill(self, ui: &Ui, button: Rect, level: f32) {
        if level <= 0.01 {
            return;
        }
        let acc = super::view::accent(ui);
        let r = button.width() / 2.0 * (1.0 + 0.55 * level);
        let shapes = vec![
            Shape::circle_filled(button.center(), r, acc.gamma_multiply(0.10 + 0.22 * level)),
            Shape::circle_filled(button.center(), button.width() / 2.0 * (1.0 + 0.2 * level), acc.gamma_multiply(0.16 + 0.3 * level)),
        ];
        ui.painter().set(self.0, Shape::Vec(shapes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_turns_only_while_playing_and_never_jumps() {
        assert_eq!(spun(1.0, 0.5, false), 1.0);
        assert!(spun(1.0, 0.033, true) > 1.0);
        // a gap of ten seconds is one frame, not a spin
        assert!((spun(0.0, 10.0, true) - 0.1 * SPIN).abs() < 1e-6);
        // the angle stays within one turn
        assert!(spun(TAU - 0.01, 0.1, true) < 0.2);
        assert_eq!(spun(1.0, -3.0, true), 1.0, "time never runs backwards");
    }

    #[test]
    fn the_picture_turns_inside_a_fixed_outline() {
        let still = rim(8, 0.0);
        let turned = rim(8, PI / 2.0);
        assert!((still[0].0 - turned[0].0).length() < 1e-6, "the circle stays where it is");
        assert!((still[0].1 - turned[0].1).length() > 0.1, "the colours move");
        // every sample stays inside the picture
        for (_, uv) in rim(64, 1.234) {
            assert!((0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y));
        }
    }

    #[test]
    fn confetti_starts_at_the_button_and_fades_out() {
        for i in 0..PIECES {
            let (off, alpha, size, _) = piece(i, 0.0);
            assert_eq!((off, alpha), (Vec2::ZERO, 1.0));
            assert!(size > 0.0);
            let mut last = 1.0;
            for k in 1..=10 {
                let a = piece(i, LIFE * k as f32 / 10.0).1;
                assert!(a <= last, "the piece only gets fainter");
                last = a;
            }
            assert_eq!(piece(i, LIFE).1, 0.0);
            assert_eq!(piece(i, LIFE * 2.0).1, 0.0);
        }
        // the burst goes up first, and the pieces differ
        assert!(piece(1, 0.15).0.y < 0.0);
        assert_ne!(piece(1, 0.3).0, piece(2, 0.3).0);
        assert_eq!(piece(5, 0.3), piece(5, 0.3), "no randomness: the same every time");
    }

    #[test]
    fn the_level_follows_the_lower_bars_and_the_glow_falls_slowly() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0, 0, 0, 0]), 0.0);
        assert_eq!(level(&[1000; 8]), 1.0);
        // only the lower half counts: a loud top does not light it
        assert_eq!(level(&[0, 0, 1000, 1000]), 0.0);
        assert!(level(&[300, 300, 0, 0]) > 0.4);
        let up = smoothed(0.0, 1.0, 0.033);
        let down = smoothed(1.0, 0.0, 0.033);
        assert!(up > 0.5, "rises at once: {up}");
        assert!(down > 0.7, "falls slowly: {down}");
        assert_eq!(smoothed(0.4, 0.4, 0.033), 0.4);
    }
}
