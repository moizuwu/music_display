//! The layout editor: the customize page of the settings menu, the right-click menu on a
//! metadata field, and drag and drop. Everything works on `App::layout` and is written to the
//! `[layout]` table of the config file as soon as it changes.
use super::*;
use crate::layout::{Button, Layout, MetaField, PanelId};

/// One selectable line of the customize page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CustItem {
    Panel(PanelId),
    Field(MetaField),
    Button(Button),
}

impl CustItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Panel(p) => p.title(),
            Self::Field(f) => f.label(),
            Self::Button(b) => b.label(),
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::Panel(_) => "Panels",
            Self::Field(_) => "Metadata fields",
            Self::Button(_) => "Control buttons",
        }
    }
}

/// The right-click menu of a metadata field.
#[derive(Clone, Copy, Debug)]
pub struct FieldMenu {
    pub field: MetaField,
    pub at: (u16, u16),
    pub sel: usize,
}

impl FieldMenu {
    pub const ITEMS: [&'static str; 3] = ["Hide", "Move up", "Move down"];
}

/// What the mouse is dragging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    /// A metadata field, dropped onto another one.
    Field(MetaField),
    /// The border between two columns: the left column's panel and where that column starts.
    Border { left: PanelId, start: u16, total: u16 },
}

/// Panel sizes (percent) a column has before anyone sets one, so `+` / `-` start somewhere sensible.
pub fn default_size(p: PanelId) -> u16 {
    match p {
        PanelId::Metadata => 40,
        PanelId::Lyrics => 60,
        PanelId::Cava => 28,
        PanelId::Annotations => 30,
        _ => 50,
    }
}

impl App {
    /// Take over a loaded layout: it decides what is shown at start.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        if self.args.annotations {
            self.layout.panels.set(PanelId::Annotations, true);
        }
        self.show_lyrics = self.layout.panels.shown(PanelId::Lyrics);
        self.show_diagrams = self.layout.panels.shown(PanelId::Chords);
        // the layout alone decides where cava goes (`--thirdpanel` was folded into it on load)
        self.placement =
            if self.layout.cava_under_lyrics { CavaPlacement::UnderLyrics } else { CavaPlacement::ThirdPanel };
        if self.layout.panels.shown(PanelId::Cava) && !self.show_cava && !self.minimal {
            self.set_cava(true);
        }
    }

    pub fn cust_items(&self) -> Vec<CustItem> {
        let mut v: Vec<CustItem> = self.layout.panels.items.iter().map(|(p, _)| CustItem::Panel(*p)).collect();
        v.extend(self.layout.fields.items.iter().map(|(f, _)| CustItem::Field(*f)));
        v.extend(self.layout.buttons.items.iter().map(|(b, _)| CustItem::Button(*b)));
        v
    }

    pub fn cust_shown(&self, it: CustItem) -> bool {
        match it {
            CustItem::Panel(p) => self.layout.panels.shown(p),
            CustItem::Field(f) => self.layout.fields.shown(f),
            CustItem::Button(b) => self.layout.buttons.shown(b),
        }
    }

    /// The explicit size of a panel, or the one it would get, for display on the page.
    pub fn cust_size(&self, p: PanelId) -> Option<u16> {
        self.layout.size_of(p)
    }

    pub fn open_customize(&mut self) {
        self.customizing = true;
        self.cust_sel = 0;
    }

    pub fn save_layout(&mut self) {
        let path = self.args.config.clone().unwrap_or_else(config::default_path);
        self.settings_note = config::save_layout(&path, &self.layout)
            .err()
            .map(|e| format!("couldn't save {}: {e}", path.display()));
    }

    /// Show / hide one entry. Panels that have a runtime switch (`l`, `c`, `D`) follow it.
    pub fn cust_toggle(&mut self, it: CustItem) {
        match it {
            CustItem::Panel(p) => {
                let on = !self.layout.panels.shown(p);
                self.layout.panels.set(p, on);
                match p {
                    PanelId::Lyrics => {
                        self.show_lyrics = on;
                        self.snap_scroll = true;
                    }
                    PanelId::Chords => self.show_diagrams = on,
                    PanelId::Annotations => {
                        self.args.annotations = on;
                        if on {
                            self.ensure_annotations();
                        }
                    }
                    PanelId::Cava => self.set_cava(on),
                    _ => {}
                }
            }
            CustItem::Field(f) => self.layout.fields.toggle(f),
            CustItem::Button(b) => self.layout.buttons.toggle(b),
        }
        self.save_layout();
    }

    /// Where cava goes: below the lyrics (`true`) or in a column of its own. This is the one place
    /// that changes it, so the layout, the running screen and the config file cannot disagree.
    pub fn set_cava_dock(&mut self, under: bool) {
        self.layout.cava_under_lyrics = under;
        self.placement = if under { CavaPlacement::UnderLyrics } else { CavaPlacement::ThirdPanel };
        self.args.thirdpanel = !under;
        // a column of its own is only worth choosing for something you can see
        if !under && !self.minimal && !self.show_cava {
            self.set_cava(true);
        }
        self.save_layout();
    }

    /// Move one entry up (`-1`) or down (`1`) within its group.
    pub fn cust_move(&mut self, it: CustItem, dir: i32) -> bool {
        let moved = match it {
            CustItem::Panel(p) => self.layout.panels.shift(p, dir),
            CustItem::Field(f) => self.layout.fields.shift(f, dir),
            CustItem::Button(b) => self.layout.buttons.shift(b, dir),
        };
        if moved {
            self.save_layout();
        }
        moved
    }

    /// Grow / shrink a panel's share by `steps` * 5 percent.
    pub fn cust_resize(&mut self, p: PanelId, steps: i32) {
        let now = self.layout.size_of(p).unwrap_or_else(|| default_size(p)) as i32;
        self.layout.set_size(p, (now + steps * 5).clamp(0, u16::MAX as i32) as u16);
        self.save_layout();
    }

    /// Back to the built-in layout (the old flags still apply on top at the next start).
    pub fn cust_reset(&mut self) {
        self.layout = Layout::default();
        self.show_lyrics = true;
        self.show_diagrams = false;
        self.placement = CavaPlacement::UnderLyrics;
        self.args.thirdpanel = false;
        if self.show_cava {
            self.set_cava(false);
        }
        self.save_layout();
        self.set_note("layout reset");
    }

    pub(super) fn handle_customize_key(&mut self, code: KeyCode) {
        let items = self.cust_items();
        let n = items.len();
        self.cust_sel = self.cust_sel.min(n.saturating_sub(1));
        let it = items[self.cust_sel];
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.cust_sel = (self.cust_sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.cust_sel = (self.cust_sel + 1) % n,
            KeyCode::Home => self.cust_sel = 0,
            KeyCode::End => self.cust_sel = n - 1,
            KeyCode::Enter | KeyCode::Char(' ') => self.cust_toggle(it),
            // reorder: the entry keeps the selection, so it can be moved on
            KeyCode::Char('u') | KeyCode::Char('K') if self.cust_move(it, -1) => self.follow(it),
            KeyCode::Char('d') | KeyCode::Char('J') if self.cust_move(it, 1) => self.follow(it),
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right | KeyCode::Char('l') => {
                if let CustItem::Panel(p) = it {
                    self.cust_resize(p, 1)
                }
            }
            KeyCode::Char('-') | KeyCode::Left | KeyCode::Char('h') => {
                if let CustItem::Panel(p) = it {
                    self.cust_resize(p, -1)
                }
            }
            // cava: below the lyrics or in a column of its own
            KeyCode::Char('p') if matches!(it, CustItem::Panel(PanelId::Cava)) => {
                let under = !self.layout.cava_under_lyrics;
                self.set_cava_dock(under);
            }
            KeyCode::Char('a') => {
                self.layout.clear_sizes();
                self.save_layout();
            }
            KeyCode::Char('r') => self.cust_reset(),
            KeyCode::Esc | KeyCode::Char('q' | 'c') => self.customizing = false,
            _ => {}
        }
    }

    fn follow(&mut self, it: CustItem) {
        if let Some(i) = self.cust_items().iter().position(|x| *x == it) {
            self.cust_sel = i;
        }
    }

    // ── right-click menu and drag and drop on the metadata block ──

    pub fn field_at(&self, x: u16, y: u16) -> Option<MetaField> {
        let pos = Position::new(x, y);
        self.hits.fields.iter().find(|(_, r)| r.contains(pos)).map(|(f, _)| *f)
    }

    pub(super) fn open_field_menu(&mut self, field: MetaField, x: u16, y: u16) {
        self.field_menu = Some(FieldMenu { field, at: (x, y), sel: 0 });
    }

    pub fn run_field_menu(&mut self, field: MetaField, choice: usize) {
        match choice {
            0 => self.layout.fields.set(field, false),
            1 => {
                self.move_visible(field, -1);
            }
            _ => {
                self.move_visible(field, 1);
            }
        }
        self.save_layout();
    }

    /// Move a field past the next *visible* neighbour, so one press always shows a change.
    fn move_visible(&mut self, field: MetaField, dir: i32) -> bool {
        let items = &mut self.layout.fields.items;
        let Some(at) = items.iter().position(|(f, _)| *f == field) else { return false };
        let mut to = at as i32;
        loop {
            to += dir;
            match items.get(to as usize) {
                _ if to < 0 => return false,
                None => return false,
                Some((_, true)) => break,
                Some(_) => {}
            }
        }
        let e = items.remove(at);
        items.insert(to as usize, e);
        true
    }

    pub(super) fn handle_field_menu_key(&mut self, code: KeyCode) {
        let Some(mut m) = self.field_menu.take() else { return };
        let n = FieldMenu::ITEMS.len();
        match code {
            KeyCode::Up | KeyCode::Char('k') => m.sel = (m.sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => m.sel = (m.sel + 1) % n,
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.run_field_menu(m.field, m.sel);
                return;
            }
            KeyCode::Esc | KeyCode::Char('q') => return,
            _ => {}
        }
        self.field_menu = Some(m);
    }

    /// Mouse while the field menu is open: a click on an item runs it, anywhere else closes it.
    pub(super) fn field_menu_mouse(&mut self, m: crossterm::event::MouseEvent) {
        let Some(menu) = self.field_menu else { return };
        if !matches!(m.kind, MouseEventKind::Down(_)) {
            return;
        }
        let pos = Position::new(m.column, m.row);
        self.field_menu = None;
        let hit = self.hits.menu_items.iter().position(|r| r.contains(pos));
        if let (Some(i), MouseEventKind::Down(MouseButton::Left)) = (hit, m.kind) {
            self.run_field_menu(menu.field, i);
        }
    }

    pub(super) fn mouse_down(&mut self, x: u16, y: u16) -> bool {
        let pos = Position::new(x, y);
        // a column border first (drag to resize), then a field (drag to reorder)
        if let Some(b) = self.hits.borders.iter().find(|b| b.rect.contains(pos)) {
            self.drag = Some(Drag::Border { left: b.left, start: b.start, total: b.total });
            return true;
        }
        if let Some(f) = self.field_at(x, y) {
            self.drag = Some(Drag::Field(f));
            return true;
        }
        false
    }

    pub(super) fn mouse_drag(&mut self, x: u16, _y: u16) {
        match self.drag {
            Some(Drag::Border { left, start, total }) => {
                let pct = (x.saturating_sub(start) as u32 * 100 / total.max(1) as u32) as u16;
                self.layout.set_size(left, pct);
            }
            Some(Drag::Field(_)) | None => {}
        }
    }

    pub(super) fn mouse_up(&mut self, x: u16, y: u16) {
        match self.drag.take() {
            Some(Drag::Field(f)) => {
                if let Some(target) = self.field_at(x, y) {
                    if target == f {
                        // released where it was pressed: a click
                        if !self.open_diagrams_for(f) {
                            self.open_library_for(f);
                        }
                    } else if self.layout.fields.move_to(f, target) {
                        self.save_layout();
                    }
                }
            }
            Some(Drag::Border { .. }) => self.save_layout(),
            None => {}
        }
    }

    /// Click on Key / BPM / Capo / Tuning: open the chord diagram panel.
    pub fn open_diagrams_for(&mut self, f: MetaField) -> bool {
        if !f.opens_diagrams() {
            return false;
        }
        if !self.show_diagrams {
            self.show_diagrams = true;
            self.layout.panels.set(PanelId::Chords, true);
        }
        if !self.show_lyrics {
            self.show_lyrics = true;
        }
        true
    }
}
