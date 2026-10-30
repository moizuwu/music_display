//! The window's tie to the rest of the process: the event channel that wakes the window, the
//! window title from `window_title`, termination signals and the spotifyd it may have started.
use music_display::player::Snapshot;
use music_display::{AppEvent, shutdown};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The egui context, once the window exists (events can arrive before that).
pub type Wake = Arc<OnceLock<egui::Context>>;

/// An event channel whose every message asks the window to repaint. The background tasks keep
/// the plain sender they have always used; a forwarder in between wakes the window, so a
/// paused window (no timer repaint) still shows a new song or an arriving cover at once.
pub fn channel() -> (UnboundedSender<AppEvent>, UnboundedReceiver<AppEvent>, Wake) {
    let (tx, mut raw) = unbounded_channel::<AppEvent>();
    let (out_tx, out_rx) = unbounded_channel::<AppEvent>();
    let wake: Wake = Arc::new(OnceLock::new());
    let w = wake.clone();
    tokio::spawn(async move {
        while let Some(ev) = raw.recv().await {
            if out_tx.send(ev).is_err() {
                return;
            }
            if let Some(ctx) = w.get() {
                ctx.request_repaint();
            }
        }
    });
    (tx, out_rx, wake)
}

/// SIGTERM / SIGHUP (closing the console on Windows) close the window the way its close button
/// does, so the program ends through its normal path and the spotifyd guard runs.
pub fn close_on_signal(wake: Wake) {
    tokio::spawn(async move {
        let Ok(mut s) = shutdown::Shutdown::new() else { return };
        s.recv().await;
        loop {
            if let Some(ctx) = wake.get() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

/// What the window should be called (`window_title`): the song, cleaned of control characters;
/// `None` = leave the title alone.
pub fn window_title(setting: &str, snap: Option<&Snapshot>) -> Option<String> {
    let s = snap.filter(|s| !s.title.is_empty())?;
    let text = match setting {
        "title" => s.title.clone(),
        "artist-title" if !s.artist.is_empty() => format!("{} – {}", s.artist, s.title),
        "artist-title" => s.title.clone(),
        _ => return None,
    };
    Some(text.chars().filter(|c| !c.is_control()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(artist: &str, title: &str) -> Snapshot {
        let line = format!("Playing\x1f5000000\x1f{title}\x1f{artist}\x1fAlbum\x1f200000000\x1f\x1f2\x1f1\x1f1999\x1f0.5\x1f\x1fid\x1fvlc\x1f");
        music_display::player::parse(&line).unwrap()
    }

    #[test]
    fn the_window_title_follows_the_setting() {
        let s = snap("Band", "Song\x1b[0m");
        assert_eq!(window_title("title", Some(&s)).as_deref(), Some("Song[0m"));
        assert_eq!(window_title("artist-title", Some(&s)).as_deref(), Some("Band – Song[0m"));
        assert_eq!(window_title("off", Some(&s)), None);
        assert_eq!(window_title("title", None), None);
        assert_eq!(window_title("artist-title", Some(&snap("", "Song"))).as_deref(), Some("Song"));
    }

    #[tokio::test]
    async fn events_pass_through_the_wake_channel() {
        let (tx, mut rx, _w) = channel();
        tx.send(AppEvent::Cava(vec![1])).unwrap();
        assert!(matches!(rx.recv().await, Some(AppEvent::Cava(_))));
    }
}
