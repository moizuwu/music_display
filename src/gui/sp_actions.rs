//! What you can do with Spotify songs, albums, artists and playlists beyond playing them: the
//! right-click menu of a row, creating / renaming / deleting playlists, and the device popup of
//! the player bar. The requests are `spotify::Cmd`s; this file decides what is offered (a plain
//! function, tested) and draws it.
use super::model::Model;
use super::style::{self, Icon, ROW_SIDE, tool};
use crate::browser::{TAB_ALBUMS, TAB_ARTISTS, TAB_LIKED, TAB_PLAYLISTS};
use egui::{Id, RectAlign, RichText, Ui};
use music_display::spotify::{self, Cmd, Item};

// ── the row menu ────────────────────────────────────────────────────────────

/// One line of the right-click menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entry {
    Play,
    Queue,
    AddToPlaylist,
    /// Save to (`true`) or remove from (`false`) Liked Songs.
    Like(bool),
    /// Save the album (`true`) or remove it from the library.
    SaveAlbum(bool),
    /// Follow (`true`) or unfollow an artist or a playlist.
    Follow(bool),
    RemoveFromPlaylist,
    MoveUp,
    MoveDown,
    GoArtist,
    GoAlbum,
    CopyLink,
}

/// What the menu needs to know besides the kind of the row.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facts {
    /// Already in your library (liked, saved, followed).
    pub saved: bool,
    /// The row is a song in one of your own playlists.
    pub in_own_playlist: bool,
    pub can_up: bool,
    pub can_down: bool,
    pub has_artist: bool,
    pub has_album: bool,
    pub has_link: bool,
    /// The row is a playlist of your own (nothing to follow).
    pub own: bool,
}

/// The lines of the menu for a row of this kind, in Spotify's order.
pub fn entries(kind: &str, f: &Facts) -> Vec<Entry> {
    let mut out = vec![Entry::Play];
    match kind {
        "track" => {
            out.extend([Entry::Queue, Entry::AddToPlaylist, Entry::Like(!f.saved)]);
            if f.in_own_playlist {
                out.push(Entry::RemoveFromPlaylist);
                out.extend(f.can_up.then_some(Entry::MoveUp));
                out.extend(f.can_down.then_some(Entry::MoveDown));
            }
            out.extend(f.has_artist.then_some(Entry::GoArtist));
            out.extend(f.has_album.then_some(Entry::GoAlbum));
        }
        "album" => out.push(Entry::SaveAlbum(!f.saved)),
        "artist" => out.push(Entry::Follow(!f.saved)),
        "playlist" if !f.own => out.push(Entry::Follow(!f.saved)),
        _ => {}
    }
    out.extend(f.has_link.then_some(Entry::CopyLink));
    out
}

/// The icon of a device in the popup: by what Spotify says it is.
pub fn device_icon(kind: &str) -> Icon {
    match kind {
        "Computer" => Icon::Computer,
        "Smartphone" | "Tablet" => Icon::Phone,
        _ => Icon::Speaker,
    }
}

fn is_liked_list(uri: &str) -> bool {
    uri == "liked" || uri.ends_with(":collection")
}

/// One of your own playlists (the Playlists tab marks them openable).
fn own_playlist(m: &Model, uri: &str) -> bool {
    m.spot.lib.tabs[TAB_PLAYLISTS].items.iter().any(|p| p.uri == uri && p.openable)
}

fn in_tab(m: &Model, tab: usize, uri: &str) -> bool {
    m.spot.lib.tabs[tab].items.iter().any(|p| p.uri == uri)
}

/// Keep the cached list in step with what was just asked for (the real list is read again once the
/// request has gone through).
fn patch_tab(m: &mut Model, tab: usize, item: &Item, on: bool) {
    let l = &mut m.spot.lib.tabs[tab];
    l.items.retain(|p| p.uri != item.uri);
    if on {
        l.items.push(Item { openable: item.kind != "playlist" && item.openable, ..item.clone() });
    }
    l.sel = l.sel.min(l.items.len().saturating_sub(1));
    m.spot.reload_after.push(tab);
}

/// A right-click menu for a row, like Spotify's. `playlist_uri` is the album, playlist or `liked`
/// the row is listed in (songs then play from there, and in one of your own playlists they can be
/// removed or moved), or `None` for a list of mixed things.
pub fn item_menu(resp: &egui::Response, m: &mut Model, item: &Item, playlist_uri: Option<&str>) {
    resp.container_context_menu(|ui| menu_body(ui, m, item, playlist_uri));
}

fn menu_body(ui: &mut Ui, m: &mut Model, item: &Item, ctx: Option<&str>) {
    let ctx = ctx.filter(|c| !c.is_empty());
    let level = m.spot.lib.level();
    let here = ctx.is_some_and(|c| level.uri == c);
    let pos = here.then(|| level.items.iter().position(|i| i.uri == item.uri)).flatten();
    let n = level.items.len();
    let in_own = ctx.is_some_and(|c| c.starts_with("spotify:playlist:") && own_playlist(m, c));
    let playing_liked = m.playing_uri().as_deref() == Some(item.uri.as_str()) && m.spot.liked == Some(true);
    let album = Some(item.album_uri.as_str()).filter(|a| !a.is_empty()).or(ctx.filter(|c| c.starts_with("spotify:album:")));
    let facts = Facts {
        saved: match item.kind.as_str() {
            "track" => playing_liked || ctx.is_some_and(is_liked_list),
            "album" => in_tab(m, TAB_ALBUMS, &item.uri),
            "artist" => in_tab(m, TAB_ARTISTS, &item.uri),
            _ => in_tab(m, TAB_PLAYLISTS, &item.uri),
        },
        in_own_playlist: in_own && item.kind == "track",
        can_up: in_own && pos.is_some_and(|p| p > 0),
        can_down: in_own && pos.is_some_and(|p| p + 1 < n),
        has_artist: !item.artist_uri.is_empty(),
        has_album: album.is_some(),
        has_link: spotify::web_link(&item.uri).is_some(),
        own: item.openable && item.kind == "playlist",
    };
    let name = |ui: &mut Ui, text: &str| ui.button(text).clicked();
    for e in entries(&item.kind, &facts) {
        match e {
            Entry::Play => {
                if name(ui, "Play") {
                    let cmd = match (item.kind.as_str(), ctx) {
                        ("track", Some(c)) => Cmd::Play { ctx: c.to_string(), track: item.uri.clone() },
                        ("liked", _) => Cmd::Play { ctx: "liked".into(), track: String::new() },
                        _ => Cmd::Play { ctx: item.uri.clone(), track: String::new() },
                    };
                    m.spot.cmd(cmd);
                    ui.close();
                }
            }
            Entry::Queue => {
                if name(ui, "Add to queue") {
                    m.spot.cmd(Cmd::Queue(item.uri.clone()));
                    ui.close();
                }
            }
            Entry::AddToPlaylist => add_to_playlist_menu(ui, m, item),
            Entry::Like(on) => {
                if name(ui, if on { "Save to Liked Songs" } else { "Remove from Liked Songs" }) {
                    if m.playing_uri().as_deref() == Some(item.uri.as_str()) {
                        m.spot.liked = Some(on);
                    }
                    m.spot.lib.tabs[TAB_LIKED].loaded = false;
                    m.spot.cmd(Cmd::Like(item.uri.clone(), on));
                    ui.close();
                }
            }
            Entry::SaveAlbum(on) => {
                if name(ui, if on { "Save album" } else { "Remove album from library" }) {
                    patch_tab(m, TAB_ALBUMS, item, on);
                    m.spot.cmd(Cmd::SaveAlbum(item.uri.clone(), on));
                    ui.close();
                }
            }
            Entry::Follow(on) => {
                let (tab, label) = if item.kind == "artist" { (TAB_ARTISTS, "artist") } else { (TAB_PLAYLISTS, "playlist") };
                if name(ui, &if on { format!("Follow {label}") } else { format!("Unfollow {label}") }) {
                    patch_tab(m, tab, item, on);
                    m.spot.cmd(if tab == TAB_ARTISTS { Cmd::FollowArtist(item.uri.clone(), on) } else { Cmd::FollowPlaylist(item.uri.clone(), on) });
                    ui.close();
                }
            }
            Entry::RemoveFromPlaylist => {
                if let (Some(c), true) = (ctx, name(ui, "Remove from this playlist")) {
                    let l = m.spot.lib.level_mut();
                    if l.uri == c {
                        l.items.retain(|i| i.uri != item.uri);
                        l.sel = l.sel.min(l.items.len().saturating_sub(1));
                    }
                    m.spot.cmd(Cmd::RemoveFromPlaylist { playlist: c.to_string(), track: item.uri.clone() });
                    ui.close();
                }
            }
            Entry::MoveUp | Entry::MoveDown => {
                let up = e == Entry::MoveUp;
                if let (Some(c), Some(from), true) = (ctx, pos, name(ui, if up { "Move up" } else { "Move down" })) {
                    let to = if up { from - 1 } else { from + 1 };
                    m.spot.lib.level_mut().items.swap(from, to);
                    m.spot.cmd(Cmd::MoveInPlaylist { playlist: c.to_string(), from, to });
                    ui.close();
                }
            }
            Entry::GoArtist | Entry::GoAlbum => {
                let (label, uri) = if e == Entry::GoArtist { ("Go to artist", item.artist_uri.clone()) } else { ("Go to album", album.unwrap_or_default().to_string()) };
                if name(ui, label) {
                    m.spot.pending_nav = Some(uri);
                    ui.close();
                }
            }
            Entry::CopyLink => {
                if let (Some(link), true) = (spotify::web_link(&item.uri), name(ui, "Copy link")) {
                    ui.ctx().copy_text(link);
                    m.say("Link copied");
                    ui.close();
                }
            }
        }
    }
}

/// "Add to playlist": your own playlists, and "New playlist" (named after the song, like Spotify).
fn add_to_playlist_menu(ui: &mut Ui, m: &mut Model, item: &Item) {
    ui.menu_button("Add to playlist", |ui| {
        m.spot.load_tab(TAB_PLAYLISTS, false);
        let own: Vec<Item> = m.spot.lib.tabs[TAB_PLAYLISTS].items.iter().filter(|p| p.openable).cloned().collect();
        if ui.button("New playlist").clicked() {
            m.spot.reload_after.push(TAB_PLAYLISTS);
            m.spot.cmd(Cmd::CreatePlaylist { name: item.name.clone(), description: String::new(), track: item.uri.clone() });
            ui.close();
        }
        ui.separator();
        if own.is_empty() {
            let loading = m.spot.lib.tabs[TAB_PLAYLISTS].loading;
            style::note(ui, if loading { "Loading…" } else { "No playlists of your own yet" });
        }
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for p in own {
                if ui.button(&p.name).clicked() {
                    m.spot.cmd(Cmd::Add { playlist: p.uri, name: p.name, track: item.uri.clone() });
                    ui.close();
                }
            }
        });
    });
}

// ── playlist tools ──────────────────────────────────────────────────────────

/// What the inline editor of the playlist tools is doing.
#[derive(Clone, Default, PartialEq)]
enum Mode {
    #[default]
    Idle,
    /// Typing the name of a new playlist.
    Create(String),
    /// Typing a new name for the playlist (its URI).
    Rename(String, String),
    /// Asking whether to delete the playlist (its URI).
    Delete(String),
}

/// The opened level, when it is one of your own playlists: (uri, name).
fn open_own_playlist(m: &Model) -> Option<(String, String)> {
    let l = m.spot.lib.stack.last()?;
    (l.uri.starts_with("spotify:playlist:") && own_playlist(m, &l.uri)).then(|| (l.uri.clone(), l.title.clone()))
}

/// The "+" of [`playlist_tools`] on its own, for a header row: opens the name field below.
pub fn new_playlist_button(ui: &mut Ui, m: &Model) {
    if !m.spot.on() {
        return;
    }
    let id = Id::unique("sp-playlist-tools");
    let mode: Mode = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let creating = matches!(mode, Mode::Create(_));
    let clicked = if super::sp_look::active(ui.ctx()) {
        // Spotify's "+ Create" pill
        let lk = super::sp_look::SpLook::spotify(0.0);
        let text = egui::RichText::new("+  Create").font(lk.bold(lk.body)).color(lk.text);
        ui.add(egui::Button::new(text).fill(if creating { lk.chip } else { lk.raised }).corner_radius(16.0).min_size(egui::vec2(0.0, 30.0)))
            .on_hover_text("New playlist")
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    } else {
        tool(ui, ROW_SIDE, Icon::Plus, creating, "New playlist")
    };
    if clicked {
        let next = if creating { Mode::Idle } else { Mode::Create(String::new()) };
        ui.data_mut(|d| d.insert_temp(id, next));
    }
}

/// A "+" that creates a playlist and, inside one of your own playlists, rename and delete, each
/// with its inline field or question (no dialogs of the system).
/// `plus`: draw the "+" here (false when the caller has put it in a header with
/// [`new_playlist_button`]).
pub fn playlist_tools(ui: &mut Ui, m: &mut Model, plus: bool) {
    if !m.spot.on() {
        return;
    }
    let id = Id::unique("sp-playlist-tools");
    let mut mode: Mode = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let open = open_own_playlist(m);
    // the playlist on screen changed: what was asked about the old one is dropped
    match (&mode, &open) {
        (Mode::Rename(u, _) | Mode::Delete(u), Some((o, _))) if u != o => mode = Mode::Idle,
        (Mode::Rename(..) | Mode::Delete(_), None) => mode = Mode::Idle,
        _ => {}
    }
    // (nothing to show on this row: no row at all)
    if plus || open.is_some() {
        ui.horizontal(|ui| {
            if plus && tool(ui, ROW_SIDE, Icon::Plus, matches!(mode, Mode::Create(_)), "New playlist") {
                mode = if matches!(mode, Mode::Create(_)) { Mode::Idle } else { Mode::Create(String::new()) };
            }
            if let Some((uri, name)) = &open {
            if tool(ui, ROW_SIDE, Icon::Edit, matches!(mode, Mode::Rename(..)), "Rename this playlist") {
                mode = Mode::Rename(uri.clone(), name.clone());
            }
            if tool(ui, ROW_SIDE, Icon::Trash, matches!(mode, Mode::Delete(_)), "Delete this playlist") {
                mode = Mode::Delete(uri.clone());
            }
            }
        });
    }
    let mut done = false;
    let creating = matches!(mode, Mode::Create(_));
    let mut submit = None;
    if let Mode::Create(text) | Mode::Rename(_, text) = &mut mode {
        let (go, cancel) = name_row(ui, text, if creating { "Name of the new playlist" } else { "New name" });
        if go && !text.trim().is_empty() {
            submit = Some(text.trim().to_string());
        }
        done |= cancel;
    }
    if let Some(name) = submit {
        match &mode {
            Mode::Rename(uri, _) => {
                rename_locally(m, uri, &name);
                m.spot.cmd(Cmd::EditPlaylist { playlist: uri.clone(), name: Some(name), description: None });
            }
            _ => {
                m.spot.reload_after.push(TAB_PLAYLISTS);
                m.spot.cmd(Cmd::CreatePlaylist { name, description: String::new(), track: String::new() });
            }
        }
        done = true;
    }
    if let Mode::Delete(uri) = mode.clone() {
        let name = open.as_ref().map(|o| o.1.clone()).unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ui.visuals().warn_fg_color, format!("Delete “{name}”? It leaves your library."));
            if tool(ui, ROW_SIDE, Icon::Check, false, "Yes, delete it") {
                delete_locally(m, &uri);
                m.spot.cmd(Cmd::DeletePlaylist(uri.clone()));
                done = true;
            }
            if tool(ui, ROW_SIDE, Icon::Close, false, "Keep it") {
                done = true;
            }
        });
    }
    if done {
        mode = Mode::Idle;
    }
    ui.data_mut(|d| d.insert_temp(id, mode));
}

/// The name field with its confirm and cancel buttons: (confirmed, cancelled).
fn name_row(ui: &mut Ui, text: &mut String, hint: &str) -> (bool, bool) {
    let (mut go, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(200.0));
        if !r.has_focus() && !r.lost_focus() && !ui.ctx().memory(|mem| mem.focused().is_some()) {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if tool(ui, ROW_SIDE, Icon::Check, false, "Confirm") {
            go = true;
        }
        if tool(ui, ROW_SIDE, Icon::Close, false, "Cancel") {
            cancel = true;
        }
    });
    (go, cancel)
}

fn rename_locally(m: &mut Model, uri: &str, name: &str) {
    for l in m.spot.lib.stack.iter_mut().filter(|l| l.uri == uri) {
        l.title = name.to_string();
    }
    for p in m.spot.lib.tabs[TAB_PLAYLISTS].items.iter_mut().filter(|p| p.uri == uri) {
        p.name = name.to_string();
    }
}

fn delete_locally(m: &mut Model, uri: &str) {
    m.spot.lib.stack.retain(|l| l.uri != uri);
    let t = &mut m.spot.lib.tabs[TAB_PLAYLISTS];
    t.items.retain(|p| p.uri != uri);
    t.sel = t.sel.min(t.items.len().saturating_sub(1));
    m.spot.reload_after.push(TAB_PLAYLISTS);
}

// ── devices ─────────────────────────────────────────────────────────────────

/// The Devices icon of the player bar. It opens a popup above the bar that lists the Spotify
/// Connect devices (the one playing in the accent colour, each with a speaker, computer or phone)
/// and moves playback to the one clicked. The `d` key opens it too.
pub fn device_button(ui: &mut Ui, m: &mut Model) {
    let open_now = m.spot.devices.is_some();
    let r = style::tool_response(ui, style::ICON_SIDE, Icon::Devices, open_now, "Devices: move playback to another Spotify Connect device");
    let popup = egui::Popup::menu(&r).align(RectAlign::TOP_END).align_alternatives(&[RectAlign::TOP_START]).gap(6.0).width(300.0);
    let (id, ctx) = (popup.get_id(), ui.ctx().clone());
    let was_open = egui::Popup::is_id_open(&ctx, id);
    if r.clicked() && !was_open {
        m.spot.open_devices();
    } else if open_now && !was_open && !r.clicked() {
        egui::Popup::open_id(&ctx, id); // asked for by the key
    }
    let (mut choose, mut refresh) = (None, false);
    let accent = super::view::accent(ui);
    popup.show(|ui| {
        let Some(d) = &m.spot.devices else { return };
        ui.label(RichText::new("Connect to a device").strong());
        if d.loading {
            style::note(ui, "Loading…");
        }
        if let Some(e) = &d.error {
            style::error(ui, e);
        }
        for (i, dev) in d.items.iter().enumerate() {
            let tip = if dev.restricted { format!("{} is restricted by Spotify", dev.name) } else { format!("Play on {}", dev.name) };
            ui.horizontal(|ui| {
                if tool(ui, ROW_SIDE, device_icon(&dev.kind), dev.active, &tip) {
                    choose = Some(i);
                }
                let mut text = RichText::new(&dev.name);
                if dev.active {
                    text = text.color(accent).strong();
                }
                if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                    choose = Some(i);
                }
                let mut tag = Vec::new();
                if dev.local {
                    tag.push("this computer");
                }
                if dev.active {
                    tag.push("playing");
                }
                if dev.restricted {
                    tag.push("restricted");
                }
                if !tag.is_empty() {
                    ui.label(RichText::new(tag.join(" · ")).small().weak());
                }
            });
        }
        if d.items.is_empty() && !d.loading && d.error.is_none() {
            style::note(ui, "No devices: start Spotify on one");
        }
        if tool(ui, ROW_SIDE, Icon::Refresh, false, "Refresh the list") {
            refresh = true;
        }
    });
    if let Some(i) = choose {
        m.spot.choose_device(i);
    } else if refresh {
        m.spot.open_devices();
    }
    // the popup and the picker open and close together
    let open_after = egui::Popup::is_id_open(&ctx, id);
    if open_after && m.spot.devices.is_none() {
        egui::Popup::close_id(&ctx, id);
    } else if !open_after && m.spot.devices.is_some() {
        m.spot.devices = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(kind: &str, f: &Facts) -> Vec<Entry> {
        entries(kind, f)
    }

    #[test]
    fn a_song_gets_spotifys_menu_in_order() {
        let f = Facts { has_artist: true, has_album: true, has_link: true, ..Facts::default() };
        assert_eq!(
            kinds("track", &f),
            [Entry::Play, Entry::Queue, Entry::AddToPlaylist, Entry::Like(true), Entry::GoArtist, Entry::GoAlbum, Entry::CopyLink]
        );
        // a liked song offers to take the heart away
        assert!(kinds("track", &Facts { saved: true, ..f }).contains(&Entry::Like(false)));
    }

    #[test]
    fn songs_in_your_own_playlist_can_be_removed_and_moved_where_there_is_room() {
        let f = Facts { in_own_playlist: true, can_up: false, can_down: true, ..Facts::default() };
        let e = kinds("track", &f);
        assert!(e.contains(&Entry::RemoveFromPlaylist) && e.contains(&Entry::MoveDown) && !e.contains(&Entry::MoveUp));
        // not in a playlist of yours: none of that
        let e = kinds("track", &Facts { can_up: true, can_down: true, ..Facts::default() });
        assert!(!e.contains(&Entry::RemoveFromPlaylist) && !e.contains(&Entry::MoveUp) && !e.contains(&Entry::MoveDown));
    }

    #[test]
    fn albums_artists_and_playlists_get_their_own_save_or_follow() {
        assert_eq!(kinds("album", &Facts::default()), [Entry::Play, Entry::SaveAlbum(true)]);
        assert_eq!(kinds("album", &Facts { saved: true, has_link: true, ..Facts::default() }), [Entry::Play, Entry::SaveAlbum(false), Entry::CopyLink]);
        assert_eq!(kinds("artist", &Facts::default()), [Entry::Play, Entry::Follow(true)]);
        assert_eq!(kinds("playlist", &Facts { saved: true, ..Facts::default() }), [Entry::Play, Entry::Follow(false)]);
        // your own playlist is not followed, and Liked Songs only plays
        assert_eq!(kinds("playlist", &Facts { own: true, ..Facts::default() }), [Entry::Play]);
        assert_eq!(kinds("liked", &Facts::default()), [Entry::Play]);
    }

    #[test]
    fn devices_get_an_icon_by_kind() {
        assert!(device_icon("Computer") == Icon::Computer);
        assert!(device_icon("Smartphone") == Icon::Phone && device_icon("Tablet") == Icon::Phone);
        assert!(device_icon("Speaker") == Icon::Speaker && device_icon("CastAudio") == Icon::Speaker && device_icon("") == Icon::Speaker);
    }

    #[test]
    fn liked_lists_are_recognised() {
        assert!(is_liked_list("liked") && is_liked_list("spotify:user:me:collection"));
        assert!(!is_liked_list("spotify:playlist:p1"));
    }
}
