//! Cultural-meaning notes for the lyrics. Source: the Genius API with a free client token
//! (`genius_token` in the config, or `GENIUS_TOKEN` in the environment): the song's referents
//! (the annotated fragments) are matched to the synced lyric lines by their quoted text. An
//! optional second mode asks the AI engine for short notes instead; those are labelled
//! [`AI_LABEL`]. Only short excerpts are kept for display, with the source named. Misses are
//! cached for a week, errors never; a circuit breaker stops asking after repeated failures.
//! The token is never put in an error message, a log or the cache.
use crate::util::{cache_dir, fnv1a, now_secs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

pub const API: &str = "https://api.genius.com";
const NEG_TTL: u64 = 7 * 24 * 3600;
const POS_TTL: u64 = 30 * 24 * 3600;
const MAX_FAILS: u32 = 3;
const BREAKER_SECS: u64 = 10 * 60;
/// Shortest fragment (in letters and digits) that may be matched inside a longer lyric line.
const MIN_FRAGMENT: usize = 12;
/// Shortest lyric line that may be matched inside a longer quoted fragment.
const MIN_LINE: usize = 8;
/// How much of an annotation is kept for display.
pub const EXCERPT_CHARS: usize = 280;
pub const SOURCE: &str = "Genius";
pub const AI_LABEL: &str = "AI-generated";

static FAILS: AtomicU32 = AtomicU32::new(0);
static LAST_FAIL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// The lyric text the annotation is about, as quoted on Genius.
    pub fragment: String,
    /// The annotation, already shortened to an excerpt.
    pub text: String,
    pub source: String,
    pub url: Option<String>,
}

#[doc(hidden)]
pub fn reset_breaker() {
    FAILS.store(0, Ordering::Relaxed);
    LAST_FAIL.store(0, Ordering::Relaxed);
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

/// Lower case, letters and digits only, single spaces: what "the same words" means.
pub fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut gap = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.extend(c.to_lowercase());
        } else if c == '\'' || c == '’' {
            // "don't" and "dont" are the same word
        } else {
            gap = true;
        }
    }
    out
}

/// At most `max` characters, cut at a word boundary with a trailing `…`; whitespace collapsed.
pub fn excerpt(text: &str, max: usize) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let cut: String = t.chars().take(max.saturating_sub(1)).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end_matches([' ', ',', ';', ':', '.', '-']))
}

/// The annotations of a Genius `/referents` response.
pub fn parse_referents(v: &Value) -> Vec<Annotation> {
    let Some(list) = v.pointer("/response/referents").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for r in list {
        let fragment = r["fragment"].as_str().unwrap_or("").trim().to_string();
        let ann = r["annotations"].as_array().and_then(|a| a.first());
        let body = ann
            .and_then(|a| a.pointer("/body/plain").or_else(|| a.get("body")))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if fragment.is_empty() || body.is_empty() {
            continue;
        }
        out.push(Annotation {
            fragment,
            text: excerpt(body, EXCERPT_CHARS),
            source: SOURCE.to_string(),
            url: ann.and_then(|a| a["url"].as_str()).or_else(|| r["url"].as_str()).map(str::to_string),
        });
    }
    out
}

/// The Genius id of the search hit that is this song (same title and artist), if any.
pub fn pick_song(v: &Value, artist: &str, title: &str) -> Option<u64> {
    let want_title = norm(&crate::year::base_title(title));
    let want_artist = norm(artist.split(", ").next().unwrap_or(artist));
    let hits = v.pointer("/response/hits")?.as_array()?;
    hits.iter()
        .filter_map(|h| h.get("result"))
        .find(|r| {
            let t = norm(&crate::year::base_title(r["title"].as_str().unwrap_or("")));
            let a = norm(r.pointer("/primary_artist/name").and_then(Value::as_str).unwrap_or(""));
            t == want_title && (a.contains(&want_artist) || want_artist.contains(&a)) && !a.is_empty()
        })
        .and_then(|r| r["id"].as_u64())
}

/// For each lyric line, the annotation (index into `anns`) whose quoted fragment it belongs to.
/// A fragment may span several lines (every line it covers gets it), appear inside a longer
/// line, or be the whole line; very short fragments and lines never match by accident.
pub fn match_lines(lines: &[String], anns: &[Annotation]) -> Vec<Option<usize>> {
    let frags: Vec<String> = anns.iter().map(|a| norm(&a.fragment)).collect();
    lines
        .iter()
        .map(|line| {
            let l = norm(line);
            if l.is_empty() {
                return None;
            }
            let mut best: Option<(usize, usize)> = None; // (score, index): lower score is better
            for (i, f) in frags.iter().enumerate() {
                let score = if *f == l {
                    0
                } else if l.len() >= MIN_LINE && f.contains(&l) {
                    // the line is part of a longer quoted passage: the shorter passage is more specific
                    1 + f.len()
                } else if f.len() >= MIN_FRAGMENT && l.contains(f.as_str()) {
                    1 + l.len() - f.len()
                } else {
                    continue;
                };
                if best.is_none_or(|(s, _)| score < s) {
                    best = Some((score, i));
                }
            }
            best.map(|(_, i)| i)
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    /// found | none
    kind: String,
    anns: Vec<Annotation>,
    saved: u64,
}

#[doc(hidden)]
pub fn cache_path(artist: &str, title: &str) -> std::path::PathBuf {
    let first = artist.split(", ").next().unwrap_or(artist);
    cache_dir().join(format!("annot-{:016x}.json", fnv1a(&format!("{first}\0{}", crate::year::base_title(title)))))
}

/// A usable cache entry: `Some(vec)` (possibly empty = a known miss), `None` = ask again.
async fn read_cache(path: &std::path::Path) -> Option<Vec<Annotation>> {
    let e: CacheEntry = serde_json::from_str(&tokio::fs::read_to_string(path).await.ok()?).ok()?;
    let age = now_secs().saturating_sub(e.saved);
    match e.kind.as_str() {
        "found" if age < POS_TTL => Some(e.anns),
        "none" if age < NEG_TTL => Some(Vec::new()),
        _ => None,
    }
}

async fn write_cache(path: &std::path::Path, anns: &[Annotation]) {
    let e = CacheEntry { kind: if anns.is_empty() { "none" } else { "found" }.into(), anns: anns.to_vec(), saved: now_secs() };
    if let Ok(j) = serde_json::to_string(&e) {
        let _ = tokio::fs::write(path, j).await;
    }
}

/// `GENIUS_TOKEN` or the configured token; never logged.
pub fn token(configured: &str) -> Option<String> {
    let t = if configured.trim().is_empty() { std::env::var("GENIUS_TOKEN").unwrap_or_default() } else { configured.to_string() };
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

async fn get(c: &reqwest::Client, base: &str, path: &str, token: &str) -> Result<Value, String> {
    let r = c
        .get(format!("{base}{path}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| {
            record_fail();
            format!("Genius unreachable ({})", e.without_url())
        })?;
    let st = r.status();
    if st.as_u16() == 401 || st.as_u16() == 403 {
        record_fail();
        return Err("Genius refused the token (check genius_token / GENIUS_TOKEN)".into());
    }
    if !st.is_success() {
        record_fail();
        return Err(format!("Genius answered HTTP {}", st.as_u16()));
    }
    r.json::<Value>().await.map_err(|_| {
        record_fail();
        "unreadable reply from Genius".to_string()
    })
}

/// The annotations of a song. `Ok(vec![])` is a (cached) miss; `Err` is never cached.
pub async fn fetch(c: &reqwest::Client, token: &str, artist: &str, title: &str) -> Result<Vec<Annotation>, String> {
    fetch_from(c, API, token, artist, title).await
}

#[doc(hidden)]
pub async fn fetch_from(c: &reqwest::Client, base: &str, token: &str, artist: &str, title: &str) -> Result<Vec<Annotation>, String> {
    let path = cache_path(artist, title);
    if let Some(hit) = read_cache(&path).await {
        return Ok(hit);
    }
    if breaker_open() {
        return Err("Genius paused after repeated failures".into());
    }
    let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::base_title(title));
    let search = get(c, base, &format!("/search?q={}", urlenc(&q)), token).await?;
    let Some(id) = pick_song(&search, artist, title) else {
        write_cache(&path, &[]).await;
        FAILS.store(0, Ordering::Relaxed);
        return Ok(Vec::new());
    };
    let refs = get(c, base, &format!("/referents?song_id={id}&text_format=plain&per_page=50"), token).await?;
    FAILS.store(0, Ordering::Relaxed);
    let anns = parse_referents(&refs);
    write_cache(&path, &anns).await;
    Ok(anns)
}

fn urlenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

// ── the AI mode ─────────────────────────────────────────────────────────────

/// (system, user) prompts asking for one short note per lyric line, or null.
pub fn ai_note_prompt(title: &str, artist: &str, lines: &[String]) -> (String, String) {
    let system = "You annotate song lyrics. For each numbered lyric line, give at most one SHORT note \
        (under 200 characters) when the line contains a cultural reference, an idiom, wordplay or needs backstory \
        to be understood; otherwise null. Be factual; if you are not sure, use null. \
        Output ONLY a JSON array with exactly one entry per input line, in order: a string or null. No commentary."
        .to_string();
    let mut user = format!("Title: {title}\nArtist: {artist}\n\nThere are {} numbered lines.\n\n", lines.len());
    for (i, l) in lines.iter().enumerate() {
        user.push_str(&format!("{}. {}\n", i + 1, l));
    }
    (system, user)
}

/// The model's answer as one optional note per line (empty strings become `None`, long notes are
/// shortened); an array of the wrong length or with other entries is an error.
pub fn parse_ai_notes(raw: &str, n: usize) -> Result<Vec<Option<String>>, String> {
    let Some(Value::Array(a)) = crate::ai::extract_array(raw) else {
        return Err("the model did not return a JSON array".into());
    };
    if a.len() != n {
        return Err(format!("the model returned {} entries, expected {n}", a.len()));
    }
    a.iter()
        .map(|v| match v {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(excerpt(s, EXCERPT_CHARS)).filter(|s| !s.is_empty())),
            _ => Err("the model returned an entry that is neither text nor null".to_string()),
        })
        .collect()
}

#[doc(hidden)]
pub fn ai_cache_for_test(cfg: &crate::ai::Cfg, artist: &str, title: &str, lines: &[String]) -> std::path::PathBuf {
    ai_cache(cfg, artist, title, lines)
}

fn ai_cache(cfg: &crate::ai::Cfg, artist: &str, title: &str, lines: &[String]) -> std::path::PathBuf {
    cache_dir().join(format!(
        "annot-ai-{:016x}.json",
        fnv1a(&format!("{artist}\0{title}\0{}\0{}\0{}", cfg.provider.name(), cfg.model, fnv1a(&lines.join("\n"))))
    ))
}

/// AI-generated notes for the whole song (one request, one retry, cached per model).
pub async fn ai_notes(
    c: &reqwest::Client,
    cfg: &crate::ai::Cfg,
    artist: &str,
    title: &str,
    lines: &[String],
) -> Result<Vec<Option<String>>, String> {
    let path = ai_cache(cfg, artist, title, lines);
    if let Ok(txt) = tokio::fs::read_to_string(&path).await {
        if let Ok(v) = serde_json::from_str::<Vec<Option<String>>>(&txt) {
            if v.len() == lines.len() {
                return Ok(v);
            }
        }
    }
    let (system, user) = ai_note_prompt(title, artist, lines);
    let mut last = String::new();
    for _ in 0..2 {
        let raw = crate::ai::complete(c, cfg, &system, &user).await?;
        match parse_ai_notes(&raw, lines.len()) {
            Ok(v) => {
                if let Ok(j) = serde_json::to_string(&v) {
                    let _ = tokio::fs::write(&path, j).await;
                }
                return Ok(v);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ann(fragment: &str, text: &str) -> Annotation {
        Annotation { fragment: fragment.into(), text: text.into(), source: SOURCE.into(), url: None }
    }

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn norm_ignores_case_punctuation_and_apostrophes() {
        assert_eq!(norm("Don't  stop — believin'!"), "dont stop believin");
        assert_eq!(norm("  "), "");
        assert_eq!(norm("Ça va, ÉTÉ"), "ça va été");
    }

    #[test]
    fn excerpt_is_short_and_cut_at_a_word() {
        assert_eq!(excerpt("  short   text ", 50), "short text");
        let long = "word ".repeat(100);
        let e = excerpt(&long, 40);
        assert!(e.chars().count() <= 40 && e.ends_with('…') && !e.contains("wor…"), "{e}");
        assert!(excerpt("x".repeat(500).as_str(), 30).chars().count() <= 30);
    }

    fn referents() -> Value {
        json!({"meta": {"status": 200}, "response": {"referents": [
            {"fragment": "Hello from the other side", "url": "https://genius.com/12-a",
             "annotations": [{"url": "https://genius.com/1-annotated", "body": {"plain": "A line about calling an ex.\n\nSecond paragraph."}}]},
            {"fragment": "", "annotations": [{"body": {"plain": "no fragment"}}]},
            {"fragment": "Empty body", "annotations": [{"body": {"plain": "  "}}]},
            {"fragment": "At least I can say\nthat I've tried", "annotations": [{"body": {"plain": "Two lines."}}]},
            {"fragment": "No annotation list", "annotations": []}
        ]}})
    }

    #[test]
    fn referents_are_parsed_and_junk_is_skipped() {
        let a = parse_referents(&referents());
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].fragment, "Hello from the other side");
        assert_eq!(a[0].text, "A line about calling an ex. Second paragraph.");
        assert_eq!((a[0].source.as_str(), a[0].url.as_deref()), ("Genius", Some("https://genius.com/1-annotated")));
        assert!(parse_referents(&json!({"response": {}})).is_empty());
        assert!(parse_referents(&json!(null)).is_empty());
    }

    #[test]
    fn the_right_search_hit_is_picked() {
        let v = json!({"response": {"hits": [
            {"result": {"id": 1, "title": "Hello (Live)", "primary_artist": {"name": "Someone Else"}}},
            {"result": {"id": 2, "title": "Hello", "primary_artist": {"name": "Adele"}}},
        ]}});
        assert_eq!(pick_song(&v, "Adele", "Hello - Remastered"), Some(2));
        assert_eq!(pick_song(&v, "Adele", "Rolling in the Deep"), None);
        assert_eq!(pick_song(&v, "Nobody", "Hello"), None);
        assert_eq!(pick_song(&json!({}), "a", "b"), None);
    }

    #[test]
    fn a_fragment_matches_the_lines_it_covers() {
        let anns = vec![
            ann("Hello from the other side", "a"),
            ann("At least I can say\nthat I've tried", "b"),
        ];
        let l = lines(&[
            "Hello, from the other side!",
            "Something unrelated entirely",
            "At least I can say",
            "That I've tried",
            "oh",
        ]);
        let m = match_lines(&l, &anns);
        assert_eq!(m, vec![Some(0), None, Some(1), Some(1), None]);
    }

    #[test]
    fn fragments_inside_longer_lines_and_punctuation_differences_match() {
        let anns = vec![ann("the other side", "x"), ann("broken heart of gold", "y")];
        // 14 letters: long enough to be found inside a longer line
        let m = match_lines(&lines(&["I called you from the other side of town", "Broken heart, of GOLD!"]), &anns);
        assert_eq!(m, vec![Some(0), Some(1)]);
    }

    #[test]
    fn short_fragments_and_short_lines_never_match_by_accident() {
        let anns = vec![ann("love", "x"), ann("the whole long quoted passage of the chorus here", "y")];
        let m = match_lines(&lines(&["All you need is love", "love", "the whole", "oh yeah"]), &anns);
        // "love" only matches a line that is exactly it; "the whole" (8 letters) is inside the passage
        assert_eq!(m, vec![None, Some(0), Some(1), None]);
    }

    #[test]
    fn the_more_specific_annotation_wins() {
        let anns = vec![
            ann("first line of the verse\nsecond line of the verse", "long"),
            ann("second line of the verse", "short"),
        ];
        let m = match_lines(&lines(&["first line of the verse", "second line of the verse"]), &anns);
        assert_eq!(m, vec![Some(0), Some(1)]);
    }

    #[test]
    fn ai_notes_are_validated() {
        let ok = parse_ai_notes(r#"```json
["A reference to Hamlet", null, ""]
```"#, 3)
        .unwrap();
        assert_eq!(ok, vec![Some("A reference to Hamlet".to_string()), None, None]);
        assert!(parse_ai_notes(r#"["a"]"#, 2).is_err());
        assert!(parse_ai_notes("no json here", 1).is_err());
        assert!(parse_ai_notes("[1, 2]", 2).is_err());
        let long = format!("[\"{}\"]", "word ".repeat(200));
        assert!(parse_ai_notes(&long, 1).unwrap()[0].as_ref().unwrap().chars().count() <= EXCERPT_CHARS);
        let (sys, user) = ai_note_prompt("T", "A", &lines(&["x", "y"]));
        assert!(sys.contains("null") && sys.contains("JSON") && user.contains("2. y"));
    }

    #[test]
    fn the_token_comes_from_the_config_or_the_environment() {
        assert_eq!(token("  abc  ").as_deref(), Some("abc"));
        assert_eq!(token("abc"), Some("abc".into()));
    }

    #[test]
    fn urlenc_escapes_what_a_query_cannot_hold() {
        assert_eq!(urlenc("a b&c/é"), "a%20b%26c%2F%C3%A9");
    }
}
