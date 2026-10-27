use crate::AppEvent;
use crate::player::{Loop, Snapshot, Status};
use crate::util::now_secs;
use reqwest::Method;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, Notify, mpsc::UnboundedSender};

const API: &str = "https://api.spotify.com/v1";
const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// The login catches Spotify's redirect here; add exactly [`redirect_uri`] to the app.
pub const PORT: u16 = 8897;
const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing \
        user-read-recently-played user-library-read user-library-modify user-follow-read \
        playlist-read-private playlist-read-collaborative playlist-modify-public playlist-modify-private";
/// Liked Songs: first N tracks (50 per request).
const LIKED_LIMIT: usize = 1000;
const LOGIN_TIMEOUT: u64 = 180;
/// Max page size development-mode apps get on /artists/{id}/albums since 2026.
const ARTIST_PAGE: usize = 10;
const ARTIST_ALBUMS: usize = 30;
const ARTIST_SINGLES: usize = 20;
/// `Snapshot::player` of playback reported by the API (no MPRIS player behind it).
pub const REMOTE: &str = "spotify-connect";

pub fn redirect_uri() -> String {
    format!("http://127.0.0.1:{PORT}/callback")
}

// ── storage ────────────────────────────────────────────────────────────

/// `~/.local/state/music_display/spotify` (the token lives here, mode 600).
#[cfg(not(test))]
pub fn dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("music_display")
        .join("spotify")
}

/// Tests never see (or delete) the real token.
#[cfg(test)]
pub fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("music_display-test-{}", std::process::id())).join("spotify")
}

fn read_json(name: &str) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(dir().join(name)).ok()?).ok()
}

/// Write through a temp file readable by the owner only, then rename over the old one.
fn write_private(name: &str, data: &Value) -> std::io::Result<()> {
    use std::io::Write;
    let d = dir();
    let tmp = d.join(format!("{name}.tmp"));
    #[cfg(unix)]
    let mut f = {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&d)?;
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        f
    };
    // the profile folder is already private to the user on Windows
    #[cfg(not(unix))]
    let mut f = {
        std::fs::create_dir_all(&d)?;
        std::fs::File::create(&tmp)?
    };
    f.write_all(data.to_string().as_bytes())?;
    #[cfg(windows)]
    let _ = std::fs::remove_file(d.join(name)); // rename does not replace on every Windows version
    std::fs::rename(tmp, d.join(name))
}

pub fn logged_in() -> bool {
    read_json("token.json").is_some_and(|t| !s(&t["refresh_token"]).is_empty())
}

pub fn logout() -> bool {
    std::fs::remove_file(dir().join("token.json")).is_ok()
}

// ── PKCE ───────────────────────────────────────────────────────────────

/// SHA-256 (FIPS 180-4), for the PKCE code challenge.
#[allow(clippy::needless_range_loop)]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Base64url without padding (RFC 7636).
pub fn b64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
        }
    }
    out
}

fn random(n: usize) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf)
}

// ── errors / token ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ApiError {
    pub msg: String,
    /// HTTP status (0 = no connection / local problem).
    pub status: u16,
}

impl ApiError {
    fn new(msg: impl Into<String>, status: u16) -> Self {
        ApiError { msg: msg.into(), status }
    }
}

type Res<T> = Result<T, ApiError>;

/// Spotify's error body: `{"error": {"message", "reason"}}`, or `{"error": "...", "error_description"}`
/// from the accounts service.
pub fn api_error(status: u16, body: &str) -> ApiError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let e = &v["error"];
    let msg = if e.is_object() {
        Some(s(&e["reason"])).filter(|r| !r.is_empty()).unwrap_or_else(|| s(&e["message"]))
    } else {
        Some(s(&v["error_description"])).filter(|r| !r.is_empty()).unwrap_or_else(|| s(e))
    };
    let msg = match msg.trim() {
        "" if body.trim().is_empty() => format!("HTTP {status}"),
        "" => body.chars().take(200).collect(),
        m => m.to_string(),
    };
    ApiError::new(msg, status)
}

/// What the footer says about a failed request: the cases a person can do something about
/// get a sentence, anything else is Spotify's own text.
/// A wait in words: `45 s`, `12 min`, `19 h 24 min`.
fn wait_text(secs: u64) -> String {
    match secs {
        0..120 => format!("{secs} s"),
        120..7200 => format!("{} min", secs.div_ceil(60)),
        _ => match secs.div_ceil(60) % 60 {
            0 => format!("{} h", secs.div_ceil(60) / 60),
            m => format!("{} h {m} min", secs.div_ceil(60) / 60),
        },
    }
}

/// How long to stay quiet after a 429: Spotify's `Retry-After` in full (at most a week), else
/// 5 s that double with every 429 in a row (`strikes`), up to 5 minutes.
fn pause_secs(retry_after: Option<u64>, strikes: u32) -> u64 {
    match retry_after {
        Some(n) => n.clamp(1, 7 * 86400),
        None => (5u64 << strikes.min(6)).min(300),
    }
}

/// The saved end of a pause (unix seconds) as time left from `now`, if it is still ahead.
fn pause_left(until: u64, now: u64) -> Option<Duration> {
    until.checked_sub(now).filter(|s| *s > 0).map(Duration::from_secs)
}

/// The pause a 429 set in an earlier run: it keeps counting while the app is closed, so
/// starting again does not ask Spotify before it said it is ready.
fn saved_pause() -> Option<std::time::Instant> {
    if cfg!(test) {
        return None; // the tests share one folder
    }
    let until = read_json("ratelimit.json")?["until"].as_u64()?;
    Some(std::time::Instant::now() + pause_left(until, now_secs())?)
}

pub fn explain(status: u16, msg: &str, retry_after: Option<u64>) -> String {
    match status {
        429 => match retry_after {
            Some(n) => format!("Spotify is rate limiting: try again in {}", wait_text(n)),
            None => "Spotify is rate limiting: try again in a moment".into(),
        },
        403 if msg.to_ascii_uppercase().contains("PREMIUM") => "Spotify Premium is needed for this".into(),
        403 => format!("Spotify refused it (403): {msg}"),
        404 if msg == "NO_ACTIVE_DEVICE" => "no active Spotify device: press d to pick one".into(),
        _ => msg.to_string(),
    }
}

async fn post_form(http: &reqwest::Client, fields: &[(&str, &str)]) -> Res<Value> {
    let r = http
        .post(TOKEN_URL)
        .form(fields)
        .send()
        .await
        .map_err(|e| ApiError::new(format!("no connection: {e}"), 0))?;
    let status = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    if status >= 400 {
        let e = api_error(status, &text);
        return Err(ApiError::new(format!("login failed: {}", e.msg), status));
    }
    serde_json::from_str(&text).map_err(|_| ApiError::new("login failed: bad answer", status))
}

/// Spotify may or may not rotate the refresh token; keep the old one if it didn't.
fn save_token(tok: &Value, old_refresh: &str, client_id: &str) -> Res<Value> {
    let refresh = Some(s(&tok["refresh_token"])).filter(|r| !r.is_empty()).unwrap_or_else(|| old_refresh.into());
    let v = json!({
        "access_token": s(&tok["access_token"]),
        "refresh_token": refresh,
        "expires_at": now_secs() + tok["expires_in"].as_u64().unwrap_or(3600).saturating_sub(60),
        "client_id": client_id,
    });
    write_private("token.json", &v).map_err(|e| ApiError::new(format!("can't save the token: {e}"), 0))?;
    Ok(v)
}

// ── login (--spotify-login) ────────────────────────────────────────────

/// What the login tells whoever started it (the command line prints it, the settings menu
/// shows it), so nothing is ever printed from inside the TUI.
pub enum LoginStep {
    /// The authorize link, always sent first.
    Url(String),
    /// Whether `xdg-open` could be started.
    Opened(bool),
}

/// Login progress for the app: a step, or the end.
pub enum LoginMsg {
    Step(LoginStep),
    Done(Result<(), String>),
}

/// Browser login with PKCE (no client secret): open the authorize page, catch the redirect
/// on [`PORT`], swap the code for tokens and save them.
pub async fn login(client_id: &str, say: &(dyn Fn(LoginStep) + Send + Sync)) -> anyhow::Result<()> {
    if client_id.trim().is_empty() {
        anyhow::bail!(
            "no client ID: create an app at https://developer.spotify.com/dashboard, add the redirect URI {} \
                 and pass its Client ID with --spotify-client-id (or spotify_client_id in the config)",
            redirect_uri()
        );
    }
    let verifier = b64url(&random(64)?);
    let challenge = b64url(&sha256(verifier.as_bytes()));
    let state = b64url(&random(16)?);
    let url = reqwest::Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", client_id),
            ("response_type", "code"),
            ("redirect_uri", &redirect_uri()),
            ("scope", SCOPES),
            ("code_challenge_method", "S256"),
            ("code_challenge", &challenge),
            ("state", &state),
        ],
    )?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT))
        .await
        .map_err(|e| anyhow::anyhow!("can't listen on 127.0.0.1:{PORT}: {e}"))?;
    say(LoginStep::Url(url.to_string()));
    #[cfg(windows)]
    let mut opener = {
        // the empty string is `start`'s window title; `&` in the URL is quoted by Command
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(not(windows))]
    let mut opener = std::process::Command::new("xdg-open");
    let opened = opener
        .arg(url.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok();
    say(LoginStep::Opened(opened));
    let code = match tokio::time::timeout(Duration::from_secs(LOGIN_TIMEOUT), wait_for_code(&listener, &state)).await {
        Ok(r) => r?,
        Err(_) => anyhow::bail!("login timed out"),
    };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).build()?;
    let tok = post_form(
        &http,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri()),
            ("client_id", client_id),
            ("code_verifier", &verifier),
        ],
    )
    .await
    .map_err(|e| anyhow::anyhow!(e.msg))?;
    save_token(&tok, "", client_id).map_err(|e| anyhow::anyhow!(e.msg))?;
    Ok(())
}

/// Answer requests on the login port until the one carrying our `state` arrives.
pub async fn wait_for_code(listener: &tokio::net::TcpListener, state: &str) -> anyhow::Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = req.split_whitespace().nth(1).unwrap_or("/");
        let Ok(u) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
            continue;
        };
        if u.path() != "/callback" {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
        let ours = q.get("state").map(String::as_str) == Some(state);
        let code = q.get("code").filter(|c| ours && !c.is_empty()).cloned();
        let body = if code.is_some() {
            "Logged in. You can close this tab."
        } else {
            "Login failed. Run music_display --spotify-login again."
        };
        let _ = sock
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        if ours {
            return code.ok_or_else(|| {
                anyhow::anyhow!("login {}", q.get("error").map_or("failed", String::as_str))
            });
        }
    }
}

// ── parsing (pure, tested) ─────────────────────────────────────────────

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn names(v: &Value) -> String {
    v.as_array()
        .map(|a| a.iter().map(|x| s(&x["name"])).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

pub fn uri_id(uri: &str) -> &str {
    uri.rsplit(':').next().unwrap_or(uri)
}

/// The smallest image at least `want` px wide (Spotify lists them largest first).
pub fn image(images: &Value, want: u64) -> String {
    let imgs = images.as_array().map(Vec::as_slice).unwrap_or(&[]);
    imgs.iter()
        .rfind(|i| i["width"].as_u64().unwrap_or(0) >= want)
        .or(imgs.first())
        .map(|i| s(&i["url"]))
        .unwrap_or_default()
}

/// The `spotify:track:` URI behind an MPRIS track id (`spotify:track:…`,
/// `/com/spotify/track/…`, spotifyd's `/spotify/track/…`).
pub fn track_uri(track_id: &str) -> Option<String> {
    if track_id.starts_with("spotify:track:") {
        return Some(track_id.to_string());
    }
    let (_, id) = track_id.rsplit_once("/track/")?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| format!("spotify:track:{id}"))
}

/// One row of the library browser.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// `track`, `album`, `playlist`, `artist` or `liked`.
    pub kind: String,
    pub uri: String,
    pub name: String,
    pub sub: String,
    /// Has rows of its own (an album, your own playlist, an artist, Liked Songs).
    pub openable: bool,
    /// Seconds (tracks only).
    pub duration: f64,
}

pub fn track_item(t: &Value) -> Option<Item> {
    let uri = s(&t["uri"]);
    if uri.is_empty() {
        return None;
    }
    Some(Item {
        kind: "track".into(),
        uri,
        name: s(&t["name"]),
        sub: names(&t["artists"]),
        openable: false,
        duration: t["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
    })
}

/// What `/me/player` says is playing, and where.
#[derive(Clone, Debug)]
pub struct Remote {
    pub snap: Snapshot,
    /// This PC plays it (the Spotify app or spotifyd).
    pub local: bool,
    pub device: String,
}

/// `/me/player` → a snapshot like the MPRIS ones; `None` when nothing is loaded.
pub fn parse_state(p: &Value, is_local: impl Fn(&str) -> bool) -> Option<Remote> {
    let t = &p["item"];
    if !t.is_object() {
        return None;
    }
    // tracks have an album; podcast episodes a show
    let show = &t["show"];
    let album = if t["album"].is_object() { &t["album"] } else { show };
    let artist = Some(names(&t["artists"])).filter(|a| !a.is_empty()).unwrap_or_else(|| s(&show["name"]));
    let images = if album["images"].is_array() { &album["images"] } else { &t["images"] };
    let num = |v: &Value| v.as_u64().map(|n| n as u32).filter(|n| *n > 0);
    let dev = &p["device"];
    let device = s(&dev["name"]);
    let snap = Snapshot {
        status: if p["is_playing"].as_bool() == Some(true) { Status::Playing } else { Status::Paused },
        position: p["progress_ms"].as_f64().unwrap_or(0.0) / 1000.0,
        title: s(&t["name"]),
        artist,
        album: s(&album["name"]),
        length: t["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
        art_url: image(images, 300),
        track_no: num(&t["track_number"]),
        disc_no: num(&t["disc_number"]),
        content_created: s(&album["release_date"]),
        volume: dev["volume_percent"].as_f64().map(|v| (v / 100.0).clamp(0.0, 1.0)),
        url: String::new(),
        track_id: s(&t["uri"]),
        player: REMOTE.into(),
        shuffle: p["shuffle_state"].as_bool(),
        repeat: Some(match p["repeat_state"].as_str() {
            Some("track") => Loop::Track,
            Some("context") => Loop::Playlist,
            _ => Loop::None,
        }),
    };
    Some(Remote { snap, local: is_local(&device), device })
}

/// Recently played → distinct (kind, context uri, a track from it), newest first. Plays without
/// a context count as their album.
pub fn recent_contexts(plays: &Value) -> Vec<(String, String, Value)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in plays["items"].as_array().into_iter().flatten() {
        let t = &p["track"];
        let (mut uri, mut kind) = (s(&p["context"]["uri"]), s(&p["context"]["type"]));
        if uri.is_empty() {
            (uri, kind) = (s(&t["album"]["uri"]), "album".into());
        }
        if uri.ends_with(":collection") {
            kind = "collection".into();
        }
        if uri.is_empty() || !seen.insert(uri.clone()) {
            continue;
        }
        out.push((kind, uri, t.clone()));
    }
    out
}

/// An artist's albums and singles, each newest first, with an "N albums · M singles" summary.
pub fn artist_rows(mut albums: Vec<Value>, mut singles: Vec<Value>) -> (String, Vec<Item>) {
    let date = |a: &Value| s(&a["release_date"]);
    albums.sort_by_key(|a| std::cmp::Reverse(date(a)));
    singles.sort_by_key(|a| std::cmp::Reverse(date(a)));
    let row = |a: &Value, label: &str| {
        let year: String = date(a).chars().take(4).collect();
        Item {
            kind: "album".into(),
            uri: s(&a["uri"]),
            name: s(&a["name"]),
            sub: if year.is_empty() { label.to_string() } else { format!("{year} · {label}") },
            openable: true,
            duration: 0.0,
        }
    };
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    let summary = format!("{} · {}", plural(albums.len(), "album"), plural(singles.len(), "single"));
    let items = albums.iter().map(|a| row(a, "Album")).chain(singles.iter().map(|a| row(a, "Single"))).collect();
    (summary, items)
}

/// One Spotify Connect device from `/me/player/devices`.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// Computer, Smartphone, Speaker…
    pub kind: String,
    pub active: bool,
    /// Spotify won't let apps control it.
    pub restricted: bool,
    /// The Spotify app on this computer, or spotifyd by the name given in the settings.
    pub local: bool,
}

pub fn parse_devices(v: &Value, is_local: impl Fn(&str) -> bool) -> Vec<Device> {
    v["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d.is_object())
        .map(|d| Device {
            id: s(&d["id"]),
            name: s(&d["name"]),
            kind: s(&d["type"]),
            active: d["is_active"].as_bool() == Some(true),
            restricted: d["is_restricted"].as_bool() == Some(true),
            local: is_local(&s(&d["name"])),
        })
        .collect()
}

/// `/search` → songs, albums, artists, playlists (in that order, each as Spotify ranks them).
/// `uid` decides which playlists are yours (only those can be opened).
pub fn parse_search(v: &Value, uid: &str) -> Vec<Item> {
    let list = |key: &str| v[key]["items"].as_array().cloned().unwrap_or_default();
    let mut out: Vec<Item> = list("tracks").iter().filter_map(track_item).collect();
    out.extend(list("albums").iter().filter(|a| a.is_object()).map(|a| Item {
        kind: "album".into(),
        uri: s(&a["uri"]),
        name: s(&a["name"]),
        sub: names(&a["artists"]),
        openable: true,
        duration: 0.0,
    }));
    out.extend(list("artists").iter().filter(|a| a.is_object()).map(|a| Item {
        kind: "artist".into(),
        uri: s(&a["uri"]),
        name: s(&a["name"]),
        sub: "Artist".into(),
        openable: true,
        duration: 0.0,
    }));
    // playlists may contain nulls (removed or private ones)
    out.extend(list("playlists").iter().filter(|p| p.is_object()).map(|p| {
        let own = !uid.is_empty() && s(&p["owner"]["id"]) == uid;
        let by = Some(s(&p["owner"]["display_name"])).filter(|n| !n.is_empty()).unwrap_or_else(|| "someone else".into());
        Item {
            kind: "playlist".into(),
            uri: s(&p["uri"]),
            name: s(&p["name"]),
            sub: if own { "Playlist".into() } else { format!("by {by}") },
            openable: own,
            duration: 0.0,
        }
    }));
    out.retain(|i| !i.uri.is_empty() && !i.name.is_empty());
    out
}

/// `/me/player/queue` → the song playing now (if any) followed by what is queued next.
/// The flag says whether the first row is the song playing now.
pub fn parse_queue(v: &Value) -> (Vec<Item>, bool) {
    let now = track_item(&v["currently_playing"]);
    let has_now = now.is_some();
    let mut out: Vec<Item> = now.into_iter().collect();
    out.extend(v["queue"].as_array().into_iter().flatten().filter_map(track_item));
    (out, has_now)
}

#[cfg(windows)]
fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

#[cfg(not(windows))]
fn hostname() -> String {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|h| h.trim().to_string())
        .unwrap_or_default()
}

// ── the API ────────────────────────────────────────────────────────────

/// A playback / library change, run by [`Api::run`].
#[derive(Clone, Debug)]
pub enum Cmd {
    Pause,
    /// Play; with nothing active anywhere, carry on here or start the last song.
    Resume,
    Next,
    Prev,
    Seek(f64),
    Shuffle(bool),
    Repeat(Loop),
    Volume(f64),
    /// A context (album / playlist / artist / `liked`), optionally starting at a track;
    /// or a single `spotify:track:` URI.
    Play { ctx: String, track: String },
    Transfer,
    /// Move playback to this device (id, name for the note).
    TransferTo { id: String, name: String },
    Queue(String),
    Like(String, bool),
    Add { playlist: String, name: String, track: String },
}

/// Results for the app, tagged with what asked for them.
pub enum Reply {
    Remote(Result<Option<Box<Remote>>, String>),
    /// A library tab's rows.
    Tab(usize, Result<Vec<Item>, String>),
    /// The rows of an opened album / playlist / artist: (summary, rows).
    Opened { uri: String, result: Result<(String, Vec<Item>), String> },
    Liked { uri: String, liked: bool },
    /// A [`Cmd`] finished: a note to show, or what went wrong.
    Done(Result<String, String>),
    /// The login started from the settings menu made progress or ended.
    Login(LoginMsg),
    /// The queue: rows, and whether the first one is the song playing now.
    Queue(Result<(Vec<Item>, bool), String>),
    /// A search finished (`q` is what was asked, so a slow old answer can be dropped).
    Search { q: String, result: Result<Vec<Item>, String> },
    /// The device list for the picker.
    Devices(Result<Vec<Device>, String>),
}

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    client_id: String,
    /// spotifyd's Spotify Connect name.
    daemon: String,
    host: String,
    /// The API root (a fake server in tests).
    base: String,
    token: Arc<Mutex<Option<Value>>>,
    me: Arc<Mutex<String>>,
    /// Poll `/me/player` again now (after a command).
    pub wake: Arc<Notify>,
    /// An MPRIS player is showing: poll the API more slowly.
    pub mpris: Arc<AtomicBool>,
    /// After a 429: no request leaves before this moment (Spotify's `Retry-After`).
    pause_until: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    /// 429s in a row without a `Retry-After`, to back off further each time.
    strikes: Arc<AtomicU32>,
}

/// How often `/me/player` is asked (ms): another device plays / nothing plays / an MPRIS
/// player is on screen. Position is worked out in between, and a command polls at once.
const POLL_PLAYING_MS: u64 = 3000;
const POLL_IDLE_MS: u64 = 5000;
const POLL_MPRIS_MS: u64 = 10_000;

impl Api {
    pub fn new(http: reqwest::Client, client_id: &str, daemon: &str) -> Self {
        Api {
            http,
            client_id: client_id.to_string(),
            daemon: daemon.to_string(),
            host: hostname(),
            base: API.to_string(),
            token: Arc::new(Mutex::new(None)),
            me: Arc::new(Mutex::new(String::new())),
            wake: Arc::new(Notify::new()),
            mpris: Arc::new(AtomicBool::new(false)),
            pause_until: Arc::new(std::sync::Mutex::new(saved_pause())),
            strikes: Arc::new(AtomicU32::new(0)),
        }
    }

    /// How much longer Spotify asked us to wait (after a 429), if it still does.
    pub fn paused_for(&self) -> Option<Duration> {
        let until = (*self.pause_until.lock().ok()?)?;
        until.checked_duration_since(std::time::Instant::now()).filter(|d| !d.is_zero())
    }

    /// Spotify said "slow down": nothing is sent for as long as it asked (see `pause_secs`),
    /// and the end is saved so a restart keeps waiting too.
    fn pause_for(&self, secs: Option<u64>) {
        let strikes = if secs.is_none() { self.strikes.fetch_add(1, Ordering::Relaxed) } else { 0 };
        let wait = Duration::from_secs(pause_secs(secs, strikes));
        if let Ok(mut p) = self.pause_until.lock() {
            let until = std::time::Instant::now() + wait;
            let until = p.map_or(until, |old| old.max(until));
            *p = Some(until);
            #[cfg(not(test))]
            {
                let end = now_secs() + until.saturating_duration_since(std::time::Instant::now()).as_secs();
                let _ = std::fs::create_dir_all(dir());
                let _ = std::fs::write(dir().join("ratelimit.json"), json!({ "until": end }).to_string());
            }
        }
    }

    /// Test hook: talk to a fake server at `base` with a token that never expires, so nothing
    /// reads the token file or reaches the real service.
    #[cfg(test)]
    pub fn with_fake_server(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self.token = Arc::new(Mutex::new(Some(
            json!({"access_token": "test", "refresh_token": "test", "expires_at": u64::MAX}),
        )));
        self
    }

    /// Test hook: pretend this computer has another host name.
    #[cfg(test)]
    pub fn with_host(mut self, host: &str) -> Self {
        self.host = host.to_string();
        self
    }

    /// The device is spotifyd, by the name it was given in the settings. Without one set, no
    /// device counts as spotifyd (an empty name must never match anything).
    fn is_daemon(&self, name: &str) -> bool {
        !self.daemon.is_empty() && name == self.daemon
    }

    /// The Spotify app on this computer (Spotify names it after the host) or spotifyd.
    pub fn is_local(&self, name: &str) -> bool {
        self.is_daemon(name) || (!self.host.is_empty() && name.eq_ignore_ascii_case(&self.host))
    }

    /// A valid access token; refreshed (one at a time) when it ran out.
    async fn access_token(&self) -> Res<String> {
        let mut tok = self.token.lock().await;
        if tok.is_none() {
            *tok = read_json("token.json");
        }
        let Some(t) = tok.as_ref().filter(|t| !s(&t["refresh_token"]).is_empty()) else {
            return Err(ApiError::new("not logged in: run music_display --spotify-login", 401));
        };
        if t["expires_at"].as_u64().unwrap_or(0) > now_secs() {
            return Ok(s(&t["access_token"]));
        }
        let refresh = s(&t["refresh_token"]);
        let cid = Some(s(&t["client_id"])).filter(|c| !c.is_empty()).unwrap_or_else(|| self.client_id.clone());
        let new = post_form(
            &self.http,
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &cid)],
        )
        .await?;
        let saved = save_token(&new, &refresh, &cid)?;
        let access = s(&saved["access_token"]);
        *tok = Some(saved);
        Ok(access)
    }

    /// One request; `None` for an empty answer. `path` may be a full `next` URL.
    async fn call(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> Res<Option<Value>> {
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{path}", self.base) };
        // inside a Retry-After window nothing is sent at all
        if let Some(left) = self.paused_for() {
            return Err(ApiError::new(explain(429, "", Some(left.as_secs() + 1)), 429));
        }
        let token = self.access_token().await?;
        let mut req = self.http.request(method.clone(), &url).bearer_auth(token).query(query);
        req = match body {
            Some(b) => req.json(&b),
            None if method == Method::PUT || method == Method::POST => req.header("Content-Length", "0"),
            None => req,
        };
        let r = req.send().await.map_err(|e| ApiError::new(format!("no connection: {e}"), 0))?;
        let status = r.status().as_u16();
        let retry_after = r
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let text = r.text().await.unwrap_or_default();
        if status >= 400 {
            if status == 429 {
                self.pause_for(retry_after);
            }
            let e = api_error(status, &text);
            return Err(ApiError::new(explain(status, &e.msg, retry_after), status));
        }
        self.strikes.store(0, Ordering::Relaxed);
        Ok(serde_json::from_str(&text).ok())
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Res<Value> {
        Ok(self.call(Method::GET, path, query, None).await?.unwrap_or(Value::Null))
    }

    /// Follow `next` links up to `limit` items; `key` digs into a wrapper (`/me/following`).
    async fn paged(&self, path: &str, query: &[(&str, String)], limit: usize, key: Option<&str>) -> Res<Vec<Value>> {
        let mut items = Vec::new();
        let mut page = self.get(path, query).await?;
        loop {
            if let Some(k) = key {
                page = page[k].take();
            }
            items.extend(page["items"].as_array().into_iter().flatten().cloned());
            if items.len() >= limit {
                items.truncate(limit);
                break;
            }
            match page["next"].as_str() {
                Some(n) if !n.is_empty() => {
                    let n = n.to_string();
                    page = self.get(&n, &[]).await?;
                }
                _ => break,
            }
        }
        Ok(items)
    }

    async fn me_id(&self) -> Res<String> {
        let mut me = self.me.lock().await;
        if me.is_empty() {
            *me = s(&self.get("/me", &[]).await?["id"]);
        }
        Ok(me.clone())
    }

    pub async fn state(&self) -> Res<Option<Remote>> {
        let p = self.get("/me/player", &[("additional_types", "track,episode".into())]).await?;
        Ok(parse_state(&p, |n| self.is_local(n)))
    }

    pub async fn liked(&self, uri: &str) -> Res<bool> {
        let r = self.get("/me/library/contains", &[("uris", uri.into())]).await?;
        Ok(r[0].as_bool() == Some(true))
    }

    pub async fn recent(&self) -> Res<Vec<Item>> {
        let plays = self.get("/me/player/recently-played", &[("limit", "50".into())]).await?;
        let mut out = Vec::new();
        for (kind, uri, t) in recent_contexts(&plays) {
            let one = |name: String, sub: &str, openable: bool| Item {
                kind: kind.clone(),
                uri: uri.clone(),
                name,
                sub: sub.into(),
                openable,
                duration: 0.0,
            };
            let item = match kind.as_str() {
                "album" => {
                    let name = if s(&t["album"]["uri"]) == uri {
                        s(&t["album"]["name"])
                    } else {
                        self.get(&format!("/albums/{}", uri_id(&uri)), &[]).await.map(|a| s(&a["name"])).unwrap_or_default()
                    };
                    one(name, "Album", true)
                }
                "playlist" => {
                    let Ok(pl) = self
                        .get(&format!("/playlists/{}", uri_id(&uri)), &[("fields", "name,owner(id)".into())])
                        .await
                    else {
                        continue;
                    };
                    let own = s(&pl["owner"]["id"]) == self.me_id().await.unwrap_or_default();
                    one(s(&pl["name"]), "Playlist", own)
                }
                "artist" => {
                    let name = self.get(&format!("/artists/{}", uri_id(&uri)), &[]).await.map(|a| s(&a["name"])).unwrap_or_default();
                    one(name, "Artist", true)
                }
                "collection" => Item {
                    kind: "liked".into(),
                    uri: format!("spotify:user:{}:collection", self.me_id().await?),
                    name: "Liked Songs".into(),
                    sub: "Playlist".into(),
                    openable: true,
                    duration: 0.0,
                },
                _ => continue,
            };
            if !item.name.is_empty() {
                out.push(item);
            }
            if out.len() == 8 {
                break;
            }
        }
        Ok(out)
    }

    pub async fn playlists(&self) -> Res<Vec<Item>> {
        let uid = self.me_id().await?;
        let lists = self.paged("/me/playlists", &[("limit", "50".into())], usize::MAX, None).await?;
        Ok(lists
            .iter()
            .filter(|p| p.is_object())
            .map(|p| {
                let own = s(&p["owner"]["id"]) == uid || p["collaborative"].as_bool() == Some(true);
                let count = p["items"]["total"].as_u64().or(p["tracks"]["total"].as_u64()).unwrap_or(0);
                let by = Some(s(&p["owner"]["display_name"])).filter(|n| !n.is_empty()).unwrap_or_else(|| "someone else".into());
                Item {
                    kind: "playlist".into(),
                    uri: s(&p["uri"]),
                    name: s(&p["name"]),
                    sub: if own { format!("{count} songs") } else { format!("by {by}") },
                    openable: own,
                    duration: 0.0,
                }
            })
            .collect())
    }

    pub async fn artists(&self) -> Res<Vec<Item>> {
        let q = [("type", "artist".to_string()), ("limit", "50".into())];
        let list = self.paged("/me/following", &q, usize::MAX, Some("artists")).await?;
        Ok(list
            .iter()
            .map(|a| Item { kind: "artist".into(), uri: s(&a["uri"]), name: s(&a["name"]), sub: "Artist".into(), openable: true, duration: 0.0 })
            .collect())
    }

    pub async fn albums(&self) -> Res<Vec<Item>> {
        let list = self.paged("/me/albums", &[("limit", "50".into())], usize::MAX, None).await?;
        Ok(list
            .iter()
            .map(|x| {
                let a = &x["album"];
                Item { kind: "album".into(), uri: s(&a["uri"]), name: s(&a["name"]), sub: names(&a["artists"]), openable: true, duration: 0.0 }
            })
            .collect())
    }

    pub async fn liked_tracks(&self) -> Res<Vec<Item>> {
        let list = self.paged("/me/tracks", &[("limit", "50".into())], LIKED_LIMIT, None).await?;
        Ok(list.iter().filter_map(|x| track_item(&x["track"])).collect())
    }

    /// The rows inside an album, your own playlist, an artist or Liked Songs.
    pub async fn open(&self, uri: &str) -> Res<(String, Vec<Item>)> {
        let id = uri_id(uri);
        let q = [("limit", "50".to_string())];
        match uri.split(':').nth(1).unwrap_or("") {
            "album" => {
                let tracks = self.paged(&format!("/albums/{id}/tracks"), &q, usize::MAX, None).await?;
                Ok((String::new(), tracks.iter().filter_map(track_item).collect()))
            }
            "playlist" => {
                let items = self.paged(&format!("/playlists/{id}/items"), &q, usize::MAX, None).await?;
                let rows = items
                    .iter()
                    .filter_map(|i| track_item(if i["item"].is_object() { &i["item"] } else { &i["track"] }))
                    .collect();
                Ok((String::new(), rows))
            }
            "artist" => {
                // albums and singles separately, so a long album list can't hide the latest singles
                let group = |g: &'static str, cap: usize| {
                    let q = [("include_groups", g.to_string()), ("limit", ARTIST_PAGE.to_string())];
                    async move { self.paged(&format!("/artists/{id}/albums"), &q, cap, None).await }
                };
                let albums = group("album", ARTIST_ALBUMS).await?;
                let singles = group("single", ARTIST_SINGLES).await?;
                Ok(artist_rows(albums, singles))
            }
            _ if uri == "liked" || uri.ends_with(":collection") => Ok((String::new(), self.liked_tracks().await?)),
            _ => Err(ApiError::new(format!("can't open {uri}"), 400)),
        }
    }

    async fn devices(&self) -> Res<Vec<Value>> {
        Ok(self.get("/me/player/devices", &[]).await?["devices"].as_array().cloned().unwrap_or_default())
    }

    /// The devices for the picker.
    pub async fn devices_list(&self) -> Res<Vec<Device>> {
        let v = self.get("/me/player/devices", &[]).await?;
        Ok(parse_devices(&v, |n| self.is_local(n)))
    }

    /// Songs, albums, artists and playlists matching `q` (10 of each: the most a development-mode
    /// app gets).
    pub async fn search(&self, q: &str) -> Res<Vec<Item>> {
        let query = [
            ("q", q.to_string()),
            ("type", "track,album,artist,playlist".into()),
            ("limit", "10".into()),
        ];
        let v = self.get("/search", &query).await?;
        // only needed to tell your playlists from the rest; a failure just makes none of them yours
        let uid = self.me_id().await.unwrap_or_default();
        Ok(parse_search(&v, &uid))
    }

    /// Now playing and next up (the first row is the song playing when the flag is set).
    pub async fn queue_list(&self) -> Res<(Vec<Item>, bool)> {
        Ok(parse_queue(&self.get("/me/player/queue", &[]).await?))
    }

    /// Move playback to one device and keep playing.
    async fn transfer_to(&self, id: &str) -> Res<()> {
        self.call(Method::PUT, "/me/player", &[], Some(json!({"device_ids": [id], "play": true}))).await?;
        Ok(())
    }

    /// Where "this PC" plays: the Spotify app when it is open (named after the host, or any
    /// device of type "Computer"), else spotifyd when its name is set.
    pub fn local_device<'a>(&self, devs: &'a [Value]) -> Option<&'a Value> {
        let name = |d: &Value| s(&d["name"]);
        devs.iter()
            .find(|d| !self.is_daemon(&name(d)) && self.is_local(&name(d)))
            .or_else(|| devs.iter().find(|d| s(&d["type"]) == "Computer" && !self.is_daemon(&name(d))))
            .or_else(|| devs.iter().find(|d| self.is_daemon(&name(d))))
    }

    /// Nothing active: pick where to play (this PC, else any device).
    async fn ensure_device(&self) -> Res<Option<String>> {
        let devs = self.devices().await?;
        if devs.iter().any(|d| d["is_active"].as_bool() == Some(true)) {
            return Ok(None);
        }
        if devs.is_empty() {
            return Err(ApiError::new("no Spotify device: start spotifyd or open Spotify", 404));
        }
        Ok(self.local_device(&devs).or(devs.first()).map(|d| s(&d["id"])))
    }

    /// A player command; on 404 (no active device) pick one and try again there.
    async fn player(&self, m: Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> Res<()> {
        match self.call(m.clone(), path, query, body.clone()).await {
            Err(e) if e.status == 404 => {
                let Some(dev) = self.ensure_device().await? else { return Err(e) };
                let mut q = query.to_vec();
                q.push(("device_id", dev));
                self.call(m, path, &q, body).await.map(|_| ())
            }
            r => r.map(|_| ()),
        }
    }

    /// Spotify Connect transfer to this PC: the same song carries on where it was.
    async fn transfer_here(&self) -> Res<String> {
        let devs = self.devices().await?;
        let Some(dev) = self.local_device(&devs).map(|d| s(&d["id"])) else {
            return Err(ApiError::new("this PC isn't a Spotify device: start spotifyd or open Spotify", 404));
        };
        self.transfer_to(&dev).await?;
        Ok(dev)
    }

    /// Play: resume where it is; with nothing active anywhere, carry on here from where Spotify
    /// last stopped, else start the last played song in its playlist / album.
    async fn resume(&self) -> Res<String> {
        match self.call(Method::PUT, "/me/player/play", &[], None).await {
            Err(e) if e.status == 404 => {}
            r => return r.map(|_| String::new()),
        }
        let dev = self.transfer_here().await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if self.get("/me/player", &[]).await?["is_playing"].as_bool() == Some(true) {
            return Ok("Playing on this PC".into());
        }
        let last = self.get("/me/player/recently-played", &[("limit", "1".into())]).await?;
        let (track, ctx) = (s(&last["items"][0]["track"]["uri"]), s(&last["items"][0]["context"]["uri"]));
        if track.is_empty() {
            return Ok("Playing on this PC".into());
        }
        let body = if ctx.is_empty() {
            json!({"uris": [track]})
        } else {
            json!({"context_uri": ctx, "offset": {"uri": track}})
        };
        self.call(Method::PUT, "/me/player/play", &[("device_id", dev)], Some(body)).await?;
        Ok("Playing the last song".into())
    }

    pub async fn run(&self, cmd: Cmd) -> Res<String> {
        let p = "/me/player";
        match cmd {
            Cmd::Pause => self.player(Method::PUT, &format!("{p}/pause"), &[], None).await?,
            Cmd::Resume => return self.resume().await,
            Cmd::Next => self.player(Method::POST, &format!("{p}/next"), &[], None).await?,
            Cmd::Prev => self.player(Method::POST, &format!("{p}/previous"), &[], None).await?,
            Cmd::Seek(secs) => {
                let ms = ((secs.max(0.0)) * 1000.0).round() as u64;
                self.player(Method::PUT, &format!("{p}/seek"), &[("position_ms", ms.to_string())], None).await?
            }
            Cmd::Shuffle(on) => self.player(Method::PUT, &format!("{p}/shuffle"), &[("state", on.to_string())], None).await?,
            Cmd::Repeat(r) => {
                let state = match r {
                    Loop::None => "off",
                    Loop::Playlist => "context",
                    Loop::Track => "track",
                };
                self.player(Method::PUT, &format!("{p}/repeat"), &[("state", state.into())], None).await?
            }
            Cmd::Volume(v) => {
                let pct = (v.clamp(0.0, 1.0) * 100.0).round() as u32;
                self.player(Method::PUT, &format!("{p}/volume"), &[("volume_percent", pct.to_string())], None).await?
            }
            Cmd::Play { ctx, track } => {
                let ctx = if ctx == "liked" { format!("spotify:user:{}:collection", self.me_id().await?) } else { ctx };
                let body = if ctx.starts_with("spotify:track:") {
                    Some(json!({"uris": [ctx]}))
                } else if !ctx.is_empty() && !track.is_empty() {
                    Some(json!({"context_uri": ctx, "offset": {"uri": track}}))
                } else if !ctx.is_empty() {
                    Some(json!({"context_uri": ctx}))
                } else if !track.is_empty() {
                    Some(json!({"uris": [track]}))
                } else {
                    None
                };
                self.player(Method::PUT, &format!("{p}/play"), &[], body).await?
            }
            Cmd::Transfer => {
                self.transfer_here().await?;
                return Ok("Playing on this PC".into());
            }
            Cmd::TransferTo { id, name } => {
                self.transfer_to(&id).await?;
                return Ok(format!("Playing on {name}"));
            }
            Cmd::Queue(uri) => {
                self.player(Method::POST, &format!("{p}/queue"), &[("uri", uri)], None).await?;
                return Ok("Added to the queue".into());
            }
            Cmd::Like(uri, on) => {
                let m = if on { Method::PUT } else { Method::DELETE };
                self.call(m, "/me/library", &[("uris", uri)], None).await?;
                return Ok(if on { "Added to Liked Songs" } else { "Removed from Liked Songs" }.into());
            }
            Cmd::Add { playlist, name, track } => {
                let path = format!("/playlists/{}/items", uri_id(&playlist));
                self.call(Method::POST, &path, &[], Some(json!({"uris": [track]}))).await?;
                return Ok(format!("Added to {name}"));
            }
        }
        Ok(String::new())
    }

    /// `(name, active)` of the devices Spotify lists for you (used by `--setup`).
    pub async fn device_names(&self) -> Res<Vec<(String, bool)>> {
        Ok(crate::setup::parse_devices(&json!({ "devices": self.devices().await? })))
    }

    /// Keep `/me/player` coming: every 3 s while another device plays, slower while paused
    /// or while an MPRIS player is on screen; at once after a command (`wake`).
    pub async fn remote_loop(self, tx: UnboundedSender<AppEvent>) {
        loop {
            let r = self.state().await;
            let mpris = self.mpris.load(Ordering::Relaxed);
            let wait = match &r {
                Err(e) if e.status == 401 => 30_000,
                _ if mpris => POLL_MPRIS_MS,
                Ok(Some(x)) if x.snap.status == Status::Playing => POLL_PLAYING_MS,
                _ => POLL_IDLE_MS,
            };
            // never ask again before Spotify's Retry-After has run out
            let wait = wait.max(self.paused_for().map_or(0, |d| d.as_millis() as u64 + 100));
            if tx.send(AppEvent::Spotify(Reply::Remote(r.map(|o| o.map(Box::new)).map_err(|e| e.msg)))).is_err() {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                _ = self.wake.notified() => {}
            }
        }
    }
}
