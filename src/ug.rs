use crate::chords::{self, Sheet};
use crate::util::{cache_dir, fnv1a, now_secs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const BASE: &str = "https://www.ultimate-guitar.com";
// UG answers non-browser agents with 403, so this looks like a normal browser.
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
const NEG_TTL: u64 = 2 * 24 * 3600;
const MAX_FAILS: u32 = 3;
/// After `MAX_FAILS` hard failures in a row the provider rests this long, then tries again.
const BREAKER_SECS: u64 = 10 * 60;
const MIN_CHORDS: usize = 8;

/// Consecutive hard failures (blocked / format changed / network). Three in a row
/// switch the provider off for `BREAKER_SECS`, so a bad minute does not cost the whole session.
static FAILS: AtomicU32 = AtomicU32::new(0);
/// When the last of them happened (unix seconds).
static LAST_FAIL: AtomicU64 = AtomicU64::new(0);
/// `--chords-probe --probe-save DIR`: keep every page we fetch.
pub static SAVE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub enum Outcome {
    Found(Sheet),
    NotFound,
    Error(String),
}

// ── page parsing ───────────────────────────────────────────────────────
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "quot" => Some('"'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The `data-content` JSON of the `js-store` element.
pub fn extract_store(html: &str) -> Option<Value> {
    let at = html.find("js-store")?;
    let tag_start = html[..at].rfind('<')?;
    let tag_end = at + html[at..].find('>')?;
    let tag = &html[tag_start..tag_end];
    let k = tag.find("data-content=\"")? + "data-content=\"".len();
    let raw = &tag[k..k + tag[k..].find('"')?];
    serde_json::from_str(&unescape(raw)).ok()
}

/// First value stored under `key` anywhere in the document — survives UG moving things.
pub fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .filter(|x| !x.is_null())
            .or_else(|| m.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

/// The object that holds `key` — the chord page itself, so its capo / tuning / key aren't
/// confused with those of related tabs elsewhere in the page data.
fn parent_of<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) if m.get(key).is_some_and(|x| !x.is_null()) => Some(v),
        Value::Object(m) => m.values().find_map(|x| parent_of(x, key)),
        Value::Array(a) => a.iter().find_map(|x| parent_of(x, key)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Cand {
    pub url: String,
    pub artist: String,
    pub song: String,
    pub rating: f64,
    pub votes: f64,
    pub score: f64,
}

/// Chord pages for this song, best first: rating × ln(votes), after checking that the
/// title and artist really are the ones we asked for.
pub fn candidates(store: &Value, artist: &str, title: &str) -> Vec<Cand> {
    let Some(Value::Array(results)) = find_key(store, "results") else {
        return vec![];
    };
    let want_t = chords::norm(&crate::year::clean(title));
    let want_a = chords::norm(artist.split(", ").next().unwrap_or(artist));
    let mut out: Vec<Cand> = results
        .iter()
        .filter_map(|r| {
            let url = r.get("tab_url")?.as_str()?.to_string();
            if r.get("type").and_then(Value::as_str).is_some_and(|t| t != "Chords") {
                return None;
            }
            let song = r.get("song_name").and_then(Value::as_str).unwrap_or("").to_string();
            let art = r.get("artist_name").and_then(Value::as_str).unwrap_or("").to_string();
            let (st, sa) = (chords::norm(&crate::year::clean(&song)), chords::norm(&art));
            let title_ok = chords::dice(&want_t, &st) >= 0.8;
            let artist_ok = !want_a.is_empty() && !sa.is_empty() && (sa.contains(&want_a) || want_a.contains(&sa) || chords::dice(&want_a, &sa) >= 0.7);
            if !title_ok || !artist_ok {
                return None;
            }
            let rating = r.get("rating").and_then(Value::as_f64).unwrap_or(0.0);
            let votes = r.get("votes").and_then(Value::as_f64).unwrap_or(0.0);
            Some(Cand {
                url,
                artist: art,
                song,
                rating,
                votes,
                score: rating * (1.0 + votes).ln(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(3);
    out
}

/// A chord page → sheet (`None` if it has too few chords to be a real chord sheet).
pub fn sheet_from_store(store: &Value) -> Option<Sheet> {
    let content = find_key(store, "wiki_tab")
        .and_then(|w| w.get("content"))
        .and_then(Value::as_str)?;
    let mut sheet = chords::parse_text(content);
    // metadata sits next to `wiki_tab` (in `tab_view`); search there before the whole page
    let tab = parent_of(store, "wiki_tab").unwrap_or(store);
    let find_key = |_: &Value, k: &str| find_key(tab, k).or_else(|| find_key(store, k));
    if sheet.capo == 0 {
        sheet.capo = find_key(store, "capo")
            .and_then(Value::as_u64)
            .filter(|c| (1..=12).contains(c))
            .unwrap_or(0) as u8;
    }
    if sheet.key.is_none() {
        sheet.key = find_key(store, "tonality_name")
            .and_then(Value::as_str)
            .filter(|k| chords::parse_key(k).is_some())
            .map(str::to_string);
    }
    if sheet.bpm.is_none() {
        sheet.bpm = find_key(store, "bpm")
            .and_then(|b| b.as_u64().or_else(|| b.as_str().and_then(|s| s.parse().ok())))
            .filter(|b| (30..=300).contains(b))
            .map(|b| b as u32);
    }
    if sheet.tuning.is_none() {
        // UG: "tuning": {"name": "Half Step Down", "value": "Eb Ab Db Gb Bb Eb"}
        let text = |v: Option<&Value>| {
            v.and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty() && n.len() < 40 && !n.contains(['}', '\n']))
                .map(str::to_string)
        };
        sheet.tuning = find_key(store, "tuning").and_then(|t| match t {
            Value::Object(m) => text(m.get("name")).or_else(|| text(m.get("value"))),
            v => text(Some(v)),
        });
    }
    (sheet.chord_count() >= MIN_CHORDS).then(|| {
        sheet.source = "ultimate-guitar".into();
        sheet
    })
}

// ── network ────────────────────────────────────────────────────────────
/// One request, and one more after a short pause when the first failed in a way that can pass
/// (no connection, 429, 5xx). A 403 or 404 is an answer and is not repeated.
async fn get(c: &reqwest::Client, url: &str, query: &[(&str, &str)]) -> Result<String, String> {
    match get_once(c, url, query).await {
        Err((true, _)) => {
            tokio::time::sleep(std::time::Duration::from_millis(if crate::util::testing() { 20 } else { 800 })).await;
            get_once(c, url, query).await.map_err(|e| e.1)
        }
        r => r.map_err(|e| e.1),
    }
}

/// `Err((worth_a_retry, message))`.
async fn get_once(c: &reqwest::Client, url: &str, query: &[(&str, &str)]) -> Result<String, (bool, String)> {
    let r = c
        .get(url)
        .query(query)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| (true, format!("network error ({})", e.without_url())))?;
    let st = r.status();
    if !st.is_success() {
        let transient = st.as_u16() == 429 || st.is_server_error();
        return Err((transient, format!("HTTP {} from ultimate-guitar", st.as_u16())));
    }
    let body = r.text().await.map_err(|e| (true, format!("read error ({e})")))?;
    if let Some(dir) = SAVE_DIR.get() {
        let name = format!("{:016x}.html", fnv1a(url) ^ fnv1a(&format!("{query:?}")));
        let _ = std::fs::write(dir.join(name), &body);
    }
    Ok(body)
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    kind: String, // found | none
    chordpro: String,
    saved: u64,
}

fn cache_path(artist: &str, title: &str) -> PathBuf {
    cache_dir().join(format!("chords-ug-{:016x}.json", fnv1a(&format!("{artist}\0{title}"))))
}

fn fail(msg: String) -> Outcome {
    FAILS.fetch_add(1, Ordering::Relaxed);
    LAST_FAIL.store(now_secs(), Ordering::Relaxed);
    Outcome::Error(msg)
}

/// Switched off after repeated failures, until the rest period is over.
pub fn disabled() -> bool {
    if FAILS.load(Ordering::Relaxed) < MAX_FAILS {
        return false;
    }
    if now_secs().saturating_sub(LAST_FAIL.load(Ordering::Relaxed)) >= BREAKER_SECS {
        FAILS.store(0, Ordering::Relaxed);
        return false;
    }
    true
}

#[doc(hidden)] // test hook, used by the binary's tests too
pub fn reset_breaker() {
    FAILS.store(0, Ordering::Relaxed);
    LAST_FAIL.store(0, Ordering::Relaxed);
}

#[doc(hidden)]
pub fn set_last_fail(secs: u64) {
    LAST_FAIL.store(secs, Ordering::Relaxed);
}

/// Cache → search (twice at most) → fetch the best pages, 3 per search at most, and
/// only for tracks that aren't cached. Errors are never cached.
pub async fn lookup(c: &reqwest::Client, base: &str, artist: &str, title: &str) -> Outcome {
    let path = cache_path(artist, title);
    if let Ok(txt) = tokio::fs::read_to_string(&path).await {
        if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
            if e.kind == "found" {
                let mut s = chords::parse_chordpro(&e.chordpro);
                if s.chord_count() >= MIN_CHORDS {
                    s.source = "ultimate-guitar (cached)".into();
                    return Outcome::Found(s);
                }
            } else if now_secs().saturating_sub(e.saved) < NEG_TTL {
                return Outcome::NotFound;
            }
        }
    }
    if disabled() {
        return Outcome::Error("ultimate-guitar switched off after repeated failures".into());
    }

    // Search "artist title" first; when none of its chord pages is the song (or none has real
    // chords) search the title alone, since UG's ranking often buries a song under the artist
    // words. A page that fails to load does not end the lookup: the next one is tried.
    let first_artist = artist.split(", ").next().unwrap_or(artist);
    let cleaned = crate::year::clean(title);
    let mut queries = vec![format!("{first_artist} {cleaned}")];
    if !first_artist.trim().is_empty() && !cleaned.trim().is_empty() {
        queries.push(cleaned.clone());
    }
    let mut sheet = None;
    let mut tried: Vec<String> = Vec::new();
    let mut page_error: Option<String> = None;
    'search: for (n, q) in queries.iter().enumerate() {
        let html = match get(c, &format!("{base}/search.php"), &[("search_type", "title"), ("value", q)]).await {
            Ok(h) => h,
            Err(e) if n == 0 => return fail(e),
            Err(e) => {
                page_error = Some(e);
                break;
            }
        };
        let Some(store) = extract_store(&html) else {
            if n == 0 {
                return fail("ultimate-guitar page format not recognised (blocked or changed)".into());
            }
            page_error = Some("ultimate-guitar page format not recognised (blocked or changed)".into());
            break;
        };
        for cand in candidates(&store, artist, title) {
            let url = if cand.url.starts_with('/') { format!("{base}{}", cand.url) } else { cand.url.clone() };
            if tried.contains(&url) {
                continue;
            }
            tried.push(url.clone());
            let html = match get(c, &url, &[]).await {
                Ok(h) => h,
                Err(e) => {
                    page_error = Some(e);
                    continue;
                }
            };
            let Some(page) = extract_store(&html) else {
                page_error = Some("ultimate-guitar tab page format not recognised".into());
                continue;
            };
            if let Some(s) = sheet_from_store(&page) {
                sheet = Some(s);
                break 'search;
            }
        }
    }
    // pages that would not load: say so (and cache nothing) instead of calling the song unknown
    if sheet.is_none() {
        if let Some(e) = page_error {
            return fail(e);
        }
    }
    FAILS.store(0, Ordering::Relaxed);
    let entry = match &sheet {
        Some(s) => CacheEntry { kind: "found".into(), chordpro: chords::to_chordpro(s), saved: now_secs() },
        None => CacheEntry { kind: "none".into(), chordpro: String::new(), saved: now_secs() },
    };
    if let Ok(j) = serde_json::to_string(&entry) {
        let _ = tokio::fs::write(&path, j).await;
    }
    match sheet {
        Some(s) => Outcome::Found(s),
        None => Outcome::NotFound,
    }
}

/// Local folder first, then (if enabled) Ultimate Guitar.
pub async fn resolve(
    c: reqwest::Client,
    dir: PathBuf,
    use_ug: bool,
    artist: String,
    title: String,
) -> Result<Option<Sheet>, String> {
    let base = crate::year::base_title(&title);
    let retry = base != title;
    if let Some(s) = chords::load_local(dir.clone(), artist.clone(), title.clone()).await {
        return Ok(Some(s));
    }
    if retry {
        if let Some(s) = chords::load_local(dir, artist.clone(), base.clone()).await {
            return Ok(Some(s));
        }
    }
    if !use_ug {
        return Ok(None);
    }
    match lookup(&c, BASE, &artist, &title).await {
        Outcome::Found(s) => Ok(Some(s)),
        // nothing under the full title: try the bare song name
        Outcome::NotFound if retry => match lookup(&c, BASE, &artist, &base).await {
            Outcome::Found(s) => Ok(Some(s)),
            Outcome::NotFound => Ok(None),
            Outcome::Error(e) => Err(e),
        },
        Outcome::NotFound => Ok(None),
        Outcome::Error(e) => Err(e),
    }
}

/// `music_display --chords-probe "Artist" "Title"`: show each step so a broken lookup
/// can be diagnosed (add `--probe-save DIR` to keep the raw pages as test fixtures).
pub async fn probe(c: &reqwest::Client, artist: &str, title: &str, save: Option<PathBuf>) {
    println!("probe: {artist} — {title}");
    if let Some(d) = save {
        let _ = std::fs::create_dir_all(&d);
        println!("saving raw pages to {}", d.display());
        let _ = SAVE_DIR.set(d);
    }
    let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::clean(title));
    println!("1. search \"{q}\"");
    let html = match get(c, &format!("{BASE}/search.php"), &[("search_type", "title"), ("value", &q)]).await {
        Ok(h) => h,
        Err(e) => return println!("   FAILED: {e}"),
    };
    println!("   {} bytes", html.len());
    let Some(store) = extract_store(&html) else {
        return println!("   FAILED: no js-store data-content JSON in the page (blocked, captcha, or format changed)");
    };
    let all = match find_key(&store, "results") {
        Some(Value::Array(a)) => a.len(),
        _ => 0,
    };
    println!("   js-store parsed; {all} results in total");
    let cands = candidates(&store, artist, title);
    if cands.is_empty() {
        return println!("   no chord pages matching this artist/title");
    }
    for (i, cd) in cands.iter().enumerate() {
        println!("   [{i}] {} — {}  rating {:.2}  votes {}  score {:.1}\n       {}", cd.artist, cd.song, cd.rating, cd.votes, cd.score, cd.url);
    }
    for (i, cd) in cands.iter().enumerate() {
        println!("2. fetch [{i}]");
        let url = if cd.url.starts_with('/') { format!("{BASE}{}", cd.url) } else { cd.url.clone() };
        let html = match get(c, &url, &[]).await {
            Ok(h) => h,
            Err(e) => return println!("   FAILED: {e}"),
        };
        let Some(page) = extract_store(&html) else {
            return println!("   FAILED: tab page has no js-store JSON");
        };
        let has_wiki = find_key(&page, "wiki_tab").is_some();
        println!("   wiki_tab present: {has_wiki}");
        match sheet_from_store(&page) {
            Some(s) => {
                println!("   OK: {} chords on {} lines, capo {}", s.chord_count(), s.lines.len(), s.capo);
                for l in s.lines.iter().take(8) {
                    let cs: Vec<_> = l.chords.iter().map(|(c, n)| format!("{n}@{c}")).collect();
                    println!("   | {:<40} {}", l.text.trim(), cs.join(" "));
                }
                return;
            }
            None => println!("   rejected (fewer than {MIN_CHORDS} recognisable chords)"),
        }
    }
    println!("no usable sheet among the candidates");
}
