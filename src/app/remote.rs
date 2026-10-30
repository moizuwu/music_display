use crate::browser::{DevicePicker, Edit, LibLink, TAB_ALBUMS, TAB_ARTISTS, TAB_LIKED, TAB_PLAYLISTS, TAB_QUEUE, TAB_SEARCH, TextInput};
use super::*;
use crate::layout::MetaField;

// ── Spotify (--spotify) ────────────────────────────────────────────────
impl App {
    /// What is on screen came from the Spotify API, not MPRIS: controls go to the API.
    pub fn remote_active(&self) -> bool {
        self.snap.as_ref().is_some_and(|s| s.player == spotify::REMOTE)
    }

    pub(super) fn set_mpris(&mut self, present: bool) {
        self.mpris_present = present;
        if let Some(api) = &self.spot {
            api.mpris.store(present, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The last API snapshot, its position moved on by the time since it came.
    pub(super) fn remote_snapshot(&self) -> Option<Snapshot> {
        self.remote_snapshot_within(spotify::REMOTE_MAX_AGE)
    }

    /// The same, but only when the answer is at most `max_age` old.
    pub(super) fn remote_snapshot_within(&self, max_age: Duration) -> Option<Snapshot> {
        let (r, at) = self.remote.as_ref().filter(|(_, at)| at.elapsed() <= max_age)?;
        let mut s = r.snap.clone();
        if s.status == Status::Playing {
            s.position += at.elapsed().as_secs_f64();
        }
        Some(s)
    }

    pub(super) fn set_note(&mut self, msg: impl Into<String>) {
        self.note = Some((msg.into(), Instant::now()));
        self.dirty = true;
    }

    /// The footer note, while it is fresh.
    pub fn note_text(&self) -> Option<&str> {
        self.note
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
            .map(|(n, _)| n.as_str())
    }

    /// The API, or a note saying why there is none.
    pub(super) fn api(&mut self) -> Option<spotify::Api> {
        match &self.spot {
            None => {
                self.set_note("Spotify is off: turn it on in Settings (s), or start with --spotify");
                None
            }
            Some(api) => Some(api.clone()),
        }
    }

    /// Build the client and start polling `/me/player`, when `spotify` is on and it isn't
    /// running yet. Nothing at all happens (no request, no task) while it is off.
    pub fn start_spotify(&mut self) {
        if !self.args.spotify || self.spot.is_some() {
            return;
        }
        let id = self.args.spotify_client_id.clone().unwrap_or_default();
        // spotifyd's Connect name only counts as "this PC" when spotifyd is in use
        let daemon = if self.args.use_spotifyd { self.args.spotify_device.as_str() } else { "" };
        let api = spotify::Api::new(self.client.clone(), &id, daemon);
        api.mpris.store(self.mpris_present, std::sync::atomic::Ordering::Relaxed);
        self.spot_status = (!spotify::logged_in())
            .then(|| "not logged in: Settings (s) › Log in, or run music_display --spotify-login".to_string());
        // the poller only runs in the real program: tests drive the client by hand
        #[cfg(not(test))]
        {
            self.spot_task = Some(tokio::spawn(api.clone().remote_loop(self.tx.clone())));
        }
        self.spot = Some(api);
        // logged in: the lists start loading now, so they are there when the browser opens
        #[cfg(not(test))]
        if self.preload_wanted() {
            self.start_preload();
        }
        self.dirty = true;
    }

    /// The background load only makes sense once Spotify is on and there is a token.
    pub fn preload_wanted(&self) -> bool {
        self.spot.is_some() && spotify::logged_in()
    }

    /// Turn Spotify off: stop the poller, forget what it showed, close the browser.
    pub fn stop_spotify(&mut self) {
        if let Some(t) = self.spot_task.take() {
            t.abort();
        }
        self.preload_queue.clear();
        self.spot = None;
        self.args.spotify = false;
        self.spot_status = None;
        self.remote = None;
        if self.remote_active() {
            self.snap = None;
            self.track_key = 0;
        }
        self.liked = None;
        self.liked_uri.clear();
        self.lib = Library::new();
        self.sync_queue_tab();
        self.dirty = true;
    }

    /// Rebuild the client (new Client ID, device name, token).
    pub(super) fn stop_spotifyd(&mut self) {
        if let Some(mut c) = self.spotifyd.take() {
            crate::spotifyd::stop(&mut c);
        }
    }

    pub(super) fn restart_spotify(&mut self) {
        if self.spot.is_some() {
            self.stop_spotify();
            self.args.spotify = true;
            self.start_spotify();
        }
    }

    /// "Log in": open the browser login without blocking the UI; pressing it again cancels.
    pub(super) fn toggle_login(&mut self) {
        if let Some(t) = self.login_task.take() {
            t.abort();
            self.login_msg = Some("login cancelled".into());
            return;
        }
        let id = self.args.spotify_client_id.clone().unwrap_or_default();
        if id.is_empty() {
            self.settings_note = Some("set the Client ID first (README › Spotify)".into());
            return;
        }
        self.login_msg = Some("waiting for the browser…".into());
        let tx = self.tx.clone();
        self.login_task = Some(tokio::spawn(async move {
            use spotify::{LoginMsg, Reply};
            let said = tx.clone();
            let say = move |step| {
                let _ = said.send(AppEvent::Spotify(Reply::Login(LoginMsg::Step(step))));
            };
            let r = spotify::login(&id, &say).await.map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Spotify(Reply::Login(LoginMsg::Done(r))));
        }));
    }

    pub(super) fn logout_spotify(&mut self) {
        if let Some(t) = self.login_task.take() {
            t.abort();
        }
        let was = spotify::logout();
        self.login_msg = Some(if was { "logged out" } else { "not logged in" }.into());
        self.restart_spotify();
    }

    /// Run a command in the background; the result comes back as a note.
    pub(super) fn spot_cmd(&mut self, cmd: spotify::Cmd) {
        let Some(api) = self.api() else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = api.run(cmd).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Done(r)));
            // the player state lags the command a little
            tokio::time::sleep(Duration::from_millis(400)).await;
            api.wake.notify_one();
        });
    }

    /// A playback command for whatever is on screen: playerctl, or the API for another device.
    pub(super) fn player_cmd(&mut self, args: &[&str], cmd: spotify::Cmd) {
        if self.remote_active() {
            self.spot_cmd(cmd);
        } else {
            self.ctl.send(args);
        }
    }

    pub(super) fn next_track(&mut self) {
        self.player_cmd(&["next"], spotify::Cmd::Next);
    }

    pub(super) fn prev_track(&mut self) {
        self.player_cmd(&["previous"], spotify::Cmd::Prev);
    }

    pub(super) fn seek_to(&mut self, secs: f64) {
        if self.remote_active() {
            self.spot_cmd(spotify::Cmd::Seek(secs));
        } else {
            self.ctl.seek_to(secs);
        }
    }

    /// The Spotify URI of the song playing, if it is a Spotify track.
    pub(super) fn playing_uri(&self) -> Option<String> {
        self.snap.as_ref().and_then(|s| spotify::track_uri(&s.track_id))
    }

    /// Ask once per song whether it is in Liked Songs.
    pub(super) fn check_liked(&mut self) {
        let uri = self.playing_uri().unwrap_or_default();
        if uri == self.liked_uri {
            return;
        }
        self.liked_uri = uri.clone();
        self.liked = None;
        let (Some(api), false) = (self.spot.clone(), uri.is_empty()) else { return };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(liked) = api.liked(&uri).await {
                let _ = tx.send(AppEvent::Spotify(spotify::Reply::Liked { uri, liked }));
            }
        });
    }

    pub(super) fn toggle_like(&mut self) {
        let Some(uri) = self.playing_uri() else {
            self.set_note("not a Spotify song");
            return;
        };
        if self.spot.is_none() {
            self.api();
            return;
        }
        let on = self.liked != Some(true);
        self.liked = Some(on);
        self.lib.tabs[TAB_LIKED].loaded = false;
        self.spot_cmd(spotify::Cmd::Like(uri, on));
    }

    pub(super) fn on_spotify(&mut self, r: spotify::Reply) {
        use spotify::{LoginMsg, LoginStep, Reply};
        self.dirty = true;
        // answers of a client that was switched off in the meantime
        if self.spot.is_none() && !matches!(r, Reply::Login(_)) {
            return;
        }
        match r {
            Reply::Login(LoginMsg::Step(LoginStep::Opened(false))) => {
                self.login_msg = Some("browser didn't open: run music_display --spotify-login".into());
            }
            Reply::Login(LoginMsg::Step(_)) => {}
            Reply::Login(LoginMsg::Done(res)) => {
                self.login_task = None;
                self.login_msg = res.err();
                self.settings_note = None;
                if self.login_msg.is_none() {
                    // a fresh client reads the new token
                    self.restart_spotify();
                }
            }
            Reply::Remote(Err(e)) => {
                // the poller runs every second or two: only what needs a word goes to the footer
                if e.starts_with("Spotify ") {
                    self.set_note(e.clone());
                }
                self.spot_status = Some(e);
            }
            Reply::Remote(Ok(r)) => {
                self.spot_status = None;
                self.remote = r.map(|r| (*r, Instant::now()));
                if self.mpris_present {
                    return;
                }
                match self.remote_snapshot() {
                    Some(s) => self.on_snapshot(s),
                    None if self.remote_active() => {
                        self.snap = None;
                        self.track_key = 0;
                    }
                    None => {}
                }
                self.check_liked();
            }
            Reply::Liked { uri, liked } if uri == self.liked_uri => self.liked = Some(liked),
            Reply::Liked { .. } => {}
            Reply::Done(Ok(n)) if n.is_empty() => {}
            Reply::Done(Ok(n)) => {
                self.lib.note = Some(n.clone());
                self.set_note(n);
            }
            Reply::Done(Err(e)) => {
                // the optimistic heart was wrong: ask again
                self.liked_uri.clear();
                self.check_liked();
                self.lib.note = Some(e.clone());
                self.set_note(e);
            }
            Reply::Tab(i, res) => {
                let failed = res.as_ref().err().cloned();
                let l = &mut self.lib.tabs[i];
                l.loading = false;
                match res {
                    Ok(items) => {
                        l.items = items;
                        l.loaded = true;
                        l.fetched = Some(Instant::now());
                        l.error = None;
                        l.sel = l.sel.min(l.items.len().saturating_sub(1));
                    }
                    // what was cached stays on screen; the footer says why it didn't refresh
                    Err(e) => l.error = Some(e),
                }
                if i == TAB_PLAYLISTS {
                    self.fill_picker();
                }
                if let Some(e) = failed {
                    self.set_note(e);
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
                    self.set_note(e);
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
                    self.set_note(e);
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
                        l.top = 0;
                    }
                    Err(e) => l.error = Some(e),
                }
                if let Some(e) = failed {
                    self.set_note(e);
                }
            }
            Reply::Devices(res) => {
                let Some(d) = &mut self.devices else { return };
                d.loading = false;
                let failed = res.as_ref().err().cloned();
                match res {
                    Ok(items) => {
                        // start on the device playing now
                        d.sel = items.iter().position(|x| x.active).unwrap_or(0);
                        d.items = items;
                        d.error = None;
                    }
                    Err(e) => d.error = Some(e),
                }
                if let Some(e) = failed {
                    self.set_note(e);
                }
            }
        }
    }

    // ── device picker (d) ──────────────────────────────────────────────

    pub(super) fn open_devices(&mut self) {
        let Some(api) = self.api() else { return };
        self.devices = Some(DevicePicker { loading: true, ..Default::default() });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = api.devices_list().await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Devices(r)));
        });
    }

    pub(super) fn handle_devices_key(&mut self, code: KeyCode) {
        let Some(d) = &mut self.devices else { return };
        let last = d.items.len().saturating_sub(1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => d.sel = d.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => d.sel = (d.sel + 1).min(last),
            KeyCode::Home => d.sel = 0,
            KeyCode::End => d.sel = last,
            KeyCode::Char('r') => self.open_devices(),
            KeyCode::Esc | KeyCode::Char('q' | 'd') => self.devices = None,
            KeyCode::Enter | KeyCode::Char(' ') => self.choose_device(),
            _ => {}
        }
    }

    /// Enter in the picker: move playback there (a device Spotify won't let apps control says so).
    pub(super) fn choose_device(&mut self) {
        let Some(dev) = self.devices.as_ref().and_then(|d| d.items.get(d.sel)).cloned() else { return };
        if dev.restricted {
            self.set_note(format!("{} is restricted by Spotify: it can't be controlled from here", dev.name));
            return;
        }
        self.devices = None;
        self.spot_cmd(spotify::Cmd::TransferTo { id: dev.id, name: dev.name });
    }

    // ── library browser ────────────────────────────────────────────────

    pub(super) fn open_library(&mut self) {
        if self.api().is_none() {
            return;
        }
        self.sync_queue_tab();
        self.lib.open = true;
        self.lib.note = self.spot_status.clone();
        // what is cached shows at once; a stale list is refreshed in the background
        self.load_tab(self.lib.tab, false);
        self.start_search_typing();
    }

    /// Click on Track / Artist / Album / Genre: show it in the library. Only with Spotify on.
    pub fn open_library_for(&mut self, f: MetaField) -> bool {
        if !f.opens_library() || self.spot.is_none() {
            return false;
        }
        let genre = self.genre.clone().unwrap_or_default();
        let remote = self.remote.as_ref().map(|(r, _)| r);
        let Some(link) = self.snap.as_ref().and_then(|s| browser::library_link(f, s, &genre, remote)) else {
            return false;
        };
        self.open_library();
        match link {
            LibLink::Open(it) => {
                self.switch_tab(if it.kind == "album" { TAB_ALBUMS } else { TAB_ARTISTS });
                self.open_item(it);
            }
            LibLink::Search(q) => {
                self.switch_tab(TAB_SEARCH);
                self.lib.input = TextInput::with(&q);
                self.run_search();
            }
        }
        true
    }

    pub(super) fn close_library(&mut self) {
        self.lib.open = false;
        self.lib.editing = false;
    }

    /// The button and the mouse toggle the browser like `o` does.
    pub(super) fn toggle_library(&mut self) {
        if self.lib.open {
            self.close_library();
        } else {
            self.open_library();
        }
    }

    /// The Queue tab follows the `spotify_queue_tab` setting.
    pub(super) fn sync_queue_tab(&mut self) {
        self.lib.queue_tab = !self.args.no_spotify_queue_tab;
        if !self.lib.queue_tab && self.lib.tab == TAB_QUEUE {
            self.lib.tab = 0;
            self.lib.stack.clear();
        }
    }

    /// Load the lists you are most likely to open (one after the other, never in a burst, and
    /// never before Spotify's Retry-After has run out) so they are in memory when the browser
    /// opens. Only runs once Spotify is on and logged in; nothing is written to disk.
    pub fn start_preload(&mut self) {
        if self.spot.is_none() {
            return;
        }
        self.preload_queue = [0usize, 1, 3, 4]
            .into_iter()
            .filter(|i| !self.lib.tabs[*i].loading && !self.lib.tabs[*i].fresh(browser::FRESH))
            .collect();
        self.preload_at = Some(Instant::now() + PRELOAD_DELAY);
    }

    /// Load the queued tabs one at a time, a few seconds apart, so the start does not spend
    /// Spotify's request budget in one burst. A tab opened (and so loaded) meanwhile is skipped,
    /// and a rate limit ends the whole thing: the browser asks again when a tab is opened.
    pub fn preload_tick(&mut self) {
        if self.preload_queue.is_empty() || self.preload_at.is_none_or(|t| Instant::now() < t) {
            return;
        }
        if self.spot.as_ref().is_none_or(|a| a.paused_for().is_some()) {
            self.preload_queue.clear();
            return;
        }
        if let Some(i) = self.preload_queue.pop_front() {
            self.load_tab(i, false);
            self.preload_at = Some(Instant::now() + PRELOAD_GAP);
        }
    }

    /// On the Search tab with nothing searched yet, the field is ready for typing.
    pub(super) fn start_search_typing(&mut self) {
        if self.lib.tab == TAB_SEARCH && self.lib.stack.is_empty() && self.lib.query.is_empty() {
            self.lib.editing = true;
        }
    }

    /// Run the search typed in the field (results replace the old ones when they arrive).
    pub(super) fn run_search(&mut self) {
        self.lib.editing = false;
        let q = self.lib.input.text.trim().to_string();
        if q.is_empty() {
            return;
        }
        let Some(api) = self.spot.clone() else { return };
        self.lib.query = q.clone();
        self.lib.stack.clear();
        let l = &mut self.lib.tabs[TAB_SEARCH];
        l.loading = true;
        l.error = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.search(&q).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Search { q, result }));
        });
    }

    /// Make sure tab `i` is loaded and fresh. A fresh list costs no request; a stale one keeps
    /// showing while it is fetched again in the background.
    pub(super) fn load_tab(&mut self, i: usize, force: bool) {
        if i == TAB_SEARCH {
            return; // searches run when asked for
        }
        let ttl = if i == TAB_QUEUE { browser::QUEUE_FRESH } else { browser::FRESH };
        let l = &mut self.lib.tabs[i];
        if l.loading || (!force && l.fresh(ttl)) {
            return;
        }
        let Some(api) = self.spot.clone() else { return };
        l.loading = true;
        l.error = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            use spotify::Reply;
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

    /// Tab / Shift-Tab: the next shown tab (the Queue tab may be switched off).
    pub(super) fn step_tab(&mut self, dir: i32) {
        let shown = self.lib.visible_tabs();
        let at = shown.iter().position(|t| *t == self.lib.tab).unwrap_or(0) as i32;
        let to = shown[(at + dir).rem_euclid(shown.len() as i32) as usize];
        self.switch_tab(to);
    }

    pub(super) fn switch_tab(&mut self, i: usize) {
        if !self.lib.visible_tabs().contains(&i) {
            return;
        }
        self.lib.tab = i;
        self.lib.stack.clear();
        self.lib.picker = None;
        self.lib.editing = false;
        self.load_tab(self.lib.tab, false);
        self.start_search_typing();
    }

    /// Open an album / own playlist / artist / Liked Songs as a new level.
    pub(super) fn open_item(&mut self, it: spotify::Item) {
        if !it.openable {
            self.lib.note = Some(match it.kind.as_str() {
                "playlist" => "Spotify only lets apps open your own playlists: Enter plays it".into(),
                _ => "Enter plays it".into(),
            });
            return;
        }
        let Some(api) = self.spot.clone() else { return };
        let uri = if it.kind == "liked" { "liked".to_string() } else { it.uri.clone() };
        self.lib.stack.push(Level { title: it.name, uri: uri.clone(), loading: true, ..Default::default() });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = api.open(&uri).await.map_err(|e| e.msg);
            let _ = tx.send(AppEvent::Spotify(spotify::Reply::Opened { uri, result }));
        });
    }

    pub(super) fn reload_level(&mut self) {
        if self.lib.picker.is_some() {
            self.load_tab(TAB_PLAYLISTS, true);
            return;
        }
        match self.lib.stack.pop() {
            Some(l) => {
                let kind = l.uri.split(':').nth(1).unwrap_or("liked").to_string();
                self.open_item(spotify::Item { kind, uri: l.uri, name: l.title, openable: true, ..Default::default() });
            }
            None if self.lib.tab == TAB_SEARCH => {
                self.lib.input = TextInput::with(&self.lib.query.clone());
                self.run_search();
            }
            None => self.load_tab(self.lib.tab, true),
        }
    }

    /// Enter: a track plays in the list's context (from that song on), anything else plays itself.
    pub(super) fn play_selected(&mut self) {
        let l = self.lib.level();
        let Some(it) = l.items.get(l.sel).cloned() else { return };
        let ctx = l.uri.clone();
        let cmd = match it.kind.as_str() {
            "track" if !ctx.is_empty() => spotify::Cmd::Play { ctx, track: it.uri },
            "track" => spotify::Cmd::Play { ctx: it.uri, track: String::new() },
            "liked" => spotify::Cmd::Play { ctx: "liked".into(), track: String::new() },
            _ => spotify::Cmd::Play { ctx: it.uri, track: String::new() },
        };
        self.spot_cmd(cmd);
        // the overlay gets out of the way; the side panel stays beside the player
        if !self.lib.hits.panel {
            self.close_library();
        }
    }

    pub(super) fn selected_track(&mut self) -> Option<spotify::Item> {
        let l = self.lib.level();
        let it = l.items.get(l.sel).filter(|it| it.kind == "track").cloned();
        if it.is_none() {
            self.lib.note = Some("pick a song first".into());
        }
        it
    }

    /// The "add to playlist" list: your own playlists (loaded with the Playlists tab).
    pub(super) fn fill_picker(&mut self) {
        let tab = &self.lib.tabs[TAB_PLAYLISTS];
        let (items, loading, error) = (
            tab.items.iter().filter(|p| p.openable).cloned().collect::<Vec<_>>(),
            tab.loading,
            tab.error.clone(),
        );
        if let Some((_, _, l)) = &mut self.lib.picker {
            l.items = items;
            l.loaded = !loading && error.is_none();
            l.loading = loading;
            l.error = error;
            l.sel = l.sel.min(l.items.len().saturating_sub(1));
        }
    }

    pub(super) fn open_picker(&mut self, it: spotify::Item) {
        let title = format!("Add “{}” to…", it.name);
        self.lib.picker = Some((it.uri, it.name, Level { title, ..Default::default() }));
        self.load_tab(TAB_PLAYLISTS, false);
        self.fill_picker();
    }

    /// A key for the browser. Returns whether it was used: the side panel leaves the player's
    /// own keys (volume, next…) working, so those come back as `false`.
    pub fn handle_library_key(&mut self, code: KeyCode) -> bool {
        // typing a search: every key is text until Enter (run it) or Esc (leave the field)
        if self.lib.editing {
            match code {
                // Tab isn't text: it moves on (so Tab can walk past an empty Search tab)
                KeyCode::Tab => self.step_tab(1),
                KeyCode::BackTab => self.step_tab(-1),
                _ => match self.lib.input.key(code) {
                    Some(Edit::Submit) => self.run_search(),
                    Some(Edit::Cancel) => self.lib.editing = false,
                    None => {}
                },
            }
            return true;
        }
        let mut used = true;
        let n = self.lib.level().items.len();
        let page = 10;
        let move_sel = |l: &mut Level, to: usize| l.sel = to.min(n.saturating_sub(1));
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                let l = self.lib.level_mut();
                l.sel = l.sel.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let l = self.lib.level_mut();
                let to = l.sel + 1;
                move_sel(l, to);
            }
            KeyCode::PageUp => {
                let l = self.lib.level_mut();
                l.sel = l.sel.saturating_sub(page);
            }
            KeyCode::PageDown => {
                let l = self.lib.level_mut();
                let to = l.sel + page;
                move_sel(l, to);
            }
            KeyCode::Home => self.lib.level_mut().sel = 0,
            KeyCode::End => move_sel(self.lib.level_mut(), n),
            KeyCode::Esc | KeyCode::Char('q') if self.lib.picker.is_some() => self.lib.picker = None,
            KeyCode::Esc | KeyCode::Char('q' | 'o') => self.close_library(),
            KeyCode::Enter if self.lib.picker.is_some() => {
                let Some((track, _, l)) = &self.lib.picker else { return true };
                let Some(pl) = l.items.get(l.sel).cloned() else { return true };
                let track = track.clone();
                self.lib.picker = None;
                self.spot_cmd(spotify::Cmd::Add { playlist: pl.uri, name: pl.name, track });
            }
            _ if self.lib.picker.is_some() => {
                if code == KeyCode::Char('r') {
                    self.reload_level();
                } else {
                    used = false;
                }
            }
            KeyCode::Tab => self.step_tab(1),
            KeyCode::BackTab => self.step_tab(-1),
            KeyCode::Char(c @ '1'..='7') => self.switch_tab(c as usize - '1' as usize),
            KeyCode::Char('Q') => self.switch_tab(TAB_QUEUE),
            KeyCode::Char('/') => {
                self.switch_tab(TAB_SEARCH);
                self.lib.editing = true;
            }
            KeyCode::Char('d') => self.open_devices(),
            KeyCode::Enter | KeyCode::Char(' ') => self.play_selected(),
            KeyCode::Right | KeyCode::Char('l') => {
                let l = self.lib.level();
                if let Some(it) = l.items.get(l.sel).cloned() {
                    self.open_item(it);
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                self.lib.stack.pop();
            }
            KeyCode::Char('a') => {
                if let Some(it) = self.selected_track() {
                    self.spot_cmd(spotify::Cmd::Queue(it.uri));
                }
            }
            KeyCode::Char('A') => {
                if let Some(it) = self.selected_track() {
                    self.open_picker(it);
                }
            }
            KeyCode::Char('f') => {
                if let Some(it) = self.selected_track() {
                    if it.uri == self.liked_uri {
                        self.liked = Some(true);
                    }
                    self.lib.tabs[TAB_LIKED].loaded = false;
                    self.spot_cmd(spotify::Cmd::Like(it.uri, true));
                }
            }
            KeyCode::Char('r') => self.reload_level(),
            _ => used = false,
        }
        used
    }

    // ── mouse in the browser, the device list and on the button ───────────

    /// A mouse event while the browser is open; returns whether it was used. The overlay takes
    /// every event (a click outside closes it); the side panel only takes those inside it, so
    /// the player beside it stays clickable. Click targets are the ones recorded while drawing.
    pub(super) fn library_mouse(&mut self, m: crossterm::event::MouseEvent) -> bool {
        let pos = Position::new(m.column, m.row);
        let h = self.lib.hits.clone();
        let inside = h.area.contains(pos);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    if h.panel {
                        return false;
                    }
                    self.close_library();
                    return true;
                }
                if let Some((_, tab)) = h.tabs.iter().find(|(r, _)| r.contains(pos)) {
                    self.switch_tab(*tab);
                } else if h.field.contains(pos) {
                    self.lib.editing = true;
                } else if let Some((_, item)) = h.rows.iter().find(|(r, _)| r.contains(pos)) {
                    self.lib.editing = false;
                    if *item == self.lib.level().sel {
                        // a click on the selected row is Enter
                        self.handle_library_key(KeyCode::Enter);
                    } else {
                        self.lib.level_mut().sel = *item;
                    }
                }
                true
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if inside => {
                let n = self.lib.level().items.len();
                let l = self.lib.level_mut();
                l.sel = if m.kind == MouseEventKind::ScrollUp {
                    l.sel.saturating_sub(3)
                } else {
                    (l.sel + 3).min(n.saturating_sub(1))
                };
                true
            }
            _ => !h.panel || inside,
        }
    }

    /// The device list takes every mouse event: click a row to select it, again to choose it,
    /// click outside to close, wheel to move.
    pub(super) fn devices_mouse(&mut self, m: crossterm::event::MouseEvent) {
        let Some(d) = &mut self.devices else { return };
        let pos = Position::new(m.column, m.row);
        let (mut close, mut choose) = (false, false);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !d.area.contains(pos) {
                    close = true;
                } else if let Some((_, i)) = d.rows.iter().find(|(r, _)| r.contains(pos)).cloned() {
                    if i == d.sel {
                        choose = true;
                    } else {
                        d.sel = i;
                    }
                }
            }
            MouseEventKind::ScrollUp => d.sel = d.sel.saturating_sub(1),
            MouseEventKind::ScrollDown => d.sel = (d.sel + 1).min(d.items.len().saturating_sub(1)),
            _ => {}
        }
        if close {
            self.devices = None;
        } else if choose {
            self.choose_device();
        }
    }

    /// Part of the layout (panel, button) changed since `before`: the screen needs a clean redraw.
    pub fn layout_key(&self) -> (bool, bool) {
        (self.lib.hits.panel, self.lib.hits.button.width > 0)
    }
}
