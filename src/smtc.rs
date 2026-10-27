//! Windows media backend: the System Media Transport Controls (the same sessions the
//! volume flyout and the lock screen show) stand in for playerctl. The blocking WinRT
//! calls are run from `spawn_blocking` by `player::Ctl`.

use crate::player::{Loop, Status};

// --- plain helpers, kept free of WinRT types so the tests run on every platform ---

/// SMTC times are 100 ns ticks.
const TICKS: f64 = 10_000_000.0;
/// Seconds between 1601-01-01 (the Windows epoch) and 1970-01-01.
#[cfg(any(windows, test))]
const EPOCH_GAP: f64 = 11_644_473_600.0;

#[cfg(any(windows, test))]
fn secs(ticks: i64) -> f64 {
    (ticks as f64 / TICKS).max(0.0)
}

#[cfg(any(windows, test))]
fn ticks(secs: f64) -> i64 {
    (secs.max(0.0) * TICKS) as i64
}

/// `Spotify.exe`, `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`, `chrome.exe` → the
/// short lower-case name the rest of the app compares against (`spotify`, `chrome`).
#[cfg(any(windows, test))]
fn app_name(app_id: &str) -> String {
    let id = app_id.rsplit('!').next().unwrap_or(app_id);
    let id = id.strip_suffix(".exe").or_else(|| id.strip_suffix(".EXE")).unwrap_or(id);
    id.to_lowercase()
}

/// Does the session of `app_id` belong to the wanted player (empty = any)?
#[cfg(any(windows, test))]
fn matches(app_id: &str, wanted: &str) -> bool {
    wanted.is_empty() || app_id.to_lowercase().contains(&wanted.to_lowercase())
}

/// A `playerctl position` argument: `12+` / `5-` are relative, anything else absolute.
#[cfg(any(windows, test))]
fn target_position(arg: &str, current: f64) -> Option<f64> {
    let arg = arg.trim();
    let t = if let Some(d) = arg.strip_suffix('+') {
        current + d.parse::<f64>().ok()?
    } else if let Some(d) = arg.strip_suffix('-') {
        current - d.parse::<f64>().ok()?
    } else {
        arg.parse::<f64>().ok()?
    };
    Some(t.max(0.0))
}

/// The timeline position is a snapshot taken at `updated` (Windows ticks since 1601);
/// while playing the real position has moved on by the time since.
#[cfg(any(windows, test))]
fn live_position(position: f64, playing: bool, updated: i64, now_unix: f64) -> f64 {
    if !playing {
        return position;
    }
    let behind = now_unix + EPOCH_GAP - updated as f64 / TICKS;
    // a clock that disagrees with the player's must not throw the bar around
    position + behind.clamp(0.0, 3600.0)
}

/// `MediaPlaybackAutoRepeatMode`: 0 none, 1 track, 2 list.
#[cfg(any(windows, test))]
fn loop_of(mode: i32) -> Loop {
    match mode {
        1 => Loop::Track,
        2 => Loop::Playlist,
        _ => Loop::None,
    }
}

#[cfg(any(windows, test))]
fn mode_of(l: Loop) -> i32 {
    match l {
        Loop::None => 0,
        Loop::Track => 1,
        Loop::Playlist => 2,
    }
}

#[cfg(any(windows, test))]
fn loop_from_arg(arg: &str) -> Option<Loop> {
    crate::player::parse_loop(arg)
}

/// `GlobalSystemMediaTransportControlsSessionPlaybackStatus`: 4 playing, 5 paused.
#[cfg(any(windows, test))]
fn status_of(code: i32) -> Status {
    match code {
        4 => Status::Playing,
        5 => Status::Paused,
        _ => Status::Stopped,
    }
}

// --- the WinRT part ---

#[cfg(windows)]
mod winrt {
    use super::*;
    use crate::player::Snapshot;
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
    };
    use windows::Media::MediaPlaybackAutoRepeatMode;
    use windows::Storage::Streams::DataReader;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    thread_local! {
        /// The blocking pool's threads have no apartment yet; join the multithreaded one
        /// (an error just means the thread already has one).
        static APARTMENT: () = unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        };
    }

    fn session(wanted: &str) -> Option<(Session, String)> {
        APARTMENT.with(|_| {});
        let manager = Manager::RequestAsync().ok()?.join().ok()?;
        if wanted.is_empty()
            && let Ok(s) = manager.GetCurrentSession()
        {
            let id = s.SourceAppUserModelId().ok()?.to_string();
            return Some((s, id));
        }
        for s in manager.GetSessions().ok()? {
            let Ok(id) = s.SourceAppUserModelId().map(|i| i.to_string()) else {
                continue;
            };
            if matches(&id, wanted) {
                return Some((s, id));
            }
        }
        None
    }

    fn now_unix() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    /// The cover as a `file://` URL: SMTC hands out a stream, the app wants a URL.
    fn cover(props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties, key: &str) -> String {
        let read = || -> windows::core::Result<Vec<u8>> {
            let stream = props.Thumbnail()?.OpenReadAsync()?.join()?;
            let size = stream.Size()? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            reader.LoadAsync(size)?.join()?;
            let mut buf = vec![0u8; size as usize];
            reader.ReadBytes(&mut buf)?;
            Ok(buf)
        };
        let Ok(bytes) = read() else {
            return String::new();
        };
        if bytes.is_empty() {
            return String::new();
        }
        let dir = crate::util::cache_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("smtc-art-{:x}", crate::util::fnv1a(key)));
        if !path.exists() && std::fs::write(&path, &bytes).is_err() {
            return String::new();
        }
        crate::util::file_url(&path)
    }

    pub fn snapshot(wanted: &str) -> Option<Snapshot> {
        let (s, app_id) = session(wanted)?;
        let props = s.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let title = props.Title().ok()?.to_string();
        let artist = props.Artist().ok()?.to_string();
        if title.is_empty() && artist.is_empty() {
            return None;
        }
        let info = s.GetPlaybackInfo().ok()?;
        let status = status_of(info.PlaybackStatus().ok()?.0);
        let shuffle = info.IsShuffleActive().ok().and_then(|b| b.Value().ok());
        let repeat = info
            .AutoRepeatMode()
            .ok()
            .and_then(|m| m.Value().ok())
            .map(|m| loop_of(m.0));
        let (mut position, mut length) = (0.0, 0.0);
        if let Ok(t) = s.GetTimelineProperties() {
            let start = t.StartTime().map(|d| d.Duration).unwrap_or(0);
            let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
            length = secs(end - start);
            position = secs(t.Position().map(|d| d.Duration).unwrap_or(0) - start);
            let updated = t.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
            position = live_position(position, status == Status::Playing, updated, now_unix());
            if length > 0.0 {
                position = position.min(length);
            }
        }
        let key = format!("{app_id}\0{artist}\0{title}\0{}", props.AlbumTitle().ok()?);
        Some(Snapshot {
            status,
            position,
            title,
            artist,
            album: props.AlbumTitle().ok()?.to_string(),
            length,
            art_url: cover(&props, &key),
            track_no: props.TrackNumber().ok().filter(|n| *n > 0).map(|n| n as u32),
            disc_no: None,
            content_created: String::new(),
            volume: None,
            url: String::new(),
            track_id: String::new(),
            player: app_name(&app_id),
            shuffle,
            repeat,
        })
    }

    /// One command in the playerctl spelling; unknown ones (volume) are ignored.
    pub fn command(wanted: &str, args: &[String]) {
        let Some((s, _)) = session(wanted) else {
            return;
        };
        let arg = |i: usize| args.get(i).map(|a| a.as_str()).unwrap_or("");
        match arg(0) {
            "play-pause" => {
                let _ = s.TryTogglePlayPauseAsync().and_then(|o| o.join());
            }
            "next" => {
                let _ = s.TrySkipNextAsync().and_then(|o| o.join());
            }
            "previous" => {
                let _ = s.TrySkipPreviousAsync().and_then(|o| o.join());
            }
            "shuffle" => {
                let on = s
                    .GetPlaybackInfo()
                    .and_then(|i| i.IsShuffleActive())
                    .and_then(|b| b.Value())
                    .unwrap_or(false);
                let _ = s.TryChangeShuffleActiveAsync(!on).and_then(|o| o.join());
            }
            "loop" => {
                if let Some(l) = loop_from_arg(arg(1)) {
                    let mode = MediaPlaybackAutoRepeatMode(mode_of(l));
                    let _ = s.TryChangeAutoRepeatModeAsync(mode).and_then(|o| o.join());
                }
            }
            "position" => {
                let now = s
                    .GetTimelineProperties()
                    .and_then(|t| t.Position())
                    .map(|d| secs(d.Duration))
                    .unwrap_or(0.0);
                if let Some(t) = target_position(arg(1), now) {
                    let _ = s.TryChangePlaybackPositionAsync(ticks(t)).and_then(|o| o.join());
                }
            }
            _ => {}
        }
    }
}

#[cfg(windows)]
pub use winrt::{command, snapshot};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_become_short_names() {
        assert_eq!(app_name("Spotify.exe"), "spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "spotify");
        assert_eq!(app_name("chrome.exe"), "chrome");
        assert_eq!(app_name("MusicBee"), "musicbee");
    }

    #[test]
    fn a_wanted_player_is_a_substring_of_the_app_id() {
        assert!(matches("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "spotify"));
        assert!(matches("Chrome", ""));
        assert!(!matches("vlc.exe", "spotify"));
    }

    #[test]
    fn positions_are_absolute_or_relative() {
        assert_eq!(target_position("12.5", 40.0), Some(12.5));
        assert_eq!(target_position("5+", 40.0), Some(45.0));
        assert_eq!(target_position("5-", 40.0), Some(35.0));
        assert_eq!(target_position("10-", 4.0), Some(0.0));
        assert_eq!(target_position("x", 4.0), None);
    }

    #[test]
    fn ticks_are_tenth_microseconds() {
        assert_eq!(secs(15_000_000), 1.5);
        assert_eq!(ticks(1.5), 15_000_000);
        assert_eq!(secs(-5), 0.0);
    }

    #[test]
    fn a_playing_position_moves_on_since_the_update() {
        let updated = ((100.0 + EPOCH_GAP) * TICKS) as i64;
        assert!((live_position(10.0, true, updated, 103.0) - 13.0).abs() < 1e-6);
        assert_eq!(live_position(10.0, false, updated, 103.0), 10.0);
        // a clock running behind the player's never moves the bar back
        assert_eq!(live_position(10.0, true, updated, 90.0), 10.0);
    }

    #[test]
    fn repeat_and_status_codes_map() {
        for l in [Loop::None, Loop::Track, Loop::Playlist] {
            assert_eq!(loop_of(mode_of(l)), l);
        }
        assert_eq!(status_of(4), Status::Playing);
        assert_eq!(status_of(5), Status::Paused);
        assert_eq!(status_of(3), Status::Stopped);
    }
}
