//! The transport: the progress bar (seeks when released, not while dragging), the control row
//! (wraps when the column is narrow, in the layout's button order) and the cover size rule.
//! What to show is a plain function of the layout and the player state, so it is tested.
use super::model::Model;
use super::style::{self, Icon};
use super::view::{accent, fmt_time};
use egui::{Sense, Stroke, Ui, Vec2};
use music_display::layout::{Button, Ordered};
use music_display::player::{Loop, Snapshot, Status};

/// One control of the row, with the state it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ctl {
    Shuffle(bool),
    Repeat(Loop),
    Prev,
    Play(bool),
    Next,
    Volume(f64),
}

/// The controls to draw, in the order the layout lists them: the buttons that are switched on
/// and that the player supports (no shuffle button for a player without shuffle).
pub fn plan(buttons: &Ordered<Button>, s: &Snapshot) -> Vec<Ctl> {
    buttons
        .visible()
        .filter_map(|b| match b {
            Button::Shuffle => s.shuffle.map(Ctl::Shuffle),
            Button::Repeat => s.repeat.map(Ctl::Repeat),
            Button::Prev => Some(Ctl::Prev),
            Button::Play => Some(Ctl::Play(s.status == Status::Playing)),
            Button::Next => Some(Ctl::Next),
            Button::Volume => s.volume.map(Ctl::Volume),
        })
        .collect()
}

/// Where a click at `frac` (0..=1) of the bar seeks to.
pub fn seek_target(frac: f64, length: f64) -> f64 {
    frac.clamp(0.0, 1.0) * length.max(0.0)
}

/// The side of the (square) cover: as wide as the column, but at most `share` of the window's
/// height, so the metadata and the controls below it stay on screen.
pub fn cover_side(avail_w: f32, window_h: f32, share: f32) -> f32 {
    avail_w.min(window_h * share).clamp(64.0, 480.0)
}

/// The buttons that go in the middle (previous, play / pause, next, in the layout's order), and
/// the rest (shuffle, repeat, volume), also in order.
pub fn split_trio(items: Vec<Ctl>) -> (Vec<Ctl>, Vec<Ctl>) {
    items.into_iter().partition(|c| matches!(c, Ctl::Prev | Ctl::Play(_) | Ctl::Next))
}

/// Shuffle and repeat stand left and right of the middle buttons, the way Spotify has them; what
/// is left (the volume) keeps its order.
pub fn take_flanks(rest: Vec<Ctl>) -> (Option<Ctl>, Option<Ctl>, Vec<Ctl>) {
    let (mut shuffle, mut repeat, mut others) = (None, None, Vec::new());
    for c in rest {
        match c {
            Ctl::Shuffle(_) => shuffle = Some(c),
            Ctl::Repeat(_) => repeat = Some(c),
            c => others.push(c),
        }
    }
    (shuffle, repeat, others)
}

pub fn controls(ui: &mut Ui, m: &mut Model) {
    let Some((length, items)) = m.snap.as_ref().map(|s| (s.length, plan(&m.layout.buttons, s))) else { return };
    progress(ui, m, length);
    ui.add_space(2.0);
    // previous, play / pause and next sit in the middle of the row with shuffle on their left and
    // repeat on their right; the volume and the Spotify buttons are on a row of their own below
    let (trio, rest) = split_trio(items);
    let (shuffle, repeat, others) = take_flanks(rest);
    transport_row(ui, m, shuffle, &trio, repeat);
    // like, devices and play here: centred under the play buttons, the same size and spacing
    let n = super::library::transport_count(m);
    if n > 0 {
        centered(ui, n, 0.0, |ui| super::library::transport_buttons(ui, m));
    }
    // the volume, centred as well: the speaker (mute), the bar and the percentage
    for c in others {
        if let Ctl::Volume(v) = c {
            centered(ui, 1, VOL_BAR + 44.0, |ui| volume(ui, m, v));
        }
    }
}

/// The Spotify layout's player bar, middle part: the buttons with the progress bar under them.
pub fn bar_center(ui: &mut Ui, m: &mut Model) {
    let Some((length, items)) = m.snap.as_ref().map(|s| (s.length, plan(&m.layout.buttons, s))) else { return };
    let (trio, rest) = split_trio(items);
    let (shuffle, repeat, _) = take_flanks(rest);
    transport_row(ui, m, shuffle, &trio, repeat);
    progress(ui, m, length);
}

/// Devices, play here and the volume, added to the row the caller is drawing (so they share its
/// centre line with the buttons before them).
pub fn bar_right_inline(ui: &mut Ui, m: &mut Model) {
    super::library::connect_buttons(ui, m);
    if let Some(v) = bar_volume(m) {
        volume(ui, m, v);
    }
}

fn bar_volume(m: &Model) -> Option<f64> {
    m.snap.as_ref().and_then(|s| plan(&m.layout.buttons, s).into_iter().find_map(|c| if let Ctl::Volume(v) = c { Some(v) } else { None }))
}

/// How wide [`bar_right_inline`] draws.
pub fn bar_right_width(ui: &Ui, m: &Model) -> f32 {
    let connect = if m.spot.on() { 2 } else { 0 };
    let gap = ui.spacing().item_spacing.x;
    let vol_w = if bar_volume(m).is_some() { style::ICON_SIDE + VOL_BAR + 44.0 + 2.0 * gap } else { 0.0 };
    connect as f32 * (style::ICON_SIDE + gap) + vol_w
}

/// Shuffle, previous, play / pause, next and repeat, centred.
fn transport_row(ui: &mut Ui, m: &mut Model, shuffle: Option<Ctl>, trio: &[Ctl], repeat: Option<Ctl>) {
    let n = trio.len() + shuffle.iter().len() + repeat.iter().len();
    if n == 0 {
        return;
    }
    ui.horizontal(|ui| {
        let n = n as f32;
        let width = n * style::ICON_SIDE + (n - 1.0) * ui.spacing().item_spacing.x;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        if let Some(Ctl::Shuffle(on)) = shuffle {
            // Smart Shuffle (Spotify, Soloist) can only be switched in the Spotify app; it shows
            // here, and a click toggles plain shuffle as always
            let smart = m.snap.as_ref().is_some_and(|s| s.smart_shuffle);
            let (icon, tip) = match (smart, on) {
                (true, _) => (Icon::SmartShuffle, "Smart Shuffle: on (switched in the Spotify app)"),
                (false, true) => (Icon::Shuffle, "Shuffle: on"),
                (false, false) => (Icon::Shuffle, "Shuffle: off"),
            };
            if style::toggle_button(ui, icon, on || smart).on_hover_text(tip).clicked() {
                m.toggle_shuffle();
            }
        }
        for c in trio {
            match *c {
                Ctl::Prev => {
                    if style::icon_button(ui, Icon::Prev).on_hover_text("previous").clicked() {
                        m.prev();
                    }
                }
                Ctl::Play(playing) => {
                    let (icon, tip) = if playing { (Icon::Pause, "pause") } else { (Icon::Play, "play") };
                    // `beat_pulse`: a glow behind the button that follows the music
                    let glow = super::fun::slot(ui);
                    // the Spotify look: a white circle with a black sign
                    let button = if super::sp_look::active(ui.ctx()) {
                        style::round_button(ui, style::ICON_SIDE + 4.0, icon, egui::Color32::WHITE, egui::Color32::BLACK, tip)
                    } else {
                        style::icon_button(ui, icon).on_hover_text(tip)
                    };
                    glow.fill(ui, button.rect, super::fun::pulse(ui.ctx(), m));
                    if button.clicked() {
                        m.play_pause();
                    }
                }
                Ctl::Next => {
                    if style::icon_button(ui, Icon::Next).on_hover_text("next").clicked() {
                        m.next();
                    }
                }
                Ctl::Shuffle(_) | Ctl::Repeat(_) | Ctl::Volume(_) => {}
            }
        }
        if let Some(Ctl::Repeat(r)) = repeat {
            let (icon, tip) = match r {
                Loop::None => (Icon::Repeat, "repeat: off"),
                Loop::Track => (Icon::RepeatOne, "repeat: this song"),
                Loop::Playlist => (Icon::Repeat, "repeat: all"),
            };
            if style::toggle_button(ui, icon, r != Loop::None).on_hover_text(tip).clicked() {
                m.cycle_repeat();
            }
        }
    });
}

/// The width of the volume bar.
const VOL_BAR: f32 = 120.0;

/// A row in the middle of the column: `buttons` icon buttons plus `extra` points of other things.
fn centered(ui: &mut Ui, buttons: usize, extra: f32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let gap = ui.spacing().item_spacing.x;
        let width = buttons as f32 * style::ICON_SIDE + extra + (buttons as f32 + if extra > 0.0 { 1.0 } else { 0.0 } - 1.0).max(0.0) * gap;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        add(ui);
    });
}

/// The mute button, the bar (the wheel over it steps by `volume_step`) and the percentage,
/// which mutes when clicked.
fn volume(ui: &mut Ui, m: &mut Model, v: f64) {
    let muted = m.muted();
    let waves = if muted || v <= 0.0 { 0 } else if v < 0.5 { 1 } else { 2 };
    if style::toggle_button(ui, Icon::Volume(waves), muted).on_hover_text("Mute / unmute (m)").clicked() {
        m.toggle_mute();
    }
    let mut pct = (v * 100.0) as f32;
    // the same thin bar with a dot as the progress bar above it
    let (r, resp) = ui.allocate_exact_size(Vec2::new(VOL_BAR, 18.0), Sense::click_and_drag());
    if let Some(pt) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        pct = (((pt.x - r.left()) / r.width()).clamp(0.0, 1.0) * 100.0).round();
        m.drag_volume(pct as f64 / 100.0);
    }
    if resp.drag_stopped() || resp.clicked() {
        m.flush_volume(true);
    }
    let hot = resp.hovered() || resp.dragged();
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            m.volume_step(if dy > 0.0 { 1 } else { -1 });
        }
    }
    thin_bar(ui, r, pct / 100.0, hot);
    if ui.add(egui::Label::new(egui::RichText::new(format!("{pct:.0}%")).weak()).sense(egui::Sense::click())).on_hover_text("Click to mute / unmute").clicked() {
        m.toggle_mute();
    }
}

/// The bar: the thumb follows the pointer while dragging and the seek is sent on release (one
/// process or API call per gesture, not one per frame).
fn progress(ui: &mut Ui, m: &mut Model, length: f64) {
    let pos = m.seek_preview.unwrap_or_else(|| m.position());
    ui.horizontal(|ui| {
        time_label(ui, pos);
        let w = (ui.available_width() - 48.0).max(40.0);
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::click_and_drag());
        let hot = resp.hovered() || resp.dragged();
        if hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if length > 0.0 {
            if let Some(pt) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.drag_stopped()) {
                m.seek_preview = Some(seek_target(((pt.x - r.left()) / r.width()) as f64, length));
            }
            if resp.drag_stopped() || resp.clicked() {
                if let Some(p) = m.seek_preview.take() {
                    m.seek(p);
                }
            }
        }
        let shown = m.seek_preview.unwrap_or(pos);
        let frac = if length > 0.0 { (shown / length).clamp(0.0, 1.0) as f32 } else { 0.0 };
        thin_bar(ui, r, frac, hot);
        time_label(ui, length);
    });
}

/// The progress and volume bars: a thin line, filled in the accent colour up to `frac`, with
/// a dot; thicker under the pointer.
fn thin_bar(ui: &Ui, r: egui::Rect, frac: f32, hot: bool) {
    let mid = r.center().y;
    let p = ui.painter();
    let x = r.left() + r.width() * frac.clamp(0.0, 1.0);
    if super::sp_look::active(ui.ctx()) {
        // Spotify: a 4 px grey track, filled white; green with a white dot under the pointer
        let line = |a: f32, b: f32, c: egui::Color32| {
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(a, mid - 2.0), egui::pos2(b, mid + 2.0)), 2.0, c);
        };
        line(r.left(), r.right(), egui::Color32::from_rgb(0x4d, 0x4d, 0x4d));
        line(r.left(), x, if hot { egui::Color32::from_rgb(0x1e, 0xd7, 0x60) } else { egui::Color32::WHITE });
        if hot {
            p.circle_filled(egui::pos2(x, mid), 6.0, egui::Color32::WHITE);
        }
        return;
    }
    let w = if hot { 5.0 } else { 3.0 };
    p.line_segment([egui::pos2(r.left(), mid), egui::pos2(r.right(), mid)], Stroke::new(w, ui.visuals().widgets.inactive.bg_fill));
    p.line_segment([egui::pos2(r.left(), mid), egui::pos2(x, mid)], Stroke::new(w, accent(ui)));
    p.circle_filled(egui::pos2(x, mid), if hot { 7.0 } else { 5.0 }, accent(ui));
}

/// A time next to the progress bar, in the text font (weak, like the other secondary text).
fn time_label(ui: &mut Ui, t: f64) {
    if super::sp_look::active(ui.ctx()) {
        ui.label(egui::RichText::new(fmt_time(t)).weak().size(11.0));
    } else {
        ui.label(egui::RichText::new(fmt_time(t)).weak());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(shuffle: bool, volume: bool) -> Snapshot {
        let mut s = music_display::player::parse(
            "Playing\x1f5000000\x1fSong\x1fBand\x1fAlbum\x1f200000000\x1f\x1f2\x1f1\x1f1999\x1f0.5\x1f\x1fid\x1fvlc\x1f",
        )
        .unwrap();
        s.shuffle = shuffle.then_some(true);
        s.repeat = Some(Loop::Track);
        if !volume {
            s.volume = None;
        }
        s
    }

    #[test]
    fn previous_play_and_next_go_in_the_middle_and_the_rest_keeps_its_order() {
        let items = vec![Ctl::Shuffle(true), Ctl::Repeat(Loop::None), Ctl::Prev, Ctl::Play(true), Ctl::Next, Ctl::Volume(0.5)];
        let (trio, rest) = split_trio(items);
        assert_eq!(trio, [Ctl::Prev, Ctl::Play(true), Ctl::Next]);
        assert_eq!(rest, [Ctl::Shuffle(true), Ctl::Repeat(Loop::None), Ctl::Volume(0.5)]);
        // a player without prev / next still has its play button alone in the middle
        let (trio, rest) = split_trio(vec![Ctl::Play(false)]);
        assert_eq!((trio.len(), rest.len()), (1, 0));
    }

    #[test]
    fn shuffle_and_repeat_flank_the_middle_and_the_volume_stays_below() {
        let (sh, rp, others) = take_flanks(vec![Ctl::Repeat(Loop::Track), Ctl::Volume(0.5), Ctl::Shuffle(false)]);
        assert_eq!((sh, rp), (Some(Ctl::Shuffle(false)), Some(Ctl::Repeat(Loop::Track))));
        assert_eq!(others, [Ctl::Volume(0.5)]);
        assert_eq!(take_flanks(vec![]), (None, None, vec![]));
    }

    #[test]
    fn the_plan_follows_the_layout_order_and_what_the_player_supports() {
        let mut l = music_display::layout::Layout::default();
        let all = plan(&l.buttons, &snap(true, true));
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], Ctl::Shuffle(true));
        assert!(matches!(all[5], Ctl::Volume(_)));
        // no shuffle or volume on the player: no such controls
        let few = plan(&l.buttons, &snap(false, false));
        assert_eq!(few, vec![Ctl::Repeat(Loop::Track), Ctl::Prev, Ctl::Play(true), Ctl::Next]);
        // hidden and moved
        l.buttons.set(Button::Repeat, false);
        l.buttons.shift(Button::Play, -3);
        let p = plan(&l.buttons, &snap(false, false));
        assert_eq!(p.first(), Some(&Ctl::Play(true)));
        assert!(!p.iter().any(|c| matches!(c, Ctl::Repeat(_))));
    }

    #[test]
    fn a_click_maps_to_a_clamped_position() {
        assert_eq!(seek_target(0.5, 200.0), 100.0);
        assert_eq!(seek_target(1.7, 200.0), 200.0);
        assert_eq!(seek_target(-0.2, 200.0), 0.0);
        assert_eq!(seek_target(0.5, 0.0), 0.0);
    }

    #[test]
    fn the_cover_leaves_room_for_the_controls() {
        assert_eq!(cover_side(400.0, 1000.0, 0.4), 400.0);
        assert_eq!(cover_side(400.0, 500.0, 0.4), 200.0, "capped by the window height");
        assert_eq!(cover_side(40.0, 1000.0, 0.4), 64.0);
        assert_eq!(cover_side(900.0, 2000.0, 0.5), 480.0);
    }
}
