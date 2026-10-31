use crate::util::{cache_dir, fnv1a};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;
use std::time::Duration;

const MB: &str = "https://musicbrainz.org/ws/2";
/// Negative results are re-checked after this many seconds.
const NEG_TTL: u64 = 7 * 24 * 3600;

static PAREN_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[\(\[][^\)\]]*\b(remaster(ed)?|live|version|edit|mono|stereo|deluxe|feat\.?|ft\.?|anniversary|bonus|demo|acoustic|explicit|radio|expanded|edition)\b[^\)\]]*[\)\]]").unwrap()
});
static DASH_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s+-\s+.*\b(remaster(ed)?|live|version|edit|mono|stereo|single|deluxe|anniversary|demo|acoustic|bonus|radio|edition)\b.*$").unwrap()
});

/// Strip "- Remastered 2011", "(Live at …)", "[Deluxe Edition]" etc.
pub fn clean(s: &str) -> String {
    let s = DASH_TAG.replace(s, "");
    let s = PAREN_TAG.replace_all(&s, "");
    let t = s.trim();
    if t.is_empty() {
        s.to_string()
    } else {
        t.to_string()
    }
}

/// The bare song title: no (…) / […] groups, no `feat.` / `ft.` credits, nothing after
/// a ` - ` (demo, remastered…). Used to retry a failed chord lookup.
pub fn base_title(s: &str) -> String {
    static GROUPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[\(\[][^\)\]]*[\)\]]").unwrap());
    static FEAT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\s+(feat\.?|ft\.?|featuring)\s+.*$").unwrap());
    static DASH_ANY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+[-–—]\s+.*$").unwrap());
    let t = GROUPS.replace_all(s, "");
    let t = FEAT.replace(&t, "");
    let t = DASH_ANY.replace(&t, "");
    let t = t.trim();
    if t.is_empty() { s.trim().to_string() } else { t.to_string() }
}

/// `"2011-09-26T…"` / `"2011"` → `Some("2011")`.
/// `YYYY-MM-DD` from an MPRIS `contentCreated` ("1969-09-26T00:00:00Z"); `None` if it
/// only carries a year (or nothing).
pub fn full_date(s: &str) -> Option<String> {
    let d = s.trim().get(..10)?;
    let b = d.as_bytes();
    let ok = b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    let (m, day) = (d.get(5..7)?, d.get(8..10)?);
    (ok && m != "00" && day != "00").then(|| d.to_string())
}

pub fn from_content_created(s: &str) -> Option<String> {
    let y: String = s.trim().chars().take(4).collect();
    let n: i32 = y.parse().ok()?;
    (1900..=2100).contains(&n).then_some(y)
}

fn year_of(date: &str) -> Option<i32> {
    date.get(..4)?
        .parse()
        .ok()
        .filter(|y| (1900..=2100).contains(y))
}

/// `"1991-08-12T…"` → `"1991-08-12"`, `"1991-08"` → `"1991"`: the full date only when all
/// three parts are there.
fn date_of(date: &str) -> Option<String> {
    let y = year_of(date)?;
    Some(full_date(date).unwrap_or_else(|| y.to_string()))
}

/// Earlier year wins; for the same year the more precise date does.
fn rank(d: &str) -> (i32, std::cmp::Reverse<usize>) {
    (year_of(d).unwrap_or(0), std::cmp::Reverse(d.len()))
}

fn lucene(s: &str) -> String {
    s.replace(['"', '\\'], " ")
}

/// Why a MusicBrainz request gave nothing: it asked us to slow down (503/429, no fault of the
/// lookup), or it failed.
#[derive(Debug, PartialEq)]
pub(crate) enum MbErr {
    Limited,
    Failed,
}

/// MusicBrainz asks for at most one request a second; the year and genre lookups share this.
const MB_GAP: Duration = Duration::from_millis(1100);
static MB_LAST: tokio::sync::Mutex<Option<std::time::Instant>> = tokio::sync::Mutex::const_new(None);

/// How long to wait before a request when the previous one started at `last`.
fn mb_wait(last: Option<std::time::Instant>, now: std::time::Instant, gap: Duration) -> Duration {
    last.map_or(Duration::ZERO, |t| gap.saturating_sub(now.saturating_duration_since(t)))
}

fn mb_status(st: reqwest::StatusCode) -> Result<(), MbErr> {
    if st.is_success() {
        Ok(())
    } else if st == reqwest::StatusCode::SERVICE_UNAVAILABLE || st == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Err(MbErr::Limited)
    } else {
        Err(MbErr::Failed)
    }
}

/// A MusicBrainz GET, at least `MB_GAP` after the start of the previous one from anywhere in the app.
pub(crate) async fn mb_get(c: &reqwest::Client, url: &str, q: &[(&str, &str)]) -> Result<Value, MbErr> {
    {
        let mut last = MB_LAST.lock().await;
        let wait = mb_wait(*last, std::time::Instant::now(), MB_GAP);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        *last = Some(std::time::Instant::now());
    }
    let r = c.get(url).query(q).timeout(Duration::from_secs(12)).send().await.map_err(|_| MbErr::Failed)?;
    mb_status(r.status())?;
    r.json::<Value>().await.map_err(|_| MbErr::Failed)
}

async fn get_json(c: &reqwest::Client, url: &str, q: &[(&str, &str)]) -> Result<Value, ()> {
    let r = c.get(url).query(q).send().await.map_err(|_| ())?;
    if !r.status().is_success() {
        return Err(());
    }
    r.json::<Value>().await.map_err(|_| ())
}

/// Earliest first-release-date among high-confidence matches in a MusicBrainz result list.
fn earliest(v: &Value, list: &str) -> Option<String> {
    v.get(list)?
        .as_array()?
        .iter()
        .filter(|r| r.get("score").and_then(Value::as_i64).unwrap_or(0) >= 90)
        .filter_map(|r| {
            r.get("first-release-date")
                .and_then(Value::as_str)
                .and_then(date_of)
        })
        .min_by_key(|d| rank(d))
}

async fn mb_recording(
    c: &reqwest::Client,
    title: &str,
    artist: &str,
) -> Result<Option<String>, ()> {
    let q = format!(
        "recording:\"{}\" AND artist:\"{}\"",
        lucene(title),
        lucene(artist)
    );
    let v = mb_get(
        c,
        &format!("{MB}/recording/"),
        &[("query", &q), ("fmt", "json"), ("limit", "25")],
    )
    .await
    .map_err(|_| ())?;
    Ok(earliest(&v, "recordings"))
}

async fn mb_release_group(
    c: &reqwest::Client,
    album: &str,
    artist: &str,
) -> Result<Option<String>, ()> {
    let q = format!(
        "releasegroup:\"{}\" AND artist:\"{}\"",
        lucene(album),
        lucene(artist)
    );
    let v = mb_get(
        c,
        &format!("{MB}/release-group/"),
        &[("query", &q), ("fmt", "json"), ("limit", "10")],
    )
    .await
    .map_err(|_| ())?;
    Ok(earliest(&v, "release-groups"))
}

async fn itunes(c: &reqwest::Client, title: &str, artist: &str) -> Result<Option<String>, ()> {
    let term = format!("{artist} {title}");
    let v = get_json(
        c,
        "https://itunes.apple.com/search",
        &[("term", &term), ("entity", "song"), ("limit", "15")],
    )
    .await?;
    let first_artist = artist.split(", ").next().unwrap_or(artist).to_lowercase();
    let tl = title.to_lowercase();
    Ok(v.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| {
            let a = r
                .get("artistName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            let t = r
                .get("trackName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            !first_artist.trim().is_empty() && a.contains(&first_artist) && (t.contains(&tl) || tl.contains(&t))
        })
        .filter_map(|r| {
            r.get("releaseDate")
                .and_then(Value::as_str)
                .and_then(date_of)
        })
        .min_by_key(|d| rank(d)))
}

/// Returns `Some("YYYY-MM-DD" or "YYYY")` or `None` (= "N/A"). Never blocks the UI: runs in its own task.
pub async fn resolve(
    c: reqwest::Client,
    artist: String,
    title: String,
    album: String,
) -> Option<String> {
    let path = cache_dir().join(format!(
        "year-{:016x}",
        fnv1a(&format!("{artist}\0{title}\0{album}"))
    ));
    if let Ok(txt) = tokio::fs::read_to_string(&path).await {
        let mut it = txt.split_whitespace();
        match (it.next(), it.next()) {
            (Some("N/A"), Some(ts))
                if crate::util::now_secs().saturating_sub(ts.parse().unwrap_or(0))
                    < NEG_TTL =>
            {
                return None;
            }
            (Some(y), _) if y != "N/A" => return Some(y.to_string()),
            _ => {}
        }
    }

    let title_c = clean(&title);
    let album_c = clean(&album);
    let first_artist = artist.split(", ").next().unwrap_or(&artist).to_string();
    let mut had_error = false;
    let mut found: Option<String> = None;

    // (MusicBrainz asks for ≤1 req/s: `mb_get` spaces the calls out.)
    let mut artists = vec![artist.clone()];
    if first_artist != artist {
        artists.push(first_artist.clone());
    }
    for a in &artists {
        match mb_recording(&c, &title_c, a).await {
            Ok(Some(y)) => {
                found = Some(y);
                break;
            }
            Ok(None) => {}
            Err(_) => had_error = true,
        }
    }
    if found.is_none() && !album_c.is_empty() {
        match mb_release_group(&c, &album_c, &first_artist).await {
            Ok(y) => found = y,
            Err(_) => had_error = true,
        }
    }
    if found.is_none() {
        match itunes(&c, &title_c, &artist).await {
            Ok(y) => found = y,
            Err(_) => had_error = true,
        }
    }

    match found {
        Some(y) => {
            let _ = tokio::fs::write(&path, &y).await;
            Some(y)
        }
        None => {
            // Only remember a miss if every source actually answered.
            if !had_error {
                let _ =
                    tokio::fs::write(&path, format!("N/A {}", crate::util::now_secs())).await;
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn musicbrainz_requests_are_spaced_and_a_limit_is_not_a_failure() {
        let t = Instant::now();
        assert_eq!(mb_wait(None, t, MB_GAP), Duration::ZERO);
        assert_eq!(mb_wait(Some(t), t + Duration::from_millis(300), MB_GAP), Duration::from_millis(800));
        assert_eq!(mb_wait(Some(t), t + Duration::from_secs(2), MB_GAP), Duration::ZERO);
        assert!(MB_GAP >= Duration::from_millis(1100));
        assert_eq!(mb_status(reqwest::StatusCode::SERVICE_UNAVAILABLE), Err(MbErr::Limited));
        assert_eq!(mb_status(reqwest::StatusCode::TOO_MANY_REQUESTS), Err(MbErr::Limited));
        assert_eq!(mb_status(reqwest::StatusCode::INTERNAL_SERVER_ERROR), Err(MbErr::Failed));
        assert_eq!(mb_status(reqwest::StatusCode::OK), Ok(()));
    }
}
