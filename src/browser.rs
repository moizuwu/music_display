//! Widgets of the Spotify library browser and the settings menu that need no `App`:
//! a one-line text input, the lists the browser shows, and hint-line fitting.

use crate::spotify;
use crossterm::event::KeyCode;

/// A one-line text field (cursor counted in characters).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextInput {
    pub text: String,
    pub cursor: usize,
}

/// What a key did to a [`TextInput`] besides editing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Submit,
    Cancel,
}

impl TextInput {
    pub fn with(text: &str) -> Self {
        TextInput { text: text.to_string(), cursor: text.chars().count() }
    }

    fn byte(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    /// The text with a `▏` where the cursor is, for drawing.
    pub fn shown(&self) -> String {
        let at = self.byte(self.cursor);
        format!("{}▏{}", &self.text[..at], &self.text[at..])
    }

    /// Edit with one key: Enter submits, Esc cancels, everything else is typing.
    pub fn key(&mut self, code: KeyCode) -> Option<Edit> {
        match code {
            KeyCode::Enter => return Some(Edit::Submit),
            KeyCode::Esc => return Some(Edit::Cancel),
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            _ => {}
        }
        None
    }
}

/// The library browser's tabs, in order.
pub const TABS: [&str; 6] = ["Recent", "Playlists", "Liked", "Albums", "Artists", "Search"];
pub const TAB_LIKED: usize = 2;
pub const TAB_PLAYLISTS: usize = 1;
pub const TAB_SEARCH: usize = 5;

/// One list in the library browser: a tab, or an album / playlist / artist opened from one.
#[derive(Default)]
pub struct Level {
    pub title: String,
    /// The context its tracks play in (`liked` for Liked Songs).
    pub uri: String,
    pub summary: String,
    pub items: Vec<spotify::Item>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub sel: usize,
    /// First row shown, counted in drawn rows (group headings included).
    pub top: usize,
    /// Search results: grouped under headings, and kept (dimmed) while a new search runs.
    pub search: bool,
}

/// A row of a drawn list: a group heading (search results) or the item at that index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRow {
    Head(&'static str),
    Item(usize),
}

/// The heading a kind of result is listed under.
pub fn group_label(kind: &str) -> &'static str {
    match kind {
        "track" => "Songs",
        "album" => "Albums",
        "artist" => "Artists",
        "playlist" => "Playlists",
        _ => "Other",
    }
}

impl Level {
    /// The list as drawn: search results get a heading where the kind changes.
    pub fn rows(&self) -> Vec<ListRow> {
        let mut out = Vec::new();
        let mut last = "";
        for (i, it) in self.items.iter().enumerate() {
            if self.search && it.kind != last {
                out.push(ListRow::Head(group_label(&it.kind)));
                last = &it.kind;
            }
            out.push(ListRow::Item(i));
        }
        out
    }

    /// Keep the selected item inside a view `view_h` rows tall (and its heading with it when
    /// it is the first of its group). Returns the rows to draw from `top` on.
    pub fn scroll(&mut self, view_h: usize) -> Vec<ListRow> {
        let rows = self.rows();
        let at = rows.iter().position(|r| *r == ListRow::Item(self.sel)).unwrap_or(0);
        let view_h = view_h.max(1);
        if at < self.top {
            self.top = at;
            if at > 0 && matches!(rows[at - 1], ListRow::Head(_)) {
                self.top = at - 1;
            }
        } else if at >= self.top + view_h {
            self.top = at + 1 - view_h;
        }
        self.top = self.top.min(rows.len().saturating_sub(1));
        rows
    }
}

/// `o`: the Spotify library browser.
#[derive(Default)]
pub struct Library {
    pub open: bool,
    pub tab: usize,
    pub tabs: Vec<Level>,
    /// Albums / playlists / artists opened from the tab, innermost last.
    pub stack: Vec<Level>,
    /// "Add to playlist": the track (uri, name) and the list of your own playlists.
    pub picker: Option<(String, String, Level)>,
    pub note: Option<String>,
    /// The search field, whether it is being typed into, and the query the results are for.
    pub input: TextInput,
    pub editing: bool,
    pub query: String,
}

/// `d`: the Spotify Connect device list.
#[derive(Default)]
pub struct DevicePicker {
    pub items: Vec<spotify::Device>,
    pub loading: bool,
    pub error: Option<String>,
    pub sel: usize,
    pub top: usize,
}

impl Library {
    pub fn new() -> Self {
        let tabs = TABS
            .iter()
            .enumerate()
            .map(|(i, t)| Level {
                title: t.to_string(),
                uri: if i == TAB_LIKED { "liked".into() } else { String::new() },
                search: i == TAB_SEARCH,
                ..Default::default()
            })
            .collect();
        Library { tabs, ..Default::default() }
    }

    /// The list on screen.
    pub fn level(&self) -> &Level {
        match &self.picker {
            Some((_, _, l)) => l,
            None => self.stack.last().unwrap_or(&self.tabs[self.tab]),
        }
    }

    pub fn level_mut(&mut self) -> &mut Level {
        match &mut self.picker {
            Some((_, _, l)) => l,
            None => self.stack.last_mut().unwrap_or(&mut self.tabs[self.tab]),
        }
    }
}

/// Join hint items with ` · ` as far as `width` columns allow, dropping whole items from the
/// end instead of cutting one in the middle.
pub fn fit_items(items: &[&str], width: usize) -> String {
    let mut out = String::new();
    for it in items {
        let add = if out.is_empty() { crate::util::width(it) } else { crate::util::width(it) + 3 };
        if crate::util::width(&out) + add > width {
            break;
        }
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(it);
    }
    out
}
