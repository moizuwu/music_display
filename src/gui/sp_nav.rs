//! The Spotify layout's pages: the stack the back / forward buttons walk through, the data each
//! page got from the API, and the requests that fetch it. Nothing here draws (see `sp_pages`).
use super::library::Spot;
use super::sp_img::Images;
use music_display::spotify::{Cmd, Item, PageData, Reply};
use music_display::AppEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A page's data younger than this is shown as it is and not asked for again.
pub const PAGE_FRESH: Duration = Duration::from_secs(90);
/// How many pages the back button remembers.
const HISTORY: usize = 60;

/// What the middle of the window shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Home,
    Search(String),
    /// By Spotify URI.
    Album(String),
    Playlist(String),
    Artist(String),
    Liked,
    Queue,
    /// The lyrics panel, full size.
    Lyrics,
}

impl Page {
    /// What the page is called in the page store.
    pub fn key(&self) -> String {
        match self {
            Page::Home => "home".into(),
            Page::Search(q) => format!("search:{q}"),
            Page::Album(u) | Page::Playlist(u) | Page::Artist(u) => u.clone(),
            Page::Liked => "liked".into(),
            Page::Queue => "queue".into(),
            Page::Lyrics => "lyrics".into(),
        }
    }

    /// The page an album / playlist / artist / Liked Songs row or card opens; a song opens its album.
    pub fn of(it: &Item) -> Option<Page> {
        if it.kind == "liked" {
            return Some(Page::Liked);
        }
        if it.uri.is_empty() && it.album_uri.is_empty() {
            return None;
        }
        match it.kind.as_str() {
            "album" => Some(Page::Album(it.uri.clone())),
            "playlist" => Some(Page::Playlist(it.uri.clone())),
            "artist" => Some(Page::Artist(it.uri.clone())),
            "liked" => Some(Page::Liked),
            "track" if !it.album_uri.is_empty() => Some(Page::Album(it.album_uri.clone())),
            _ => None,
        }
    }

    /// The page of a bare `spotify:album:…` / `spotify:artist:…` / `spotify:playlist:…` URI
    /// ("Go to album" / "Go to artist" in a row menu).
    pub fn from_uri(uri: &str) -> Option<Page> {
        match uri.split(':').nth(1)? {
            "album" => Some(Page::Album(uri.to_string())),
            "artist" => Some(Page::Artist(uri.to_string())),
            "playlist" => Some(Page::Playlist(uri.to_string())),
            _ => None,
        }
    }

    /// Does the page have data of its own to ask the API for? (The queue comes from the library's
    /// queue list, the lyrics from the player.)
    pub fn fetched(&self) -> bool {
        !matches!(self, Page::Queue | Page::Lyrics)
    }

    /// The context the songs of this page play in; empty = each song plays alone.
    pub fn context(&self) -> String {
        match self {
            Page::Album(u) | Page::Playlist(u) => u.clone(),
            Page::Liked => "liked".into(),
            _ => String::new(),
        }
    }

    /// What the big play button plays (none on pages that are not one thing).
    pub fn play_target(&self) -> Option<String> {
        match self {
            Page::Album(u) | Page::Playlist(u) | Page::Artist(u) => Some(u.clone()),
            Page::Liked => Some("liked".into()),
            _ => None,
        }
    }
}

/// The pages visited, and where the back / forward buttons stand in them.
#[derive(Debug)]
pub struct Nav {
    stack: Vec<Page>,
    at: usize,
}

impl Default for Nav {
    fn default() -> Self {
        Nav { stack: vec![Page::Home], at: 0 }
    }
}

impl Nav {
    pub fn current(&self) -> &Page {
        &self.stack[self.at]
    }

    /// Open a page: what was ahead of the current one is dropped. False when it is already there.
    pub fn go(&mut self, p: Page) -> bool {
        if *self.current() == p {
            return false;
        }
        self.stack.truncate(self.at + 1);
        self.stack.push(p);
        self.at += 1;
        if self.stack.len() > HISTORY {
            self.stack.remove(0);
            self.at -= 1;
        }
        true
    }

    pub fn can_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.stack.len()
    }

    pub fn back(&mut self) -> bool {
        let ok = self.can_back();
        if ok {
            self.at -= 1;
        }
        ok
    }

    pub fn forward(&mut self) -> bool {
        let ok = self.can_forward();
        if ok {
            self.at += 1;
        }
        ok
    }
}

/// What the API said about a page (the old data stays on screen while it is fetched again).
#[derive(Default)]
pub struct Entry {
    pub data: Option<Arc<PageData>>,
    pub loading: bool,
    pub error: Option<String>,
    pub fetched: Option<Instant>,
}

impl Entry {
    fn fresh(&self) -> bool {
        self.data.is_some() && self.fetched.is_some_and(|t| t.elapsed() < PAGE_FRESH)
    }
}

/// The Spotify layout's state: pages, their data, pictures, and the small bits of the top strip
/// and the library column.
#[derive(Default)]
pub struct Pages {
    pub nav: Nav,
    pub store: HashMap<String, Entry>,
    pub images: Images,
    /// The text of the search field.
    pub query: String,
    /// The library column's filters: Playlists, Albums, Artists (none on = all of them).
    pub chips: [bool; 3],
    /// The Spotify layout is what is on screen (a click on the title or artist then opens a page
    /// instead of the library window).
    pub active: bool,
    /// Put the cursor in the search field once (after the Search key or a click on its icon).
    pub focus_search: bool,
}

impl Pages {
    pub fn entry(&self, p: &Page) -> Option<&Entry> {
        self.store.get(&p.key())
    }

    /// Open a page. A page that failed before is asked again.
    pub fn go(&mut self, p: Page) {
        if let Page::Search(q) = &p {
            self.query = q.clone();
        }
        self.nav.go(p);
        self.arrive();
    }

    pub fn back(&mut self) {
        if self.nav.back() {
            self.arrive();
        }
    }

    pub fn forward(&mut self) {
        if self.nav.forward() {
            self.arrive();
        }
    }

    /// The page on screen changed: clear an old failure so it is asked again; the search field
    /// shows what the page searched for.
    fn arrive(&mut self) {
        let cur = self.nav.current().clone();
        if let Some(e) = self.store.get_mut(&cur.key()) {
            if !e.loading {
                e.error = None;
            }
        }
        if let Page::Search(q) = cur {
            self.query = q;
        }
    }

    /// Should the page be requested now?
    fn needs(&self, p: &Page) -> bool {
        p.fetched() && self.store.get(&p.key()).is_none_or(|e| !e.loading && e.error.is_none() && !e.fresh())
    }

    pub fn on_page(&mut self, key: &str, result: Result<Box<PageData>, String>) {
        let e = self.store.entry(key.to_string()).or_default();
        e.loading = false;
        match result {
            Ok(d) => {
                e.data = Some(Arc::new(*d));
                e.error = None;
                e.fetched = Some(Instant::now());
            }
            // what was there stays; the error shows above it
            Err(m) => e.error = Some(m),
        }
    }
}

/// The command that plays `it` from a page whose songs play in `ctx` (empty: the song alone).
pub fn play_cmd(ctx: &str, it: &Item) -> Cmd {
    match it.kind.as_str() {
        "track" if !ctx.is_empty() => Cmd::Play { ctx: ctx.into(), track: it.uri.clone() },
        "liked" => Cmd::Play { ctx: "liked".into(), track: String::new() },
        _ => Cmd::Play { ctx: it.uri.clone(), track: String::new() },
    }
}

impl Spot {
    /// Ask for the page on screen when it is missing or stale (cheap when it is not).
    pub fn ensure_page(&mut self) {
        let p = self.pg.nav.current().clone();
        self.load_page(&p, false);
    }

    /// Fetch a page's data in the background; `force` also when it is fresh.
    pub fn load_page(&mut self, page: &Page, force: bool) {
        if !page.fetched() || (!force && !self.pg.needs(page)) {
            return;
        }
        let Some(api) = self.api.clone() else { return };
        let key = page.key();
        let e = self.pg.store.entry(key.clone()).or_default();
        if e.loading {
            return;
        }
        e.loading = true;
        e.error = None;
        let (tx, page) = (self.tx.clone(), page.clone());
        tokio::spawn(async move {
            let r = match &page {
                Page::Home => api.page_home().await,
                Page::Search(q) => api.page_search(q).await,
                Page::Album(u) => api.page_album(u).await,
                Page::Playlist(u) => api.page_playlist(u).await,
                Page::Artist(u) => api.page_artist(u).await,
                Page::Liked => api.page_liked().await,
                Page::Queue | Page::Lyrics => return,
            };
            let _ = tx.send(AppEvent::Spotify(Reply::Page { key, result: r.map(Box::new).map_err(|e| e.msg) }));
        });
    }

    /// The search field was submitted.
    pub fn search_page(&mut self) {
        let q = self.pg.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        let page = Page::Search(q);
        if self.pg.nav.current() == &page {
            self.load_page(&page, true);
        } else {
            self.pg.go(page);
        }
    }

    /// A click on a row, card or name: open its page.
    pub fn open_page(&mut self, it: &Item) {
        match Page::of(it) {
            Some(p) => self.pg.go(p),
            None => self.say_pg("nothing to open here"),
        }
    }

    /// A click on Track / Artist / Album in the window's own metadata or the player bar.
    pub fn open_link(&mut self, link: crate::browser::LibLink) {
        use crate::browser::LibLink;
        match link {
            LibLink::Open(it) => self.open_page(&it),
            LibLink::Search(q) => {
                self.pg.query = q;
                self.search_page();
            }
        }
    }

    /// Play a song of a page (in the page's context, else alone).
    pub fn play_in(&mut self, ctx: &str, it: &Item) {
        self.cmd(play_cmd(ctx, it));
    }

    /// The big play button of a page: everything from the start, or shuffled.
    pub fn play_page(&mut self, page: &Page, shuffle: bool) {
        let Some(ctx) = page.play_target() else { return };
        self.cmd(if shuffle { Cmd::PlayShuffled(ctx) } else { Cmd::Play { ctx, track: String::new() } });
    }

    /// Reload the page on screen (and the lists behind Home and the queue).
    pub fn reload_page(&mut self) {
        let p = self.pg.nav.current().clone();
        match p {
            Page::Home => {
                self.load_tab(0, true);
                self.load_tab(1, true);
            }
            Page::Queue => self.load_tab(crate::browser::TAB_QUEUE, true),
            _ => {}
        }
        self.load_page(&p, true);
    }

    fn say_pg(&mut self, msg: &str) {
        self.note = Some(msg.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, uri: &str) -> Item {
        Item { kind: kind.into(), uri: uri.into(), ..Default::default() }
    }

    #[test]
    fn back_and_forward_walk_the_pages_and_a_new_page_drops_the_forward_ones() {
        let mut n = Nav::default();
        assert_eq!(n.current(), &Page::Home);
        assert!(!n.can_back() && !n.can_forward());
        assert_eq!(Page::from_uri("spotify:artist:x"), Some(Page::Artist("spotify:artist:x".into())));
        assert_eq!(Page::from_uri("spotify:track:x"), None);
        assert_eq!(Page::from_uri("nonsense"), None);
        assert!(n.go(Page::Album("a".into())));
        assert!(n.go(Page::Artist("b".into())));
        assert!(!n.go(Page::Artist("b".into())), "the same page again is no new step");
        assert!(n.back() && n.back());
        assert_eq!(n.current(), &Page::Home);
        assert!(!n.back());
        assert!(n.forward());
        assert_eq!(n.current(), &Page::Album("a".into()));
        assert!(n.go(Page::Liked));
        assert!(!n.can_forward(), "going somewhere new cuts the way forward");
        assert!(n.back());
        assert_eq!(n.current(), &Page::Album("a".into()));
    }

    #[test]
    fn the_history_is_capped() {
        let mut n = Nav::default();
        for i in 0..200 {
            n.go(Page::Album(i.to_string()));
        }
        assert_eq!(n.current(), &Page::Album("199".into()));
        let mut steps = 0;
        while n.back() {
            steps += 1;
        }
        assert_eq!(steps, HISTORY - 1);
    }

    #[test]
    fn rows_open_the_page_of_what_they_are() {
        assert_eq!(Page::of(&item("album", "spotify:album:1")), Some(Page::Album("spotify:album:1".into())));
        assert_eq!(Page::of(&item("playlist", "spotify:playlist:1")), Some(Page::Playlist("spotify:playlist:1".into())));
        assert_eq!(Page::of(&item("artist", "spotify:artist:1")), Some(Page::Artist("spotify:artist:1".into())));
        assert_eq!(Page::of(&item("liked", "spotify:user:me:collection")), Some(Page::Liked));
        // a song opens its album when it knows it
        let mut t = item("track", "spotify:track:1");
        assert_eq!(Page::of(&t), None);
        t.album_uri = "spotify:album:9".into();
        assert_eq!(Page::of(&t), Some(Page::Album("spotify:album:9".into())));
        assert_eq!(Page::of(&Item::default()), None);
    }

    #[test]
    fn what_a_page_plays() {
        let t = item("track", "spotify:track:1");
        let alb = Page::Album("spotify:album:1".into());
        assert!(matches!(play_cmd(&alb.context(), &t), Cmd::Play { ctx, track } if ctx == "spotify:album:1" && track == "spotify:track:1"));
        // search, home, queue, artist: the song alone
        assert!(matches!(play_cmd(&Page::Home.context(), &t), Cmd::Play { ctx, track } if ctx == "spotify:track:1" && track.is_empty()));
        assert!(matches!(play_cmd(&Page::Artist("a".into()).context(), &t), Cmd::Play { ctx, .. } if ctx == "spotify:track:1"));
        // a card plays the whole thing
        let card = item("album", "spotify:album:2");
        assert!(matches!(play_cmd("", &card), Cmd::Play { ctx, track } if ctx == "spotify:album:2" && track.is_empty()));
        assert!(matches!(play_cmd("", &item("liked", "x")), Cmd::Play { ctx, .. } if ctx == "liked"));
        assert_eq!(Page::Liked.play_target().as_deref(), Some("liked"));
        assert_eq!(Page::Artist("a".into()).play_target().as_deref(), Some("a"));
        assert_eq!(Page::Search("x".into()).play_target(), None);
    }

    #[test]
    fn a_failed_page_is_not_asked_again_until_it_is_visited_again() {
        let mut p = Pages::default();
        let alb = Page::Album("spotify:album:1".into());
        assert!(p.needs(&alb));
        p.on_page(&alb.key(), Err("Spotify is rate limiting".into()));
        assert!(!p.needs(&alb), "no request storm after an error");
        p.go(alb.clone());
        assert!(p.entry(&alb).unwrap().error.is_none(), "visiting it again clears the error");
        assert!(p.needs(&alb));
        // data arrives: fresh, then stale pages are asked for again; errors keep the old data
        p.on_page(&alb.key(), Ok(Box::default()));
        assert!(!p.needs(&alb));
        p.store.get_mut(&alb.key()).unwrap().fetched = Some(Instant::now() - PAGE_FRESH - Duration::from_secs(1));
        assert!(p.needs(&alb));
        p.on_page(&alb.key(), Err("x".into()));
        assert!(p.entry(&alb).unwrap().data.is_some());
        assert!(!p.needs(&Page::Queue) && !p.needs(&Page::Lyrics));
    }

    #[test]
    fn the_search_field_follows_the_search_page() {
        let mut p = Pages::default();
        p.go(Page::Search("night drive".into()));
        assert_eq!(p.query, "night drive");
        p.go(Page::Liked);
        p.query = "typed but not sent".into();
        p.back();
        assert_eq!(p.query, "night drive");
    }
}
