use super::*;

/// Everything a key can do. The input handler and the `?` help overlay both read
/// [`BINDS`], so the two can't drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Quit,
    PlayPause,
    Next,
    Prev,
    Shuffle,
    Repeat,
    SeekBack,
    SeekFwd,
    SyncEarlier,
    SyncLater,
    VolUp,
    VolDown,
    Mute,
    ToggleLyrics,
    ToggleBig,
    Karaoke,
    NextLyrics,
    BigScale,
    LineSpacing,
    LyricPad,
    ToggleCava,
    ToggleChords,
    TransposeUp,
    TransposeDown,
    CapoDown,
    CapoUp,
    CapoReset,
    Diagrams,
    Minimal,
    Translate,
    TranslateBoth,
    Settings,
    Library,
    Like,
    PlayHere,
    Devices,
    BrowseUp,
    BrowseDown,
    BrowsePageUp,
    BrowsePageDown,
    BrowseEnd,
    Help,
}

/// The key editor inside the help overlay (`e` there).
#[derive(Clone, Debug, Default)]
pub struct HelpEdit {
    /// The selected row (an index into the key table).
    pub sel: usize,
    /// Waiting for the key to give the selected action.
    pub capture: bool,
    pub note: Option<String>,
}

pub struct Bind {
    pub group: &'static str,
    /// How the key is written in the help overlay.
    pub keys: &'static str,
    pub desc: &'static str,
    pub codes: &'static [KeyCode],
    pub action: Action,
}

macro_rules! bind {
    ($g:literal, $k:literal, $d:literal, [$($c:expr),+], $a:ident) => {
        Bind { group: $g, keys: $k, desc: $d, codes: &[$($c),+], action: Action::$a }
    };
}

pub const GROUPS: [&str; 5] = ["Playback", "Lyrics", "Chords", "View", "Spotify"];

pub const BINDS: &[Bind] = &[
    bind!("Playback", "space", "play / pause", [KeyCode::Char(' ')], PlayPause),
    bind!("Playback", "n", "next track", [KeyCode::Char('n')], Next),
    bind!("Playback", "p", "previous track", [KeyCode::Char('p')], Prev),
    bind!("Playback", "z", "shuffle on / off", [KeyCode::Char('z')], Shuffle),
    bind!("Playback", "r", "repeat: off / playlist / track", [KeyCode::Char('r')], Repeat),
    bind!("Playback", "←", "seek back 5 s", [KeyCode::Left], SeekBack),
    bind!("Playback", "→", "seek forward 5 s", [KeyCode::Right], SeekFwd),
    bind!("Playback", "+", "volume up", [KeyCode::Char('+'), KeyCode::Char('=')], VolUp),
    bind!("Playback", "-", "volume down", [KeyCode::Char('-')], VolDown),
    bind!("Playback", "m", "mute / unmute", [KeyCode::Char('m')], Mute),
    bind!("Lyrics", ",", "lyrics 0.1 s later", [KeyCode::Char(',')], SyncEarlier),
    bind!("Lyrics", ".", "lyrics 0.1 s earlier", [KeyCode::Char('.')], SyncLater),
    bind!("Lyrics", "l", "show / hide lyrics", [KeyCode::Char('l')], ToggleLyrics),
    bind!("Lyrics", "b", "big-font active line", [KeyCode::Char('b')], ToggleBig),
    bind!("Lyrics", "a", "light up the line word by word", [KeyCode::Char('a')], Karaoke),
    bind!("Lyrics", "L", "next lyrics version from lrclib (e.g. Hangul instead of romanized)", [KeyCode::Char('L')], NextLyrics),
    bind!("Lyrics", "x", "big-font size 1 / 2 / 3", [KeyCode::Char('x')], BigScale),
    bind!("Lyrics", "i", "line spacing 0-3", [KeyCode::Char('i')], LineSpacing),
    bind!("Lyrics", "u", "side padding 0-10", [KeyCode::Char('u')], LyricPad),
    bind!("Chords", "g", "chords on / off", [KeyCode::Char('g')], ToggleChords),
    bind!("Chords", "t", "transpose up", [KeyCode::Char('t')], TransposeUp),
    bind!("Chords", "T", "transpose down", [KeyCode::Char('T')], TransposeDown),
    bind!("Chords", "]", "capo up", [KeyCode::Char(']')], CapoUp),
    bind!("Chords", "[", "capo down", [KeyCode::Char('[')], CapoDown),
    bind!("Chords", "0", "sheet's own capo", [KeyCode::Char('0')], CapoReset),
    bind!("Chords", "D", "chord diagrams", [KeyCode::Char('D')], Diagrams),
    bind!("View", "M", "minimal view", [KeyCode::Char('M')], Minimal),
    bind!("View", "c", "cava visualiser", [KeyCode::Char('c')], ToggleCava),
    bind!("Lyrics", "↑", "browse lyrics up (playback goes on)", [KeyCode::Up], BrowseUp),
    bind!("Lyrics", "↓", "browse lyrics down", [KeyCode::Down], BrowseDown),
    bind!("Lyrics", "PgUp", "browse a page up", [KeyCode::PageUp], BrowsePageUp),
    bind!("Lyrics", "PgDn", "browse a page down", [KeyCode::PageDown], BrowsePageDown),
    bind!("Lyrics", "End", "back to the playing line", [KeyCode::End], BrowseEnd),
    bind!("Lyrics", "K", "translate lyrics on / off (translation only)", [KeyCode::Char('K')], Translate),
    bind!("Lyrics", "k", "translation under each original line on / off", [KeyCode::Char('k')], TranslateBoth),
    bind!("View", "s / S", "settings menu", [KeyCode::Char('s'), KeyCode::Char('S')], Settings),
    bind!("View", "?", "this help", [KeyCode::Char('?')], Help),
    bind!("View", "q / Esc", "quit", [KeyCode::Char('q'), KeyCode::Esc], Quit),
    bind!("Spotify", "o", "library: playlists, liked, albums…", [KeyCode::Char('o')], Library),
    bind!("Spotify", "f", "like / unlike this song", [KeyCode::Char('f')], Like),
    bind!("Spotify", "H", "play here (move to this PC)", [KeyCode::Char('H')], PlayHere),
    bind!("Spotify", "d", "choose a Spotify device", [KeyCode::Char('d')], Devices),
];

/// Keys inside the library browser (documented in the help and the README).
pub const LIBRARY_KEYS: &[(&str, &str)] = &[
    ("Tab / 1-7", "switch tab (6 search, 7 queue)"),
    ("Q", "the queue: now playing, next up"),
    ("/", "search (Enter runs it)"),
    ("Enter", "play"),
    ("→ / ←", "open / back"),
    ("a / A", "queue / add to playlist"),
    ("f / r", "like / reload"),
    ("d", "choose a device"),
];

/// Mouse gestures (they are hit-tested where they are drawn, so this is documentation
/// kept next to the table): (group, gesture, what it does).
pub const MOUSE: &[(&str, &str, &str)] = &[
    ("Playback", "click bar", "seek there"),
    ("Playback", "click buttons", "shuf / rep / prev / play / next"),
    ("Playback", "click - / +", "volume down / up"),
    ("Playback", "click vol %", "mute / restore"),
    ("Playback", "wheel: volume", "volume up / down"),
    ("Lyrics", "click a line's text", "seek to that line"),
];

/// The action of `code` in the default table (tests; the program uses `App::keys`).
#[cfg(test)]
pub fn action_for(code: KeyCode) -> Option<Action> {
    BINDS
        .iter()
        .find(|b| b.codes.contains(&code))
        .map(|b| b.action)
}
