//! The Spotify layout's measures and colours in one place. With `spotify_look` on (the built-in
//! Spotify preset) they are the Spotify app's own, measured from screenshots of it; without it the
//! colours come from the window's theme, so the layout keeps the colours the user picked.
use egui::{Color32, FontFamily, FontId, Ui};
use music_display::guiconf::GuiConf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpLook {
    /// Spotify's own look (not only its arrangement).
    pub on: bool,
    /// The window behind the panels.
    pub bg: Color32,
    /// The panels (library, page, Now playing).
    pub panel: Color32,
    /// Raised things on a panel: the home button, the search field, "+ Create".
    pub raised: Color32,
    /// Chips, and a row or card under the pointer.
    pub chip: Color32,
    pub hover: Color32,
    pub text: Color32,
    /// Secondary text: artists, "Album • 2021", times.
    pub sub: Color32,
    /// Spotify's green: the play button, what plays, "saved".
    pub green: Color32,
    /// Between panels, and around the window.
    pub gap: f32,
    pub radius: u8,
    pub top_h: f32,
    pub bar_h: f32,
    pub side_w: f32,
    /// Text sizes: small (sub lines, times), body (names), heading (shelves), title (pages).
    pub small: f32,
    pub body: f32,
    pub heading: f32,
    pub title: f32,
}

impl SpLook {
    /// The look for this frame.
    pub fn get(ui: &Ui, g: &GuiConf) -> SpLook {
        if g.spotify_look {
            return SpLook::spotify(ui.ctx().content_rect().width());
        }
        let v = ui.visuals();
        let panel = g.card_color.map_or(v.extreme_bg_color, |c| Color32::from_rgb(c.0, c.1, c.2));
        SpLook {
            on: false,
            bg: v.window_fill,
            panel,
            raised: v.widgets.inactive.bg_fill,
            chip: v.widgets.inactive.weak_bg_fill,
            hover: v.widgets.hovered.weak_bg_fill,
            text: v.strong_text_color(),
            sub: v.weak_text_color(),
            green: v.hyperlink_color,
            ..SpLook::spotify(ui.ctx().content_rect().width())
        }
    }

    /// Spotify's values (desktop app, about 1900 × 1040), the columns narrower in a small window.
    pub fn spotify(window_w: f32) -> SpLook {
        SpLook {
            on: true,
            bg: Color32::BLACK,
            panel: Color32::from_rgb(0x12, 0x12, 0x12),
            raised: Color32::from_rgb(0x1f, 0x1f, 0x1f),
            chip: Color32::from_rgb(0x2a, 0x2a, 0x2a),
            hover: Color32::from_rgb(0x1f, 0x1f, 0x1f),
            text: Color32::WHITE,
            sub: Color32::from_rgb(0xb3, 0xb3, 0xb3),
            green: Color32::from_rgb(0x1e, 0xd7, 0x60),
            gap: 8.0,
            radius: 8,
            top_h: 52.0,
            bar_h: 72.0,
            side_w: (window_w * 0.166).clamp(240.0, 420.0),
            small: 12.0,
            body: 13.0,
            heading: 22.0,
            title: 72.0,
        }
    }

    /// Text of `size` in the regular face.
    pub fn font(&self, size: f32) -> FontId {
        FontId::proportional(size)
    }

    /// Text of `size` in the bold face (Figtree Bold with the Spotify look).
    pub fn bold(&self, size: f32) -> FontId {
        FontId::new(size, FontFamily::Name(super::style::BOLD.into()))
    }
}

/// Is the Spotify look on this frame? Set once per frame by the window, read by the parts the
/// normal layout shares (the progress bar, the play button).
pub fn active(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("sp-look-on"))).unwrap_or(false)
}

pub fn set_active(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("sp-look-on"), on));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_measures_and_a_narrow_window() {
        let l = SpLook::spotify(1900.0);
        assert!(l.on && l.bg == Color32::BLACK && l.gap == 8.0);
        assert_eq!(l.green, Color32::from_rgb(0x1e, 0xd7, 0x60));
        assert!((l.side_w - 315.4).abs() < 1.0, "about 316 px at 1900");
        assert_eq!(SpLook::spotify(900.0).side_w, 240.0, "never narrower than 240");
    }
}
