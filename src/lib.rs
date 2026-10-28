//! music_display core: everything that is not terminal code (players, lyrics, chords,
//! translation, Spotify, config, cover decoding, cava), so another front end can reuse it.
//!
//! Every slow thing (subprocesses, HTTP, image decode, cava) runs in its own task and reports
//! back over an unbounded mpsc channel as an [`AppEvent`]. Results are tagged with the track
//! key they were requested for, so stale results from a skipped track are dropped. The front
//! end (`src/main.rs`, the terminal UI) owns all state in one `tokio` select loop.

pub mod cli;
pub mod config;
pub mod util;
pub mod color;
pub mod player;
// Windows replacement for playerctl: the system media session (SMTC)
#[cfg(any(windows, test))]
pub mod smtc;
pub mod spotify;
pub mod spotifyd;
pub mod lyrics;
pub mod translate;
pub mod chords;
pub mod ug;
pub mod genius;
pub mod year;
pub mod tags;
pub mod cover;
pub mod cava;
pub mod diagram;
pub mod shutdown;
pub mod setup;
pub mod layout;
pub mod genre;
pub mod ai;
pub mod annotations;

/// What the background tasks report to whichever front end owns the state. Results are tagged
/// with the track they were requested for, so stale ones can be dropped.
pub enum AppEvent {
    Player(Option<player::Snapshot>),
    Year {
        key: u64,
        year: Option<String>,
    },
    Lyrics {
        key: u64,
        lyrics: lyrics::Lyrics,
    },
    /// Genius annotations of the track.
    Annotations {
        key: u64,
        result: Result<Vec<annotations::Annotation>, String>,
    },
    /// AI-written notes, one optional note per lyric line.
    AiNotes {
        key: u64,
        result: Result<Vec<Option<String>>, String>,
    },
    /// Genres from MusicBrainz for the track (`None` = nothing usable).
    Genre {
        key: u64,
        genres: Option<Vec<String>>,
    },
    /// Every lyrics version lrclib has for the song (kind, text), best first.
    LyricVersions {
        key: u64,
        list: Vec<(String, String)>,
    },
    Cover {
        url: String,
        loaded: Option<cover::Loaded>,
    },
    Chords {
        key: u64,
        sheet: Result<Option<chords::Sheet>, String>,
        /// Saved per-track capo override, if any.
        capo: Option<u8>,
    },
    /// Tags read from a local file (`None` = unreadable).
    Tags {
        url: String,
        tags: Option<tags::Tags>,
    },
    Translated {
        key: u64,
        lang: String,
        lines: Result<(Vec<String>, String), String>,
    },
    /// Lines with the members singing them, from Genius (empty = nothing usable).
    Members {
        key: u64,
        parts: Vec<genius::Part>,
    },
    Cava(Vec<u16>),
    /// Informational status while cava has produced nothing yet.
    CavaNote(String),
    /// cava failed or exited; the task is gone and `c` twice restarts it.
    CavaError(String),
    /// Spotify Web API results (`--spotify`).
    Spotify(spotify::Reply),
}
