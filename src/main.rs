//! music_display — a zero-flicker terminal "now playing" display for MPRIS players.
//!
//! Features:
//!   * theme auto-detection from Noctalia / Matugen colour files (+ CLI overrides); with
//!     `--terminal-colors`, or when no file is usable, it inherits the terminal's own colours
//!   * playerctl polling + `--follow` stream for instant track switches
//!   * release-year fallback (xesam:contentCreated → MusicBrainz → iTunes), cached
//!   * synced lyrics from lrclib.net (exact → search fallback), cached, live offset nudging;
//!     spaced lines, faded context, optional big block-font for the active line
//!   * cover art via Kitty / Sixel / iTerm2 graphics (halfblock fallback) or coloured ASCII,
//!     duotone-tinted; optional UI accent taken from the cover (`--cover-theme`)
//!   * optional mono cava visualiser as a third column or under the lyrics
//!   * optional guitar chords (`g`) from local ChordPro / plain-text sheets, matched onto the
//!     synced lyrics line by line; transposable
//!   * clickable progress bar + transport buttons, runtime panel toggles
//!   * optional Spotify Web API (`--spotify`, PKCE login): library browser, likes, queue,
//!     and playback shown / controlled on other devices (the phone)
//!
//! Architecture: one `tokio` select loop owns all UI state (`App`). Every slow thing
//! (subprocesses, HTTP, image decode, cava) runs in its own task and reports back over an
//! unbounded mpsc channel as an `AppEvent`. Results are tagged with the track key they were
//! requested for, so stale results from a skipped track are dropped. The UI is only redrawn
//! when something changed, and ratatui's diffing buffer means only changed cells hit the tty.

use std::io::{Write, stdout};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, EventStream};
use crossterm::execute;
use futures::StreamExt;
use tokio::sync::mpsc;

// ════════════════════════════════════════════════════════════════════════════
// CLI
// ════════════════════════════════════════════════════════════════════════════
mod cli;

// ════════════════════════════════════════════════════════════════════════════
// Config file: ~/.config/music_display/config.toml (flags > file > built-in defaults)
// ════════════════════════════════════════════════════════════════════════════
mod config;

// ════════════════════════════════════════════════════════════════════════════
// Small shared helpers
// ════════════════════════════════════════════════════════════════════════════
mod util;

// ════════════════════════════════════════════════════════════════════════════
// Theme detection
// ════════════════════════════════════════════════════════════════════════════
mod theme;

// ════════════════════════════════════════════════════════════════════════════
// Player (playerctl / MPRIS)
// ════════════════════════════════════════════════════════════════════════════
mod player;

// ════════════════════════════════════════════════════════════════════════════
// Spotify Web API (--spotify): PKCE login, library, playback on any device
// ════════════════════════════════════════════════════════════════════════════
mod spotify;

// ════════════════════════════════════════════════════════════════════════════
// Library browser / settings widgets (text input, lists, device picker)
// ════════════════════════════════════════════════════════════════════════════
mod browser;

// ════════════════════════════════════════════════════════════════════════════
// Release-year resolution: contentCreated → cache → MusicBrainz → iTunes
// ════════════════════════════════════════════════════════════════════════════
mod tags;

// ════════════════════════════════════════════════════════════════════════════
mod year;

// ════════════════════════════════════════════════════════════════════════════
// Synced lyrics (lrclib.net)
// ════════════════════════════════════════════════════════════════════════════
mod lyrics;

// ════════════════════════════════════════════════════════════════════════════
// Cover art: fetch (file:// or http), cache in /tmp, downscale + duotone tint
// ════════════════════════════════════════════════════════════════════════════
mod translate;

// ════════════════════════════════════════════════════════════════════════════
mod cover;

// ════════════════════════════════════════════════════════════════════════════
// Cava: headless raw ASCII output
// ════════════════════════════════════════════════════════════════════════════
mod cava;

// ════════════════════════════════════════════════════════════════════════════
// Guitar chords: sheet model, parsers, transpose, matching onto synced lyrics
// ════════════════════════════════════════════════════════════════════════════
mod chords;

// ════════════════════════════════════════════════════════════════════════════
// Ultimate Guitar lookup (opt-in: --chords-ug). Unofficial: the site has no public API, so
// this reads the JSON blob embedded in its pages and is written to fail soft.
// ════════════════════════════════════════════════════════════════════════════
mod ug;

// ════════════════════════════════════════════════════════════════════════════
// Member parts (experimental, opt-in: --member-parts). Genius has no open API for this, so
// the song page is scraped for section headers like `[Verse 1: Jimin]` and everything
// is written to fail soft and silent.
// ════════════════════════════════════════════════════════════════════════════
mod genius;

// ════════════════════════════════════════════════════════════════════════════
// Guitar chord diagrams (standard tuning, strings low E → high e)
// ════════════════════════════════════════════════════════════════════════════
mod diagram;

// ════════════════════════════════════════════════════════════════════════════
// Application state + event handling
// ════════════════════════════════════════════════════════════════════════════
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
        lines: Result<(Vec<String>, &'static str), String>,
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

mod app;

// ════════════════════════════════════════════════════════════════════════════
// Key table + the [keys] section of config.toml
// ════════════════════════════════════════════════════════════════════════════
mod keymap;

// ════════════════════════════════════════════════════════════════════════════
// Big block font for the active lyric line (5×7 pixels, drawn with half blocks)
// ════════════════════════════════════════════════════════════════════════════
mod bigfont;

// ════════════════════════════════════════════════════════════════════════════
// Rendering
// ════════════════════════════════════════════════════════════════════════════
mod ui;

// ════════════════════════════════════════════════════════════════════════════
// --setup: guided spotifyd + Spotify login
// ════════════════════════════════════════════════════════════════════════════
mod setup;

// ════════════════════════════════════════════════════════════════════════════
// spotifyd: started with the app when use_spotifyd is on
// ════════════════════════════════════════════════════════════════════════════
mod spotifyd;

// ════════════════════════════════════════════════════════════════════════════
// main: terminal setup, worker spawn, event loop, clean teardown
// ════════════════════════════════════════════════════════════════════════════
#[tokio::main]
async fn main() -> Result<()> {
    // flags > config file > built-in defaults
    let matches = <cli::Args as clap::CommandFactory>::command().get_matches();
    let mut args = <cli::Args as clap::FromArgMatches>::from_arg_matches(&matches)?;
    let cfg_path = args.config.clone().unwrap_or_else(config::default_path);
    let explicit = args.config.is_some();
    let found = config::apply(&mut args, &matches, &cfg_path, explicit, &mut |w| {
        eprintln!("music_display: {w}")
    });
    let keys = keymap::load(&cfg_path, &mut |w| eprintln!("music_display: {w}"));
    if args.print_config {
        print!("{}", config::render(&args, &cfg_path, found));
        print!("{}", keys.render());
        return Ok(());
    }
    if args.setup {
        return setup::run_live(&args, cfg_path).await;
    }
    if args.spotify_login {
        let say = |step: spotify::LoginStep| match step {
            spotify::LoginStep::Url(url) => {
                println!("Log in to Spotify in your browser. If it didn't open, visit:\n\n{url}\n");
                println!("(The app's redirect URI must be exactly {})", spotify::redirect_uri());
            }
            spotify::LoginStep::Opened(_) => {}
        };
        spotify::login(args.spotify_client_id.as_deref().unwrap_or(""), &say).await?;
        println!("Logged in. The token is in {}", spotify::dir().join("token.json").display());
        return Ok(());
    }
    if args.spotify_logout {
        println!("{}", if spotify::logout() { "Logged out of Spotify." } else { "Not logged in." });
        return Ok(());
    }
    if let Some(p) = &args.chords_probe {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
        ug::probe(&client, &p[0], &p[1], args.probe_save.clone()).await;
        return Ok(());
    }
    let theme = theme::load(&args);

    // ratatui::init = raw mode + alternate screen + a panic hook that restores both.
    let mut terminal = ratatui::init();
    if !args.no_mouse {
        execute!(stdout(), EnableMouseCapture)?;
    }
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        prev_hook(info);
    }));

    // Query graphics protocol + font size *after* entering the alt screen, *before* reading events.
    // (ASCII covers need no graphics protocol, so skip the terminal query entirely.)
    let mut picker = if args.ascii_cover {
        ratatui_image::picker::Picker::halfblocks()
    } else {
        ratatui_image::picker::Picker::from_query_stdio()
            .unwrap_or_else(|_| ratatui_image::picker::Picker::halfblocks())
    };
    // Only fill image padding with the surface colour when it is a real, painted colour.
    let bg = theme
        .surface
        .filter(|_| !args.transparent)
        .map(|s| image::Rgba([s.0, s.1, s.2, 255]));
    picker.set_background_color(bg);

    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = app::App::new(args, theme, picker, tx.clone());
    app.keys = keys;

    // Player workers: periodic poll + instant change stream.
    tokio::spawn(app.ctl.clone().poll_loop(tx.clone()));
    tokio::spawn(app.ctl.clone().follow_loop(tx.clone()));
    // Spotify Web API: playback on other devices, `local` / volume for this one.
    app.start_spotify();
    if app.args.use_spotifyd {
        app.spotifyd = spotifyd::start();
    }

    let res = run(&mut terminal, app, rx).await;

    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

/// Redraw interval while the active line fills up word by word.
const KARAOKE_FRAME: Duration = Duration::from_millis(33);

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: app::App,
    mut rx: mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(16));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    // the window title this program set (None = never touched, so the old one stays)
    let mut title: Option<String> = None;

    // SIGTERM / SIGHUP end the loop like `q` does, so the terminal and spotifyd are cleaned up.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut hup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;

    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = hup.recv() => break,
            ev = events.next() => match ev {
                Some(Ok(ev)) => if app.handle(ev) { break },
                Some(Err(_)) | None => break,
            },
            Some(msg) = rx.recv() => {
                app.apply(msg);
                // Drain any backlog (e.g. a burst of cava frames) before drawing.
                while let Ok(m) = rx.try_recv() { app.apply(m); }
            }
            _ = tick.tick() => {
                app.preload_tick();
                sync_window_title(&app, &mut title);
                // Redraw when something changed, while the lyric scroll is easing,
                // or ~10×/s during playback (clock, progress bar, lyric line changes).
                let every = if app.karaoke_live() { KARAOKE_FRAME } else { Duration::from_millis(100) };
                let periodic = app.playing() && last_draw.elapsed() >= every;
                if app.dirty || app.animating() || periodic {
                    let layout = app.layout_key();
                    terminal.draw(|f| ui::draw(f, &mut app))?;
                    // The side panel or the library button came or went: wipe the screen and
                    // draw it again, so no old cover image (Sixel / iTerm2) is left behind.
                    if app.layout_key() != layout {
                        // not `terminal.clear()`: that asks the terminal for its cursor position,
                        // and the answer can time out (or be eaten by the event stream), which
                        // ended the app with "The cursor position could not be read…"
                        let area = terminal.size()?;
                        terminal.resize(area.into())?;
                        terminal.draw(|f| ui::draw(f, &mut app))?;
                    }
                    app.dirty = false;
                    last_draw = Instant::now();
                }
            }
        }
    }
    if title.is_some() {
        let mut out = stdout();
        let _ = write!(out, "\x1b[23;0t");
        let _ = out.flush();
    }
    Ok(())
}

/// Keep the terminal window's title in step with `window_title`. The old title is saved
/// (xterm's title stack) before the first change and restored when the setting goes off or
/// the program ends; terminals without the stack just keep the last song.
fn sync_window_title(app: &app::App, set: &mut Option<String>) {
    let want = app.window_title();
    if want == *set {
        return;
    }
    let mut out = stdout();
    if set.is_none() {
        let _ = write!(out, "\x1b[22;0t");
    }
    match &want {
        Some(t) => {
            let _ = write!(out, "\x1b]2;{t}\x07");
        }
        None => {
            let _ = write!(out, "\x1b[23;0t");
        }
    }
    let _ = out.flush();
    *set = want;
}

#[cfg(test)]
mod tests;
