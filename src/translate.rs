//! Lyric translation: Google Translate's unauthenticated web endpoints first
//! (unofficial; they can be rate-limited or change without notice), then a
//! LibreTranslate server when Google answers 429 or can't be reached.
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

const GTX: &str = "https://translate.googleapis.com/translate_a/single";
const DICT: &str = "https://clients5.google.com/translate_a/t";
pub const LIBRE_URL: &str = "https://libretranslate.com/translate";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";

#[derive(Clone, Debug, Default)]
pub struct Cfg {
    /// Test hook: replaces both Google endpoints.
    pub google_base: Option<String>,
    pub fallback: bool,
    pub fallback_url: String,
    pub fallback_key: Option<String>,
    /// Lines the service detects as one of these languages (`sv`, `de`…) stay as they are.
    pub skip: Vec<String>,
}

/// The source language Google detected: `[[…],null,"sv"]`.
pub fn detected(v: &Value) -> Option<String> {
    v.get(2).and_then(Value::as_str).map(base_code)
}

/// `zh-CN` → `zh`, case-insensitive; the old ISO codes Google still answers with become
/// the current ones (`iw` → `he`, `jw` → `jv`, `in` → `id`, `ji` → `yi`, `mo` → `ro`).
pub fn base_code(c: &str) -> String {
    let b = c.split(['-', '_']).next().unwrap_or(c).trim().to_lowercase();
    match b.as_str() {
        "iw" => "he".into(),
        "jw" => "jv".into(),
        "in" => "id".into(),
        "ji" => "yi".into(),
        "mo" => "ro".into(),
        _ => b,
    }
}

/// Shown in the lyrics title while the song's language can't be told and its language is on the
/// never-translate list: nothing is translated, the check is retried.
pub const HELD: &str = "translation held: language not detected yet, retrying";
/// The same, once the retries ran out.
pub const HELD_FINAL: &str = "translation held: language not detected (rate-limited?)";
/// Prefix of the message for a song whose language is on the never-translate list.
pub const SKIPPED: &str = "never translated";

pub fn is_held(e: &str) -> bool {
    e == HELD
}

pub fn is_skipped(e: &str) -> bool {
    e.starts_with(SKIPPED)
}

/// Why one request failed: `Limited` / `Net` switch to the fallback, `Other` is retried.
enum Fail {
    Limited,
    Net,
    Other,
}

/// Pull the translated text out of `[[["hello","hallo",…],…],…]` (the `gtx` shape).
pub fn parse(v: &Value) -> Option<String> {
    let segs = v.get(0)?.as_array()?;
    let out: String = segs
        .iter()
        .filter_map(|s| s.get(0).and_then(Value::as_str))
        .collect();
    (!out.is_empty()).then_some(out)
}

/// The `dict-chrome-ex` shapes: `{"sentences":[{"trans":…}]}`, `["…"]` or `[["…","ko"]]`.
pub fn parse_dict(v: &Value) -> Option<String> {
    let out: String = if let Some(s) = v.get("sentences").and_then(Value::as_array) {
        s.iter().filter_map(|x| x.get("trans").and_then(Value::as_str)).collect()
    } else {
        let first = v.get(0)?;
        first.as_str().or_else(|| first.get(0).and_then(Value::as_str))?.to_string()
    };
    (!out.is_empty()).then_some(out)
}

/// The source language `dict-chrome-ex` names: `{"sentences":[…],"src":"sv"}`. The array shapes
/// carry none.
pub fn dict_lang(v: &Value) -> Option<String> {
    v.get("src").and_then(Value::as_str).map(base_code).filter(|c| !c.is_empty())
}

fn classify(e: &reqwest::Error) -> Fail {
    if e.is_timeout() || e.is_connect() || e.is_request() { Fail::Net } else { Fail::Other }
}

async fn get(c: &reqwest::Client, url: &str, client: &str, lang: &str, text: &str) -> Result<Value, Fail> {
    let mut q = vec![("client", client), ("sl", "auto"), ("tl", lang), ("q", text)];
    if client == "gtx" {
        q.push(("dt", "t"));
    }
    let r = c
        .get(url)
        .query(&q)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| classify(&e))?;
    match r.status().as_u16() {
        200..=299 => r.json::<Value>().await.map_err(|_| Fail::Other),
        429 => Err(Fail::Limited),
        500..=599 => Err(Fail::Net),
        _ => Err(Fail::Other),
    }
}

fn google_urls(cfg: &Cfg) -> (String, String) {
    match &cfg.google_base {
        Some(b) => (b.clone(), format!("{b}/dict")),
        None => (GTX.to_string(), DICT.to_string()),
    }
}

/// `gtx`, then `dict-chrome-ex` when the first is limited or unreachable.
async fn google(c: &reqwest::Client, cfg: &Cfg, lang: &str, text: &str) -> Result<(String, Option<String>), Fail> {
    let (u1, u2) = google_urls(cfg);
    match get(c, &u1, "gtx", lang, text).await.map(|v| (parse(&v), detected(&v))) {
        Ok((Some(t), det)) => return Ok((t, det)),
        Ok((None, _)) | Err(Fail::Other) => {}
        Err(_) => {}
    }
    match get(c, &u2, "dict-chrome-ex", lang, text).await {
        Ok(v) => parse_dict(&v).map(|t| (t, dict_lang(&v))).ok_or(Fail::Other),
        Err(e) => Err(e),
    }
}

async fn libre(c: &reqwest::Client, cfg: &Cfg, lang: &str, text: &str) -> Option<(String, Option<String>)> {
    // LibreTranslate wants "zh", not "zh-CN"
    let target = lang.split('-').next().unwrap_or(lang);
    let mut body = json!({ "q": text, "source": "auto", "target": target, "format": "text" });
    if let Some(k) = cfg.fallback_key.as_deref().filter(|k| !k.is_empty()) {
        body["api_key"] = json!(k);
    }
    let r = c
        .post(&cfg.fallback_url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v = r.json::<Value>().await.ok()?;
    let det = v
        .pointer("/detectedLanguage/language")
        .and_then(Value::as_str)
        .map(base_code);
    v.get("translatedText")?.as_str().filter(|t| !t.is_empty()).map(|t| (t.to_string(), det))
}

async fn line(c: &reqwest::Client, cfg: &Cfg, use_fallback: &AtomicBool, lang: &str, l: String) -> Option<String> {
    let t = l.trim();
    if t.is_empty() || t == "♪" {
        return Some(l);
    }
    if !use_fallback.load(Ordering::Relaxed) {
        // one retry for a plain failure: a single dropped request shouldn't leave a line
        for _ in 0..2 {
            match google(c, cfg, lang, t).await {
                Ok((x, det)) => return Some(keep_skipped(cfg, det, x, &l)),
                Err(Fail::Other) => continue,
                Err(_) => {
                    // limited / unreachable: stay on the fallback for the rest of the song
                    if cfg.fallback {
                        use_fallback.store(true, Ordering::Relaxed);
                    }
                    break;
                }
            }
        }
    }
    if !cfg.fallback {
        return None;
    }
    libre(c, cfg, lang, t).await.map(|(x, det)| keep_skipped(cfg, det, x, &l))
}

/// What names the song's language: a sample of its lines through the same backends, in the same
/// order (`gtx`, `dict-chrome-ex`, LibreTranslate), stopping at the first that says. `None` when
/// none could.
async fn detect(c: &reqwest::Client, cfg: &Cfg, lang: &str, sample: &str) -> Option<String> {
    let (u1, u2) = google_urls(cfg);
    if let Ok(v) = get(c, &u1, "gtx", lang, sample).await {
        if let Some(d) = detected(&v).filter(|d| !d.is_empty()) {
            return Some(d);
        }
    }
    if let Ok(v) = get(c, &u2, "dict-chrome-ex", lang, sample).await {
        if let Some(d) = dict_lang(&v) {
            return Some(d);
        }
    }
    if cfg.fallback {
        return libre(c, cfg, lang, sample).await.and_then(|(_, d)| d).filter(|d| !d.is_empty());
    }
    None
}

/// The first few real lines of the song, one request's worth.
fn sample(src: &[String]) -> String {
    let lines: Vec<&str> = src
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && *l != "♪")
        .take(8)
        .collect();
    lines.join("\n").chars().take(600).collect()
}

/// The original line when its language is on the skip list, else the translation.
fn keep_skipped(cfg: &Cfg, detected: Option<String>, translated: String, original: &str) -> String {
    match detected {
        Some(d) if cfg.skip.contains(&d) => original.to_string(),
        _ => translated,
    }
}

/// Translate every line (order preserved; a line that fails keeps its original text).
/// Returns the lines and which backend did the work. `Err` when nearly everything failed, and
/// for a song whose language is on the skip list ([`is_skipped`]) or can't be told while the
/// list is in use ([`is_held`]): those make no per-line requests at all.
///
/// With a skip list the song's language is asked once, from a sample, before any line is sent.
/// Where a finished translation is kept: one file per (language, never-translate list,
/// lines), so another lyrics version or a changed list never reads an old answer.
fn cache_file(lang: &str, skip: &[String], src: &[String]) -> std::path::PathBuf {
    let key = crate::util::fnv1a(&format!("{lang}\0{}\0{}", skip.join(","), src.join("\n")));
    crate::util::cache_dir().join(format!("translation-{key:016x}.json"))
}

/// A translation saved by [`store`], with the service that made it.
pub async fn cached(lang: &str, skip: &[String], src: &[String]) -> Option<(Vec<String>, &'static str)> {
    read_cache(&cache_file(lang, skip, src), src.len()).await
}

/// Keep a translation (`cache_translations`): the next time the song plays it costs no request.
pub async fn store(lang: &str, skip: &[String], src: &[String], lines: &[String], backend: &str) {
    write_cache(&cache_file(lang, skip, src), lines, backend).await;
}

pub async fn read_cache(path: &std::path::Path, n: usize) -> Option<(Vec<String>, &'static str)> {
    let txt = tokio::fs::read_to_string(path).await.ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    let lines: Vec<String> = v["lines"].as_array()?.iter().map(|l| l.as_str().unwrap_or("").to_string()).collect();
    let backend = if v["backend"].as_str() == Some("libretranslate") { "libretranslate" } else { "google" };
    (lines.len() == n).then_some((lines, backend))
}

pub async fn write_cache(path: &std::path::Path, lines: &[String], backend: &str) {
    let _ = tokio::fs::write(path, json!({ "backend": backend, "lines": lines }).to_string()).await;
}

pub async fn lines(c: &reqwest::Client, cfg: &Cfg, lang: &str, src: &[String]) -> Result<(Vec<String>, &'static str), String> {
    // legacy codes (`iw`, `zh-CN`) in the list meet the codes the services answer with
    let skip: Vec<String> = cfg.skip.iter().map(|s| base_code(s)).filter(|s| !s.is_empty()).collect();
    let cfg = &Cfg { skip: skip.clone(), ..cfg.clone() };
    let sample = sample(src);
    if !skip.is_empty() && !sample.is_empty() {
        match detect(c, cfg, lang, &sample).await {
            Some(d) if skip.contains(&d) => return Err(format!("{SKIPPED} ({d})")),
            Some(_) => {}
            None => return Err(HELD.into()),
        }
    }
    let use_fallback = AtomicBool::new(false);
    let out: Vec<Option<String>> =
        futures::stream::iter(src.iter().cloned().map(|l| line(c, cfg, &use_fallback, lang, l)))
            .buffered(6)
            .collect()
            .await;
    let failed = out.iter().filter(|o| o.is_none()).count();
    let real = src.iter().filter(|l| !l.trim().is_empty() && l.trim() != "♪").count();
    if failed > 0 && failed * 5 > real {
        return Err(if cfg.fallback {
            "translation failed (Google and LibreTranslate unavailable)".into()
        } else {
            "translation failed (Google Translate unreachable or rate-limited)".into()
        });
    }
    let backend = if use_fallback.load(Ordering::Relaxed) { "libretranslate" } else { "google" };
    Ok((out.into_iter().zip(src).map(|(o, s)| o.unwrap_or_else(|| s.clone())).collect(), backend))
}
