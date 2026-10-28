use crate::util::{cache_dir, fnv1a, now_secs};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

const MB: &str = "https://musicbrainz.org/ws/2";
/// Negative results are re-checked after this many seconds.
const NEG_TTL: u64 = 7 * 24 * 3600;
const MAX_FAILS: u32 = 3;
const BREAKER_SECS: u64 = 10 * 60;
const MAX_GENRES: usize = 3;

/// Consecutive hard failures; three in a row switch the lookup off for `BREAKER_SECS`.
static FAILS: AtomicU32 = AtomicU32::new(0);
static LAST_FAIL: AtomicU64 = AtomicU64::new(0);

/// Genres MusicBrainz users vote on that are real genres (not moods, years, places).
const ALLOWED: &[&str] = &[
    "rock", "pop", "hip hop", "k-pop", "j-pop", "metal", "heavy metal", "thrash metal",
    "death metal", "black metal", "doom metal", "power metal", "nu metal", "folk metal",
    "metalcore", "hardcore", "punk", "punk rock", "pop punk", "post-punk", "new wave",
    "jazz", "blues", "country", "folk", "classical", "electronic", "house", "techno",
    "trance", "ambient", "drum and bass", "dubstep", "edm", "disco", "r&b", "soul", "funk",
    "reggae", "ska", "indie rock", "indie pop", "indie", "alternative rock", "alternative",
    "hard rock", "progressive rock", "psychedelic rock", "soft rock", "glam rock",
    "folk rock", "garage rock", "grunge", "synth-pop", "electropop", "dance-pop",
    "art pop", "dream pop", "shoegaze", "noise", "industrial", "gothic rock", "emo",
    "post-rock", "math rock", "britpop", "rap", "trap", "gangsta rap", "latin", "salsa",
    "bossa nova", "samba", "flamenco", "gospel", "swing", "bebop", "smooth jazz",
    "opera", "baroque", "soundtrack", "new age", "lo-fi", "downtempo", "trip hop",
    "synthwave", "vaporwave", "chillout", "bluegrass", "singer-songwriter",
    "blues rock", "southern rock", "americana", "world", "afrobeat", "dancehall",
    "dub", "easy listening", "schlager", "chanson", "enka", "city pop", "eurodance",
    "hardstyle", "idm", "electro", "jungle", "breakbeat", "garage", "uk garage",
    "contemporary r&b", "neo soul", "pop rock", "power pop", "symphonic metal",
    "progressive metal", "groove metal", "hard bop", "cool jazz", "fusion", "celtic",
];

fn alias(t: &str) -> &str {
    match t {
        "hip-hop" | "hiphop" | "hip hop/rap" => "hip hop",
        "rhythm and blues" | "rnb" | "r and b" | "r&b/soul" => "r&b",
        "synthpop" | "synth pop" => "synth-pop",
        "kpop" | "k pop" => "k-pop",
        "jpop" | "j pop" => "j-pop",
        "electronica" => "electronic",
        "drum & bass" | "drum n bass" | "dnb" => "drum and bass",
        "post punk" => "post-punk",
        "trip-hop" => "trip hop",
        "lofi" | "lo fi" => "lo-fi",
        other => other,
    }
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            if w.contains('&') {
                return w.to_uppercase();
            }
            let mut out = String::with_capacity(w.len());
            let mut up = true;
            for c in w.chars() {
                if up {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                up = c == '-';
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalise a raw genre string ("rock; Hard Rock / blues") into at most three
/// title-cased, de-duplicated genres.
pub fn from_tags(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split([';', '/', ',']) {
        let p = part.split_whitespace().collect::<Vec<_>>().join(" ");
        if p.is_empty() {
            continue;
        }
        let t = title_case(&p);
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
        if out.len() == MAX_GENRES {
            break;
        }
    }
    out
}

/// Top MusicBrainz tags by votes that are real genres (no moods, years, nationalities).
pub fn rank_musicbrainz(tags: &[(String, i64)]) -> Vec<String> {
    let mut v: Vec<(String, i64)> = tags
        .iter()
        .map(|(n, c)| (n.trim().to_lowercase(), *c))
        .map(|(n, c)| (alias(&n).to_string(), c))
        .filter(|(n, _)| ALLOWED.contains(&n.as_str()))
        .collect();
    v.sort_by_key(|t| std::cmp::Reverse(t.1)); // stable: ties keep MusicBrainz order
    let mut out: Vec<String> = Vec::new();
    for (n, _) in v {
        let t = title_case(&n);
        if !out.contains(&t) {
            out.push(t);
        }
        if out.len() == MAX_GENRES {
            break;
        }
    }
    out
}

/// Order of trust: the player's `xesam:genre`, then the file's tags, then MusicBrainz.
pub fn resolve(
    mpris: Option<&str>,
    file_tags: Option<&str>,
    mb: Option<Vec<String>>,
) -> Option<String> {
    for raw in [mpris, file_tags].into_iter().flatten() {
        let g = from_tags(raw);
        if !g.is_empty() {
            return Some(g.join(" / "));
        }
    }
    mb.filter(|g| !g.is_empty()).map(|g| g.join(" / "))
}

fn lucene(s: &str) -> String {
    s.replace(['"', '\\'], " ")
}

fn record_fail() {
    FAILS.fetch_add(1, Ordering::Relaxed);
    LAST_FAIL.store(now_secs(), Ordering::Relaxed);
}

fn breaker_open() -> bool {
    if FAILS.load(Ordering::Relaxed) < MAX_FAILS {
        return false;
    }
    if now_secs().saturating_sub(LAST_FAIL.load(Ordering::Relaxed)) >= BREAKER_SECS {
        FAILS.store(0, Ordering::Relaxed);
        return false;
    }
    true
}

/// Votes per tag over the high-confidence recordings of a MusicBrainz search reply.
fn tags_of(v: &Value) -> Vec<(String, i64)> {
    let mut acc: Vec<(String, i64)> = Vec::new();
    let recs = v.get("recordings").and_then(Value::as_array).into_iter().flatten();
    for r in recs.filter(|r| r.get("score").and_then(Value::as_i64).unwrap_or(0) >= 90) {
        for t in r.get("tags").and_then(Value::as_array).into_iter().flatten() {
            let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
            let n = t.get("count").and_then(Value::as_i64).unwrap_or(1);
            match acc.iter_mut().find(|(k, _)| k == name) {
                Some(e) => e.1 += n,
                None => acc.push((name.to_string(), n)),
            }
        }
    }
    acc
}

/// Genres for a song from MusicBrainz, cached on disk (misses for a week, errors never).
/// Network use is opt-in: the caller decides whether to call this at all.
pub async fn lookup(client: &reqwest::Client, artist: &str, title: &str) -> Option<Vec<String>> {
    let first_artist = artist.split(", ").next().unwrap_or(artist);
    let title_c = crate::year::clean(title);
    let path = cache_dir().join(format!(
        "genre-{:016x}",
        fnv1a(&format!("{first_artist}\0{title_c}"))
    ));
    if let Ok(txt) = tokio::fs::read_to_string(&path).await {
        let mut lines = txt.lines();
        match lines.next() {
            Some(h) if h.starts_with("N/A") => {
                let ts: u64 = h.split_whitespace().nth(1).and_then(|t| t.parse().ok()).unwrap_or(0);
                if now_secs().saturating_sub(ts) < NEG_TTL {
                    return None;
                }
            }
            Some(first) if !first.is_empty() => {
                let g: Vec<String> = txt.lines().map(str::to_string).collect();
                return Some(g);
            }
            _ => {}
        }
    }
    if breaker_open() || artist.trim().is_empty() || title_c.trim().is_empty() {
        return None;
    }

    let q = format!("recording:\"{}\" AND artist:\"{}\"", lucene(&title_c), lucene(first_artist));
    let reply = async {
        let r = client
            .get(format!("{MB}/recording/"))
            .query(&[("query", q.as_str()), ("fmt", "json"), ("limit", "10")])
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .map_err(|_| ())?;
        if !r.status().is_success() {
            return Err(());
        }
        r.json::<Value>().await.map_err(|_| ())
    }
    .await;
    let Ok(v) = reply else {
        record_fail();
        return None;
    };
    FAILS.store(0, Ordering::Relaxed);
    let genres = rank_musicbrainz(&tags_of(&v));
    if genres.is_empty() {
        let _ = tokio::fs::write(&path, format!("N/A {}", now_secs())).await;
        None
    } else {
        let _ = tokio::fs::write(&path, genres.join("\n")).await;
        Some(genres)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: &str, c: i64) -> (String, i64) {
        (n.to_string(), c)
    }

    #[test]
    fn from_tags_splits_dedupes_and_title_cases() {
        assert_eq!(from_tags("rock; hard rock / BLUES"), vec!["Rock", "Hard Rock", "Blues"]);
        assert_eq!(from_tags("Pop, pop ,  "), vec!["Pop"]);
        assert_eq!(from_tags("a;b;c;d;e"), vec!["A", "B", "C"]);
        assert_eq!(from_tags("r&b / synth-pop"), vec!["R&B", "Synth-Pop"]);
        assert!(from_tags("  ").is_empty());
    }

    #[test]
    fn rank_filters_and_orders() {
        let tags = vec![
            t("seen live", 20),
            t("british", 15),
            t("90s", 14),
            t("favorites", 12),
            t("rock", 5),
            t("hard rock", 9),
            t("Hip-Hop", 7),
            t("metal", 1),
        ];
        assert_eq!(rank_musicbrainz(&tags), vec!["Hard Rock", "Hip Hop", "Rock"]);
        assert!(rank_musicbrainz(&[t("seen live", 3)]).is_empty());
        assert!(ALLOWED.len() >= 60);
    }

    #[test]
    fn resolve_order() {
        let mb = Some(vec!["Jazz".to_string()]);
        assert_eq!(resolve(Some("rock"), Some("pop"), mb.clone()).unwrap(), "Rock");
        assert_eq!(resolve(Some(" "), Some("pop; soul"), mb.clone()).unwrap(), "Pop / Soul");
        assert_eq!(resolve(None, None, mb).unwrap(), "Jazz");
        assert_eq!(resolve(None, Some(""), Some(vec![])), None);
    }

    #[test]
    fn tags_are_summed_over_confident_matches() {
        let v: Value = serde_json::json!({"recordings": [
            {"score": 100, "tags": [{"name": "rock", "count": 2}]},
            {"score": 95, "tags": [{"name": "rock", "count": 3}, {"name": "pop", "count": 1}]},
            {"score": 40, "tags": [{"name": "jazz", "count": 9}]}
        ]});
        assert_eq!(rank_musicbrainz(&tags_of(&v)), vec!["Rock", "Pop"]);
    }
}
