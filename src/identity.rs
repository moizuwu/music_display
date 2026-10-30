//! Which song a player snapshot is, and what to keep when the next poll spells it differently.
//! Pure rules with no front-end state, shared by the terminal UI and the window so both treat
//! a flapping player, a second player or a remaster the same way.
use crate::player::{Snapshot, Status};
use crate::tags::{self, Tags};
use std::time::Duration;

/// After the user changes the volume, player polls are ignored this long so the slider does
/// not snap back to the old value before the player has caught up.
pub const VOL_HOLD: Duration = Duration::from_millis(1500);
/// The player on screen counts as gone when it has not reported in for this long.
pub const PLAYER_QUIET: Duration = Duration::from_secs(3);

/// Several players at once (`any`, a list): stay with the one being shown while it reports in,
/// unless another one starts playing while ours does not. True = drop this snapshot.
pub fn ignore_other_player(shown: Option<&Snapshot>, s: &Snapshot, seen_ago: Duration) -> bool {
    let Some(o) = shown else { return false };
    if s.player.is_empty() || o.player.is_empty() || s.player == o.player {
        return false;
    }
    let ours_quiet = seen_ago > PLAYER_QUIET;
    let takeover = s.status == Status::Playing && o.status != Status::Playing;
    !ours_quiet && !takeover
}

/// A snapshot whose strings changed but that is still the item already playing: the player's
/// own id is unchanged, or the position runs on where it was (a real track change starts near
/// 0 or jumps) with the same length. `shown_pos` is the interpolated position on screen.
pub fn same_recording(shown: Option<&Snapshot>, track_key: u64, shown_pos: f64, s: &Snapshot) -> bool {
    let Some(o) = shown else { return false };
    if track_key == 0 || o.player != s.player {
        return false;
    }
    // Some players keep one id or url for everything they play (a browser tab, a stream), so a
    // match only counts while the position has not gone back to the start: a new song does.
    let restarted = s.position + 3.0 < shown_pos;
    if !restarted && !s.track_id.is_empty() && s.track_id == o.track_id {
        return true;
    }
    if !restarted && !s.url.is_empty() && s.url == o.url {
        return true;
    }
    let same_len = o.length > 0.0 && (o.length - s.length).abs() <= 1.0;
    let runs_on = s.position > 2.0 && (shown_pos - s.position).abs() < 1.5;
    same_len && runs_on
}

/// Same track, different spelling of the strings (edition names, art): keep what is on screen
/// and only fill in what was missing. `keep_art` is the cover URL in use, when it should stay.
pub fn keep_known(old: &Snapshot, new: &mut Snapshot, keep_art: Option<&str>) {
    let keep = |old: &str, new: &mut String| {
        if !old.is_empty() {
            *new = old.to_string();
        }
    };
    keep(&old.title, &mut new.title);
    keep(&old.artist, &mut new.artist);
    keep(&old.album, &mut new.album);
    keep(&old.content_created, &mut new.content_created);
    keep(&old.genre, &mut new.genre);
    if let Some(url) = keep_art.filter(|u| !u.is_empty()) {
        new.art_url = url.to_string();
    }
    new.track_no = old.track_no.or(new.track_no);
    new.disc_no = old.disc_no.or(new.disc_no);
}

/// Fill what the player left empty from the file's own tags.
pub fn merge_tags(s: &mut Snapshot, t: &Tags) {
    let fill = |dst: &mut String, src: &str| {
        if dst.is_empty() && !src.is_empty() {
            *dst = src.to_string();
        }
    };
    // a title that is only the file name gives way to the real one
    if !t.title.is_empty() && s.title == tags::title_from_url(&s.url) {
        s.title = t.title.clone();
    }
    fill(&mut s.artist, &t.artist);
    fill(&mut s.album, &t.album);
    fill(&mut s.content_created, &t.year);
    fill(&mut s.genre, &t.genre);
    fill(&mut s.art_url, &t.art_url);
    s.track_no = s.track_no.or(t.track);
    s.disc_no = s.disc_no.or(t.disc);
}

/// Has the volume hold run out, so a polled volume may be taken over?
pub fn volume_settled(since_change: Duration) -> bool {
    since_change > VOL_HOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(player: &str, status: &str, pos: f64, id: &str) -> Snapshot {
        let line = format!(
            "{status}\x1f{}\x1fSong (Remastered)\x1fBand\x1fAlbum\x1f200000000\x1f\x1f2\x1f1\x1f1999-01-02T00:00:00Z\x1f0.5\x1f\x1f{id}\x1f{player}\x1f[Rock]",
            (pos * 1_000_000.0) as u64
        );
        crate::player::parse(&line).unwrap()
    }

    #[test]
    fn a_second_player_is_ignored_until_ours_is_quiet_or_it_takes_over() {
        let ours = snap("vlc", "Playing", 10.0, "a");
        let other = snap("spotify", "Paused", 3.0, "b");
        assert!(ignore_other_player(Some(&ours), &other, Duration::from_secs(1)));
        assert!(!ignore_other_player(Some(&ours), &other, Duration::from_secs(5)), "ours went quiet");
        let playing = snap("spotify", "Playing", 3.0, "b");
        let paused = snap("vlc", "Paused", 10.0, "a");
        assert!(!ignore_other_player(Some(&paused), &playing, Duration::from_secs(1)), "takeover");
        assert!(ignore_other_player(Some(&ours), &playing, Duration::from_secs(1)), "both playing: stay");
        assert!(!ignore_other_player(Some(&ours), &snap("vlc", "Playing", 11.0, "a"), Duration::ZERO));
        assert!(!ignore_other_player(None, &other, Duration::ZERO));
    }

    #[test]
    fn the_same_recording_is_told_by_id_url_or_a_running_position() {
        let o = snap("vlc", "Playing", 30.0, "a");
        // same id, whatever else changed
        assert!(same_recording(Some(&o), 7, 30.0, &snap("vlc", "Playing", 31.0, "a")));
        // another id, but it runs on where we were with the same length
        assert!(same_recording(Some(&o), 7, 30.0, &snap("vlc", "Playing", 30.5, "b")));
        // a player that keeps one id for every song: a restart is still a new song
        assert!(!same_recording(Some(&o), 7, 120.0, &snap("vlc", "Playing", 1.0, "a")));
        // a jump or a restart is a different item
        assert!(!same_recording(Some(&o), 7, 30.0, &snap("vlc", "Playing", 1.0, "b")));
        assert!(!same_recording(Some(&o), 7, 30.0, &snap("vlc", "Playing", 90.0, "b")));
        // nothing shown yet, or another player
        assert!(!same_recording(Some(&o), 0, 30.0, &snap("vlc", "Playing", 31.0, "a")));
        assert!(!same_recording(Some(&o), 7, 30.0, &snap("mpv", "Playing", 31.0, "a")));
        assert!(!same_recording(None, 7, 30.0, &o));
    }

    #[test]
    fn a_second_title_with_a_constant_id_is_not_the_same_recording() {
        let mut o = snap("firefox", "Playing", 150.0, "/org/mpris/MediaPlayer2/firefox");
        o.title = "Into The Sun".into();
        let mut n = snap("firefox", "Playing", 0.5, "/org/mpris/MediaPlayer2/firefox");
        n.title = "My Universe".into();
        assert!(!same_recording(Some(&o), 7, 150.0, &n));
    }

    #[test]
    fn what_was_known_is_kept_and_gaps_are_filled() {
        let mut old = snap("vlc", "Playing", 30.0, "a");
        old.title = "Song".into();
        old.genre = "Blues".into();
        old.track_no = Some(4);
        let mut new = snap("vlc", "Playing", 31.0, "a");
        new.genre = String::new();
        new.content_created = String::new();
        new.track_no = None;
        new.disc_no = Some(2);
        new.art_url = "file:///new.jpg".into();
        keep_known(&old, &mut new, Some("file:///old.jpg"));
        assert_eq!((new.title.as_str(), new.genre.as_str()), ("Song", "Blues"));
        assert_eq!(new.content_created, old.content_created);
        assert_eq!((new.track_no, new.disc_no), (Some(4), Some(1)));
        assert_eq!(new.art_url, "file:///old.jpg");
        let mut again = snap("vlc", "Playing", 31.0, "a");
        again.art_url = "file:///new.jpg".into();
        keep_known(&old, &mut again, None);
        assert_eq!(again.art_url, "file:///new.jpg", "a failed cover is allowed to change");
    }

    #[test]
    fn local_tags_fill_gaps_and_replace_a_file_name_title() {
        let mut s = snap("vlc", "Playing", 1.0, "a");
        s.url = "file:///music/01%20Intro.flac".into();
        s.title = "01 Intro".into();
        s.artist = String::new();
        s.genre = String::new();
        s.track_no = None;
        let t = Tags { title: "Intro".into(), artist: "Band".into(), genre: "Rock".into(), track: Some(1), ..Default::default() };
        merge_tags(&mut s, &t);
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.genre.as_str(), s.track_no), ("Intro", "Band", "Rock", Some(1)));
        // a real title stays
        let mut s2 = snap("vlc", "Playing", 1.0, "a");
        s2.url = "file:///music/x.flac".into();
        merge_tags(&mut s2, &t);
        assert_eq!(s2.title, "Song (Remastered)");
    }

    #[test]
    fn the_volume_hold_runs_out() {
        assert!(!volume_settled(Duration::from_millis(900)));
        assert!(volume_settled(Duration::from_secs(2)));
    }
}
