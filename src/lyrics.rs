use crate::util::{cache_dir, fnv1a, now_secs};
use serde::{Deserialize, Serialize};

const NEG_TTL: u64 = 3 * 24 * 3600;

#[derive(Clone, Debug)]
pub enum Lyrics {
    /// (time in seconds, text), sorted by time.
    Synced(Vec<(f64, String)>),
    Plain(Vec<String>),
    Instrumental,
    NotFound,
    Error(String),
}

#[derive(Deserialize, Debug)]
struct Rec {
    #[serde(rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain: Option<String>,
    instrumental: Option<bool>,
    duration: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    kind: String, // synced | plain | instrumental | none
    text: String,
    saved: u64,
}

/// Parse LRC: multiple stamps per line, `[mm:ss.xx]`, `[mm:ss]`, `[mm:ss:xx]`, and `[offset:±ms]`.
pub fn parse_lrc(src: &str) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    let mut file_offset = 0.0;
    for raw in src.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some(t) = parse_stamp(tag) {
                stamps.push(t);
            } else if let Some(ms) = tag.strip_prefix("offset:") {
                // Positive LRC offset = lyrics appear earlier.
                file_offset = ms.trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
            }
            rest = &rest[end + 1..];
        }
        let text = rest.trim();
        for t in stamps {
            out.push((
                t,
                if text.is_empty() {
                    "♪".to_string()
                } else {
                    text.to_string()
                },
            ));
        }
    }
    for l in &mut out {
        l.0 = (l.0 - file_offset).max(0.0);
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

fn parse_stamp(tag: &str) -> Option<f64> {
    let (m, s) = tag.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let s: f64 = s.trim().replacen(':', ".", 1).parse().ok()?;
    Some(m * 60.0 + s)
}

fn to_lyrics(kind: &str, text: &str) -> Lyrics {
    match kind {
        "synced" => {
            let l = parse_lrc(text);
            if l.is_empty() {
                Lyrics::NotFound
            } else {
                Lyrics::Synced(l)
            }
        }
        "plain" => Lyrics::Plain(text.lines().map(|s| s.trim().to_string()).collect()),
        "instrumental" => Lyrics::Instrumental,
        _ => Lyrics::NotFound,
    }
}

fn classify(r: &Rec) -> Option<(&'static str, String)> {
    if let Some(s) = r.synced.as_ref().filter(|s| !s.trim().is_empty()) {
        return Some(("synced", s.clone()));
    }
    if r.instrumental == Some(true) {
        return Some(("instrumental", String::new()));
    }
    r.plain
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|p| ("plain", p.clone()))
}

pub fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0xAC00..=0xD7A3 | 0x1100..=0x11FF | 0x3130..=0x318F)
}

/// Share of the letters in `text` that are Hangul (0.0 when there are no letters).
pub fn hangul_ratio(text: &str) -> f32 {
    let (mut h, mut n) = (0usize, 0usize);
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        n += 1;
        h += is_hangul(c) as usize;
    }
    if n == 0 { 0.0 } else { h as f32 / n as f32 }
}

/// Whether a lyric text is mostly Hangul.
fn is_hangul_text(text: &str) -> bool {
    hangul_ratio(text) >= 0.3
}

/// Best search hit: synced with matching duration > synced > plain/instrumental. In a
/// Korean context (`korean`, or any candidate being mostly Hangul) the more-Hangul text
/// wins among equals, so the Hangul lyrics beat the romanized ones.
/// Which script the wanted lyrics are in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    /// Nothing known: Hangul is preferred only if the candidates themselves are Hangul.
    Auto,
    Korean,
    /// An explicitly non-Korean release (e.g. "English Version"): Hangul text is wrong.
    Other,
}

fn pick(recs: Vec<Rec>, length: f64, lang: Lang) -> Option<(&'static str, String)> {
    let korean = lang == Lang::Korean
        || (lang == Lang::Auto && recs.iter().any(|r| classify(r).is_some_and(|c| is_hangul_text(&c.1))));
    let close = |r: &Rec| length <= 0.0 || r.duration.is_none_or(|d| (d - length).abs() <= 4.0);
    let rank = |r: &Rec| match (classify(r).map(|c| c.0), close(r)) {
        (Some("synced"), true) => 0,
        (Some("synced"), false) => 1,
        (Some(_), true) => 2,
        (Some(_), false) => 3,
        (None, _) => 9,
    };
    // Hangul first within the synced > plain order, then the duration match
    let hangul = |r: &Rec| match classify(r) {
        Some((_, t)) if korean => 1000 - (hangul_ratio(&t) * 1000.0) as u32,
        // a non-Korean release: the least Hangul wins
        Some((_, t)) if lang == Lang::Other => (hangul_ratio(&t) * 1000.0) as u32,
        _ => 0,
    };
    let kind = |r: &Rec| rank(r) / 2 * 2; // 0 synced, 2 plain, 8 none
    recs.iter()
        .filter(|r| rank(r) < 9)
        .filter(|r| lang != Lang::Other || !classify(r).is_some_and(|c| is_hangul_text(&c.1)))
        .min_by_key(|r| (kind(r), hangul(r), rank(r)))
        .and_then(classify)
}

pub const BASE: &str = "https://lrclib.net";

/// How hard to try before giving up on a lookup.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// Retries after the first attempt (so `3` = up to 4 requests per step).
    pub retries: u32,
    /// First backoff; doubles each retry (500 ms, 1 s, 2 s …).
    pub backoff: std::time::Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            retries: 3,
            backoff: std::time::Duration::from_millis(500),
        }
    }
}

enum Reply {
    Body(String),
    /// HTTP 404: the server answered and has nothing.
    NotFound,
    /// Retries exhausted (timeout, disconnect, 429, 5xx …); the text says what.
    Failed(&'static str),
}

/// GET with retry + exponential backoff. 404 is a real answer (no retry). Timeouts,
/// connection errors, 429 (honouring `Retry-After`) and 5xx are retried.
async fn request(
    c: &reqwest::Client,
    url: &str,
    query: &[(&str, &str)],
    retries: u32,
    backoff: std::time::Duration,
) -> Reply {
    let mut why = "network error";
    for attempt in 0..=retries {
        let mut wait = backoff * 2u32.pow(attempt);
        match c.get(url).query(query).send().await {
            Ok(r) => {
                let st = r.status();
                if st == reqwest::StatusCode::NOT_FOUND {
                    return Reply::NotFound;
                }
                if st.is_success() {
                    match r.text().await {
                        Ok(t) => return Reply::Body(t),
                        Err(_) => why = "network error", // dropped mid-body
                    }
                } else if st == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    why = "rate limited";
                    if let Some(s) = r
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                    {
                        wait = std::time::Duration::from_secs(s.min(10));
                    }
                } else if st.is_server_error() {
                    why = "server error";
                } else {
                    return Reply::Failed("unexpected reply"); // other 4xx: retrying won't help
                }
            }
            Err(_) => why = "network error",
        }
        if attempt < retries {
            tokio::time::sleep(wait).await;
        }
    }
    Reply::Failed(why)
}

/// Result of one step of the fallback chain.
enum Step<T> {
    Hit(T),
    Nothing,
    Failed(&'static str),
}

async fn get(
    c: &reqwest::Client,
    base: &str,
    q: &[(&str, &str)],
    retries: u32,
    p: &Policy,
) -> Step<Rec> {
    match request(c, &format!("{base}/api/get"), q, retries, p.backoff).await {
        Reply::Body(t) => match serde_json::from_str::<Rec>(&t) {
            Ok(r) => Step::Hit(r),
            Err(_) => Step::Failed("unreadable reply"),
        },
        Reply::NotFound => Step::Nothing,
        Reply::Failed(w) => Step::Failed(w),
    }
}

async fn search(
    c: &reqwest::Client,
    base: &str,
    artist: &str,
    title: &str,
    retries: u32,
    p: &Policy,
) -> Step<Vec<Rec>> {
    let q = [("artist_name", artist), ("track_name", title)];
    match request(c, &format!("{base}/api/search"), &q, retries, p.backoff).await {
        Reply::Body(t) => match serde_json::from_str::<Vec<Rec>>(&t) {
            Ok(v) => Step::Hit(v),
            Err(_) => Step::Failed("unreadable reply"),
        },
        Reply::NotFound => Step::Nothing,
        Reply::Failed(w) => Step::Failed(w),
    }
}

pub async fn fetch(
    c: reqwest::Client,
    artist: String,
    title: String,
    album: String,
    length: f64,
    policy: Policy,
) -> Lyrics {
    fetch_with(&c, BASE, &artist, &title, &album, length, policy, true).await
}

/// Fallback chain: (a) exact artist+title+album+duration, (b) first artist + cleaned
/// title without album/duration, (c) search by (cleaned) title. Each step retries with
/// backoff; once a step has exhausted its retries the rest try once each instead of
/// multiplying the wait. "Not found" is only reported — and cached — when every step got
/// a real answer.
#[allow(clippy::too_many_arguments)]
pub async fn fetch_with(
    c: &reqwest::Client,
    base: &str,
    artist: &str,
    title: &str,
    album: &str,
    length: f64,
    policy: Policy,
    use_cache: bool,
) -> Lyrics {
    let key = fnv1a(&format!(
        "{artist}\0{title}\0{album}\0{}",
        length.round() as i64
    ));
    let path = cache_dir().join(format!("lyrics-{key:016x}.json"));
    if use_cache {
        if let Ok(txt) = tokio::fs::read_to_string(&path).await {
            if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
                if e.kind != "none" || now_secs().saturating_sub(e.saved) < NEG_TTL {
                    return to_lyrics(&e.kind, &e.text);
                }
            }
        }
    }

    let cleaned = crate::year::clean(title);
    let first = artist.split(", ").next().unwrap_or(artist).to_string();
    // a Korean title or artist: romanized lyrics are worth a search for Hangul ones
    let hint = title.chars().chain(artist.chars()).any(is_hangul);
    let lang = match crate::util::version_marker(title, album).as_deref() {
        Some("korean") => Lang::Korean,
        Some("instrumental") | None if hint => Lang::Korean,
        Some(_) => Lang::Other,
        None => Lang::Auto,
    };
    let korean = lang == Lang::Korean;
    // Hangul text is the wrong answer for an explicitly non-Korean release
    let classify = |r: &Rec| classify(r).filter(|c| lang != Lang::Other || !is_hangul_text(&c.1));
    let mut failure: Option<&'static str> = None;
    let mut result: Option<(&'static str, String)> = None;
    let mut retries = policy.retries;
    let mut note = |w: &'static str, retries: &mut u32| {
        failure = Some(w);
        *retries = 0; // network is struggling: one try per remaining step
    };

    // (a) exact signature
    let dur = format!("{}", length.round() as i64);
    let mut q = vec![("artist_name", artist), ("track_name", title)];
    if !album.is_empty() {
        q.push(("album_name", album));
    }
    if length > 0.0 {
        q.push(("duration", &dur));
    }
    match get(c, base, &q, retries, &policy).await {
        Step::Hit(r) => result = classify(&r),
        Step::Nothing => {}
        Step::Failed(w) => note(w, &mut retries),
    }

    // good enough to stop: synced, and Hangul when the song is Korean
    let done = |r: &Option<(&'static str, String)>| {
        r.as_ref().is_some_and(|r| r.0 == "synced" && (!korean || is_hangul_text(&r.1)))
    };

    // (b) first artist + cleaned title, no album / duration
    if !done(&result) {
        let q = [("artist_name", first.as_str()), ("track_name", cleaned.as_str())];
        match get(c, base, &q, retries, &policy).await {
            Step::Hit(r) => {
                if let Some(hit) = classify(&r) {
                    let better = match &result {
                        None => true,
                        Some(x) => {
                            (x.0 != "synced" && hit.0 == "synced")
                                || (korean && is_hangul_text(&hit.1) && !is_hangul_text(&x.1))
                        }
                    };
                    if better {
                        result = Some(hit);
                    }
                }
            }
            Step::Nothing => {}
            Step::Failed(w) => note(w, &mut retries),
        }
    }

    // (c) search: cleaned title, then the raw title, then the first artist alone
    if !done(&result) {
        let mut tries = vec![(artist.to_string(), cleaned.clone())];
        if cleaned != title {
            tries.push((artist.to_string(), title.to_string()));
        }
        if first != artist {
            tries.push((first.clone(), cleaned.clone()));
        }
        for (a, t) in tries {
            match search(c, base, &a, &t, retries, &policy).await {
                Step::Hit(recs) => {
                    if let Some(hit) = pick(recs, length, lang) {
                        let better = match &result {
                            None => true,
                            Some(x) => {
                                (x.0 != "synced"
                                    && !(hit.0 != "synced" && korean && is_hangul_text(&x.1) && !is_hangul_text(&hit.1)))
                                    || (korean && is_hangul_text(&hit.1) && !is_hangul_text(&x.1))
                            }
                        };
                        if better {
                            result = Some(hit);
                        }
                        if done(&result) {
                            break;
                        }
                    }
                }
                Step::Nothing => {}
                Step::Failed(w) => note(w, &mut retries),
            }
        }
    }

    let save = |kind: &str, text: &str| {
        let e = CacheEntry {
            kind: kind.into(),
            text: text.into(),
            saved: now_secs(),
        };
        serde_json::to_string(&e).ok()
    };
    match result {
        Some((kind, text)) => {
            if use_cache {
                if let Some(s) = save(kind, &text) {
                    let _ = tokio::fs::write(&path, s).await;
                }
            }
            to_lyrics(kind, &text)
        }
        None if failure.is_some() => Lyrics::Error(format!(
            "lyrics unavailable ({})",
            failure.unwrap_or("network error")
        )),
        None => {
            if use_cache {
                if let Some(s) = save("none", "") {
                    let _ = tokio::fs::write(&path, s).await;
                }
            }
            Lyrics::NotFound
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(synced: &str, plain: &str, duration: f64) -> Rec {
        Rec {
            synced: (!synced.is_empty()).then(|| synced.to_string()),
            plain: (!plain.is_empty()).then(|| plain.to_string()),
            instrumental: None,
            duration: Some(duration),
        }
    }

    #[test]
    fn hangul_share() {
        assert_eq!(hangul_ratio(""), 0.0);
        assert_eq!(hangul_ratio("hello world"), 0.0);
        assert_eq!(hangul_ratio("안녕하세요"), 1.0);
        let r = hangul_ratio("안녕 hi"); // 2 of 4 letters
        assert!((r - 0.5).abs() < 1e-6, "{r}");
    }

    #[test]
    fn pick_prefers_hangul_only_in_korean_context() {
        let roman = "[00:01.00]saranghae neomu\n[00:05.00]gaji ma";
        let hangul = "[00:01.00]사랑해 너무\n[00:05.00]가지 마";
        let recs = || vec![rec(roman, "", 200.0), rec(hangul, "", 200.0)];
        // Korean title/artist: the Hangul synced lyrics win over the romanized ones
        assert_eq!(pick(recs(), 200.0, Lang::Korean).unwrap().1, hangul);
        // an "English Version" never takes the Hangul lyrics
        assert_eq!(pick(recs(), 200.0, Lang::Other).unwrap().1, roman);
        // a Hangul candidate also makes the context Korean by itself
        assert_eq!(pick(recs(), 200.0, Lang::Auto).unwrap().1, hangul);
        // no Hangul candidate and no hint: plain first-come ranking
        let eng = vec![rec("[00:01.00]one", "", 200.0), rec("[00:01.00]two", "", 200.0)];
        assert_eq!(pick(eng, 200.0, Lang::Auto).unwrap().1, "[00:01.00]one");
        // synced still beats plain Hangul
        let mixed = vec![rec("", "사랑해", 200.0), rec(roman, "", 200.0)];
        assert_eq!(pick(mixed, 200.0, Lang::Korean).unwrap().0, "synced");
    }
}
