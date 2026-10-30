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

pub fn controls(ui: &mut Ui, m: &mut Model) {
    let Some((length, items)) = m.snap.as_ref().map(|s| (s.length, plan(&m.layout.buttons, s))) else { return };
    progress(ui, m, length);
    ui.add_space(2.0);
    // previous, play / pause and next sit in the middle of the row; shuffle, repeat and the
    // volume are on a row of their own below them
    let (trio, rest) = split_trio(items);
    if !trio.is_empty() {
        ui.horizontal(|ui| {
            let n = trio.len() as f32;
            let width = n * style::ICON_SIDE + (n - 1.0) * ui.spacing().item_spacing.x;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            for c in &trio {
                match *c {
                    Ctl::Prev => {
                        if style::icon_button(ui, Icon::Prev).on_hover_text("previous").clicked() {
                            m.prev();
                        }
                    }
                    Ctl::Play(playing) => {
                        let (icon, tip) = if playing { (Icon::Pause, "pause") } else { (Icon::Play, "play") };
                        if style::icon_button(ui, icon).on_hover_text(tip).clicked() {
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
        });
    }
    ui.horizontal_wrapped(|ui| {
        for c in rest {
            match c {
                Ctl::Shuffle(on) => {
                    if ui.selectable_label(on, "shuffle").clicked() {
                        m.toggle_shuffle();
                    }
                }
                Ctl::Repeat(r) => {
                    let label = match r {
                        Loop::None => "repeat",
                        Loop::Track => "repeat 1",
                        Loop::Playlist => "repeat all",
                    };
                    if ui.selectable_label(r != Loop::None, label).clicked() {
                        m.cycle_repeat();
                    }
                }
                Ctl::Prev | Ctl::Play(_) | Ctl::Next => {}
                Ctl::Volume(v) => {
                    // mute button, the slider (the wheel over it steps by `volume_step`), and
                    // the percentage, which mutes when clicked
                    let muted = m.muted();
                    if ui.selectable_label(muted, if muted { "muted" } else { "mute" }).on_hover_text("mute / unmute (m)").clicked() {
                        m.toggle_mute();
                    }
                    let mut pct = (v * 100.0) as f32;
                    ui.spacing_mut().slider_width = 90.0;
                    let r = ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").show_value(false));
                    if r.changed() {
                        m.drag_volume(pct as f64 / 100.0);
                    }
                    if r.drag_stopped() || r.lost_focus() {
                        m.flush_volume(true);
                    }
                    if r.hovered() {
                        let dy = ui.input(|i| i.smooth_scroll_delta.y);
                        if dy != 0.0 {
                            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                            m.volume_step(if dy > 0.0 { 1 } else { -1 });
                        }
                    }
                    if ui.add(egui::Label::new(format!("{pct:.0}%")).sense(egui::Sense::click())).on_hover_text("click to mute / unmute").clicked() {
                        m.toggle_mute();
                    }
                }
            }
        }
        super::library::transport_buttons(ui, m);
    });
}

/// The bar: the thumb follows the pointer while dragging and the seek is sent on release (one
/// process or API call per gesture, not one per frame).
fn progress(ui: &mut Ui, m: &mut Model, length: f64) {
    let pos = m.seek_preview.unwrap_or_else(|| m.position());
    ui.horizontal(|ui| {
        ui.monospace(fmt_time(pos));
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
        let mid = r.center().y;
        let p = ui.painter();
        p.line_segment([egui::pos2(r.left(), mid), egui::pos2(r.right(), mid)], Stroke::new(if hot { 5.0 } else { 3.0 }, ui.visuals().widgets.inactive.bg_fill));
        let x = r.left() + r.width() * frac;
        p.line_segment([egui::pos2(r.left(), mid), egui::pos2(x, mid)], Stroke::new(if hot { 5.0 } else { 3.0 }, accent(ui)));
        p.circle_filled(egui::pos2(x, mid), if hot { 7.0 } else { 5.0 }, accent(ui));
        ui.monospace(fmt_time(length));
    });
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
