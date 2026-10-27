//! Optional AI whole-song lyric translation. One request per song; the answer must be a JSON
//! array with exactly one string per input line, otherwise the caller falls back to the
//! standard engine. The API key is read from an environment variable at call time and never
//! logged, stored or put into an error.
use crate::util::{cache_dir, fnv1a, now_secs};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

const MAX_FAILS: u32 = 3;
const BREAKER_SECS: u64 = 10 * 60;
const TIMEOUT: u64 = 120;

static FAILS: AtomicU32 = AtomicU32::new(0);
static LAST_FAIL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Ollama, LM Studio and anything else that speaks `/v1/chat/completions`.
    OpenAiCompatible,
    Gemini,
    Claude,
    OpenAi,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Provider> {
        match s.trim().to_lowercase().as_str() {
            "openai-compatible" | "openai_compatible" | "compatible" | "ollama" | "lmstudio"
            | "lm-studio" | "local" => Some(Provider::OpenAiCompatible),
            "gemini" | "google" => Some(Provider::Gemini),
            "claude" | "anthropic" => Some(Provider::Claude),
            "openai" => Some(Provider::OpenAi),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Provider::OpenAiCompatible => "openai-compatible",
            Provider::Gemini => "gemini",
            Provider::Claude => "claude",
            Provider::OpenAi => "openai",
        }
    }

    pub fn default_endpoint(self) -> &'static str {
        match self {
            Provider::OpenAiCompatible => "http://localhost:11434/v1",
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Provider::Claude => "https://api.anthropic.com/v1",
            Provider::OpenAi => "https://api.openai.com/v1",
        }
    }

    pub fn default_key_env(self) -> &'static str {
        match self {
            Provider::OpenAiCompatible => "",
            Provider::Gemini => "GEMINI_API_KEY",
            Provider::Claude => "ANTHROPIC_API_KEY",
            Provider::OpenAi => "OPENAI_API_KEY",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cfg {
    pub provider: Provider,
    pub model: String,
    pub endpoint: String,
    pub api_key_env: String,
}

impl Cfg {
    /// Empty `endpoint` / `api_key_env` fall back to the provider's defaults.
    pub fn new(provider: Provider, model: &str, endpoint: &str, api_key_env: &str) -> Cfg {
        let pick = |v: &str, d: &str| if v.trim().is_empty() { d.to_string() } else { v.trim().to_string() };
        Cfg {
            provider,
            model: model.trim().to_string(),
            endpoint: pick(endpoint, provider.default_endpoint()),
            api_key_env: pick(api_key_env, provider.default_key_env()),
        }
    }

    /// True when the request stays on this machine (no privacy notice needed).
    pub fn is_local(&self) -> bool {
        is_local_endpoint(&self.endpoint)
    }

    fn key_required(&self) -> bool {
        match self.provider {
            Provider::OpenAiCompatible => !self.is_local() && !self.api_key_env.is_empty(),
            _ => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub source_lang: String,
    pub target_lang: String,
    pub lines: Vec<String>,
}

/// `localhost`, `127.0.0.1` and `[::1]` (any port, any scheme).
pub fn is_local_endpoint(url: &str) -> bool {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit('@').next().unwrap_or(auth);
    let host = if let Some(v6) = auth.strip_prefix('[') {
        v6.split(']').next().map(|h| format!("[{h}]")).unwrap_or_default()
    } else {
        auth.split(':').next().unwrap_or("").to_string()
    };
    matches!(host.to_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

/// (system, user) prompts.
pub fn build_prompt(s: &Song) -> (String, String) {
    let system = format!(
        "You are an expert literary translator of song lyrics. Translate the whole song from {src} into {dst}.\n\
         Rules:\n\
         - Preserve the true meaning, tone and imagery of the original.\n\
         - Prefer natural, poetic phrasing in {dst} over a literal word-for-word rendering.\n\
         - Keep idioms meaningful: translate what they mean, not what the words say.\n\
         - Return exactly one translated line for every input line, in the same order. Never merge or split lines; keep blank or sound-only lines as they are.\n\
         - Output ONLY a JSON array of strings, with no commentary and no code fences.",
        src = s.source_lang,
        dst = s.target_lang
    );
    let mut user = format!("Title: {}\nArtist: {}\n", s.title, s.artist);
    if !s.album.trim().is_empty() {
        user.push_str(&format!("Album: {}\n", s.album));
    }
    if let Some(g) = s.genre.as_deref().filter(|g| !g.is_empty()) {
        user.push_str(&format!("Genre: {g}\n"));
    }
    if let Some(y) = s.year.as_deref().filter(|y| !y.is_empty()) {
        user.push_str(&format!("Year: {y}\n"));
    }
    user.push_str(&format!(
        "Source language: {}\nTarget language: {}\n\nThere are {} numbered lyric lines. Answer with a JSON array of exactly {} strings.\n\n",
        s.source_lang,
        s.target_lang,
        s.lines.len(),
        s.lines.len()
    ));
    for (i, l) in s.lines.iter().enumerate() {
        user.push_str(&format!("{}. {}\n", i + 1, l));
    }
    (system, user)
}

/// The first well-formed JSON array of `raw` (skipping prose, fences and stray `[tags]`).
pub fn extract_array(raw: &str) -> Option<Value> {
    let b = raw.as_bytes();
    for (start, _) in raw.match_indices('[') {
        let (mut depth, mut in_str, mut esc) = (0usize, false, false);
        for (i, &c) in b.iter().enumerate().skip(start) {
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                b'"' => in_str = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v @ Value::Array(_)) = serde_json::from_str::<Value>(&raw[start..=i]) {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// A JSON array of exactly `expected` strings, tolerating fences and surrounding prose.
pub fn parse_response(raw: &str, expected: usize) -> Result<Vec<String>, String> {
    let Some(Value::Array(a)) = extract_array(raw) else {
        return Err("the model did not return a JSON array".into());
    };
    let mut out = Vec::with_capacity(a.len());
    for v in &a {
        match v {
            Value::String(s) => out.push(s.clone()),
            _ => return Err("the model returned a non-string entry".into()),
        }
    }
    if out.len() != expected {
        return Err(format!("the model returned {} lines, expected {expected}", out.len()));
    }
    Ok(out)
}

fn base(cfg: &Cfg) -> String {
    cfg.endpoint.trim().trim_end_matches('/').to_string()
}

/// (url, description of the headers WITHOUT the secret, body) for OpenAI-style chat APIs.
pub fn request_openai(cfg: &Cfg, system: &str, user: &str) -> (String, String, Value) {
    let desc = if cfg.api_key_env.is_empty() {
        "Content-Type: application/json".to_string()
    } else {
        format!("Content-Type: application/json; Authorization: Bearer <${}>", cfg.api_key_env)
    };
    let body = json!({
        "model": cfg.model,
        "temperature": 0.3,
        "stream": false,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    });
    (format!("{}/chat/completions", base(cfg)), desc, body)
}

pub fn request_gemini(cfg: &Cfg, system: &str, user: &str) -> (String, String, Value) {
    let desc = format!("Content-Type: application/json; x-goog-api-key: <${}>", cfg.api_key_env);
    let body = json!({
        "systemInstruction": {"parts": [{"text": system}]},
        "contents": [{"role": "user", "parts": [{"text": user}]}],
        "generationConfig": {"temperature": 0.3, "responseMimeType": "application/json"}
    });
    (format!("{}/models/{}:generateContent", base(cfg), cfg.model), desc, body)
}

pub fn request_claude(cfg: &Cfg, system: &str, user: &str) -> (String, String, Value) {
    let desc = format!(
        "Content-Type: application/json; anthropic-version: 2023-06-01; x-api-key: <${}>",
        cfg.api_key_env
    );
    let body = json!({
        "model": cfg.model,
        "max_tokens": 8192,
        "temperature": 0.3,
        "system": system,
        "messages": [{"role": "user", "content": user}]
    });
    (format!("{}/messages", base(cfg)), desc, body)
}

/// The assistant text inside a provider reply.
fn reply_text(p: Provider, v: &Value) -> Option<String> {
    let t = match p {
        Provider::OpenAiCompatible | Provider::OpenAi => v.pointer("/choices/0/message/content"),
        Provider::Claude => v.pointer("/content/0/text"),
        Provider::Gemini => v.pointer("/candidates/0/content/parts/0/text"),
    };
    t.and_then(Value::as_str).map(str::to_string)
}

fn redact(s: &str, key: &str) -> String {
    if key.len() >= 4 { s.replace(key, "***") } else { s.to_string() }
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

async fn ask(c: &reqwest::Client, cfg: &Cfg, key: &str, system: &str, user: &str) -> Result<String, String> {
    let (url, _, body) = match cfg.provider {
        Provider::OpenAiCompatible | Provider::OpenAi => request_openai(cfg, system, user),
        Provider::Gemini => request_gemini(cfg, system, user),
        Provider::Claude => request_claude(cfg, system, user),
    };
    let mut req = c.post(&url).timeout(Duration::from_secs(TIMEOUT)).json(&body);
    if !key.is_empty() {
        req = match cfg.provider {
            Provider::Gemini => req.header("x-goog-api-key", key),
            Provider::Claude => req.header("x-api-key", key).header("anthropic-version", "2023-06-01"),
            _ => req.bearer_auth(key),
        };
    }
    let r = req.send().await.map_err(|e| {
        record_fail();
        redact(&format!("AI request failed ({})", e.without_url()), key)
    })?;
    let st = r.status();
    if !st.is_success() {
        record_fail();
        let detail = r
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.pointer("/error"))
                    .and_then(Value::as_str)
                    .map(|m| m.chars().take(160).collect::<String>())
            })
            .map(|m| format!(": {}", redact(&m, key)))
            .unwrap_or_default();
        return Err(format!("AI provider answered HTTP {}{detail}", st.as_u16()));
    }
    let v = r.json::<Value>().await.map_err(|_| {
        record_fail();
        "unreadable reply from the AI provider".to_string()
    })?;
    FAILS.store(0, Ordering::Relaxed);
    reply_text(cfg.provider, &v).ok_or_else(|| "the AI reply had no text".to_string())
}

/// Translate the whole song in one request; one retry if the answer is not a valid array of
/// the right length. `Err` means "use the standard engine". Returns the lines and a label.
pub async fn translate(c: &reqwest::Client, cfg: &Cfg, song: &Song) -> Result<(Vec<String>, String), String> {
    if song.lines.is_empty() {
        return Err("no lyrics to translate".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("no AI model configured".into());
    }
    let key = std::env::var(&cfg.api_key_env).unwrap_or_default();
    if cfg.key_required() && key.trim().is_empty() {
        return Err(format!("environment variable {} is not set", cfg.api_key_env));
    }
    let key = key.trim().to_string();
    if breaker_open() {
        return Err("AI translation paused after repeated failures".into());
    }
    let (system, user) = build_prompt(song);
    let mut last = String::new();
    for _ in 0..2 {
        let raw = ask(c, cfg, &key, &system, &user).await?;
        match parse_response(&raw, song.lines.len()) {
            Ok(lines) => return Ok((lines, label(cfg))),
            Err(e) => last = e,
        }
    }
    Err(redact(&last, &key))
}

/// Shown as the translation source, e.g. `claude claude-sonnet-5`.
pub fn label(cfg: &Cfg) -> String {
    format!("AI {} {}", cfg.provider.name(), cfg.model)
}

/// Cache key: track + target language + provider + model (+ the lyric text, so a different
/// lyrics version is not served the wrong translation).
pub fn cache_key(cfg: &Cfg, song: &Song) -> String {
    let lines = fnv1a(&song.lines.join("\n"));
    format!(
        "ai-{:016x}",
        fnv1a(&format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{lines:016x}",
            song.artist, song.title, song.album, song.target_lang, cfg.provider.name(), cfg.model
        ))
    )
}

pub async fn cached(cfg: &Cfg, song: &Song) -> Option<Vec<String>> {
    let txt = tokio::fs::read_to_string(cache_dir().join(cache_key(cfg, song))).await.ok()?;
    let v: Vec<String> = serde_json::from_str(&txt).ok()?;
    (v.len() == song.lines.len()).then_some(v)
}

pub async fn store(cfg: &Cfg, song: &Song, lines: &[String]) {
    if lines.len() != song.lines.len() {
        return;
    }
    if let Ok(j) = serde_json::to_string(lines) {
        let _ = tokio::fs::write(cache_dir().join(cache_key(cfg, song)), j).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Song {
        Song {
            title: "La Vie en Rose".into(),
            artist: "Edith Piaf".into(),
            album: "Best of".into(),
            genre: Some("Chanson".into()),
            year: Some("1947".into()),
            source_lang: "French".into(),
            target_lang: "English".into(),
            lines: vec!["Des yeux qui font baisser les miens".into(), "Un rire qui se perd".into()],
        }
    }

    fn cfg(p: Provider) -> Cfg {
        Cfg::new(p, "some-model", "", "")
    }

    #[test]
    fn prompt_contents() {
        let (sys, user) = build_prompt(&song());
        for w in ["meaning", "tone", "imagery", "poetic", "idioms", "JSON array", "exactly one", "French", "English"] {
            assert!(sys.contains(w), "{w}");
        }
        for w in ["La Vie en Rose", "Edith Piaf", "Best of", "Chanson", "1947", "1. Des yeux", "2. Un rire", "exactly 2"] {
            assert!(user.contains(w), "{w}");
        }
    }

    #[test]
    fn parse_variants() {
        assert_eq!(parse_response(r#"["a","b"]"#, 2).unwrap(), vec!["a", "b"]);
        assert_eq!(parse_response("```json\n[\"a\", \"b\"]\n```", 2).unwrap().len(), 2);
        assert_eq!(parse_response("Sure! Here it is: [\"a [x]\",\"b\"] Hope that helps", 2).unwrap()[0], "a [x]");
        assert_eq!(parse_response("[Verse] then [\"a\",\"b\"]", 2).unwrap().len(), 2);
        assert!(parse_response(r#"["a"]"#, 2).is_err());
        assert!(parse_response(r#"["a", 3]"#, 2).is_err());
        assert!(parse_response(r#"["a", null]"#, 2).is_err());
        assert!(parse_response("no json here", 1).is_err());
        assert!(parse_response("[\"a\",", 1).is_err());
        assert!(parse_response(r#"{"a":1}"#, 1).is_err());
    }

    #[test]
    fn request_shapes() {
        let (u, d, b) = request_openai(&cfg(Provider::OpenAi), "S", "U");
        assert_eq!(u, "https://api.openai.com/v1/chat/completions");
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "U");
        assert!(d.contains("OPENAI_API_KEY"));
        let (u, _, b) = request_claude(&cfg(Provider::Claude), "S", "U");
        assert_eq!(u, "https://api.anthropic.com/v1/messages");
        assert_eq!(b["system"], "S");
        assert!(b["max_tokens"].as_i64().unwrap() > 0);
        let (u, d, b) = request_gemini(&cfg(Provider::Gemini), "S", "U");
        assert!(u.ends_with("/models/some-model:generateContent"));
        assert!(!u.contains("key="));
        assert_eq!(b["contents"][0]["parts"][0]["text"], "U");
        assert!(d.contains("x-goog-api-key"));
        let local = cfg(Provider::OpenAiCompatible);
        assert_eq!(request_openai(&local, "S", "U").0, "http://localhost:11434/v1/chat/completions");
        assert!(!local.key_required());
    }

    #[test]
    fn key_never_in_debug_or_requests() {
        let secret = "sk-SECRET-12345";
        let c = cfg(Provider::Claude);
        let all = format!("{c:?}{:?}{:?}{:?}", request_claude(&c, "S", "U"), request_openai(&c, "S", "U"), request_gemini(&c, "S", "U"));
        assert!(!all.contains(secret));
        assert_eq!(redact(&format!("bad key {secret}"), secret), "bad key ***");
    }

    #[test]
    fn local_endpoints() {
        for u in ["http://localhost:11434/v1", "http://127.0.0.1:1234", "http://[::1]:8080/v1", "localhost"] {
            assert!(is_local_endpoint(u), "{u}");
        }
        for u in ["https://api.openai.com/v1", "http://localhost.evil.com/v1", "http://example.com/localhost", "http://user@evil.com"] {
            assert!(!is_local_endpoint(u), "{u}");
        }
    }

    #[test]
    fn provider_names_roundtrip() {
        for p in [Provider::OpenAiCompatible, Provider::Gemini, Provider::Claude, Provider::OpenAi] {
            assert_eq!(Provider::parse(p.name()), Some(p));
        }
        assert_eq!(Provider::parse("Ollama"), Some(Provider::OpenAiCompatible));
        assert_eq!(Provider::parse("nope"), None);
    }

    #[test]
    fn reply_extraction() {
        let v = json!({"choices": [{"message": {"content": "x"}}]});
        assert_eq!(reply_text(Provider::OpenAi, &v).unwrap(), "x");
        let v = json!({"content": [{"text": "y"}]});
        assert_eq!(reply_text(Provider::Claude, &v).unwrap(), "y");
        let v = json!({"candidates": [{"content": {"parts": [{"text": "z"}]}}]});
        assert_eq!(reply_text(Provider::Gemini, &v).unwrap(), "z");
    }

    #[test]
    fn cache_key_differs_per_model_and_language() {
        let s = song();
        let a = cfg(Provider::Claude);
        let mut b = a.clone();
        b.model = "other".into();
        assert_ne!(cache_key(&a, &s), cache_key(&b, &s));
        assert_ne!(cache_key(&a, &s), cache_key(&cfg(Provider::OpenAi), &s));
        let mut s2 = s.clone();
        s2.target_lang = "German".into();
        assert_ne!(cache_key(&a, &s), cache_key(&a, &s2));
        assert_eq!(cache_key(&a, &s), cache_key(&a.clone(), &s.clone()));
    }
}
