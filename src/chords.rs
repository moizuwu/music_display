use std::path::{Path, PathBuf};

/// One line of a chord sheet. `chords` are `(char column in text, chord name)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetLine {
    pub text: String,
    pub chords: Vec<(usize, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    /// Where it came from (file name), for the footer.
    pub source: String,
    pub capo: u8,
    /// Key of the chords *as written* (the shapes), e.g. "Am". From `{key: Am}`, a
    /// `Key: Am` line, or Ultimate Guitar's `tonality_name`.
    pub key: Option<String>,
    /// From `{tempo: 83}` / `BPM: 83` / the page metadata when the source has it.
    pub bpm: Option<u32>,
    /// The song's original guitar tuning ("Standard", "Drop D", "Half Step Down"…),
    /// from `{tuning: …}` or Ultimate Guitar's page metadata.
    pub tuning: Option<String>,
    pub lines: Vec<SheetLine>,
}

impl Sheet {
    pub fn chord_count(&self) -> usize {
        self.lines.iter().map(|l| l.chords.len()).sum()
    }
    /// Flat keys (Bb, Eb…) in the sheet → show transposed chords with flats too.
    pub fn prefers_flats(&self) -> bool {
        self.lines
            .iter()
            .flat_map(|l| l.chords.iter())
            .any(|(_, c)| {
                let b = c.as_bytes();
                b.len() > 1 && b[1] == b'b' && b[0].is_ascii_uppercase()
            })
    }
}

// ── chord recognition ──────────────────────────────────────────────────
fn root_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.is_empty() || !(b'A'..=b'G').contains(&b[0]) {
        return None;
    }
    Some(if b.len() > 1 && (b[1] == b'#' || b[1] == b'b') {
        2
    } else {
        1
    })
}

/// `Am7`, `F#m`, `Bb/D`, `Gsus4`, `Cadd9`, `E7b9`, `D(add9)`, `N.C.` is *not* a chord.
pub fn is_chord(tok: &str) -> bool {
    let (main, bass) = match tok.split_once('/') {
        Some((m, b)) => (m, Some(b)),
        None => (tok, None),
    };
    if let Some(b) = bass {
        if root_len(b) != Some(b.len()) {
            return false;
        }
    }
    let Some(n) = root_len(main) else { return false };
    let mut rest = &main[n..];
    const WORDS: [&str; 8] = ["maj", "min", "dim", "aug", "sus", "add", "no", "M"];
    while !rest.is_empty() {
        if let Some(w) = WORDS.iter().find(|w| rest.starts_with(**w)) {
            rest = &rest[w.len()..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        if c.is_ascii_digit() || matches!(c, 'm' | '+' | '-' | '°' | 'ø' | '#' | 'b' | '(' | ')' | ',')
        {
            rest = &rest[c.len_utf8()..];
        } else {
            return false;
        }
    }
    true
}

/// Shift one chord by `n` semitones (root and bass note).
pub fn transpose(chord: &str, n: i32, flats: bool) -> String {
    if n.rem_euclid(12) == 0 {
        return chord.to_string();
    }
    let shift = |note: &str| -> String {
        const SHARP: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        const FLAT: [&str; 12] = [
            "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
        ];
        let base = match note.as_bytes()[0] {
            b'C' => 0,
            b'D' => 2,
            b'E' => 4,
            b'F' => 5,
            b'G' => 7,
            b'A' => 9,
            _ => 11,
        };
        let acc = match note.as_bytes().get(1) {
            Some(b'#') => 1,
            Some(b'b') => -1,
            _ => 0,
        };
        let i = (base + acc + n).rem_euclid(12) as usize;
        (if flats { FLAT[i] } else { SHARP[i] }).to_string()
    };
    let (main, bass) = match chord.split_once('/') {
        Some((m, b)) => (m, Some(b)),
        None => (chord, None),
    };
    let Some(r) = root_len(main) else {
        return chord.to_string();
    };
    let mut out = shift(&main[..r]);
    out.push_str(&main[r..]);
    if let Some(b) = bass {
        out.push('/');
        out.push_str(&if root_len(b).is_some() { shift(b) } else { b.to_string() });
    }
    out
}

/// Semitones to shift the sheet's chord shapes: a higher capo means lower shapes.
/// `transpose` is the manual `t`/`T` shift on top.
pub fn capo_shift(sheet_capo: u8, effective: u8, transpose: i32) -> i32 {
    sheet_capo as i32 - effective as i32 + transpose
}

// ── keys, Camelot wheel ────────────────────────────────────────────────
/// A musical key: pitch class of the tonic (0 = C) and mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub root: u8,
    pub minor: bool,
}

const MAJOR_NAMES: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
const MINOR_NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "G#", "A", "Bb", "B"];

fn pitch_class(note: &str) -> Option<u8> {
    let b = note.as_bytes();
    let base: i32 = match b.first()? {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let acc = match b.get(1) {
        Some(b'#') => 1,
        Some(b'b') => -1,
        _ => 0,
    };
    Some((base + acc).rem_euclid(12) as u8)
}

/// "Am", "F#", "Bbm", "E minor", "D Major" → key.
pub fn parse_key(s: &str) -> Option<KeyInfo> {
    let s = s.trim();
    let n = root_len(s)?;
    let root = pitch_class(&s[..n])?;
    let rest = s[n..].trim().to_lowercase();
    let minor = match rest.as_str() {
        "" | "maj" | "major" => false,
        "m" | "min" | "minor" => true,
        r if r.starts_with('m') && !r.starts_with("maj") => true, // Am7 etc.
        _ => return None,
    };
    Some(KeyInfo { root, minor })
}

/// Best guess at the key of the shapes from the chords alone (diatonic fit, then the
/// tonic that is played first / last). Used when the source states no key.
pub fn estimate_key(sheet: &Sheet) -> Option<String> {
    // (pitch class, minor) of each chord, in song order
    let seq: Vec<(u8, bool)> = sheet
        .lines
        .iter()
        .flat_map(|l| l.chords.iter())
        .filter_map(|(_, name)| {
            let main = name.split('/').next()?;
            let n = root_len(main)?;
            let root = pitch_class(&main[..n])?;
            let rest = &main[n..];
            Some((root, rest.starts_with('m') && !rest.starts_with("maj")))
        })
        .collect();
    if seq.len() < 4 {
        return None;
    }
    let count = |root: u8, minor: bool| seq.iter().filter(|c| **c == (root, minor)).count() as i32;
    let (first, last) = (seq[0], seq[seq.len() - 1]);
    // major-key fit: I ii iii IV V vi
    let fit = |r: u8| -> i32 {
        [(0, false), (2, true), (4, true), (5, false), (7, false), (9, true)]
            .iter()
            .map(|(d, m)| count((r + d) % 12, *m))
            .sum::<i32>()
    };
    let best = (0..12u8).max_by_key(|r| {
        let bonus = [first, last].iter().filter(|c| **c == (*r, false) || **c == ((*r + 9) % 12, true)).count() as i32;
        fit(*r) * 4 + bonus
    })?;
    let rel = (best + 9) % 12;
    let major_score = count(best, false) * 2 + (last == (best, false)) as i32 * 3 + (first == (best, false)) as i32;
    let minor_score = count(rel, true) * 2 + (last == (rel, true)) as i32 * 3 + (first == (rel, true)) as i32;
    let k = if minor_score > major_score { KeyInfo { root: rel, minor: true } } else { KeyInfo { root: best, minor: false } };
    Some(k.short())
}

impl KeyInfo {
    pub fn shift(self, n: i32) -> KeyInfo {
        KeyInfo {
            root: (self.root as i32 + n).rem_euclid(12) as u8,
            ..self
        }
    }
    /// "Am", "Bb".
    pub fn short(self) -> String {
        let names = if self.minor { &MINOR_NAMES } else { &MAJOR_NAMES };
        format!("{}{}", names[self.root as usize], if self.minor { "m" } else { "" })
    }
    /// "E Minor".
    pub fn long(self) -> String {
        let names = if self.minor { &MINOR_NAMES } else { &MAJOR_NAMES };
        format!("{} {}", names[self.root as usize], if self.minor { "Minor" } else { "Major" })
    }
    /// Camelot wheel code: "9A" = E minor, "8B" = C major.
    pub fn camelot(self) -> String {
        // minor keys share the number of their relative major
        let major_pc = if self.minor { (self.root + 3) % 12 } else { self.root } as u32;
        let n = (major_pc * 7 + 7) % 12 + 1;
        format!("{n}{}", if self.minor { 'A' } else { 'B' })
    }
    /// Flat names (Bb, Eb …) read better in F, Bb, Eb, Ab, Db major and their relatives.
    pub fn prefers_flats(self) -> bool {
        let major_pc = if self.minor { (self.root + 3) % 12 } else { self.root };
        matches!(major_pc, 5 | 10 | 3 | 8 | 1)
    }
}

// ── parsing ────────────────────────────────────────────────────────────
fn strip_tags(s: &str) -> String {
    s.replace("[ch]", "")
        .replace("[/ch]", "")
        .replace("[tab]", "")
        .replace("[/tab]", "")
        .replace('\r', "")
}

fn parse_capo(line: &str) -> Option<u8> {
    let l = line.trim().to_lowercase();
    let rest = l.strip_prefix("capo")?;
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok().filter(|n| *n <= 12)
}

/// `[Verse 1]`, `Chorus:`, `Intro (2x)` — but not a lyric like "Bridge over troubled water".
fn is_section(line: &str) -> bool {
    let t = line
        .trim()
        .trim_matches(|c| c == '[' || c == ']' || c == '(' || c == ')')
        .trim_end_matches(':')
        .to_lowercase();
    if t.is_empty() || t.chars().count() > 32 {
        return false;
    }
    const KINDS: [&str; 14] = [
        "verse",
        "chorus",
        "bridge",
        "intro",
        "outro",
        "pre-chorus",
        "prechorus",
        "interlude",
        "solo",
        "instrumental",
        "refrain",
        "hook",
        "coda",
        "tag",
    ];
    let mut it = t.split_whitespace();
    let Some(first) = it.next() else { return false };
    if !KINDS.contains(&first.trim_end_matches(':')) {
        return false;
    }
    it.all(|w| {
        w.chars().any(|c| c.is_ascii_digit()) || w == "x" || w.starts_with('(') || w == "play"
    })
}

/// `key` / `tonality` / `bpm` / `tempo` metadata; `true` if it was one.
fn apply_meta(sheet: &mut Sheet, k: &str, v: &str) -> bool {
    match k.to_lowercase().as_str() {
        "key" | "tonality" if parse_key(v).is_some() => {
            sheet.key = Some(v.to_string());
            true
        }
        "tuning" if !v.trim().is_empty() && v.len() < 40 => {
            sheet.tuning = Some(v.trim().to_string());
            true
        }
        "bpm" | "tempo" => match v.split_whitespace().next().and_then(|n| n.parse::<u32>().ok()) {
            Some(n) if (30..=300).contains(&n) => {
                sheet.bpm = Some(n);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

fn is_tab_line(line: &str) -> bool {
    line.contains("--") && line.contains('|')
}

fn tokens_with_pos(line: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut cur = String::new();
    for (i, c) in line.chars().chain(std::iter::once(' ')).enumerate() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, std::mem::take(&mut cur)));
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            cur.push(c);
        }
    }
    out
}

fn chord_tokens(line: &str) -> Option<Vec<(usize, String)>> {
    let toks = tokens_with_pos(line);
    // `|` bar lines and `x2` repeat marks may sit among the chords.
    let toks: Vec<_> = toks
        .into_iter()
        .filter(|(_, t)| !matches!(t.as_str(), "|" | "||" | "-" | "/" | "%") && !is_repeat(t))
        .collect();
    (!toks.is_empty() && toks.iter().all(|(_, t)| is_chord(t))).then_some(toks)
}

fn is_repeat(t: &str) -> bool {
    let t = t.to_lowercase();
    let t = t.trim_matches(|c| c == '(' || c == ')');
    (t.starts_with('x') && t[1..].chars().all(|c| c.is_ascii_digit()) && t.len() > 1)
        || (t.ends_with('x') && t[..t.len() - 1].chars().all(|c| c.is_ascii_digit()) && t.len() > 1)
}

/// Chords written above the words they belong to (the common plain-text format).
pub fn parse_text(src: &str) -> Sheet {
    let src = strip_tags(src);
    let lines: Vec<&str> = src.lines().map(str::trim_end).collect();
    let mut sheet = Sheet::default();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.trim().is_empty() || is_section(line) || is_tab_line(line) {
            continue;
        }
        if let Some(c) = parse_capo(line) {
            sheet.capo = c;
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if apply_meta(&mut sheet, k.trim(), v.trim()) {
                continue;
            }
        }
        if let Some(chords) = chord_tokens(line) {
            let next = lines.get(i).copied().unwrap_or("");
            let usable = !next.trim().is_empty()
                && !is_section(next)
                && !is_tab_line(next)
                && chord_tokens(next).is_none();
            if usable {
                sheet.lines.push(SheetLine {
                    text: next.to_string(),
                    chords,
                });
                i += 1;
            } else {
                sheet.lines.push(SheetLine {
                    text: String::new(),
                    chords,
                });
            }
        } else {
            sheet.lines.push(SheetLine {
                text: line.to_string(),
                chords: Vec::new(),
            });
        }
    }
    sheet
}

/// ChordPro: `[Am]Hello [G]world`, `{capo: 2}`, `{comment: …}`.
pub fn parse_chordpro(src: &str) -> Sheet {
    let mut sheet = Sheet::default();
    for raw in src.replace('\r', "").lines() {
        let line = raw.trim_end();
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if let Some(d) = t.strip_prefix('{') {
            let d = d.trim_end_matches('}');
            if let Some((k, v)) = d.split_once(':') {
                if k.trim().eq_ignore_ascii_case("capo") {
                    sheet.capo = v.trim().parse().ok().filter(|n| *n <= 12).unwrap_or(0);
                } else {
                    apply_meta(&mut sheet, k.trim(), v.trim());
                }
            }
            continue;
        }
        if is_section(t) {
            continue;
        }
        let mut text = String::new();
        let mut col = 0usize;
        let mut chords = Vec::new();
        let mut it = line.chars().peekable();
        while let Some(c) = it.next() {
            if c == '[' {
                let mut inner = String::new();
                let mut closed = false;
                for d in it.by_ref() {
                    if d == ']' {
                        closed = true;
                        break;
                    }
                    inner.push(d);
                }
                if closed && is_chord(&inner) {
                    chords.push((col, inner));
                } else {
                    text.push('[');
                    text.push_str(&inner);
                    col += 1 + inner.chars().count();
                    if closed {
                        text.push(']');
                        col += 1;
                    }
                }
            } else {
                text.push(c);
                col += 1;
            }
        }
        sheet.lines.push(SheetLine { text, chords });
    }
    sheet
}

/// Pick the format: inline `[Am]` chords → ChordPro, otherwise chords-over-lyrics.
pub fn parse_auto(src: &str) -> Sheet {
    let src = strip_tags(src);
    let inline = src.lines().any(|l| {
        l.match_indices('[').any(|(i, _)| {
            l[i + 1..]
                .split_once(']')
                .is_some_and(|(inner, after)| is_chord(inner) && !after.trim().is_empty())
        })
    });
    if inline { parse_chordpro(&src) } else { parse_text(&src) }
}

// ── local files ────────────────────────────────────────────────────────
pub const EXTS: [&str; 6] = ["cho", "chopro", "chordpro", "crd", "pro", "txt"];

pub fn default_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("music_display")
        .join("chords")
}

fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Find `Artist - Title.<ext>` (or just `Title.<ext>`) in `dir`, ignoring case,
/// punctuation, "(Remastered)" suffixes and extra artists.
pub fn find_local(dir: &Path, artist: &str, title: &str) -> Option<PathBuf> {
    let cleaned = crate::year::clean(title);
    let first = artist.split(", ").next().unwrap_or(artist);
    let mut wanted = vec![];
    for t in [title, cleaned.as_str()] {
        for a in [artist, first] {
            wanted.push(key(&format!("{a}{t}")));
            wanted.push(key(&format!("{t}{a}")));
        }
        wanted.push(key(t));
    }
    let mut best: Option<(usize, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let Some(rank) = EXTS.iter().position(|x| x.eq_ignore_ascii_case(ext)) else {
            continue;
        };
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if wanted.contains(&key(stem)) && best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, p));
        }
    }
    best.map(|(_, p)| p)
}

/// Read + parse a local sheet. `None` when missing or it has too few chords to be real.
pub async fn load_local(dir: PathBuf, artist: String, title: String) -> Option<Sheet> {
    let path = find_local(&dir, &artist, &title)?;
    let bytes = tokio::fs::read(&path).await.ok()?;
    let src = String::from_utf8_lossy(&bytes);
    let ext = path.extension()?.to_str()?.to_lowercase();
    let mut sheet = if ext == "txt" {
        parse_auto(&src)
    } else {
        parse_chordpro(&src)
    };
    // A .cho file may still contain chords-over-lyrics; fall back if it parsed to nothing.
    if sheet.chord_count() == 0 {
        sheet = parse_auto(&src);
    }
    if sheet.chord_count() < 4 {
        return None;
    }
    sheet.source = path.file_name()?.to_string_lossy().into_owned();
    Some(sheet)
}

/// ChordPro text for the cache (round-trips through [`parse_chordpro`]).
pub fn to_chordpro(s: &Sheet) -> String {
    let mut out = String::new();
    if s.capo > 0 {
        out.push_str(&format!("{{capo: {}}}\n", s.capo));
    }
    if let Some(k) = &s.key {
        out.push_str(&format!("{{key: {k}}}\n"));
    }
    if let Some(b) = s.bpm {
        out.push_str(&format!("{{tempo: {b}}}\n"));
    }
    if let Some(t) = &s.tuning {
        out.push_str(&format!("{{tuning: {t}}}\n"));
    }
    for l in &s.lines {
        let mut cs = l.chords.clone();
        cs.sort_by_key(|c| c.0);
        let chars: Vec<char> = l.text.chars().collect();
        let mut pos = 0usize;
        for (col, name) in cs {
            while pos < col {
                out.push(chars.get(pos).copied().unwrap_or(' '));
                pos += 1;
            }
            out.push_str(&format!("[{name}]"));
        }
        out.extend(chars.iter().skip(pos));
        out.push('\n');
    }
    out
}

// ── matching a sheet onto synced lyrics ────────────────────────────────
/// Lowercase, letters/digits only, single spaces.
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() || matches!(c, '-' | '/' | ',' | '.' | '!' | '?' | ';' | ':') {
            out.push(' ');
        } // apostrophes etc. vanish: "don't" → "dont"
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn dice(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let grams = |s: &str| -> std::collections::HashMap<(char, char), u32> {
        let v: Vec<char> = s.chars().collect();
        let mut m = std::collections::HashMap::new();
        for w in v.windows(2) {
            *m.entry((w[0], w[1])).or_insert(0) += 1;
        }
        m
    };
    let (ga, gb) = (grams(a), grams(b));
    let (na, nb): (u32, u32) = (ga.values().sum(), gb.values().sum());
    if na == 0 || nb == 0 {
        return if a == b { 1.0 } else { 0.0 };
    }
    let common: u32 = ga
        .iter()
        .map(|(k, v)| (*v).min(gb.get(k).copied().unwrap_or(0)))
        .sum();
    2.0 * common as f32 / (na + nb) as f32
}

/// `(start char, length)` of each whitespace-separated word.
pub fn words_pos(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut n = 0;
    for (i, c) in s.chars().chain(std::iter::once(' ')).enumerate() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push((st, n));
                n = 0;
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            n += 1;
        }
    }
    out
}

/// Move a chord column from the sheet's wording onto the lyric's wording, anchored to
/// the word it sits on (so small wording differences don't misplace it).
fn map_col(col: usize, sheet_text: &str, lyric: &str) -> usize {
    let sw = words_pos(sheet_text);
    let lw = words_pos(lyric);
    let lyric_len = lyric.chars().count();
    if lw.is_empty() {
        return 0;
    }
    let Some(w) = sw.iter().position(|(s, l)| col < s + l) else {
        return lyric_len; // past the last word
    };
    let off = col.saturating_sub(sw[w].0);
    let w2 = if sw.len() == lw.len() {
        w
    } else {
        (w * lw.len() / sw.len()).min(lw.len() - 1)
    };
    lw[w2].0 + off.min(lw[w2].1)
}

#[derive(Clone, Debug, Default)]
pub struct Aligned {
    /// Per lyric line: chords with columns relative to the single-spaced lyric text.
    pub per_line: Vec<Vec<(usize, String)>>,
    pub matched: usize,
    pub total: usize,
    pub flats: bool,
    /// Chord-only sheet lines before the first matched lyric line (the intro), one
    /// `Vec` of chord names per sheet line.
    pub intro: Vec<Vec<String>>,
    /// Per lyric line: chord-only sheet lines that follow it (solo / instrumental break),
    /// one `Vec` of chord names per sheet line. The last entry is the outro.
    pub gaps: Vec<Vec<Vec<String>>>,
}

impl Aligned {
    pub fn percent(&self) -> u32 {
        (self.matched * 100).checked_div(self.total).unwrap_or(0) as u32
    }
    /// Below this the chords would more often be wrong than right, so none are shown.
    #[doc(hidden)]
    pub fn usable(&self) -> bool {
        self.usable_at(DEFAULT_MATCH)
    }
    /// [`usable`](Self::usable) with your own limit (`chords_match`, percent of the lines).
    pub fn usable_at(&self, min_pct: u32) -> bool {
        self.total > 0 && self.matched * 100 >= self.total * min_pct as usize
    }
}

/// The share of lyric lines (percent) that must match the sheet for its chords to be shown.
pub const DEFAULT_MATCH: u32 = 40;

#[doc(hidden)]
pub fn align(sheet: &Sheet, lyrics: &[(f64, String)]) -> Aligned {
    align_with(sheet, lyrics, DEFAULT_MATCH)
}

pub fn align_with(sheet: &Sheet, lyrics: &[(f64, String)], min_pct: u32) -> Aligned {
    let sung: Vec<(usize, String)> = sheet
        .lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let n = norm(&l.text);
            (!n.is_empty()).then_some((i, n))
        })
        .collect();
    let mut out = Aligned {
        per_line: vec![Vec::new(); lyrics.len()],
        gaps: vec![Vec::new(); lyrics.len()],
        flats: sheet.prefers_flats(),
        ..Default::default()
    };
    let mut hits: Vec<(usize, usize)> = Vec::new(); // (lyric line, sheet line)
    let mut j = 0usize;
    let mut first_hit: Option<usize> = None;
    for (li, (_, text)) in lyrics.iter().enumerate() {
        let n = norm(text);
        if n.is_empty() {
            continue;
        }
        out.total += 1;

        // 1. next few sheet lines in order
        let mut best: Option<(usize, f32)> = None;
        for (k, (_, s)) in sung.iter().enumerate().skip(j).take(10) {
            let d = dice(&n, s);
            if d >= 0.55 && best.is_none_or(|(_, b)| d > b) {
                best = Some((k, d));
            }
        }
        let mut advance = true;
        // 2. anywhere (a chorus the sheet writes once but the song repeats)
        if best.is_none() {
            for (k, (_, s)) in sung.iter().enumerate() {
                let d = dice(&n, s);
                if d >= 0.75 && best.is_none_or(|(_, b)| d > b) {
                    best = Some((k, d));
                }
            }
            advance = false;
        }
        let Some((k, _)) = best else { continue };
        if first_hit.is_none() {
            first_hit = Some(sung[k].0);
        }
        if advance {
            j = k + 1;
        }
        out.matched += 1;
        hits.push((li, sung[k].0));
        let line = &sheet.lines[sung[k].0];
        let lyric_norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut cs: Vec<(usize, String)> = line
            .chords
            .iter()
            .map(|(c, name)| (map_col(*c, &line.text, &lyric_norm), name.clone()))
            .collect();
        cs.sort_by_key(|c| c.0);
        out.per_line[li] = cs;
    }
    out.intro = sheet
        .lines
        .iter()
        .take(first_hit.unwrap_or(0))
        .filter(|l| norm(&l.text).is_empty() && !l.chords.is_empty())
        .map(|l| l.chords.iter().map(|c| c.1.clone()).collect())
        .collect();
    // chord-only sheet lines between one matched line and the next: the play-along
    // chords for the break after that lyric line
    for (n, (li, si)) in hits.iter().enumerate() {
        let end = hits.iter().skip(n + 1).map(|h| h.1).find(|s| s > si).unwrap_or(sheet.lines.len());
        out.gaps[*li] = sheet.lines[si + 1..end.max(si + 1)]
            .iter()
            .filter(|l| norm(&l.text).is_empty() && !l.chords.is_empty())
            .map(|l| l.chords.iter().map(|c| c.1.clone()).collect())
            .collect();
    }
    if !out.usable_at(min_pct) {
        for l in &mut out.per_line {
            l.clear();
        }
        for g in &mut out.gaps {
            g.clear();
        }
        out.intro.clear();
    }
    out
}
