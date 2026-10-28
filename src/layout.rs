//! What is shown and where: the panels in order, the metadata fields in order, the control
//! buttons, split sizes and direction. Pure data with no terminal types, so any front end can use
//! it; it is read from the `[layout]` table of `config.toml` and written back by the customize
//! page. The old flags (`--nolyrics`, `--cava`, `--thirdpanel`, `--underlyrics`, `--minimal`) are
//! named presets of this ([`Layout::preset`]).

use std::fmt;

/// Anything that has a stable text id in the config file.
pub trait Item: Copy + PartialEq + fmt::Debug + 'static {
    const ALL: &'static [Self];
    fn id(self) -> &'static str;
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|i| i.id() == s.trim().to_ascii_lowercase())
    }
}

macro_rules! items {
    ($(#[$m:meta])* $name:ident { $($v:ident => $id:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
        pub enum $name { $($v),+ }
        impl Item for $name {
            const ALL: &'static [Self] = &[$(Self::$v),+];
            fn id(self) -> &'static str { match self { $(Self::$v => $id),+ } }
        }
    };
}

items! {
    /// A part of the screen with its own config section and click handling.
    PanelId {
        Cover => "cover",
        Metadata => "metadata",
        Controls => "controls",
        Lyrics => "lyrics",
        Chords => "chords",
        Cava => "cava",
        Annotations => "annotations",
        Library => "library",
    }
}

items! {
    /// One line of the metadata block.
    MetaField {
        Title => "title",
        Artist => "artist",
        Album => "album",
        TrackNo => "track_no",
        Release => "release",
        Genre => "genre",
        Tuning => "tuning",
        Capo => "capo",
        Bpm => "bpm",
        Key => "key",
    }
}

items! {
    /// One control of the transport row.
    Button {
        Shuffle => "shuffle",
        Repeat => "repeat",
        Prev => "prev",
        Play => "play",
        Next => "next",
        Volume => "volume",
    }
}

impl PanelId {
    pub fn title(self) -> &'static str {
        match self {
            Self::Metadata => "Metadata",
            Self::Cover => "Cover",
            Self::Controls => "Controls",
            Self::Lyrics => "Lyrics",
            Self::Chords => "Chord diagrams",
            Self::Cava => "Cava",
            Self::Annotations => "Annotations",
            Self::Library => "Library",
        }
    }

    /// The config table holding this panel's own settings.
    pub fn section(self) -> &'static str {
        match self {
            Self::Metadata => "layout.fields",
            Self::Controls => "layout.buttons",
            _ => "layout",
        }
    }

    /// Smallest (width, height) in cells at which the panel is worth drawing.
    pub fn min_size(self) -> (u16, u16) {
        match self {
            Self::Metadata => (24, 3),
            Self::Cover => (8, 4),
            Self::Controls => (20, 1),
            Self::Lyrics => (24, 3),
            Self::Chords => (22, 6),
            Self::Cava => (12, 3),
            Self::Annotations => (24, 3),
            Self::Library => (30, 6),
        }
    }

    /// The three panels stacked in the "now playing" column.
    pub fn in_now_playing(self) -> bool {
        matches!(self, Self::Metadata | Self::Cover | Self::Controls)
    }
}

impl MetaField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Track",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::TrackNo => "Track No",
            Self::Release => "Release",
            Self::Genre => "Genre",
            Self::Tuning => "Tuning",
            Self::Capo => "Capo",
            Self::Bpm => "BPM",
            Self::Key => "Key",
        }
    }

    /// Fields that open the chord diagram panel when clicked.
    pub fn opens_diagrams(self) -> bool {
        matches!(self, Self::Key | Self::Bpm | Self::Capo | Self::Tuning)
    }
}

impl Button {
    pub fn label(self) -> &'static str {
        match self {
            Self::Shuffle => "Shuffle",
            Self::Repeat => "Repeat",
            Self::Prev => "Previous",
            Self::Play => "Play / pause",
            Self::Next => "Next",
            Self::Volume => "Volume",
        }
    }
}

/// A list in a chosen order where each entry can be hidden. Saved as `order = [..]` (everything,
/// in order) and `hidden = [..]`, so hiding something keeps its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ordered<T: Item> {
    pub items: Vec<(T, bool)>,
}

impl<T: Item> Ordered<T> {
    pub fn new(visible: impl Fn(T) -> bool) -> Self {
        Self { items: T::ALL.iter().map(|i| (*i, visible(*i))).collect() }
    }

    pub fn shown(&self, item: T) -> bool {
        self.items.iter().any(|(i, v)| *i == item && *v)
    }

    pub fn set(&mut self, item: T, on: bool) {
        if let Some(e) = self.items.iter_mut().find(|(i, _)| *i == item) {
            e.1 = on;
        }
    }

    pub fn toggle(&mut self, item: T) {
        self.set(item, !self.shown(item));
    }

    /// The visible entries, in order.
    pub fn visible(&self) -> impl Iterator<Item = T> + '_ {
        self.items.iter().filter(|(_, v)| *v).map(|(i, _)| *i)
    }

    pub fn position(&self, item: T) -> Option<usize> {
        self.items.iter().position(|(i, _)| *i == item)
    }

    /// Move `item` by `dir` places among all entries (hidden ones included). Returns whether it moved.
    pub fn shift(&mut self, item: T, dir: i32) -> bool {
        let Some(at) = self.position(item) else { return false };
        let to = at as i32 + dir;
        if to < 0 || to >= self.items.len() as i32 {
            return false;
        }
        self.items.swap(at, to as usize);
        true
    }

    /// Move `item` so it sits where `target` is (drag and drop).
    pub fn move_to(&mut self, item: T, target: T) -> bool {
        let (Some(from), Some(to)) = (self.position(item), self.position(target)) else { return false };
        if from == to {
            return false;
        }
        let e = self.items.remove(from);
        self.items.insert(to, e);
        true
    }

    /// Read `order` / `hidden` from a TOML table. Unknown ids are reported through `warn` and
    /// skipped; anything missing is appended (visible), so a field added in a later version
    /// shows up instead of vanishing.
    fn read(&mut self, t: &toml::Table, what: &str, warn: &mut dyn FnMut(String)) {
        let mut items: Vec<(T, bool)> = Vec::new();
        let list = |key: &str, warn: &mut dyn FnMut(String)| -> Option<Vec<T>> {
            let v = t.get(key)?;
            let Some(arr) = v.as_array() else {
                warn(format!("{what}.{key}: expected a list of names; ignored"));
                return None;
            };
            let mut out = Vec::new();
            for e in arr {
                match e.as_str().and_then(T::parse) {
                    Some(i) => out.push(i),
                    None => warn(format!(
                        "{what}.{key}: unknown name {}; expected one of {}",
                        e,
                        T::ALL.iter().map(|i| i.id()).collect::<Vec<_>>().join(", ")
                    )),
                }
            }
            Some(out)
        };
        let hidden = list("hidden", warn).unwrap_or_default();
        if let Some(order) = list("order", warn) {
            for i in order {
                if !items.iter().any(|(x, _)| *x == i) {
                    items.push((i, !hidden.contains(&i)));
                }
            }
            for i in T::ALL {
                if !items.iter().any(|(x, _)| x == i) {
                    items.push((*i, !hidden.contains(i)));
                }
            }
            self.items = items;
        } else {
            for i in hidden {
                self.set(i, false);
            }
        }
    }

    fn write(&self) -> toml::Table {
        let mut t = toml::Table::new();
        let names = |f: &dyn Fn(bool) -> bool| {
            toml::Value::Array(
                self.items
                    .iter()
                    .filter(|(_, v)| f(*v))
                    .map(|(i, _)| toml::Value::String(i.id().into()))
                    .collect(),
            )
        };
        t.insert("order".into(), names(&|_| true));
        t.insert("hidden".into(), names(&|v| !v));
        t
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Split {
    #[default]
    Horizontal,
    Vertical,
}

impl Split {
    pub fn id(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

/// Smallest and largest explicit size of a panel, in percent of the space it is split from.
pub const SIZE_MIN: u16 = 10;
pub const SIZE_MAX: u16 = 80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub direction: Split,
    pub panels: Ordered<PanelId>,
    pub fields: Ordered<MetaField>,
    pub buttons: Ordered<Button>,
    /// Explicit sizes in percent; a panel not listed here gets the automatic size.
    pub sizes: Vec<(PanelId, u16)>,
    /// Cava below the lyrics instead of in a column of its own.
    pub cava_under_lyrics: bool,
}

/// Names of the built-in presets.
pub const PRESETS: &[&str] = &["default", "no-lyrics", "cava", "under-lyrics", "minimal"];

impl Default for Layout {
    fn default() -> Self {
        Self {
            direction: Split::Horizontal,
            panels: Ordered::new(|p| matches!(p, PanelId::Metadata | PanelId::Cover | PanelId::Controls | PanelId::Lyrics)),
            // everything except the optional ones that only show when a source has them: they
            // are visible and drawn only if there is a value (see the front end)
            fields: Ordered::new(|_| true),
            buttons: Ordered::new(|_| true),
            sizes: Vec::new(),
            // cava below the lyrics unless asked for a column of its own (`--thirdpanel`)
            cava_under_lyrics: true,
        }
    }
}

impl Layout {
    /// The layout a flag stands for. `None` for an unknown name.
    pub fn preset(name: &str) -> Option<Self> {
        let mut l = Self::default();
        match name.trim().to_ascii_lowercase().as_str() {
            "default" => {}
            "no-lyrics" => l.panels.set(PanelId::Lyrics, false),
            // cava in a column of its own
            "cava" => {
                l.panels.set(PanelId::Cava, true);
                l.cava_under_lyrics = false;
            }
            "under-lyrics" => {
                l.panels.set(PanelId::Cava, true);
                l.cava_under_lyrics = true;
            }
            // lyrics only; the minimal view itself is drawn by the front end
            "minimal" => {
                for p in [PanelId::Metadata, PanelId::Cover, PanelId::Controls] {
                    l.panels.set(p, false);
                }
            }
            _ => return None,
        }
        Some(l)
    }

    /// Apply the legacy flags on top (a flag always wins over the file).
    pub fn with_flags(mut self, no_lyrics: bool, cava: bool, under_lyrics: bool, third_panel: bool) -> Self {
        if no_lyrics {
            self.panels.set(PanelId::Lyrics, false);
        }
        if cava || under_lyrics || third_panel {
            self.panels.set(PanelId::Cava, true);
        }
        if under_lyrics {
            self.cava_under_lyrics = true;
        }
        if third_panel {
            self.cava_under_lyrics = false;
        }
        self
    }

    pub fn size_of(&self, p: PanelId) -> Option<u16> {
        self.sizes.iter().find(|(i, _)| *i == p).map(|(_, s)| *s)
    }

    pub fn set_size(&mut self, p: PanelId, pct: u16) {
        let pct = pct.clamp(SIZE_MIN, SIZE_MAX);
        match self.sizes.iter_mut().find(|(i, _)| *i == p) {
            Some(e) => e.1 = pct,
            None => self.sizes.push((p, pct)),
        }
    }

    pub fn clear_sizes(&mut self) {
        self.sizes.clear();
    }

    /// True when nothing was customised beyond the old flags: the front end then keeps its
    /// original automatic layout, pixel for pixel.
    pub fn is_automatic(&self) -> bool {
        self.sizes.is_empty()
            && self.direction == Split::Horizontal
            && self.column_order() == self.default_column_order()
    }

    fn default_column_order(&self) -> Vec<PanelId> {
        let mut d = Self::default();
        d.panels.set(PanelId::Lyrics, self.panels.shown(PanelId::Lyrics));
        d.panels.set(PanelId::Cava, self.panels.shown(PanelId::Cava));
        d.cava_under_lyrics = self.cava_under_lyrics;
        d.column_order()
    }

    /// The columns (or rows, when stacked) left to right: the now-playing group stands for
    /// `Metadata` at the place of its first visible member; chord diagrams dock beside the
    /// lyrics and are not a column of their own; the library is always the far-left side panel.
    pub fn column_order(&self) -> Vec<PanelId> {
        let mut out = Vec::new();
        let mut group = false;
        for p in self.panels.visible() {
            match p {
                PanelId::Metadata | PanelId::Cover | PanelId::Controls => {
                    if !group {
                        out.push(PanelId::Metadata);
                        group = true;
                    }
                }
                PanelId::Chords | PanelId::Library => {}
                PanelId::Cava if self.cava_under_lyrics => {}
                p => out.push(p),
            }
        }
        out
    }

    /// The now-playing panels in the order they stack.
    pub fn now_playing(&self) -> Vec<PanelId> {
        self.panels.visible().filter(|p| p.in_now_playing()).collect()
    }

    // ── config file ──

    /// Read the `[layout]` table of a parsed config file onto `self`. Problems go to `warn`.
    pub fn read(&mut self, root: &toml::Table, warn: &mut dyn FnMut(String)) {
        let Some(v) = root.get("layout") else { return };
        let Some(t) = v.as_table() else {
            warn("layout: expected a table ([layout]); ignored".into());
            return;
        };
        if let Some(d) = t.get("direction") {
            match d.as_str() {
                Some("horizontal") => self.direction = Split::Horizontal,
                Some("vertical") => self.direction = Split::Vertical,
                _ => warn("layout.direction: expected \"horizontal\" or \"vertical\"; using horizontal".into()),
            }
        }
        if let Some(p) = t.get("preset") {
            match p.as_str().and_then(Layout::preset) {
                Some(base) => *self = Layout { direction: self.direction, ..base },
                None => warn(format!("layout.preset: expected one of {}; ignored", PRESETS.join(", "))),
            }
        }
        self.panels.read(t, "layout", warn);
        // Files written before the cover moved to the top carry the old default order verbatim
        // (metadata, cover, controls, …): that was never a choice, so it becomes the new default.
        const OLD_DEFAULT: [PanelId; 8] = [
            PanelId::Metadata,
            PanelId::Cover,
            PanelId::Controls,
            PanelId::Lyrics,
            PanelId::Chords,
            PanelId::Cava,
            PanelId::Annotations,
            PanelId::Library,
        ];
        if self.panels.items.iter().map(|(p, _)| *p).eq(OLD_DEFAULT) {
            self.panels.items.swap(0, 1);
        }
        if let Some(b) = t.get("cava_under_lyrics") {
            match b.as_bool() {
                Some(b) => self.cava_under_lyrics = b,
                None => warn("layout.cava_under_lyrics: expected true or false; ignored".into()),
            }
        }
        if let Some(sz) = t.get("size") {
            match sz.as_table() {
                Some(sz) => {
                    self.sizes.clear();
                    for (k, v) in sz {
                        match (PanelId::parse(k), v.as_integer()) {
                            (Some(p), Some(n)) if (SIZE_MIN as i64..=SIZE_MAX as i64).contains(&n) => {
                                self.sizes.push((p, n as u16))
                            }
                            (Some(_), _) => warn(format!(
                                "layout.size.{k}: expected a whole number from {SIZE_MIN} to {SIZE_MAX}; ignored"
                            )),
                            (None, _) => warn(format!("layout.size.{k}: unknown panel; ignored")),
                        }
                    }
                }
                None => warn("layout.size: expected a table of percentages; ignored".into()),
            }
        }
        if let Some(f) = t.get("fields").and_then(|v| v.as_table()) {
            self.fields.read(f, "layout.fields", warn);
        }
        if let Some(b) = t.get("buttons").and_then(|v| v.as_table()) {
            self.buttons.read(b, "layout.buttons", warn);
        }
        for k in t.keys() {
            if !["direction", "preset", "order", "hidden", "cava_under_lyrics", "size", "fields", "buttons"]
                .contains(&k.as_str())
            {
                warn(format!("layout.{k}: unknown key (ignored)"));
            }
        }
    }

    /// The `[layout]` table to write back.
    pub fn write(&self) -> toml::Table {
        let mut t = self.panels.write();
        t.insert("direction".into(), toml::Value::String(self.direction.id().into()));
        t.insert("cava_under_lyrics".into(), toml::Value::Boolean(self.cava_under_lyrics));
        if !self.sizes.is_empty() {
            let mut sz = toml::Table::new();
            for (p, n) in &self.sizes {
                sz.insert(p.id().into(), toml::Value::Integer(*n as i64));
            }
            t.insert("size".into(), toml::Value::Table(sz));
        }
        t.insert("fields".into(), toml::Value::Table(self.fields.write()));
        t.insert("buttons".into(), toml::Value::Table(self.buttons.write()));
        t
    }
}

/// Split `total` cells among columns: explicit percentages are honoured (scaled down together if
/// they add up to more than the whole), the rest share what is left by weight. Every column gets
/// at least `min` cells while there is room. The sizes always add up to `total`.
pub fn split_sizes(total: u16, cols: &[(Option<u16>, u16)], min: u16) -> Vec<u16> {
    if cols.is_empty() {
        return Vec::new();
    }
    let total_f = total as f64;
    let explicit_sum: f64 = cols.iter().filter_map(|c| c.0).map(f64::from).sum();
    let free_w: f64 = cols.iter().filter(|c| c.0.is_none()).map(|c| f64::from(c.1.max(1))).sum();
    // explicit sizes take at most 90% when automatic columns exist
    let cap = if free_w > 0.0 { 90.0 } else { 100.0 };
    let scale = if explicit_sum > cap { cap / explicit_sum } else { 1.0 };
    let used_pct = explicit_sum * scale;
    let mut raw: Vec<f64> = cols
        .iter()
        .map(|c| match c.0 {
            Some(p) => total_f * f64::from(p) * scale / 100.0,
            None => (total_f * (100.0 - used_pct) / 100.0) * f64::from(c.1.max(1)) / free_w,
        })
        .collect();
    if free_w == 0.0 && explicit_sum > 0.0 {
        // all explicit: normalise so the row is filled
        let s: f64 = raw.iter().sum();
        raw.iter_mut().for_each(|r| *r = *r * total_f / s);
    }
    let mut out: Vec<u16> = raw.iter().map(|r| r.floor().max(0.0) as u16).collect();
    // keep every column usable while there is room
    if (min as usize) * out.len() <= total as usize {
        for i in 0..out.len() {
            if out[i] < min {
                let need = min - out[i];
                out[i] = min;
                // take it from the widest column
                if let Some(j) = (0..out.len()).filter(|j| *j != i).max_by_key(|j| out[*j]) {
                    out[j] = out[j].saturating_sub(need);
                }
            }
        }
    }
    // rounding: give the leftover to the last column, or take the excess from the widest
    let sum: u32 = out.iter().map(|v| *v as u32).sum();
    if sum < total as u32 {
        *out.last_mut().unwrap() += (total as u32 - sum) as u16;
    } else if sum > total as u32 {
        let mut extra = sum - total as u32;
        while extra > 0 {
            let j = (0..out.len()).max_by_key(|j| out[*j]).unwrap();
            if out[j] == 0 {
                break;
            }
            out[j] -= 1;
            extra -= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> (Layout, Vec<String>) {
        let root: toml::Table = s.parse().unwrap();
        let mut l = Layout::default();
        let mut w = Vec::new();
        l.read(&root, &mut |m| w.push(m));
        (l, w)
    }

    #[test]
    fn default_layout_is_the_classic_screen() {
        let l = Layout::default();
        assert!(l.is_automatic());
        assert_eq!(l.column_order(), vec![PanelId::Metadata, PanelId::Lyrics]);
        // the cover on top, the track information under it
        assert_eq!(l.now_playing(), vec![PanelId::Cover, PanelId::Metadata, PanelId::Controls]);
        assert!(l.cava_under_lyrics, "cava goes below the lyrics unless asked otherwise");
        assert_eq!(l.fields.visible().count(), 10);
        // Album can be moved above Title
        let mut f = l.fields.clone();
        assert!(f.shift(MetaField::Album, -2));
        assert_eq!(f.items[0].0, MetaField::Album);
        assert!(!f.shift(MetaField::Album, -1), "already first");
    }

    #[test]
    fn flags_are_presets() {
        // --thirdpanel: a column of its own
        let l = Layout::default().with_flags(true, false, false, true);
        assert!(!l.panels.shown(PanelId::Lyrics) && l.panels.shown(PanelId::Cava) && !l.cava_under_lyrics);
        assert_eq!(l.column_order(), vec![PanelId::Metadata, PanelId::Cava]);
        // --cava alone keeps the default: below the lyrics
        let l = Layout::default().with_flags(false, true, false, false);
        assert!(l.panels.shown(PanelId::Cava) && l.cava_under_lyrics);
        let l = Layout::default().with_flags(false, false, true, false);
        assert!(l.cava_under_lyrics);
        assert_eq!(l.column_order(), vec![PanelId::Metadata, PanelId::Lyrics], "cava lives under the lyrics");
        assert_eq!(Layout::preset("cava").unwrap().column_order().len(), 3, "the cava preset is a column of its own");
        assert!(Layout::preset("under-lyrics").unwrap().cava_under_lyrics);
        assert!(Layout::preset("nope").is_none());
        for n in PRESETS {
            assert!(Layout::preset(n).is_some(), "{n}");
        }
    }

    #[test]
    fn config_round_trip_keeps_order_and_hidden() {
        let mut l = Layout::default();
        l.panels.set(PanelId::Cava, true);
        l.panels.shift(PanelId::Lyrics, -1);
        l.fields.set(MetaField::Genre, false);
        l.fields.shift(MetaField::Album, -2);
        l.buttons.set(PanelButton::Shuffle.into_button(), false);
        l.set_size(PanelId::Lyrics, 55);
        l.direction = Split::Vertical;
        let mut root = toml::Table::new();
        root.insert("layout".into(), toml::Value::Table(l.write()));
        let text = toml::to_string(&root).unwrap();
        let (back, warns) = parse(&text);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(back, l);
    }

    // helper so the test reads naturally
    enum PanelButton {
        Shuffle,
    }
    impl PanelButton {
        fn into_button(self) -> Button {
            match self {
                Self::Shuffle => Button::Shuffle,
            }
        }
    }

    #[test]
    fn bad_input_warns_and_keeps_going() {
        let (l, w) = parse(
            "[layout]\ndirection = \"diagonal\"\norder = [\"lyrics\", \"bogus\", \"metadata\"]\n[layout.size]\nlyrics = 5\nbogus = 30\n[layout.fields]\nhidden = [\"genre\", \"mood\"]\n",
        );
        assert_eq!(l.direction, Split::Horizontal);
        assert!(w.iter().any(|m| m.contains("layout.direction")));
        assert!(w.iter().any(|m| m.contains("bogus") && m.contains("one of")));
        assert!(w.iter().any(|m| m.contains("layout.size.lyrics")));
        assert!(w.iter().any(|m| m.contains("layout.size.bogus")));
        assert!(w.iter().any(|m| m.contains("mood")));
        assert!(!l.fields.shown(MetaField::Genre));
        // lyrics listed first; everything not listed is still known (appended)
        assert_eq!(l.panels.items[0].0, PanelId::Lyrics);
        assert_eq!(l.panels.items.len(), PanelId::ALL.len());
    }

    #[test]
    fn the_old_default_order_is_migrated_but_a_chosen_one_is_kept() {
        // written by the first version, when the track information came before the cover
        let old = "[layout]\norder = [\"metadata\", \"cover\", \"controls\", \"lyrics\", \"chords\", \"cava\", \"annotations\", \"library\"]\nhidden = [\"chords\", \"cava\"]\n";
        let (l, w) = parse(old);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(l.now_playing(), vec![PanelId::Cover, PanelId::Metadata, PanelId::Controls]);
        assert!(!l.panels.shown(PanelId::Chords) && !l.panels.shown(PanelId::Cava), "visibility survives");
        // a deliberate rearrangement is left alone
        let own = "[layout]\norder = [\"metadata\", \"controls\", \"cover\", \"lyrics\", \"chords\", \"cava\", \"annotations\", \"library\"]\n";
        let (l, _) = parse(own);
        assert_eq!(l.now_playing(), vec![PanelId::Metadata, PanelId::Controls, PanelId::Cover]);
    }

    #[test]
    fn missing_entries_are_appended_visible() {
        let (l, _) = parse("[layout.fields]\norder = [\"album\", \"title\"]\n");
        assert_eq!(l.fields.items[0].0, MetaField::Album);
        assert_eq!(l.fields.items.len(), MetaField::ALL.len());
        assert!(l.fields.shown(MetaField::Key));
    }

    #[test]
    fn drag_and_drop_moves_before_target() {
        let mut f = Layout::default().fields;
        assert!(f.move_to(MetaField::Key, MetaField::Title));
        assert_eq!(f.items[0].0, MetaField::Key);
        assert!(!f.move_to(MetaField::Key, MetaField::Key));
    }

    #[test]
    fn preset_and_custom_order_are_not_automatic() {
        let mut l = Layout::default();
        assert!(l.is_automatic());
        l.panels.shift(PanelId::Lyrics, -3);
        assert!(!l.is_automatic());
        let mut l = Layout::default();
        l.set_size(PanelId::Lyrics, 50);
        assert!(!l.is_automatic());
        assert_eq!(l.size_of(PanelId::Lyrics), Some(50));
        l.set_size(PanelId::Lyrics, 99);
        assert_eq!(l.size_of(PanelId::Lyrics), Some(SIZE_MAX));
    }

    #[test]
    fn sizes_always_add_up() {
        for total in [0u16, 1, 7, 40, 80, 119, 200] {
            for cols in [
                vec![(None, 1)],
                vec![(Some(40), 1), (None, 1)],
                vec![(Some(60), 1), (Some(60), 1)],
                vec![(Some(30), 1), (None, 2), (Some(30), 1)],
                vec![(None, 40), (None, 60)],
            ] {
                let s = split_sizes(total, &cols, 8);
                assert_eq!(s.iter().map(|v| *v as u32).sum::<u32>(), total as u32, "{total} {cols:?} -> {s:?}");
            }
        }
        // explicit share is respected
        let s = split_sizes(100, &[(Some(30), 1), (None, 1)], 8);
        assert_eq!(s, vec![30, 70]);
        // minimum width wins over a tiny share
        let s = split_sizes(100, &[(Some(10), 1), (None, 1)], 15);
        assert_eq!(s[0], 15);
    }
}
