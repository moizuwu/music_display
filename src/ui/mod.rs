use crate::app::{App, CavaPlacement, CoverState, LyricsState, Rendered, YearState};
use crate::lyrics::Lyrics;
use crate::player::Status;
use crate::theme::Theme;
use crate::util::fmt_time;
use crate::{bigfont, chords, cover, diagram};
use image::imageops::FilterType;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui_image::{Image, Resize};

mod cava;
mod controls;
mod cover_art;
mod diagrams;
mod focus;
mod footer;
mod help;
mod karaoke;
mod left;
mod library;
mod lyrics;
mod settings;
mod text;

use cava::*;
pub use controls::*;
use cover_art::*;
use diagrams::*;
pub use focus::*;
use footer::*;
pub use help::*;
pub use karaoke::*;
pub use left::*;
use library::*;
use lyrics::*;
use settings::*;
pub use text::*;

/// Metadata | Lyrics | Cava. The left and right columns get exactly the same width, so
/// the cava panel mirrors the album/metadata column; lyrics take whatever is left.
pub fn three_panel(main: Rect) -> [Rect; 3] {
    let side = ((main.width as u32 * 28 / 100) as u16)
        .max(20)
        .min(main.width.saturating_sub(20) / 2);
    Layout::horizontal([
        Constraint::Length(side),
        Constraint::Min(1),
        Constraint::Length(side),
    ])
    .areas(main)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    draw_main(f, app);
    if app.show_help {
        draw_help(f, app);
    }
    // (as a side panel it was already drawn as part of the layout)
    if app.lib.open && !app.lib.hits.panel {
        draw_library(f, app);
    }
    if app.devices.is_some() {
        draw_devices(f, app);
    }
    if app.show_settings {
        draw_settings(f, app);
    }
}

/// Terminals at least this wide show the library as a panel beside the player; narrower
/// ones get the overlay.
pub const PANEL_MIN_WIDTH: u16 = 110;

/// Width of the side panel, borders included.
pub const PANEL_WIDTH: u16 = 44;

/// Key hints joined with ` · ` into `width` columns by dropping whole items, never cutting one:
/// the item with the highest priority number goes first (the later one on a tie). Nothing that
/// fits gives an empty string. Items stay in the order given.
pub fn fit_hints(items: &[(u8, &str)], width: usize) -> String {
    let mut keep = vec![true; items.len()];
    loop {
        let line = items
            .iter()
            .zip(&keep)
            .filter(|(_, k)| **k)
            .map(|(i, _)| i.1)
            .collect::<Vec<_>>()
            .join(" · ");
        if !line.is_empty() && crate::util::width(&line) <= width {
            return line;
        }
        let Some(drop) = (0..items.len()).filter(|i| keep[*i]).max_by_key(|i| (items[*i].0, *i)) else {
            // even one item doesn't fit joined with the rest: show the most important that fits alone
            return items
                .iter()
                .enumerate()
                .filter(|(_, i)| crate::util::width(i.1) <= width)
                .min_by_key(|(n, i)| (i.0, *n))
                .map(|(_, i)| i.1.to_string())
                .unwrap_or_default();
        };
        keep[drop] = false;
    }
}

/// `text` shortened to `width` columns with a trailing `…` (display width, not characters).
pub fn ellipsize(text: &str, width: usize) -> String {
    if crate::util::width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        if crate::util::width(&out) + crate::util::width(&c.to_string()) + 1 > width {
            break;
        }
        out.push(c);
    }
    if width > 0 {
        out.push('…');
    }
    out
}

fn draw_main(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let th = app.theme.clone();
    // Only paint a background when we actually know the surface colour; otherwise the
    // terminal's own (possibly transparent) background is left alone.
    if !app.args.transparent && th.surface.is_some() {
        f.render_widget(
            Block::new().style(Style::new().bg(th.surface()).fg(th.text())),
            area,
        );
    }

    let [mut main, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    app.lyric_hits = Default::default();
    app.lib.hits = Default::default();

    // the minimal view has no room for a button or a panel: the browser is an overlay there
    if app.minimal {
        draw_minimal(f, app, main);
        draw_footer(f, app, footer);
        return;
    }

    // Spotify: a one-row strip with the library button, and the browser as a panel on the
    // left when the terminal is wide enough (the player keeps the rest of the width)
    if app.spot.is_some() && !app.args.no_spotify_button && main.height >= 10 {
        let [strip, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(main);
        draw_library_button(f, app, strip);
        main = rest;
    }
    if app.lib.open && area.width >= PANEL_MIN_WIDTH {
        let [panel, rest] =
            Layout::horizontal([Constraint::Length(PANEL_WIDTH), Constraint::Min(0)]).areas(main);
        app.lib.hits.panel = true;
        draw_library_box(f, app, panel);
        main = rest;
    }

    // ── column layout ──
    let (left, lyr, cav) = match (app.show_lyrics, app.show_cava, app.placement) {
        (false, false, _) => (main, None, None),
        (true, false, _) => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main);
            (l, Some(r), None)
        }
        (false, true, CavaPlacement::ThirdPanel) => {
            // Lyrics hidden: keep the two remaining columns the same width.
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .areas(main);
            (l, None, Some(r))
        }
        (false, true, CavaPlacement::UnderLyrics) => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main);
            (l, None, Some(r))
        }
        (true, true, CavaPlacement::ThirdPanel) => {
            let [l, m, r] = three_panel(main);
            (l, Some(m), Some(r))
        }
        (true, true, CavaPlacement::UnderLyrics) => {
            let [l, r] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(main);
            let [t, b] =
                Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .areas(r);
            (l, Some(t), Some(b))
        }
    };

    draw_left(f, app, left);
    if let Some(r) = lyr {
        draw_lyrics_and_diagrams(f, app, r);
    }
    if let Some(r) = cav {
        draw_cava(f, app, r);
    }
    draw_footer(f, app, footer);
}

/// `M`: title, lyrics across the whole area, and a thin progress bar. No cover,
/// metadata, buttons or visualiser.
fn draw_minimal(f: &mut Frame, app: &mut App, main: Rect) {
    app.hits = Default::default();
    let th = app.theme.clone();
    let [title, body, prog] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(main);
    if let Some(s) = app.snap.clone() {
        let text = if s.artist.is_empty() {
            s.title.clone()
        } else {
            format!("{} — {}", s.artist, s.title)
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("◆ ", Style::new().fg(th.primary())),
                Span::styled(text, Style::new().fg(th.text()).add_modifier(Modifier::BOLD)),
            ]))
            .alignment(Alignment::Center),
            title,
        );
        app.hits.progress = timed_bar(f, &th, prog, app.position(), s.length);
    }
    draw_lyrics_and_diagrams(f, app, body);
}

fn panel<'a>(app: &App, title: &'a str) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(app.theme.accent_fade(0.6)))
        .title(Span::styled(
            format!(" {title} "),
            Style::new()
                .fg(app.theme.primary())
                .add_modifier(Modifier::BOLD),
        ))
}

fn centered_msg(f: &mut Frame, area: Rect, msgs: &[(&str, Style)]) {
    let start = area.y + area.height.saturating_sub(msgs.len() as u16) / 2;
    for (i, (m, s)) in msgs.iter().enumerate() {
        let y = start + i as u16;
        if y < area.bottom() {
            f.render_widget(
                Paragraph::new(Line::styled(*m, *s)).alignment(Alignment::Center),
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );
        }
    }
}
