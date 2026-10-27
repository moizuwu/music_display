use super::*;

/// What every part of the screen offers, so the layout code treats them alike: who it is, how
/// small it may get, how to draw it, what a click on it does, and where its settings live.
// (the title, id and section accessors are for the customize page and the GUI front end)
#[allow(dead_code)]
pub trait Panel {
    fn id(&self) -> PanelId;
    fn title(&self) -> &'static str {
        self.id().title()
    }
    fn min_size(&self) -> (u16, u16) {
        self.id().min_size()
    }
    /// The `config.toml` table holding this panel's own settings.
    fn config_section(&self) -> &'static str {
        self.id().section()
    }
    fn render(&self, f: &mut Frame, app: &mut App, area: Rect);
    /// A left click at (`x`, `y`) inside the panel; `true` when the panel used it.
    fn click(&self, _app: &mut App, _x: u16, _y: u16) -> bool {
        false
    }
}

struct Metadata;
struct Lyrics;
struct Cava;
struct Annotations;
/// The panels the layout stacks inside the now-playing column (drawn with it) and the library
/// (a side panel / overlay with its own mouse handling) have no area of their own to draw into.
#[allow(dead_code)]
struct Inline(PanelId);

impl Panel for Metadata {
    fn id(&self) -> PanelId {
        PanelId::Metadata
    }
    fn render(&self, f: &mut Frame, app: &mut App, area: Rect) {
        draw_left(f, app, area);
    }
    fn click(&self, app: &mut App, x: u16, y: u16) -> bool {
        app.field_at(x, y).is_some_and(|f| app.open_diagrams_for(f))
    }
}

impl Panel for Lyrics {
    fn id(&self) -> PanelId {
        PanelId::Lyrics
    }
    fn render(&self, f: &mut Frame, app: &mut App, area: Rect) {
        // cava under the lyrics shares their column (70 / 30)
        if app.show_cava && app.placement == CavaPlacement::UnderLyrics {
            let [t, b] = Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)]).areas(area);
            draw_lyrics_and_diagrams(f, app, t);
            draw_cava(f, app, b);
        } else {
            draw_lyrics_and_diagrams(f, app, area);
        }
    }
    fn click(&self, app: &mut App, x: u16, y: u16) -> bool {
        let inside = app.lyric_hits.area.contains(ratatui::layout::Position::new(x, y));
        if inside {
            app.click_lyric(x, y);
        }
        inside
    }
}

impl Panel for Cava {
    fn id(&self) -> PanelId {
        PanelId::Cava
    }
    fn render(&self, f: &mut Frame, app: &mut App, area: Rect) {
        draw_cava(f, app, area);
    }
}

impl Panel for Annotations {
    fn id(&self) -> PanelId {
        PanelId::Annotations
    }
    fn render(&self, _f: &mut Frame, _app: &mut App, _area: Rect) {}
}

impl Panel for Inline {
    fn id(&self) -> PanelId {
        self.0
    }
    fn render(&self, _f: &mut Frame, _app: &mut App, _area: Rect) {}
}

/// The panel behind an id.
pub fn panel_for(id: PanelId) -> Box<dyn Panel> {
    match id {
        PanelId::Metadata => Box::new(Metadata),
        PanelId::Lyrics => Box::new(Lyrics),
        PanelId::Cava => Box::new(Cava),
        PanelId::Annotations => Box::new(Annotations),
        other => Box::new(Inline(other)),
    }
}
