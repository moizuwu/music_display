use crate::chords;
use crate::util::{cache_dir, fnv1a, now_secs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BASE: &str = "https://genius.com";
// looks like a normal browser, as for Ultimate Guitar
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
const NEG_TTL: u64 = 7 * 24 * 3600;

/// One lyric line and the members singing it (empty = unknown / everyone).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub members: Vec<String>,
    pub text: String,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    kind: String, // found | none
    parts: Vec<Part>,
    saved: u64,
}

// ── page parsing ───────────────────────────────────────────────────────
/// `[Verse 1: Jimin]` → `["Jimin"]`, `[Chorus: Jungkook, V]` → both, `[Pre-Chorus: RM &
/// SUGA]` → both. `Some(vec![])` for a header without names (`[Bridge]`); `None` if the
/// line isn't a header.
pub fn parse_header(line: &str) -> Option<Vec<String>> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let Some((_, names)) = inner.split_once(':') else {
        return Some(Vec::new());
    };
    let mut out: Vec<String> = Vec::new();
    for n in names.replace(" and ", ",").split([',', '&', '+', ';', '/']) {
        // "Jin (진)" → "Jin"
        let n = n.split('(').next().unwrap_or("").trim();
        if !n.is_empty() && n.chars().count() <= 24 && !out.iter().any(|o| o == n) {
            out.push(n.to_string());
        }
    }
    Some(out)
}

/// The inner HTML of every `data-lyrics-container` div (tag-depth aware).
fn containers(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = html[from..].find("data-lyrics-container") {
        let at = from + i;
        let Some(open_end) = html[at..].find('>').map(|e| at + e + 1) else { break };
        let (mut depth, mut pos) = (1usize, open_end);
        let mut end = html.len();
        while let Some(lt) = html[pos..].find('<').map(|l| pos + l) {
            let rest = &html[lt..];
            if rest.starts_with("<div") {
                depth += 1;
            } else if rest.starts_with("</div") {
                depth -= 1;
                if depth == 0 {
                    end = lt;
                    break;
                }
            }
            pos = lt + 1;
        }
        out.push(&html[open_end..end]);
        from = end.max(open_end);
    }
    out
}

/// Container HTML → text: `<br>` is a newline, other tags vanish (and the contents of
/// `data-exclude-from-selection` blocks with them), entities are decoded.
fn text_of(html: &str) -> String {
    let (mut out, mut skip, mut pos) = (String::new(), 0usize, 0usize);
    while pos < html.len() {
        let Some(lt) = html[pos..].find('<').map(|l| pos + l) else {
            if skip == 0 {
                out.push_str(&html[pos..]);
            }
            break;
        };
        if skip == 0 {
            out.push_str(&html[pos..lt]);
        }
        let Some(gt) = html[lt..].find('>').map(|g| lt + g) else { break };
        let tag = html[lt + 1..gt].trim().to_ascii_lowercase();
        if tag.starts_with("div") {
            if skip > 0 {
                skip += 1;
            } else if tag.contains("data-exclude-from-selection") {
                skip = 1;
            }
        } else if tag.starts_with("/div") {
            skip = skip.saturating_sub(1);
        } else if skip == 0 && (tag == "br" || tag.starts_with("br ") || tag.starts_with("br/")) {
            out.push('\n');
        }
        pos = gt + 1;
    }
    crate::ug::unescape(&out)
}

/// A song page → its lines, each with the members of the section it sits in.
pub fn parse_page(html: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut members: Vec<String> = Vec::new();
    for c in containers(html) {
        for line in text_of(c).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match parse_header(line) {
                Some(m) => members = m,
                None => parts.push(Part { members: members.clone(), text: line.to_string() }),
            }
        }
    }
    parts
}

/// Path (`/Bts-dynamite-lyrics`) of the best song hit in a `search/multi` reply: the
/// title and artist must both really be the ones asked for.
pub fn best_hit(reply: &Value, artist: &str, title: &str) -> Option<String> {
    let want_t = chords::norm(&crate::year::clean(title));
    let want_a = chords::norm(artist.split(", ").next().unwrap_or(artist));
    let mut best: Option<(f32, String)> = None;
    let sections = reply.pointer("/response/sections")?.as_array()?;
    for hit in sections.iter().filter_map(|s| s.get("hits")?.as_array()).flatten() {
        if hit.get("type").and_then(Value::as_str).is_some_and(|t| t != "song") {
            continue;
        }
        let Some(r) = hit.get("result") else { continue };
        let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
        let song = chords::norm(&crate::year::clean(text("title")));
        let art = chords::norm(
            r.pointer("/primary_artist/name")
                .and_then(Value::as_str)
                .unwrap_or_else(|| text("artist_names")),
        );
        let t = chords::dice(&want_t, &song);
        let artist_ok =
            !art.is_empty() && !want_a.is_empty() && (art.contains(&want_a) || want_a.contains(&art) || chords::dice(&want_a, &art) >= 0.7);
        let path = match (text("path"), text("url")) {
            ("", "") => continue,
            ("", u) => u.split_once("://").and_then(|(_, h)| h.find('/').map(|i| h[i..].to_string())).unwrap_or_default(),
            (p, _) => p.to_string(),
        };
        if t >= 0.7 && artist_ok && !path.is_empty() && best.as_ref().is_none_or(|(b, _)| t > *b) {
            best = Some((t, path));
        }
    }
    best.map(|b| b.1)
}

/// Per synced lyric line, the members singing it: Genius lines are matched in order
/// (a short look-ahead window, so a skipped line doesn't derail the rest). A line with
/// no match, or in an unattributed section, gets an empty list.
pub fn attribute(parts: &[Part], lyrics: &[String]) -> Vec<Vec<String>> {
    let norms: Vec<String> = parts.iter().map(|p| chords::norm(&p.text)).collect();
    let mut out = vec![Vec::new(); lyrics.len()];
    let mut j = 0usize;
    for (li, text) in lyrics.iter().enumerate() {
        let n = chords::norm(text);
        if n.is_empty() {
            continue;
        }
        let mut best: Option<(usize, f32)> = None;
        for (k, s) in norms.iter().enumerate().skip(j).take(12) {
            let d = chords::dice(&n, s);
            if d >= 0.6 && best.is_none_or(|(_, b)| d > b) {
                best = Some((k, d));
            }
        }
        if let Some((k, _)) = best {
            j = k + 1;
            out[li] = parts[k].members.clone();
        }
    }
    out
}

// ── network ────────────────────────────────────────────────────────────
async fn get(c: &reqwest::Client, url: &str, query: &[(&str, &str)]) -> Result<String, String> {
    let r = c
        .get(url)
        .query(query)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("network error ({})", e.without_url()))?;
    let st = r.status();
    if !st.is_success() {
        return Err(format!("HTTP {} from genius", st.as_u16()));
    }
    r.text().await.map_err(|e| format!("read error ({e})"))
}

fn cache_path(artist: &str, title: &str) -> std::path::PathBuf {
    cache_dir().join(format!("members-{:016x}.json", fnv1a(&format!("{artist}\0{title}"))))
}

/// Cache → search → song page. `Ok(None)` = Genius has nothing usable (remembered for a
/// week); `Err` is never cached.
pub async fn lookup(c: &reqwest::Client, base: &str, artist: &str, title: &str) -> Result<Option<Vec<Part>>, String> {
    let path = cache_path(artist, title);
    if let Ok(txt) = tokio::fs::read_to_string(&path).await {
        if let Ok(e) = serde_json::from_str::<CacheEntry>(&txt) {
            if e.kind == "found" {
                return Ok(Some(e.parts));
            } else if now_secs().saturating_sub(e.saved) < NEG_TTL {
                return Ok(None);
            }
        }
    }
    let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::clean(title));
    let body = get(c, &format!("{base}/api/search/multi"), &[("q", &q)]).await?;
    let reply: Value = serde_json::from_str(&body).map_err(|_| "unreadable genius reply".to_string())?;
    let parts = match best_hit(&reply, artist, title) {
        Some(p) => parse_page(&get(c, &format!("{base}{p}"), &[]).await?),
        None => Vec::new(),
    };
    // a page without a single member header is no use, however many lines it has
    let parts = if parts.iter().any(|p| !p.members.is_empty()) { Some(parts) } else { None };
    let entry = match &parts {
        Some(p) => CacheEntry { kind: "found".into(), parts: p.clone(), saved: now_secs() },
        None => CacheEntry { kind: "none".into(), parts: Vec::new(), saved: now_secs() },
    };
    if let Ok(j) = serde_json::to_string(&entry) {
        let _ = tokio::fs::write(&path, j).await;
    }
    Ok(parts)
}

/// The plain lyrics of the Genius song page for this song (section headers dropped), for when
/// no other source has them. `Ok(None)` when the song is not found or the page has too little
/// text; `Err` when Genius could not answer for now (network, 429, 5xx).
pub async fn plain_lyrics(c: &reqwest::Client, base: &str, artist: &str, title: &str) -> Result<Option<String>, String> {
    // an answer that is a refusal (404, 403 …) says nothing will change soon; the rest may
    let held = |e: String| if e.starts_with("HTTP 4") && !e.starts_with("HTTP 429") { None } else { Some(e) };
    let q = format!("{} {}", artist.split(", ").next().unwrap_or(artist), crate::year::clean(title));
    let body = match get(c, &format!("{base}/api/search/multi"), &[("q", &q)]).await {
        Ok(b) => b,
        Err(e) => return held(e).map_or(Ok(None), Err),
    };
    let Some(reply) = serde_json::from_str::<Value>(&body).ok() else { return Ok(None) };
    let Some(hit) = best_hit(&reply, artist, title) else { return Ok(None) };
    let page = match get(c, &format!("{base}{hit}"), &[]).await {
        Ok(p) => p,
        Err(e) => return held(e).map_or(Ok(None), Err),
    };
    let text = containers(&page).into_iter().map(text_of).collect::<Vec<_>>().join("\n");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !(l.starts_with('[') && l.ends_with(']')))
        .collect();
    let text = lines.join("\n").trim().to_string();
    Ok((text.lines().filter(|l| !l.trim().is_empty()).count() >= 4).then_some(text))
}

/// Lookup for the app: any failure just means no member tags.
pub async fn fetch(c: reqwest::Client, artist: String, title: String) -> Vec<Part> {
    lookup(&c, BASE, &artist, &title).await.ok().flatten().unwrap_or_default()
}
