//! The Spotify side of the window: the library browser (`o`), the device list, like / play here,
//! and the remote player state. The state and the requests are the terminal front end's own
//! (`browser::Library`, `spotify::Api`); this file only drives them from egui. Nothing here
//! starts, or talks to Spotify, unless `spotify = true` is set.
use super::model::Model;
use crate::browser::{DevicePicker, Level, ListRow, Library, TAB_LIKED, TAB_PLAYLISTS, TAB_QUEUE, TAB_SEARCH, TABS};
use egui::{RichText, Ui};
use music_display::cli::Args;
use music_display::player::{Snapshot, Status};
use music_display::spotify::{self, Api, Cmd, Reply, Remote};
use music_display::AppEvent;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

pub struct Spot {
    /// `None` while Spotify is off (nothing is requested then).
    pub api: Option<Api>,
    pub lib: Library,
    pub devices: Option<DevicePicker>,
    /// The library window is shown.
    pub open: bool,
    /// Why the API can't be used right now (not logged in, rate limit…).
    pub status: Option<String>,
    remote: Option<(Remote, Instant)>,
    /// Is the song playing in your Liked Songs (`None` = not asked yet / not a Spotify song).
    pub liked: Option<bool>,
    liked_uri: String,
    mpris: bool,
    /// A message for the note line, taken by the model.
    pub note: Option<String>,
    tx: UnboundedSender<AppEvent>,
    client: reqwest::Client,
    /// The poller of `/me/player`, stopped when Spotify is switched off or rebuilt.
    task: Option<tokio::task::JoinHandle<()>>,
    /// A browser login in progress, what it last said, and the link to open by hand.
    login: Option<tokio::task::JoinHandle<()>>,
    pub login_msg: Option<String>,
    pub login_url: Option<String>,
    /// A login finished: the model rebuilds the client so it reads the new token.
    relogin: bool,
    /// The keyboard moved the selection: the window scrolls to it once.
    pub nav_moved: bool,
}

impl Spot {
    pub fn new(args: &Args, tx: UnboundedSender<AppEvent>, client: reqwest::Client) -> Self {
        let mut s = Spot {
            api: None,
            lib: Library::new(),
            devices: None,
            open: false,
            status: None,
            remote: None,
            liked: None,
            liked_uri: String::new(),
            mpris: false,
            note: None,
            tx,
            client: client.clone(),
            task: None,
            login: None,
            login_msg: None,
            login_url: None,
            relogin: false,
            nav_moved: false,
        };
        s.lib.queue_tab = !args.no_spotify_queue_tab;
        if args.spotify {
            s.start(args, client);
        }
        s
    }

    /// Build the client and start polling `/me/player`.
    fn start(&mut self, args: &Args, client: reqwest::Client) {
        let id = args.spotify_client_id.clone().unwrap_or_default();
        let daemon = if args.use_spotifyd { args.spotify_device.as_str() } else { "" };
        let api = Api::new(client, &id, daemon);
        self.status = (!spotify::logged_in()).then(|| "not logged in: run music_display --spotify-login".to_string());
        // the poller only runs in the real program: tests drive the client by hand
        #[cfg(not(test))]
        {
            self.task = Some(tokio::spawn(api.clone().remote_loop(self.tx.clone())));
        }
        self.api = Some(api);
    }

    /// Switch the client off: no poller, no requests, an empty library.
    pub fn stop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.api = None;
        self.remote = None;
        self.liked = None;
        self.liked_uri.clear();
        self.status = None;
        self.devices = None;
        self.open = false;
        self.lib = Library::new();
    }

    /// Build the client again from the flags (`spotify`, Client ID, device name, spotifyd, queue
    /// tab): `Spot::new` only reads them at start, so a changed setting goes through here.
    pub fn rebuild(&mut self, args: &Args) {
        self.stop();
        self.lib.queue_tab = !args.no_spotify_queue_tab;
        if args.spotify {
            let c = self.client.clone();
            self.start(args, c);
        }
    }

    /// "Log in": the browser login without blocking the window. Calling it again cancels.
    pub fn toggle_login(&mut self, client_id: &str) {
        if let Some(t) = self.login.take() {
            t.abort();
            self.login_msg = Some("login cancelled".into());
            self.login_url = None;
            return;
        }
        let id = client_id.trim().to_string();
        if id.is_empty() {
            self.login_msg = Some("set the Client ID first".into());
            return;
        }
        self.login_msg = Some("waiting for the browser…".into());
        self.login_url = None;
        let tx = self.tx.clone();
        self.login = Some(tokio::spawn(async move {
            use spotify::LoginMsg;
            let said = tx.clone();
            let say = move |step| {
                let _ = said.send(AppEvent::Spotify(Reply::Login(LoginMsg::Step(step))));
            };
            let r = spotify::login(&id, &say).await.map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Spotify(Reply::Login(LoginMsg::Done(r))));
        }));
    }

    pub fn logging_in(&self) -> bool {
        self.login.is_some()
    }

    /// Forget the token (and any login in progress).
    pub fn logout(&mut self) {
        if let Some(t) = self.login.take() {
            t.abort();
        }
        self.login_url = None;
        self.login_msg = Some(if spotify::logout() { "logged out" } else { "not logged in" }.into());
        self.relogin = true;
    }

    pub fn take_relogin(&mut self) -> bool {
        std::mem::take(&mut self.relogin)
    }

    fn on_login(&mut self, msg: spotify::LoginMsg) {
        use spotify::{LoginMsg, LoginStep};
        match msg {
            LoginMsg::Step(LoginStep::Url(u)) => self.login_url = Some(u),
            LoginMsg::Step(LoginStep::Opened(false)) => {
                self.login_msg = Some("the browser did not open: open the link below by hand".into());
            }
            LoginMsg::Step(LoginStep::Opened(true)) => {}
            LoginMsg::Done(r) => {
                self.login = None;
                self.login_url = None;
                match r {
                    Ok(()) => {
                        self.login_msg = Some("logged in".into());
                        self.relogin = true;
                    }
                    Err(e) => self.login_msg = Some(e),
                }
            }
        }
    }

    pub fn on(&self) -> bool {
        self.api.is_some()
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.note = Some(msg.into());
    }

    /// The API, or a note saying why there is none.
    fn api(&mut self) -> Option<Api> {
        if self.api.is_none() {
            self.say("Spotify is off: set spotify = true in config.toml, or start with --spotify");
        }
        self.api.clone()
    }

    /// Spotify's rate limit, as long as it holds.
    pub fn limited(&self) -> Option<String> {
        let d = self.api.as_ref()?.paused_for()?;
        Some(format!("Spotify is rate limiting: try again in {} s", d.as_secs() + 1))
    }

    // ── the remote player ──

    /// An MPRIS player is (not) on screen: the API is polled more slowly then.
    pub fn set_mpris(&mut self, present: bool) {
        self.mpris = present;
        if let Some(api) = &self.api {
            api.mpris.store(present, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn mpris(&self) -> bool {
        self.mpris
    }

    /// The last API snapshot, its position moved on by the time since it came.
    pub fn remote_snapshot(&self) -> Option<Snapshot> {
        let (r, at) = self.remote.as_ref()?;
        let mut s = r.snap.clone();
        if s.status == Status::Playing {
            s.position += at.elapsed().as_secs_f64();
        }
        Some(s)
    }

    /// Run a command in the background; the result comes back as a note.
    pub fn cmd(&mut self, cmd: Cmd) {
        let Some(api) = self.api() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = api.run(cmd).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(Reply::Done(r)));
            // the player state lags the command a little
            tokio::time::sleep(Duration::from_millis(400)).await;
            api.wake.notify_one();
        });
    }

    /// Ask once per song whether it is in Liked Songs.
    pub fn check_liked(&mut self, playing_uri: Option<String>) {
        let uri = playing_uri.unwrap_or_default();
        if uri == self.liked_uri {
            return;
        }
        self.liked_uri = uri.clone();
        self.liked = None;
        let (Some(api), false) = (self.api.clone(), uri.is_empty()) else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(liked) = api.liked(&uri).await {
                let _ = tx.send(AppEvent::Spotify(Reply::Liked { uri, liked }));
            }
        });
    }

    pub fn toggle_like(&mut self, playing_uri: Option<String>) {
        let Some(uri) = playing_uri else {
            self.say("not a Spotify song");
            return;
        };
        if self.api().is_none() {
            return;
        }
        let on = self.liked != Some(true);
        self.liked = Some(on);
        self.lib.tabs[TAB_LIKED].loaded = false;
        self.cmd(Cmd::Like(uri, on));
    }

    /// An answer from a background task (the same ones the terminal front end handles).
    pub fn on_reply(&mut self, r: Reply) {
        // a login can finish while the client is off
        let r = match r {
            Reply::Login(msg) => return self.on_login(msg),
            r => r,
        };
        // answers of a client that was never started
        if self.api.is_none() {
            return;
        }
        match r {
            Reply::Login(_) => {}
            Reply::Remote(Err(e)) => {
                if e.starts_with("Spotify ") {
                    self.say(e.clone());
                }
                self.status = Some(e);
            }
            Reply::Remote(Ok(r)) => {
                self.status = None;
                self.remote = r.map(|r| (*r, Instant::now()));
            }
            Reply::Liked { uri, liked } if uri == self.liked_uri => self.liked = Some(liked),
            Reply::Liked { .. } => {}
            Reply::Done(Ok(n)) if n.is_empty() => {}
            Reply::Done(Ok(n)) => {
                self.lib.note = Some(n.clone());
                self.say(n);
            }
            Reply::Done(Err(e)) => {
                // the optimistic heart was wrong: ask again
                let uri = std::mem::take(&mut self.liked_uri);
                self.check_liked(Some(uri));
                self.lib.note = Some(e.clone());
                self.say(e);
            }
            Reply::Tab(i, res) => {
                let l = &mut self.lib.tabs[i];
                l.loading = false;
                let failed = res.as_ref().err().cloned();
                match res {
                    Ok(items) => {
                        l.items = items;
                        l.loaded = true;
                        l.fetched = Some(Instant::now());
                        l.error = None;
                        l.sel = l.sel.min(l.items.len().saturating_sub(1));
                    }
                    // what was cached stays on screen; the note says why it didn't refresh
                    Err(e) => l.error = Some(e),
                }
                if i == TAB_PLAYLISTS {
                    self.fill_picker();
                }
                if let Some(e) = failed {
                    self.say(e);
                }
            }
            Reply::Queue(res) => {
                let failed = res.as_ref().err().map(|e| format!("Queue unavailable: {e}"));
                let l = &mut self.lib.tabs[TAB_QUEUE];
                l.loading = false;
                match res {
                    Ok((items, has_now)) => {
                        l.heads = if has_now { vec![(0, "Now playing"), (1, "Next up")] } else { vec![(0, "Next up")] };
                        l.summary = format!("{} next", items.len().saturating_sub(has_now as usize));
                        l.items = items;
                        l.loaded = true;
                        l.fetched = Some(Instant::now());
                        l.error = None;
                        l.sel = l.sel.min(l.items.len().saturating_sub(1));
                    }
                    Err(_) => l.error = failed.clone(),
                }
                if let Some(e) = failed {
                    self.say(e);
                }
            }
            Reply::Opened { uri, result } => {
                let failed = result.as_ref().err().cloned();
                let Some(l) = self.lib.stack.iter_mut().rev().find(|l| l.uri == uri && l.loading) else {
                    return;
                };
                l.loading = false;
                match result {
                    Ok((summary, items)) => {
                        l.summary = summary;
                        l.items = items;
                        l.loaded = true;
                    }
                    Err(e) => l.error = Some(e),
                }
                if let Some(e) = failed {
                    self.say(e);
                }
            }
            Reply::Search { q, result } => {
                if q != self.lib.query {
                    return; // an older search that finished late
                }
                let failed = result.as_ref().err().cloned();
                let l = &mut self.lib.tabs[TAB_SEARCH];
                l.loading = false;
                match result {
                    Ok(items) => {
                        l.summary = format!("{} results", items.len());
                        l.items = items;
                        l.loaded = true;
                        l.error = None;
                        l.sel = 0;
                    }
                    Err(e) => l.error = Some(e),
                }
                if let Some(e) = failed {
                    self.say(e);
                }
            }
            Reply::Devices(res) => {
                let Some(d) = &mut self.devices else { return };
                d.loading = false;
                let failed = res.as_ref().err().cloned();
                match res {
                    Ok(items) => {
                        d.sel = items.iter().position(|x| x.active).unwrap_or(0);
                        d.items = items;
                        d.error = None;
                    }
                    Err(e) => d.error = Some(e),
                }
                if let Some(e) = failed {
                    self.say(e);
                }
            }
        }
    }

    // ── devices (d) ──

    pub fn open_devices(&mut self) {
        let Some(api) = self.api() else { return };
        self.devices = Some(DevicePicker { loading: true, ..Default::default() });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = api.devices_list().await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(Reply::Devices(r)));
        });
    }

    /// Move playback to device `i` of the list (a device Spotify won't let apps control says so).
    pub fn choose_device(&mut self, i: usize) {
        let Some(dev) = self.devices.as_ref().and_then(|d| d.items.get(i)).cloned() else { return };
        if dev.restricted {
            self.say(format!("{} is restricted by Spotify: it can't be controlled from here", dev.name));
            return;
        }
        self.devices = None;
        self.cmd(Cmd::TransferTo { id: dev.id, name: dev.name });
    }

    // ── library ──

    pub fn open_library(&mut self) {
        if self.api().is_none() {
            return;
        }
        self.open = true;
        self.lib.note = self.status.clone();
        // what is cached shows at once; a stale list is refreshed in the background
        self.load_tab(self.lib.tab, false);
    }

    /// Make sure tab `i` is loaded and fresh. A fresh list costs no request; a stale one keeps
    /// showing while it is fetched again in the background.
    pub fn load_tab(&mut self, i: usize, force: bool) {
        if i == TAB_SEARCH {
            return; // searches run when asked for
        }
        let ttl = if i == TAB_QUEUE { crate::browser::QUEUE_FRESH } else { crate::browser::FRESH };
        let l = &mut self.lib.tabs[i];
        if l.loading || (!force && l.fresh(ttl)) {
            return;
        }
        let Some(api) = self.api.clone() else { return };
        l.loading = true;
        l.error = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if i == TAB_QUEUE {
                let r = api.queue_list().await.map_err(|e| e.msg);
                let _ = tx.send(AppEvent::Spotify(Reply::Queue(r)));
                return;
            }
            let r = match i {
                0 => api.recent().await,
                1 => api.playlists().await,
                2 => api.liked_tracks().await,
                3 => api.albums().await,
                _ => api.artists().await,
            };
            let _ = tx.send(AppEvent::Spotify(Reply::Tab(i, r.map_err(|e| e.msg))));
        });
    }

    pub fn switch_tab(&mut self, i: usize) {
        if !self.lib.visible_tabs().contains(&i) {
            return;
        }
        self.lib.tab = i;
        self.lib.stack.clear();
        self.lib.picker = None;
        self.load_tab(i, false);
    }

    /// Run the search typed in the field (results replace the old ones when they arrive).
    pub fn run_search(&mut self) {
        let q = self.lib.input.text.trim().to_string();
        if q.is_empty() {
            return;
        }
        let Some(api) = self.api.clone() else { return };
        self.lib.query = q.clone();
        self.lib.stack.clear();
        let l = &mut self.lib.tabs[TAB_SEARCH];
        l.loading = true;
        l.error = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.search(&q).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(Reply::Search { q, result }));
        });
    }

    /// Open an album / own playlist / artist / Liked Songs as a new level.
    pub fn open_item(&mut self, it: spotify::Item) {
        if !it.openable {
            self.lib.note = Some(match it.kind.as_str() {
                "playlist" => "Spotify only lets apps open your own playlists: play it instead".into(),
                _ => "play it instead".into(),
            });
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let uri = if it.kind == "liked" { "liked".to_string() } else { it.uri.clone() };
        self.lib.stack.push(Level { title: it.name, uri: uri.clone(), loading: true, ..Default::default() });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.open(&uri).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(Reply::Opened { uri, result }));
        });
    }

    /// Fetch the level on screen again.
    pub fn reload_level(&mut self) {
        if self.lib.picker.is_some() {
            self.load_tab(TAB_PLAYLISTS, true);
            return;
        }
        match self.lib.stack.pop() {
            Some(l) => {
                let kind = l.uri.split(':').nth(1).unwrap_or("liked").to_string();
                self.open_item(spotify::Item { kind, uri: l.uri, name: l.title, openable: true, ..Default::default() });
            }
            None if self.lib.tab == TAB_SEARCH => self.run_search(),
            None => self.load_tab(self.lib.tab, true),
        }
    }

    /// A track plays in the list's context (from that song on), anything else plays itself.
    pub fn play_item(&mut self, i: usize) {
        let l = self.lib.level();
        let Some(it) = l.items.get(i).cloned() else { return };
        let ctx = l.uri.clone();
        let cmd = match it.kind.as_str() {
            "track" if !ctx.is_empty() => Cmd::Play { ctx, track: it.uri },
            "track" => Cmd::Play { ctx: it.uri, track: String::new() },
            "liked" => Cmd::Play { ctx: "liked".into(), track: String::new() },
            _ => Cmd::Play { ctx: it.uri, track: String::new() },
        };
        self.cmd(cmd);
    }

    pub fn queue_item(&mut self, i: usize) {
        if let Some(it) = self.track_at(i) {
            self.cmd(Cmd::Queue(it.uri));
        }
    }

    /// Like a song from a list (Liked Songs are re-read the next time they are opened).
    pub fn like_item(&mut self, i: usize) {
        if let Some(it) = self.track_at(i) {
            if it.uri == self.liked_uri {
                self.liked = Some(true);
            }
            self.lib.tabs[TAB_LIKED].loaded = false;
            self.cmd(Cmd::Like(it.uri, true));
        }
    }

    fn track_at(&mut self, i: usize) -> Option<spotify::Item> {
        let it = self.lib.level().items.get(i).filter(|it| it.kind == "track").cloned();
        if it.is_none() {
            self.lib.note = Some("pick a song first".into());
        }
        it
    }

    /// "Add to playlist": the list of your own playlists (loaded with the Playlists tab).
    pub fn open_picker(&mut self, i: usize) {
        let Some(it) = self.track_at(i) else { return };
        let title = format!("Add “{}” to…", it.name);
        self.lib.picker = Some((it.uri, it.name, Level { title, ..Default::default() }));
        self.load_tab(TAB_PLAYLISTS, false);
        self.fill_picker();
    }

    fn fill_picker(&mut self) {
        let tab = &self.lib.tabs[TAB_PLAYLISTS];
        let (items, loading, error) =
            (tab.items.iter().filter(|p| p.openable).cloned().collect::<Vec<_>>(), tab.loading, tab.error.clone());
        if let Some((_, _, l)) = &mut self.lib.picker {
            l.loaded = !loading && error.is_none();
            l.loading = loading;
            l.error = error;
            l.sel = l.sel.min(items.len().saturating_sub(1));
            l.items = items;
        }
    }

    /// Add the picked song to playlist `i` of the picker.
    pub fn add_to(&mut self, i: usize) {
        let Some((track, _, l)) = &self.lib.picker else { return };
        let Some(pl) = l.items.get(i).cloned() else { return };
        let track = track.clone();
        self.lib.picker = None;
        self.cmd(Cmd::Add { playlist: pl.uri, name: pl.name, track });
    }
}

// ── drawing ─────────────────────────────────────────────────────────────────

/// What a click in the library window asked for (applied after the lists are drawn).
enum Act {
    Tab(usize),
    Back,
    Reload,
    Search,
    Play(usize),
    Open(usize),
    Queue(usize),
    Like(usize),
    AddTo(usize),
    Pick(usize),
    CancelPick,
}

fn mmss(secs: f64) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The like / devices / play here / library buttons for the end of the transport row.
pub fn transport_buttons(ui: &mut Ui, m: &mut Model) {
    if !m.spot.on() {
        return;
    }
    let uri = m.playing_uri();
    if uri.is_some() {
        let on = m.spot.liked == Some(true);
        if ui.selectable_label(on, if on { "♥" } else { "♡" }).on_hover_text("like / unlike this song (Liked Songs)").clicked() {
            m.spot.toggle_like(uri);
        }
    }
    if ui.button("devices").on_hover_text("move playback to another Spotify Connect device").clicked() {
        m.spot.open_devices();
    }
    if ui.button("play here").on_hover_text("move playback to this computer").clicked() {
        m.spot.cmd(Cmd::Transfer);
    }
    // `spotify_button = false` hides it (the `o` key still opens the library)
    if !m.args.no_spotify_button && ui.selectable_label(m.spot.open, "library").on_hover_text("Spotify library (o)").clicked() {
        toggle(m);
    }
    m.take_spot_note();
}

/// `o` and the button.
pub fn toggle(m: &mut Model) {
    if m.spot.open {
        m.spot.open = false;
    } else {
        m.spot.open_library();
    }
    m.take_spot_note();
}

/// Keys while the library window is open (the terminal browser's): ↑ ↓ PgUp PgDn move, Enter
/// plays, → opens, ← goes back, Tab switches tab. True when the key was used.
pub fn key(m: &mut Model, code: crossterm::event::KeyCode) -> bool {
    use crossterm::event::KeyCode as K;
    if !m.spot.open || m.spot.lib.picker.is_some() {
        return false;
    }
    let l = m.spot.lib.level_mut();
    let last = l.items.len().saturating_sub(1);
    match code {
        K::Up => l.sel = l.sel.saturating_sub(1),
        K::Down => l.sel = (l.sel + 1).min(last),
        K::PageUp => l.sel = l.sel.saturating_sub(8),
        K::PageDown => l.sel = (l.sel + 8).min(last),
        K::Enter => {
            let i = l.sel;
            m.spot.play_item(i);
            return true;
        }
        K::Right => {
            if let Some(it) = l.items.get(l.sel).filter(|it| it.openable).cloned() {
                m.spot.open_item(it);
            }
            return true;
        }
        K::Left => {
            m.spot.lib.stack.pop();
            return true;
        }
        K::Tab => {
            let tabs = m.spot.lib.visible_tabs();
            let at = tabs.iter().position(|t| *t == m.spot.lib.tab).unwrap_or(0);
            m.spot.switch_tab(tabs[(at + 1) % tabs.len()]);
            return true;
        }
        _ => return false,
    }
    m.spot.nav_moved = true;
    true
}

/// The library window and the device list, when they are open.
pub fn windows(ctx: &egui::Context, m: &mut Model) {
    if m.spot.open {
        library_window(ctx, m);
    }
    if m.spot.devices.is_some() {
        devices_window(ctx, m);
    }
    m.take_spot_note();
}

fn library_window(ctx: &egui::Context, m: &mut Model) {
    let mut open = true;
    let mut acts: Vec<Act> = Vec::new();
    let moved = std::mem::take(&mut m.spot.nav_moved);
    let s = &mut m.spot;
    egui::Window::new("Spotify library").open(&mut open).default_size([460.0, 520.0]).show(ctx, |ui| {
        if let Some(e) = s.limited().or_else(|| s.status.clone()) {
            ui.label(RichText::new(e).weak());
        }
        ui.horizontal_wrapped(|ui| {
            for i in s.lib.visible_tabs() {
                if ui.selectable_label(s.lib.tab == i && s.lib.picker.is_none(), TABS[i]).clicked() {
                    acts.push(Act::Tab(i));
                }
            }
        });
        ui.separator();
        let picking = s.lib.picker.is_some();
        ui.horizontal(|ui| {
            if picking {
                if ui.button("cancel").clicked() {
                    acts.push(Act::CancelPick);
                }
            } else if !s.lib.stack.is_empty() && ui.button("‹ back").clicked() {
                acts.push(Act::Back);
            }
            let l = s.lib.level();
            ui.strong(&l.title);
            if !l.summary.is_empty() {
                ui.label(RichText::new(&l.summary).weak());
            }
            if ui.small_button("reload").clicked() {
                acts.push(Act::Reload);
            }
        });
        if s.lib.tab == TAB_SEARCH && s.lib.stack.is_empty() && !picking {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut s.lib.input.text).hint_text("songs, albums, artists, playlists").desired_width(280.0));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("search").clicked() || enter) && !s.lib.input.text.trim().is_empty() {
                    acts.push(Act::Search);
                }
            });
        }
        let l = s.lib.level();
        if l.loading {
            ui.label(RichText::new("loading…").weak());
        }
        if let Some(e) = &l.error {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
        if let Some(n) = &s.lib.note {
            ui.label(RichText::new(n).small().weak());
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for row in l.rows() {
                match row {
                    ListRow::Head(h) => {
                        ui.add_space(4.0);
                        ui.label(RichText::new(h).strong());
                    }
                    ListRow::Item(i) => {
                        let it = &l.items[i];
                        let bg = ui.painter().add(egui::Shape::Noop);
                        let row = ui.horizontal(|ui| {
                            if picking {
                                if ui.button("add").clicked() {
                                    acts.push(Act::Pick(i));
                                }
                            } else {
                                if ui.small_button("▶").on_hover_text("play").clicked() {
                                    acts.push(Act::Play(i));
                                }
                                if it.openable && ui.small_button("›").on_hover_text("open").clicked() {
                                    acts.push(Act::Open(i));
                                }
                                if it.kind == "track" {
                                    if ui.small_button("+q").on_hover_text("add to the queue").clicked() {
                                        acts.push(Act::Queue(i));
                                    }
                                    if ui.small_button("+list").on_hover_text("add to a playlist").clicked() {
                                        acts.push(Act::AddTo(i));
                                    }
                                    if ui.small_button("♡").on_hover_text("like").clicked() {
                                        acts.push(Act::Like(i));
                                    }
                                }
                            }
                            ui.label(&it.name);
                            if !it.sub.is_empty() {
                                ui.label(RichText::new(&it.sub).weak());
                            }
                            if it.duration > 0.0 {
                                ui.label(RichText::new(mmss(it.duration)).weak().monospace());
                            }
                        });
                        // the keyboard's row
                        if i == l.sel {
                            let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.5);
                            ui.painter().set(bg, egui::Shape::rect_filled(row.response.rect.expand(1.0), 3.0, fill));
                            if moved {
                                row.response.scroll_to_me(None);
                            }
                        }
                    }
                }
            }
            if l.loaded && l.items.is_empty() && l.error.is_none() {
                ui.label(RichText::new("nothing here").weak());
            }
        });
    });
    for a in acts {
        match a {
            Act::Tab(i) => m.spot.switch_tab(i),
            Act::Back => {
                m.spot.lib.stack.pop();
            }
            Act::Reload => m.spot.reload_level(),
            Act::Search => m.spot.run_search(),
            Act::Play(i) => m.spot.play_item(i),
            Act::Open(i) => {
                if let Some(it) = m.spot.lib.level().items.get(i).cloned() {
                    m.spot.open_item(it);
                }
            }
            Act::Queue(i) => m.spot.queue_item(i),
            Act::Like(i) => m.spot.like_item(i),
            Act::AddTo(i) => m.spot.open_picker(i),
            Act::Pick(i) => m.spot.add_to(i),
            Act::CancelPick => m.spot.lib.picker = None,
        }
    }
    if !open {
        m.spot.open = false;
    }
}

fn devices_window(ctx: &egui::Context, m: &mut Model) {
    let mut open = true;
    let (mut choose, mut refresh) = (None, false);
    let Some(d) = &m.spot.devices else { return };
    egui::Window::new("Devices").open(&mut open).default_width(320.0).show(ctx, |ui| {
        if d.loading {
            ui.label(RichText::new("loading…").weak());
        }
        if let Some(e) = &d.error {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
        for (i, dev) in d.items.iter().enumerate() {
            let mut label = format!("{} ({})", dev.name, dev.kind);
            if dev.local {
                label.push_str(" · this PC");
            }
            if dev.restricted {
                label.push_str(" · restricted");
            }
            if ui.selectable_label(dev.active, label).clicked() {
                choose = Some(i);
            }
        }
        if d.items.is_empty() && !d.loading && d.error.is_none() {
            ui.label(RichText::new("no devices: start Spotify on one").weak());
        }
        if ui.small_button("refresh").clicked() {
            refresh = true;
        }
    });
    if let Some(i) = choose {
        m.spot.choose_device(i);
    } else if refresh {
        m.spot.open_devices();
    } else if !open {
        m.spot.devices = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::io::{Read, Write};

    fn spot(base: Option<&str>) -> (Spot, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        music_display::util::enter_test_mode();
        let args = Args::parse_from(["music_display", "--spotify"]);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut s = Spot::new(&args, tx, reqwest::Client::new());
        if let Some(b) = base {
            s.api = Some(Api::new(reqwest::Client::new(), "id", "").with_fake_server(b));
        }
        (s, rx)
    }

    fn track(uri: &str) -> spotify::Item {
        spotify::Item { kind: "track".into(), uri: uri.into(), name: uri.into(), ..Default::default() }
    }

    /// One answer for every request; the request lines are logged.
    fn site(body: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for mut st in l.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                log2.lock().unwrap().push(req.lines().next().unwrap_or("").to_string());
                let _ = write!(st, "HTTP/1.1 200 X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, log)
    }

    async fn next(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> Reply {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(AppEvent::Spotify(r))) => r,
            _ => panic!("no Spotify reply came"),
        }
    }

    #[tokio::test]
    async fn spotify_stays_off_unless_asked_for() {
        music_display::util::enter_test_mode();
        let args = Args::parse_from(["music_display"]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut s = Spot::new(&args, tx, reqwest::Client::new());
        assert!(!s.on());
        s.open_library();
        assert!(!s.open && s.note.as_deref().unwrap().contains("Spotify is off"));
        // an answer with no client is ignored
        s.on_reply(Reply::Tab(1, Ok(vec![track("a")])));
        assert!(s.lib.tabs[1].items.is_empty());
    }

    #[tokio::test]
    async fn lists_keep_their_cache_on_errors_and_late_searches_are_dropped() {
        let (mut s, _rx) = spot(Some("http://127.0.0.1:1"));
        s.on_reply(Reply::Tab(2, Ok(vec![track("a"), track("b")])));
        assert!(s.lib.tabs[2].loaded && s.lib.tabs[2].fresh(crate::browser::FRESH));
        s.lib.tabs[2].sel = 1;
        s.on_reply(Reply::Tab(2, Err("Spotify is rate limiting: try again in 30 s".into())));
        assert_eq!(s.lib.tabs[2].items.len(), 2, "the cached rows stay");
        assert!(s.lib.tabs[2].error.is_some());
        assert_eq!(s.note.take().as_deref(), Some("Spotify is rate limiting: try again in 30 s"));
        // search: only the answer to the last query counts, rows are grouped under headings
        s.lib.query = "new".into();
        s.on_reply(Reply::Search { q: "old".into(), result: Ok(vec![track("x")]) });
        assert!(s.lib.tabs[TAB_SEARCH].items.is_empty());
        let mut album = track("al");
        album.kind = "album".into();
        s.on_reply(Reply::Search { q: "new".into(), result: Ok(vec![track("t"), album]) });
        let rows = s.lib.tabs[TAB_SEARCH].rows();
        assert_eq!(rows[0], ListRow::Head("Songs"));
        assert_eq!(rows[2], ListRow::Head("Albums"));
        // the queue gets its headings
        s.on_reply(Reply::Queue(Ok((vec![track("n"), track("q")], true))));
        assert_eq!(s.lib.tabs[TAB_QUEUE].heads, vec![(0, "Now playing"), (1, "Next up")]);
        assert_eq!(s.lib.tabs[TAB_QUEUE].summary, "1 next");
        // a remote state is kept and its position moves on
        let v: serde_json::Value = serde_json::from_str(
            r#"{"is_playing":true,"progress_ms":5000,"device":{"name":"Phone"},"item":{"uri":"spotify:track:z","name":"Zed","duration_ms":90000,"artists":[{"name":"Q"}],"album":{"name":"W"}}}"#,
        )
        .unwrap();
        let r = spotify::parse_state(&v, |_| false).map(Box::new);
        s.on_reply(Reply::Remote(Ok(r)));
        let snap = s.remote_snapshot().unwrap();
        assert_eq!((snap.title.as_str(), snap.player.as_str()), ("Zed", spotify::REMOTE));
        s.on_reply(Reply::Remote(Err("Spotify Premium is needed for this".into())));
        assert_eq!(s.note.take().as_deref(), Some("Spotify Premium is needed for this"));
    }

    #[tokio::test]
    async fn the_picker_lists_only_your_playlists_and_adding_sends_the_request() {
        let (base, log) = site("{}");
        let (mut s, mut rx) = spot(Some(&base));
        let mut own = track("spotify:playlist:mine");
        own.kind = "playlist".into();
        own.openable = true;
        let mut other = own.clone();
        other.uri = "spotify:playlist:theirs".into();
        other.openable = false;
        s.lib.tabs[TAB_PLAYLISTS].loaded = true;
        s.lib.tabs[TAB_PLAYLISTS].fetched = Some(Instant::now());
        s.on_reply(Reply::Tab(TAB_PLAYLISTS, Ok(vec![own, other])));
        s.lib.tabs[0].items = vec![track("spotify:track:t1")];
        s.open_picker(0);
        assert_eq!(s.lib.picker.as_ref().unwrap().2.items.len(), 1);
        s.add_to(0);
        assert!(s.lib.picker.is_none());
        assert!(matches!(next(&mut rx).await, Reply::Done(Ok(n)) if n == "Added to spotify:playlist:mine"));
        assert!(log.lock().unwrap().iter().any(|l| l.starts_with("POST /playlists/mine/items")), "{:?}", log.lock().unwrap());
        // a row that isn't a song can't be queued
        s.lib.tabs[0].items = vec![spotify::Item { kind: "album".into(), uri: "spotify:album:a".into(), ..Default::default() }];
        s.queue_item(0);
        assert_eq!(s.lib.note.as_deref(), Some("pick a song first"));
    }

    #[tokio::test]
    async fn devices_like_and_play_use_the_clients_requests() {
        let (base, log) = site(r#"{"devices":[{"id":"d1","name":"Phone","type":"Smartphone","is_active":true},{"id":"d3","name":"Echo","type":"Speaker","is_restricted":true}]}"#);
        let (mut s, mut rx) = spot(Some(&base));
        s.open_devices();
        let r = next(&mut rx).await;
        s.on_reply(r);
        let d = s.devices.as_ref().unwrap();
        assert_eq!((d.items.len(), d.sel, d.loading), (2, 0, false));
        // a restricted device says so and the list stays
        s.choose_device(1);
        assert!(s.devices.is_some() && s.note.take().unwrap().contains("restricted"));
        s.choose_device(0);
        assert!(s.devices.is_none());
        assert!(matches!(next(&mut rx).await, Reply::Done(Ok(n)) if n == "Playing on Phone"));
        // like: the heart flips at once; no Spotify song = a note and no request
        s.toggle_like(None);
        assert_eq!(s.note.take().as_deref(), Some("not a Spotify song"));
        s.toggle_like(Some("spotify:track:z".into()));
        assert_eq!(s.liked, Some(true));
        assert!(matches!(next(&mut rx).await, Reply::Done(Ok(n)) if n == "Added to Liked Songs"));
        assert!(log.lock().unwrap().iter().any(|l| l.starts_with("PUT /me/library")));
        // a track plays in the context of the list it is in
        s.lib.stack.push(Level { uri: "spotify:album:a".into(), items: vec![track("spotify:track:t")], ..Default::default() });
        s.play_item(0);
        assert!(matches!(next(&mut rx).await, Reply::Done(Ok(_))));
        assert!(log.lock().unwrap().iter().any(|l| l.starts_with("PUT /me/player/play")));
    }
}
