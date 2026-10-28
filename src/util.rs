use std::path::{Path, PathBuf};

/// Aggressive identity normalisation for track/album/artist strings: drops any
/// (...) / [...] group containing an edition noise word, trailing "- Remastered 2011"
/// style suffixes and stray trailing hyphens, then collapses case and whitespace.
pub fn normalize_track(s: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static BRACKET: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s*[\(\[][^\)\]]*\b(remaster(ed)?|re-?master|deluxe|anniversar\w*|live|version|edit|mono|stereo|bonus|demo|acoustic|explicit|radio|expanded|edition|reissue|special|collector\w*|super|single)\b[^\)\]]*[\)\]]").unwrap()
    });
    static DASH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s+[-–—]\s+.*\b(remaster(ed)?|re-?master|deluxe|anniversar\w*|live|version|edit|mono|stereo|single|bonus|demo|acoustic|radio|expanded|edition|reissue)\b.*$").unwrap()
    });
    static TRAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s\-–—]+$").unwrap());
    let orig = s;
    let s = BRACKET.replace_all(s, "");
    let s = DASH.replace(&s, "");
    let s = TRAIL.replace(&s, "");
    let fold = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let out = fold(&s);
    // "(Live)" or "---" as the whole title: keep it rather than collapse to nothing
    if out.is_empty() { fold(orig) } else { out }
}

/// The language or content a release is explicitly marked with in its title or album
/// ("(English Version)", "Japanese Ver.", "Instrumental"): such a release has different
/// lyrics from the original, so it is a different track however alike the names are.
pub fn version_marker(title: &str, album: &str) -> Option<String> {
    use regex::Regex;
    use std::sync::LazyLock;
    static LANG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(english|korean|japanese|chinese|mandarin|cantonese|spanish|french|german|italian|portuguese|russian|thai|vietnamese|indonesian|turkish|arabic|hindi)\s*(ver\.?|version)\b|\b(instrumental)\b").unwrap()
    });
    [title, album].iter().find_map(|s| {
        LANG.captures(s)
            .map(|c| c.get(1).or_else(|| c.get(3)).map_or("", |m| m.as_str()).to_lowercase())
    })
}

/// Terminal cell width of a char (0 for combining marks, 2 for CJK/Hangul/emoji).
pub fn cw(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Terminal cell width of a string.
pub fn width(s: &str) -> usize {
    // summed per char so it always agrees with how the wrappers measure
    s.chars().map(cw).sum()
}

/// Stable 64-bit FNV-1a — used for cache file names (std's hasher isn't stable across releases).
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The `file://` URL of a path: `/home/me/a.flac` → `file:///home/me/a.flac`,
/// `C:\Music\a.flac` → `file:///C:/Music/a.flac`.
pub fn file_url(path: &Path) -> String {
    let p = path.display().to_string().replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

/// The path in a `file://` URL (percent-decoded); `file:///C:/x` gives `C:/x`, not `/C:/x`.
pub fn path_from_url(url: &str) -> Option<PathBuf> {
    let p = percent_decode(url.strip_prefix("file://")?);
    let b = p.as_bytes();
    let drive = b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':';
    Some(PathBuf::from(if drive { &p[1..] } else { &p }))
}

/// `~/.cache/music_display`, created on demand.
pub fn cache_dir() -> PathBuf {
    let d = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("music_display");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// `/tmp/music_display` for downloaded cover art.
pub fn tmp_dir() -> PathBuf {
    let d = std::env::temp_dir().join("music_display");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Replace a leading $HOME with `~`.
pub fn shorten_home(p: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Decode %XX escapes in `file://` URLs.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `m:ss` (or `h:mm:ss`).
pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

static TESTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(cfg!(test));

/// True under `cargo test`: no real daemon, no saved rate-limit pause, no real token folder,
/// short retry sleeps. The library's own tests get it from `cfg!(test)`; the front end's tests
/// (a different crate, where that flag is off here) turn it on with [`enter_test_mode`].
pub fn testing() -> bool {
    TESTING.load(std::sync::atomic::Ordering::Relaxed)
}

#[doc(hidden)]
pub fn enter_test_mode() {
    TESTING.store(true, std::sync::atomic::Ordering::Relaxed);
}
