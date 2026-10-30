//! Other places to look for lyrics when lrclib has none (or could not be reached often
//! enough): NetEase Cloud Music (synced, no login), lyrics.ovh (plain) and, as the last resort,
//! the lyrics on the Genius song page (plain). All three are unofficial and can change or
//! vanish without notice, so every failure just means "nothing found here". Which of them is
//! asked is decided by `lyrics::fetch_with`; a hit has to be the song asked for.
use crate::chords;
use serde_json::Value;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

/// The sites to ask (swapped for a local test server in the tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extra {
    pub netease: &'static str,
    pub ovh: &'static str,
    pub genius: &'static str,
}

impl Default for Extra {
    fn default() -> Self {
        Extra { netease: "https://music.163.com", ovh: "https://api.lyrics.ovh", genius: crate::genius::BASE }
    }
}

async fn get(c: &reqwest::Client, url: reqwest::Url) -> Option<String> {
    let r = c
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::REFERER, "https://music.163.com/")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?;
    r.status().is_success().then_some(())?;
    r.text().await.ok()
}

fn url(base: &str, segments: &[&str], query: &[(&str, &str)]) -> Option<reqwest::Url> {
    let mut u = reqwest::Url::parse(base).ok()?;
    u.path_segments_mut().ok()?.pop_if_empty().extend(segments);
    if !query.is_empty() {
        u.query_pairs_mut().extend_pairs(query);
    }
    Some(u)
}

fn first_artist(artist: &str) -> &str {
    artist.split(", ").next().unwrap_or(artist).trim()
}

/// Try the sources one after the other: `("synced", LRC text)` or `("plain", text)`.
pub async fn find(c: &reqwest::Client, x: &Extra, artist: &str, title: &str, length: f64) -> Option<(&'static str, String)> {
    if let Some(hit) = netease(c, x.netease, artist, title, length).await {
        return Some(hit);
    }
    if let Some(text) = ovh(c, x.ovh, artist, title).await {
        return Some(("plain", text));
    }
    crate::genius::plain_lyrics(c, x.genius, artist, title).await.map(|t| ("plain", t))
}

/// A song of a NetEase search that is the one asked for: the title and one artist match, and
/// the length is close when both sides know it. Returns its id.
fn netease_pick(reply: &Value, artist: &str, title: &str, length: f64) -> Option<String> {
    let want_a = chords::norm(first_artist(artist));
    reply.pointer("/result/songs")?.as_array()?.iter().find_map(|s| {
        let name = s.get("name")?.as_str()?;
        if !crate::lyrics::names_title(Some(name), title) {
            return None;
        }
        let by = s.get("artists").and_then(Value::as_array).is_some_and(|a| {
            a.iter().filter_map(|x| x.get("name")?.as_str()).any(|n| {
                let n = chords::norm(n);
                !n.is_empty() && (n.contains(&want_a) || want_a.contains(&n) || chords::dice(&n, &want_a) >= 0.7)
            })
        });
        let secs = s.get("duration").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
        let near = length <= 0.0 || secs <= 0.0 || (secs - length).abs() <= 8.0;
        (by && near).then(|| s.get("id").map(|i| i.to_string())).flatten()
    })
}

async fn netease(c: &reqwest::Client, base: &str, artist: &str, title: &str, length: f64) -> Option<(&'static str, String)> {
    let q = format!("{} {}", first_artist(artist), crate::year::clean(title));
    let found = get(c, url(base, &["api", "search", "get", "web"], &[("s", &q), ("type", "1"), ("offset", "0"), ("limit", "8"), ("total", "true")])?).await?;
    let id = netease_pick(&serde_json::from_str(&found).ok()?, artist, title, length)?;
    let body = get(c, url(base, &["api", "song", "lyric"], &[("id", &id), ("lv", "1"), ("kv", "1"), ("tv", "-1")])?).await?;
    let v: Value = serde_json::from_str(&body).ok()?;
    let lrc = v.pointer("/lrc/lyric")?.as_str()?;
    if crate::lyrics::parse_lrc(lrc).len() >= 4 {
        Some(("synced", lrc.to_string()))
    } else {
        None
    }
}

async fn ovh(c: &reqwest::Client, base: &str, artist: &str, title: &str) -> Option<String> {
    let body = get(c, url(base, &["v1", first_artist(artist), &crate::year::clean(title)], &[])?).await?;
    let v: Value = serde_json::from_str(&body).ok()?;
    let text = v.get("lyrics")?.as_str()?;
    // (the service puts a "Paroles de la chanson …" line first on some songs)
    let lines: Vec<&str> = text.lines().map(str::trim_end).skip_while(|l| l.trim().is_empty() || l.starts_with("Paroles de la chanson")).collect();
    (lines.iter().filter(|l| !l.trim().is_empty()).count() >= 4).then(|| lines.join("\n"))
}
