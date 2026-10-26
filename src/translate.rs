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

/// `zh-CN` → `zh`, case-insensitive.
pub fn base_code(c: &str) -> String {
    c.split(['-', '_']).next().unwrap_or(c).trim().to_lowercase()
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

/// `gtx`, then `dict-chrome-ex` when the first is limited or unreachable.
async fn google(c: &reqwest::Client, cfg: &Cfg, lang: &str, text: &str) -> Result<(String, Option<String>), Fail> {
    let (u1, u2) = match &cfg.google_base {
        Some(b) => (b.clone(), format!("{b}/dict")),
        None => (GTX.to_string(), DICT.to_string()),
    };
    match get(c, &u1, "gtx", lang, text).await.map(|v| (parse(&v), detected(&v))) {
        Ok((Some(t), det)) => return Ok((t, det)),
        Ok((None, _)) | Err(Fail::Other) => {}
        Err(_) => {}
    }
    match get(c, &u2, "dict-chrome-ex", lang, text).await {
        Ok(v) => parse_dict(&v).map(|t| (t, None)).ok_or(Fail::Other),
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

/// The original line when its language is on the skip list, else the translation.
fn keep_skipped(cfg: &Cfg, detected: Option<String>, translated: String, original: &str) -> String {
    match detected {
        Some(d) if cfg.skip.contains(&d) => original.to_string(),
        _ => translated,
    }
}

/// Translate every line (order preserved; a line that fails keeps its original text).
/// Returns the lines and which backend did the work. `Err` only when nearly everything failed.
pub async fn lines(c: &reqwest::Client, cfg: &Cfg, lang: &str, src: &[String]) -> Result<(Vec<String>, &'static str), String> {
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
