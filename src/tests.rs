#[test]
fn track_identity_ignores_editions() {
    let n = util::normalize_track;
    assert_eq!(n("Master of Puppets (Remastered)"), n("Master of Puppets"));
    assert_eq!(n("Album [Deluxe Edition]"), n("Album (25th Anniversary Edition)"));
    assert_eq!(n("Song - Remastered 2011"), n("Song -"));
    assert_ne!(n("Song (Part 2)"), n("Song"));
}

#[test]
fn track_identity_never_empty_and_tuning_clean() {
    assert!(!util::normalize_track("(Live)").is_empty());
    assert!(!util::normalize_track("---").is_empty());
    assert_eq!(util::normalize_track("Song - 1999"), "song - 1999");
    let store: serde_json::Value = serde_json::from_str(r#"{"tuning":{"name":"","value":""}}"#).unwrap();
    assert!(ug::find_key(&store, "tuning").is_some());
}

#[test]
fn chords_align_on_cell_width() {
    // each Hangul syllable is two cells wide, so a chord over the 3rd char sits at cell 4
    let text: Vec<char> = "안녕하세요".chars().collect();
    assert_eq!(ui::chord_line(&[(2, "Am".into())], &text, 99, 0, false), "    Am");
    // pulled left to fit instead of overflowing
    assert_eq!(ui::chord_line(&[(2, "Am7".into())], &text, 6, 0, false), "   Am7");
    let text: Vec<char> = "Åäö".chars().collect();
    assert_eq!(ui::chord_line(&[(2, "G".into())], &text, 99, 0, false), "  G");
}

#[test]
fn google_translate_response_parses() {
    let v: serde_json::Value = serde_json::from_str(r#"[[["Hello ","안녕",null],["world","세계",null]],null,"ko"]"#).unwrap();
    assert_eq!(translate::parse(&v).as_deref(), Some("Hello world"));
}

#[test]
fn settings_save_keeps_other_keys() {
    let dir = std::env::temp_dir().join(format!("md-cfg-{}", std::process::id()));
    let path = dir.join("config.toml");
    let _ = std::fs::remove_dir_all(&dir);
    config::save(&path, &[("chords", toml::Value::Boolean(true))]).unwrap();
    let first = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{first}player = \"mpd\"\n")).unwrap();
    config::save(&path, &[("translate", toml::Value::Boolean(true))]).unwrap();
    let t: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(t["chords"].as_bool(), Some(true));
    assert_eq!(t["translate"].as_bool(), Some(true));
    assert_eq!(t["player"].as_str(), Some("mpd"));
    let _ = std::fs::remove_dir_all(&dir);
}

use super::*;
use clap::Parser;

#[test]
fn lrc_parsing() {
    let l = lyrics::parse_lrc(
        "[ar:x]\n[00:12.50]hello\n[00:05.00][00:20.00]multi\n[00:30]\n[offset:+500]",
    );
    assert_eq!(l.len(), 4);
    assert!((l[0].0 - 4.5).abs() < 1e-9 && l[0].1 == "multi");
    assert_eq!(l[1].1, "hello");
    assert_eq!(l[3].1, "♪");
}

#[test]
fn title_cleaning() {
    assert_eq!(
        year::clean("Come Together - Remastered 2009"),
        "Come Together"
    );
    assert_eq!(
        year::clean("Song (Live at Wembley) [Deluxe Edition]"),
        "Song"
    );
    assert_eq!(year::clean("Normal Title"), "Normal Title");
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("md-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn theme_from(extra: &[&str], file: Option<&str>) -> theme::Theme {
    let mut argv = vec!["x".to_string()];
    if let Some(content) = file {
        let p = tmp("theme").join(format!("{}.json", util::fnv1a(content)));
        std::fs::write(&p, content).unwrap();
        argv.extend(["-c".into(), p.to_str().unwrap().to_string()]);
    }
    argv.extend(extra.iter().map(|s| s.to_string()));
    theme::load(&cli::Args::parse_from(argv))
}

#[test]
fn hex_and_theme_schemas() {
    assert_eq!(theme::parse_hex("#ff8000"), Some((255, 128, 0)));
    assert_eq!(theme::parse_hex("ff8000cc"), Some((255, 128, 0)));
    let cases = [
        r##"{"mSurface":"#101010","mPrimary":"#ff0000"}"##,
        r##"{"colors":{"dark":{"surface":"#101010","primary":"#ff0000"}}}"##,
        r##"{"colors":{"surface":{"dark":"#101010","light":"#ffffff"},"primary":{"dark":{"color":"#ff0000"}}}}"##,
        r##"{"dark":{"mSurface":"#101010","mPrimary":"#ff0000"},"light":{"mSurface":"#eeeeee"}}"##,
    ];
    for (i, c) in cases.iter().enumerate() {
        let t = theme_from(&[], Some(c));
        assert_eq!(t.surface, Some((16, 16, 16)), "case {i}");
        assert_eq!(t.primary, Some((255, 0, 0)), "case {i}");
        assert!(t.on_surface.is_some(), "case {i}");
    }
}

#[test]
fn empty_or_invalid_json_falls_back_to_terminal_colors() {
    for bad in [
        "{}",
        "",
        "not json",
        "[]",
        r#"{"foo": 1}"#,
        r##"{"mOnSurface":"#fff"}"##,
    ] {
        let t = theme_from(&[], Some(bad));
        assert_eq!(
            (t.surface, t.primary, t.on_surface),
            (None, None, None),
            "{bad:?}"
        );
        assert!(
            t.source.starts_with("terminal colors"),
            "{bad:?}: {}",
            t.source
        );
        // terminal slots map to reset/ANSI colours, never a hex value
        assert_eq!(t.surface(), ratatui::style::Color::Reset);
        assert_eq!(t.text(), ratatui::style::Color::Reset);
        assert_eq!(t.primary(), ratatui::style::Color::Cyan);
        assert!(!matches!(t.fade(0.5), ratatui::style::Color::Rgb(..)));
        assert!(!matches!(
            t.accent_fade(0.6),
            ratatui::style::Color::Rgb(..)
        ));
    }
    // a missing explicit file behaves the same
    let t = theme::load(&cli::Args::parse_from([
        "x",
        "-c",
        "/nonexistent/colors.json",
    ]));
    assert_eq!((t.surface, t.primary), (None, None));
}

#[test]
fn terminal_colors_flag_bypasses_files_and_accepts_overrides() {
    let file = r##"{"mSurface":"#101010","mPrimary":"#ff0000"}"##;
    let t = theme_from(&["--terminal-colors"], Some(file));
    assert_eq!((t.surface, t.primary, t.on_surface), (None, None, None));
    assert_eq!(t.source, "terminal colors");

    // both overrides together → fully custom
    let t = theme_from(
        &[
            "--terminal-colors",
            "--surface",
            "#112233",
            "--primary",
            "#ffaa00",
        ],
        Some(file),
    );
    assert_eq!(t.surface, Some((0x11, 0x22, 0x33)));
    assert_eq!(t.primary, Some((0xff, 0xaa, 0x00)));
    assert!(t.on_surface.is_some());
    assert!(t.source.contains("CLI overrides"));

    // only --primary: the background and text stay the terminal's own
    let t = theme_from(&["--terminal-colors", "--primary", "#ffaa00"], None);
    assert_eq!((t.surface, t.on_surface), (None, None));
    assert_eq!(t.primary, Some((0xff, 0xaa, 0x00)));
    assert_eq!(t.surface(), ratatui::style::Color::Reset);

    // only --surface: text colour is derived so it stays readable on it
    let t = theme_from(&["--terminal-colors", "--surface", "#101010"], None);
    assert_eq!(t.surface, Some((16, 16, 16)));
    assert!(theme::luminance(t.on_surface.unwrap()) > 0.7);
    assert_eq!(t.primary(), ratatui::style::Color::Cyan);
}

#[test]
fn accent_from_cover() {
    let solid = |c: [u8; 3]| {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(32, 32, image::Rgb(c)))
    };
    let a = cover::dominant_accent(&solid([220, 30, 40])).unwrap();
    assert!(a.0 > 200 && a.1 < 60 && a.2 < 70, "{a:?}");

    // a mostly-black cover with a small vivid blue patch → blue, not gray/black
    let mut img = image::RgbImage::from_pixel(64, 64, image::Rgb([8, 8, 8]));
    for y in 0..24 {
        for x in 0..24 {
            img.put_pixel(x, y, image::Rgb([20, 60, 230]));
        }
    }
    let a = cover::dominant_accent(&image::DynamicImage::ImageRgb8(img)).unwrap();
    assert!(a.2 > 180 && a.0 < 60, "{a:?}");

    // grayscale → falls back to a neutral instead of None; pure black → None
    assert!(cover::dominant_accent(&solid([140, 140, 140])).is_some());
    assert!(cover::dominant_accent(&solid([0, 0, 0])).is_none());

    // readable() lifts a dark accent on a dark surface and keeps unknown-bg accents bright
    let dark_bg = Some((16, 16, 16));
    let r = color::readable((30, 20, 90), dark_bg);
    assert!(
        theme::luminance(r) - theme::luminance((16, 16, 16)) >= 0.4 - 1e-3,
        "{r:?}"
    );
    assert!(theme::luminance(color::readable((10, 10, 40), None)) >= 0.35);
}

#[test]
fn ascii_cover_dimensions_and_charset() {
    let mut img = image::RgbImage::new(64, 64);
    for (x, _, p) in img.enumerate_pixels_mut() {
        let v = (x * 4) as u8; // left → right gradient
        *p = image::Rgb([v, 40, 200 - v / 2]);
    }
    let art = cover::ascii_art(&image::DynamicImage::ImageRgb8(img), 20, 10);
    assert_eq!((art.cols, art.rows, art.cells.len()), (20, 10, 200));
    assert!(
        art.cells
            .iter()
            .all(|(c, _)| c.is_ascii() && !c.is_control())
    );
    // dark side is sparse, bright side is dense
    let density = |c: char| " .:-=+*#%@".find(c).unwrap();
    assert!(density(art.cells[0].0) < density(art.cells[19].0));
}

#[test]
fn big_font_renders_and_falls_back() {
    let rows = bigfont::render("Hi there", 60, 3).unwrap();
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .all(|r| r.chars().count() == rows[0].chars().count())
    );
    assert!(rows.iter().any(|r| r.contains('█')));
    // wraps into pieces (4 rows each + 1 blank row between) when narrow
    let rows = bigfont::render("go on", 24, 3).unwrap();
    assert_eq!(rows.len(), 9);
    let rows = bigfont::render("hello world again", 40, 3).unwrap();
    assert_eq!(rows.len(), 14);
    assert!(rows.iter().all(|r| r.chars().count() <= 40));
    // too many pieces / word wider than the panel / unsupported script → None
    assert!(bigfont::render("one two three four five six seven", 20, 2).is_none());
    assert!(bigfont::render("supercalifragilistic", 30, 3).is_none());
    assert!(bigfont::render("你好", 60, 3).is_none());
    // curly quotes and ellipsis work
    assert!(bigfont::render("Cafe – don’t…", 100, 1).is_some());
    // accented letters are drawn with their accent, not as the ASCII lookalike
    let plain = bigfont::render("AAO", 100, 1).unwrap();
    let acc = bigfont::render("ÅÄÖ", 100, 1).unwrap();
    assert_eq!(acc.len(), plain.len());
    assert_ne!(acc, plain);
    assert!(bigfont::render("Café", 100, 1).is_some());
    assert!(bigfont::render("ß", 100, 1).is_none());
}

#[test]
fn three_panel_side_columns_are_equal() {
    for w in [80u16, 100, 120, 139, 140, 200, 301] {
        let main = ratatui::layout::Rect::new(0, 0, w, 30);
        let [l, m, r] = ui::three_panel(main);
        assert_eq!(l.width, r.width, "width {w}");
        assert_eq!(l.width + m.width + r.width, w, "width {w}");
        assert!(m.width >= 20, "width {w}");
        assert_eq!((l.x, m.x, r.x), (0, l.width, l.width + m.width));
    }
}

#[test]
fn cava_runs_mono() {
    let c = cava::config(48);
    assert!(c.contains("channels = mono"));
    assert!(c.contains("bars = 48"));
    assert!(c.contains("method = raw") && c.contains("data_format = ascii"));
}

#[test]
fn chord_recognition_and_transpose() {
    for ok in ["A", "Am", "F#m7", "Bb/D", "Gsus4", "Cadd9", "E7b9", "D(add9)", "Cmaj7", "G/B", "Em7"] {
        assert!(chords::is_chord(ok), "{ok}");
    }
    for bad in ["Hello", "I", "And", "Bridge", "Hm", "N.C.", "x2", "A/", "Dog"] {
        assert!(!chords::is_chord(bad), "{bad}");
    }
    assert_eq!(chords::transpose("Am7", 2, false), "Bm7");
    assert_eq!(chords::transpose("C/E", 1, false), "C#/F");
    assert_eq!(chords::transpose("C/E", 1, true), "Db/F");
    assert_eq!(chords::transpose("Bb", 2, true), "C");
    assert_eq!(chords::transpose("E", 12, false), "E");
    assert_eq!(chords::transpose("G", 7, false), "D");
}

const OVER: &str = "[Verse 1]\nCapo 2\n   Am        G\nHello darkness my old friend\n   C         E7\nI've come to talk with you again\n\n[Chorus]\n   F      G\nSing along tonight\n\nBridge over troubled water\n";

#[test]
fn parses_chords_over_lyrics() {
    let s = chords::parse_text(OVER);
    assert_eq!(s.capo, 2);
    assert_eq!(s.lines.len(), 4, "{:?}", s.lines);
    assert_eq!(s.lines[0].text, "Hello darkness my old friend");
    assert_eq!(s.lines[0].chords, vec![(3, "Am".to_string()), (13, "G".to_string())]);
    assert_eq!(s.lines[2].chords.len(), 2);
    // a lyric that merely starts with a section word stays a lyric
    assert_eq!(s.lines[3].text, "Bridge over troubled water");
    assert!(s.lines[3].chords.is_empty());
    // UG-style tags and tab lines are cleaned
    let ug = chords::parse_text("[Verse]\n[ch]Am[/ch]   [ch]G[/ch]\nhello there\ne|---0---|\n");
    assert_eq!(ug.lines.len(), 1);
    assert_eq!(ug.chord_count(), 2);
}

#[test]
fn parses_chordpro_and_autodetects() {
    let src = "{title: x}\n{capo: 3}\n# comment\n[Am]Hello dar[G]kness\n[Chorus]\n[C] [G]\nplain [text] here\n";
    let s = chords::parse_chordpro(src);
    assert_eq!(s.capo, 3);
    assert_eq!(s.lines[0].text, "Hello darkness");
    assert_eq!(s.lines[0].chords, vec![(0, "Am".to_string()), (9, "G".to_string())]);
    assert_eq!(s.lines[1].text, " ");
    assert_eq!(s.lines[1].chords.len(), 2);
    assert_eq!(s.lines[2].text, "plain [text] here");
    assert_eq!(chords::parse_auto(src).lines[0].chords.len(), 2);
    assert_eq!(chords::parse_auto(OVER).lines.len(), 4);
}

fn lrc(lines: &[&str]) -> Vec<(f64, String)> {
    lines.iter().enumerate().map(|(i, l)| (i as f64, l.to_string())).collect()
}

#[test]
fn aligns_sheet_to_synced_lyrics() {
    let sheet = chords::parse_text(OVER);
    // chorus appears twice in the song but once in the sheet; wording differs a little
    let l = lrc(&[
        "Hello darkness, my old friend",
        "♪",
        "I've come to talk with you again",
        "Sing along tonight",
        "Hello darkness, my old friend",
        "Sing along tonight",
    ]);
    let a = chords::align(&sheet, &l);
    assert_eq!((a.matched, a.total), (5, 5), "{a:?}");
    assert!(a.usable());
    assert!(a.per_line[1].is_empty());
    // "Am" over "Hello" → col 0; "G" over "my"/"old" region, anchored to a word
    assert_eq!(a.per_line[0][0], (3, "Am".into())); // col 3 is inside "Hello"
    assert_eq!(a.per_line[0].len(), 2);
    // repeated chorus line and repeated verse line (global fallback) get chords again
    assert_eq!(a.per_line[3].len(), 2);
    assert_eq!(a.per_line[5].len(), 2);
    assert_eq!(a.per_line[4].len(), 2);
    // unrelated lyrics → nothing is shown
    let bad = chords::align(&sheet, &lrc(&["completely different words here", "nothing alike at all", "zzz qqq"]));
    assert!(!bad.usable() && bad.per_line.iter().all(Vec::is_empty));
}

#[test]
fn wrap_ranges_matches_wrap() {
    for text in [
        "hello darkness my old friend",
        "a verylongtokenthatmustbebroken here and there",
        "one",
        "",
    ] {
        for w in [5usize, 8, 12, 40] {
            let rows = ui::wrap(text, w);
            let chars: Vec<char> = text.chars().collect();
            let r2: Vec<String> = ui::wrap_ranges(text, w)
                .iter()
                .map(|(s, e)| chars[*s..*e].iter().collect())
                .collect();
            assert_eq!(rows, r2, "{text:?} @ {w}");
        }
    }
}

#[test]
fn finds_local_sheets() {
    let dir = tmp("chords");
    std::fs::write(
        dir.join("The Sound Of Silence - Simon & Garfunkel.txt"),
        "x",
    )
    .unwrap();
    std::fs::write(dir.join("simon garfunkel - the sound of silence.cho"), "[Am]a [G]b [C]c [F]d").unwrap();
    let hit = chords::find_local(&dir, "Simon & Garfunkel", "The Sound of Silence - Remastered").unwrap();
    assert_eq!(hit.extension().unwrap(), "cho"); // .cho outranks .txt
    assert!(chords::find_local(&dir, "Nobody", "Nothing").is_none());
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let sheet = rt
        .block_on(chords::load_local(dir.clone(), "Simon & Garfunkel".into(), "The Sound of Silence".into()))
        .unwrap();
    assert_eq!(sheet.chord_count(), 4);
    assert!(sheet.source.ends_with(".cho"));
}

fn ug_page(json: &serde_json::Value) -> String {
    let esc = json.to_string().replace('&', "&amp;").replace('"', "&quot;").replace('\'', "&#039;");
    format!("<html><body><div class=\"js-store\" data-content=\"{esc}\"></div></body></html>")
}

/// Tiny HTTP server: `routes` maps a path fragment to (status, body); counts requests.
fn fake_site(routes: Vec<(&'static str, u16, String)>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let h2 = hits.clone();
    std::thread::spawn(move || {
        for mut st in l.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = st.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = req.lines().next().unwrap_or("").to_string();
            h2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (code, body) = routes
                .iter()
                .find(|(frag, _, _)| first.contains(frag))
                .map(|(_, c, b)| (*c, b.clone()))
                .unwrap_or((404, String::new()));
            let _ = write!(st, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    (base, hits)
}

#[test]
fn ultimate_guitar_provider_with_fake_site() {
    assert_eq!(ug::unescape("a &quot;b&quot; &amp; c &#039;d&#x27; &bogus; &"), "a \"b\" & c 'd' &bogus; &");

    let tab = "[Verse 1]\n[ch]Am[/ch]        [ch]G[/ch]\nHello darkness my old friend\n[ch]C[/ch]         [ch]F[/ch]\nI've come to talk with you again\n[ch]Am[/ch]        [ch]G[/ch]\nBecause a vision softly creeping\n[ch]C[/ch]         [ch]F[/ch]\nLeft its seeds while I was sleeping\n";
    let search = serde_json::json!({"store":{"page":{"data":{"results":[
        {"song_name":"The Sound of Silence","artist_name":"Simon & Garfunkel","type":"Tabs","tab_url":"/tab/x/tabs-1","rating":5.0,"votes":9999},
        {"song_name":"The Sound of Silence (Live)","artist_name":"Someone Else","type":"Chords","tab_url":"/tab/y/chords-2","rating":5.0,"votes":9999},
        {"song_name":"The Sound of Silence","artist_name":"Simon & Garfunkel","type":"Chords","tab_url":"/tab/z/chords-3","rating":4.1,"votes":50},
        {"song_name":"The Sound of Silence","artist_name":"Simon & Garfunkel","type":"Chords","tab_url":"/tab/z/chords-4","rating":4.8,"votes":4000}
    ]}}}});
    let page = |content: &str| ug_page(&serde_json::json!({"store":{"page":{"data":{"tab_view":{"wiki_tab":{"content":content},"meta":{"capo":3}}}}}}));

    let store = ug::extract_store(&ug_page(&search)).unwrap();
    let c = ug::candidates(&store, "Simon & Garfunkel", "The Sound of Silence - Remastered");
    assert_eq!(c.len(), 2, "tabs and other artists are filtered: {c:?}");
    assert!(c[0].url.ends_with("chords-4"), "best rated × votes first");

    // best page is junk (no chords) → falls through to the next candidate
    let (base, hits) = fake_site(vec![
        ("chords-4", 200, page("just words, no chords here\nat all\n")),
        ("chords-3", 200, page(tab)),
        ("search.php", 200, ug_page(&search)),
    ]);
    let client = reqwest::Client::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (artist, title) = ("Simon & Garfunkel", "The Sound of Silence UGTEST");
    // title differs slightly from the fixture, so use a title that matches it exactly:
    let _ = title;
    let title = "The Sound of Silence";
    let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a(&format!("{artist}\0{title}")))));
    ug::reset_breaker();
    let out = rt.block_on(ug::lookup(&client, &base, artist, title));
    let ug::Outcome::Found(sheet) = out else { panic!("expected a sheet") };
    assert_eq!(sheet.chord_count(), 8);
    assert_eq!(sheet.capo, 3, "capo comes from the page metadata");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3, "search + 2 tab pages");
    // second lookup: from the cache, zero requests
    let again = rt.block_on(ug::lookup(&client, &base, artist, title));
    let ug::Outcome::Found(s2) = again else { panic!("cached sheet") };
    assert_eq!(s2.chord_count(), 8);
    assert!(s2.source.contains("cached") && s2.capo == 3);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);
    let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a(&format!("{artist}\0{title}")))));

    // blocked (403) three times in a row → provider switches itself off, no more requests
    let (base2, hits2) = fake_site(vec![("search.php", 403, "no".into())]);
    ug::reset_breaker();
    for i in 0..3 {
        let t = format!("Blocked Song {i}");
        let o = rt.block_on(ug::lookup(&client, &base2, "Nobody", &t));
        assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("403")));
    }
    let o = rt.block_on(ug::lookup(&client, &base2, "Nobody", "Blocked Song 9"));
    assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("switched off")));
    assert_eq!(hits2.load(std::sync::atomic::Ordering::SeqCst), 3);

    // page without js-store (captcha / redesign) is an error, never a cached "not found"
    let (base3, _) = fake_site(vec![("search.php", 200, "<html>captcha</html>".into())]);
    ug::reset_breaker();
    let o = rt.block_on(ug::lookup(&client, &base3, "Nobody", "Format Change"));
    assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("not recognised")));

    // no matching chord page → NotFound (and cached as such)
    let empty = ug_page(&serde_json::json!({"store":{"page":{"data":{"results":[]}}}}));
    let (base4, _) = fake_site(vec![("search.php", 200, empty.clone())]);
    ug::reset_breaker();
    let o = rt.block_on(ug::lookup(&client, &base4, "Nobody", "Truly Unknown Song"));
    assert!(matches!(o, ug::Outcome::NotFound));
    let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a("Nobody\0Truly Unknown Song"))));
    ug::reset_breaker();

    // not found under "artist title": the title alone is searched too
    let cache = |a: &str, t: &str| util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a(&format!("{a}\0{t}"))));
    let (artist, title) = ("Simon & Garfunkel", "The Sound of Silence");
    let (b5, hits5) = fake_site(vec![
        ("value=Simon", 200, empty.clone()),
        ("chords-4", 200, page(tab)),
        ("search.php", 200, ug_page(&search)),
    ]);
    let _ = std::fs::remove_file(cache(artist, title));
    let o = rt.block_on(ug::lookup(&client, &b5, artist, title));
    assert!(matches!(o, ug::Outcome::Found(_)), "found by the second search");
    assert_eq!(hits5.load(std::sync::atomic::Ordering::SeqCst), 3, "empty search, title search, one tab page");
    let _ = std::fs::remove_file(cache(artist, title));

    // a tab page that will not load does not end the lookup: the next candidate is tried
    let (b6, _) = fake_site(vec![
        ("chords-4", 500, String::new()),
        ("chords-3", 200, page(tab)),
        ("search.php", 200, ug_page(&search)),
    ]);
    ug::reset_breaker();
    let o = rt.block_on(ug::lookup(&client, &b6, artist, title));
    assert!(matches!(o, ug::Outcome::Found(_)), "second page used after the first failed");
    let _ = std::fs::remove_file(cache(artist, title));

    // every page failing is an error and is not remembered as "not found"
    let (b7, _) = fake_site(vec![("chords-", 500, String::new()), ("search.php", 200, ug_page(&search))]);
    ug::reset_breaker();
    let o = rt.block_on(ug::lookup(&client, &b7, artist, title));
    assert!(matches!(o, ug::Outcome::Error(ref e) if e.contains("500")));
    assert!(!cache(artist, title).exists(), "nothing cached after errors");

    // the provider rests after three blocks in a row, then tries again
    ug::reset_breaker();
    for i in 0..3 {
        let _ = rt.block_on(ug::lookup(&client, &base2, "Nobody", &format!("Blocked Again {i}")));
    }
    assert!(ug::disabled());
    ug::set_last_fail(util::now_secs() - 601);
    assert!(!ug::disabled(), "ten minutes later it is on again");
    ug::reset_breaker();
}

#[test]
fn chordpro_cache_round_trip() {
    let s = chords::parse_text(OVER);
    let back = chords::parse_chordpro(&chords::to_chordpro(&s));
    assert_eq!(back.capo, s.capo);
    assert_eq!(back.lines.len(), s.lines.len());
    for (a, b) in s.lines.iter().zip(&back.lines) {
        assert_eq!(a.text.trim(), b.text.trim());
        assert_eq!(a.chords.iter().map(|c| &c.1).collect::<Vec<_>>(), b.chords.iter().map(|c| &c.1).collect::<Vec<_>>());
    }
}

fn snap(artist: &str, title: &str, album: &str) -> player::Snapshot {
    player::Snapshot {
        status: player::Status::Playing,
        position: 0.0,
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        length: 100.0,
        art_url: String::new(),
        track_no: None,
        disc_no: None,
        content_created: String::new(),
        genre: String::new(),
        volume: None,
        url: String::new(),
        track_id: String::new(),
        player: String::new(),
        shuffle: None,
        repeat: None,
    }
}

#[test]
fn track_key_ignores_editions_and_case() {
    let k = |a, t, al| snap(a, t, al).key();
    let base = k("Metallica", "Enter Sandman", "Metallica");
    assert_eq!(base, k("Metallica", "Enter Sandman", "Metallica (Remastered Deluxe Box Set)"));
    assert_eq!(base, k("metallica ", "Enter  Sandman - Remastered 2021", "METALLICA (Anniversary Box Set)"));
    assert_eq!(base, k("Metallica", "Enter Sandman (Remastered)", "Metallica [Deluxe Edition]"));
    // different songs / artists stay different
    assert_ne!(base, k("Metallica", "Sad But True", "Metallica"));
    assert_ne!(base, k("Megadeth", "Enter Sandman", "Metallica"));
}

#[tokio::test]
async fn display_stays_on_first_spelling_of_a_track() {
    use clap::Parser;
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
    let mut a = snap("Metallica", "Enter Sandman", "Metallica");
    a.content_created = "1991-08-12".into();
    app.apply(AppEvent::Player(Some(a)));
    let mut b = snap("Metallica", "Enter Sandman - Remastered 2021", "Metallica (Deluxe Edition)");
    b.content_created = "2021-09-10".into();
    for s in [b.clone(), snap("Metallica", "Enter Sandman", "Metallica"), b] {
        app.apply(AppEvent::Player(Some(s)));
        let shown = app.snap.as_ref().unwrap();
        assert_eq!(shown.album, "Metallica");
        assert_eq!(shown.title, "Enter Sandman");
        assert_eq!(shown.content_created, "1991-08-12");
    }
}

#[test]
fn key_is_estimated_from_chords() {
    let sheet = |names: &[&str]| chords::Sheet {
        lines: vec![chords::SheetLine {
            text: "la la la la la la la la".into(),
            chords: names.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect(),
        }],
        ..Default::default()
    };
    assert_eq!(chords::estimate_key(&sheet(&["C", "G", "Am", "F", "C", "G", "F", "C"])).as_deref(), Some("C"));
    assert_eq!(chords::estimate_key(&sheet(&["Am", "F", "C", "G", "Am", "F", "G", "Am"])).as_deref(), Some("Am"));
    assert_eq!(chords::estimate_key(&sheet(&["C", "G"])), None);
}

#[tokio::test]
async fn translation_falls_back_to_libretranslate_on_429() {
    let (site, log) = scripted_site(vec![
        ("/g/dict", vec![R::Http(429, "", String::new())]),
        ("/g?", vec![R::Http(429, "", String::new())]),
        ("/libre", vec![R::Http(200, "Content-Type: application/json\r\n", r#"{"translatedText":"hello"}"#.into())]),
    ]);
    let cfg = translate::Cfg {
        google_base: Some(format!("{site}/g")),
        fallback: true,
        fallback_url: format!("{site}/libre"),
        fallback_key: Some("k".into()),
        skip: vec![],
    };
    let src = vec!["hola".to_string(), "♪".to_string(), "adiós".to_string()];
    let (out, backend) = translate::lines(&reqwest::Client::new(), &cfg, "en", &src).await.unwrap();
    assert_eq!(backend, "libretranslate");
    assert_eq!(out, vec!["hello", "♪", "hello"]);
    // once Google said 429 the rest of the song goes straight to the fallback
    assert!(log.lock().unwrap().iter().filter(|l| l.contains("/g")).count() <= 4);
    // no fallback: a limited Google is an error
    let off = translate::Cfg { fallback: false, ..cfg };
    assert!(translate::lines(&reqwest::Client::new(), &off, "en", &src).await.is_err());
}

#[tokio::test]
async fn skipped_languages_are_left_as_written() {
    let (site, _log) = scripted_site(vec![
        // the song as a whole (both lines sent as one sample) is detected as Spanish: not blocked
        ("q=hej%0Ahola", vec![R::Http(200, "Content-Type: application/json\r\n", r#"[[["hello","hej hola"]],null,"es"]"#.into())]),
        ("tl=en&q=hej", vec![R::Http(200, "Content-Type: application/json\r\n", r#"[[["hello","hej"]],null,"sv"]"#.into())]),
        ("tl=en&q=hola", vec![R::Http(200, "Content-Type: application/json\r\n", r#"[[["hello","hola"]],null,"es"]"#.into())]),
    ]);
    let cfg = translate::Cfg {
        google_base: Some(format!("{site}/g")),
        fallback: false,
        skip: vec!["sv".into()],
        ..Default::default()
    };
    let src = vec!["hej".to_string(), "hola".to_string()];
    let (out, _) = translate::lines(&reqwest::Client::new(), &cfg, "en", &src).await.unwrap();
    assert_eq!(out, vec!["hej", "hello"]);
    assert_eq!(translate::base_code("SV"), "sv");
    assert_eq!(translate::base_code("zh-CN"), "zh");
}

const JSON_HDR: &str = "Content-Type: application/json\r\n";

fn hits(log: &std::sync::Mutex<Vec<String>>, frag: &str) -> usize {
    log.lock().unwrap().iter().filter(|l| l.contains(frag)).count()
}

fn skip_cfg(site: &str, skip: &[&str], fallback: bool) -> translate::Cfg {
    translate::Cfg {
        google_base: Some(format!("{site}/g")),
        fallback,
        fallback_url: format!("{site}/libre"),
        skip: skip.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn blocked_language_is_kept_when_google_is_limited_and_dict_answers() {
    // gtx says 429, the dict endpoint translates and names the source language
    let (site, log) = scripted_site(vec![
        ("/g/dict", vec![R::Http(200, JSON_HDR, r#"{"sentences":[{"trans":"hello"}],"src":"sv"}"#.into())]),
        ("/g?", vec![R::Http(429, "", String::new())]),
    ]);
    let src = vec!["hej".to_string(), "♪".to_string(), "hallå".to_string()];
    let r = translate::lines(&reqwest::Client::new(), &skip_cfg(&site, &["sv"], false), "en", &src).await;
    assert!(r.as_ref().is_err_and(|e| translate::is_skipped(e)), "{r:?}");
    // detection cost the two requests; no line was sent after it
    assert_eq!(hits(&log, "/g"), 2);
}

#[tokio::test]
async fn blocked_song_makes_one_request_under_normal_google() {
    let (site, log) = scripted_site(vec![(
        "/g?",
        vec![R::Http(200, JSON_HDR, r#"[[["hello","hej"]],null,"sv"]"#.into())],
    )]);
    let src: Vec<String> = (0..20).map(|i| format!("rad {i}")).collect();
    let cfg = skip_cfg(&site, &["SV"], true);
    let r = translate::lines(&reqwest::Client::new(), &cfg, "en", &src).await;
    assert!(r.is_err_and(|e| translate::is_skipped(&e)));
    assert_eq!(log.lock().unwrap().len(), 1);
    // the list changed: the same song is evaluated again and now translated
    let none = skip_cfg(&site, &[], true);
    let (out, _) = translate::lines(&reqwest::Client::new(), &none, "en", &src).await.unwrap();
    assert!(out.iter().all(|l| l == "hello"));
}

#[tokio::test]
async fn blocked_language_under_libretranslate() {
    let (site, log) = scripted_site(vec![
        ("/g/dict", vec![R::Http(429, "", String::new())]),
        ("/g?", vec![R::Http(429, "", String::new())]),
        (
            "/libre",
            vec![R::Http(200, JSON_HDR, r#"{"translatedText":"hello","detectedLanguage":{"language":"sv"}}"#.into())],
        ),
    ]);
    let src = vec!["hej".to_string(), "hej igen".to_string()];
    let r = translate::lines(&reqwest::Client::new(), &skip_cfg(&site, &["sv"], true), "en", &src).await;
    assert!(r.is_err_and(|e| translate::is_skipped(&e)));
    assert_eq!(hits(&log, "/libre"), 1);
    // a language that is not on the list is translated
    let (out, backend) = translate::lines(&reqwest::Client::new(), &skip_cfg(&site, &["fi"], true), "en", &src).await.unwrap();
    assert_eq!((backend, out), ("libretranslate", vec!["hello".to_string(), "hello".to_string()]));
}

#[tokio::test]
async fn unknown_language_holds_the_translation() {
    // nothing can name the language: not translated, not silently "not blocked"
    let (site, log) = scripted_site(vec![
        ("/g/dict", vec![R::Http(200, JSON_HDR, r#"{"sentences":[{"trans":"hello"}]}"#.into())]),
        ("/g?", vec![R::Http(429, "", String::new())]),
        ("/libre", vec![R::Http(200, JSON_HDR, r#"{"translatedText":"hello"}"#.into())]),
    ]);
    let src = vec!["hej".to_string(), "hallå".to_string()];
    for fallback in [false, true] {
        log.lock().unwrap().clear();
        let r = translate::lines(&reqwest::Client::new(), &skip_cfg(&site, &["sv"], fallback), "en", &src).await;
        assert!(r.is_err_and(|e| translate::is_held(&e)), "fallback {fallback}");
        assert_eq!(hits(&log, "/g"), 2, "no per-line requests, fallback {fallback}");
    }
    // without a skip list nothing is held back and no detection request is made
    log.lock().unwrap().clear();
    assert!(translate::lines(&reqwest::Client::new(), &skip_cfg(&site, &[], false), "en", &src).await.is_ok());
}

#[test]
fn language_codes_are_normalised() {
    assert_eq!(translate::base_code("iw"), "he");
    assert_eq!(translate::base_code("IW"), "he");
    assert_eq!(translate::base_code("jw"), "jv");
    assert_eq!(translate::base_code("in"), "id");
    assert_eq!(translate::base_code("zh-CN"), "zh");
    assert_eq!(translate::base_code("zh_TW"), "zh");
    assert_eq!(translate::base_code(" Sv "), "sv");
}

#[tokio::test]
async fn legacy_codes_on_the_list_match_what_google_detects() {
    // the list says iw (old Hebrew code), Google answers he
    let (site, _log) = scripted_site(vec![(
        "/g?",
        vec![R::Http(200, JSON_HDR, r#"[[["hello","שלום"]],null,"he"]"#.into())],
    )]);
    let src = vec!["שלום".to_string()];
    let r = translate::lines(&reqwest::Client::new(), &skip_cfg(&site, &["iw"], false), "en", &src).await;
    assert!(r.is_err_and(|e| translate::is_skipped(&e)));
}

#[tokio::test]
async fn changing_the_skip_list_forgets_old_translations() {
    let mut app = test_app();
    // no song has played yet, so the track key is still 0
    app.translations.insert((0, app.args.translate_lang.clone()), vec!["x".into()]);
    app.translate_on = true;
    assert!(app.translation().is_some());
    app.toggle_skip("sv");
    assert_eq!(app.skip_langs(), vec!["sv"]);
    assert!(app.translation().is_none());
    app.toggle_skip("SV");
    assert!(app.skip_langs().is_empty());
}

#[test]
fn dict_chrome_response_parses() {
    let a: serde_json::Value = serde_json::from_str(r#"{"sentences":[{"trans":"Hi ","orig":"x"},{"trans":"there"}]}"#).unwrap();
    assert_eq!(translate::parse_dict(&a).as_deref(), Some("Hi there"));
    let b: serde_json::Value = serde_json::from_str(r#"["Hi"]"#).unwrap();
    assert_eq!(translate::parse_dict(&b).as_deref(), Some("Hi"));
}

#[test]
fn cjk_wrapping_keeps_punctuation_off_line_starts() {
    let rows = ui::wrap("今日は、いい天気です。", 8);
    assert!(rows.iter().all(|r| crate::util::width(r) <= 8), "{rows:?}");
    assert!(rows.iter().all(|r| !r.starts_with(['、', '。'])), "{rows:?}");
    assert_eq!(rows.concat(), "今日は、いい天気です。");
    let big = bigfont::render_scaled("HI", 60, 3, 2).unwrap();
    assert_eq!(big.len(), 8);
}

#[test]
fn hangul_big_font_renders_blocks() {
    // one 8-pixel-tall glyph row group = 4 text rows; "안녕" = 2 × 9 − 1 columns
    let rows = bigfont::render("안녕", 60, 3).unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|r| r.chars().count() == 17), "{rows:?}");
    assert!(rows.iter().any(|r| r.contains('█')));
    // scaling doubles both dimensions
    let big = bigfont::render_scaled("안녕", 60, 3, 2).unwrap();
    assert_eq!((big.len(), big[0].chars().count()), (8, 34));
    // every syllable composes (initials, vowel classes, finals, clusters)
    for c in ['가', '꿈', '봄', '왜', '읽', '값', '뷁', '힣', '쀍'] {
        assert!(bigfont::render(&c.to_string(), 20, 1).is_some(), "{c}");
    }
}

#[test]
fn mixed_latin_and_hangul_render_together() {
    // H(6) I(6) space(3) 안(9) 녕(8, no trailing gap)
    let rows = bigfont::render("Hi 안녕", 60, 3).unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|r| r.chars().count() == 6 + 6 + 3 + 9 + 8), "{rows:?}");
    // ideographs and Arabic still fall back to normal text
    assert!(bigfont::render("你好", 60, 3).is_none());
    assert!(bigfont::render("مرحبا", 60, 3).is_none());
    assert!(bigfont::render_compact("안녕", 60, 3).is_none());
}

#[test]
fn compact_font_takes_over_when_the_normal_one_is_too_wide() {
    // "HELLO" is 29 columns normally, 19 compact
    assert!(bigfont::render_scaled("HELLO", 20, 4, 1).is_none());
    let rows = bigfont::render_compact("HELLO", 20, 4).unwrap();
    assert_eq!((rows.len(), rows[0].chars().count()), (3, 19));
    let stage = ui::big_stage("Hello world", 20, 12, 3, 0).unwrap();
    assert_eq!(stage.len(), 3 + 1 + 3);
    // too short for two pieces plus room for neighbours: bold text instead
    assert!(ui::big_stage("Hello world", 20, 8, 3, 0).is_none());
    // wide panels still get the largest scale that fits
    assert_eq!(ui::big_stage("HI", 60, 40, 3, 0).unwrap().len(), 12);
    assert_eq!(ui::big_stage("HI", 60, 12, 3, 0).unwrap().len(), 8);
    // a spare row for the member tag can shrink the stage by one step
    assert_eq!(ui::big_stage("HI", 60, 10, 3, 0).unwrap().len(), 8);
    assert_eq!(ui::big_stage("HI", 60, 10, 3, 1).unwrap().len(), 4);
}

#[test]
fn member_tag_shows_above_the_big_line_and_never_overlaps() {
    use clap::Parser;
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let lines: Vec<String> = ["first line", "HI THERE", "third line"].map(String::from).to_vec();
    let aligned = chords::Aligned {
        per_line: vec![vec![], vec![(0, "Am".into())], vec![]],
        matched: 1,
        total: 3,
        flats: false,
        intro: vec![],
        gaps: vec![vec![], vec![], vec![]],
    };
    let members = ["Jimin".to_string(), "V".to_string()];
    let run = |w: u16, h: u16, with_chords: bool, members: &[String], reserve: bool| -> (Vec<String>, Vec<Option<usize>>) {
        let fc = ui::Focus {
            lines: &lines,
            current: Some(1),
            gap: 0,
            scale: 1,
            chords: with_chords.then_some(&aligned),
            tr: 0,
            flats: false,
            context: 99,
            in_break: false,
            members,
            reserve_tag: reserve,
            tag_below: false,
            progress: None,
        };
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Vec::new();
        term.draw(|f| hits = ui::draw_focus(f, Rect::new(0, 0, w, h), &theme, &fc)).unwrap();
        let b = term.backend().buffer();
        ((0..h).map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect::<String>()).collect(), hits)
    };
    let row_of = |text: &[String], needle: &str| text.iter().position(|r| r.contains(needle));
    // without chords: the tag is the row right above the big glyphs (the stage's first row)
    let (text, hits) = run(40, 24, false, &members, true);
    let tag = row_of(&text, "[Jimin, V]").expect("tag drawn");
    assert_eq!(hits[tag], None, "the tag row belongs to no lyric line");
    let stage_top = text.iter().position(|r| r.contains('█') || r.contains('▀') || r.contains('▄')).unwrap();
    assert_eq!(tag + 1, stage_top, "{text:#?}");
    // with chords: chord summary directly above the big text, the tag above that
    let (text, _) = run(40, 24, true, &members, true);
    let (tag, chord) = (row_of(&text, "[Jimin, V]").unwrap(), row_of(&text, "Am").unwrap());
    assert_eq!(tag + 1, chord, "{text:#?}");
    // the earlier line stays above the tag, nothing is overwritten
    let first = row_of(&text, "first line").unwrap();
    assert!(first < tag, "{text:#?}");
    // no member data: nothing drawn, and the stage keeps its place for lines without a tag
    let (text, _) = run(40, 24, false, &[], false);
    assert!(row_of(&text, "[").is_none(), "{text:#?}");
    // narrow panels shorten the tag, whole or with an ellipsis, and never overflow the row
    for w in [6u16, 8, 10, 12, 14, 20] {
        let (text, _) = run(w, 24, true, &members, true);
        for r in &text {
            assert_eq!(r.chars().count(), w as usize);
        }
        if let Some(t) = text.iter().find(|r| r.contains('[') && r.contains(']')) {
            let t = t.trim();
            assert!(t.starts_with('[') && t.ends_with(']'), "w={w} {t:?}");
        }
    }
    let three = ["Jimin".to_string(), "V".to_string(), "Jungkook".to_string()];
    let (text, _) = run(12, 24, false, &three, true);
    assert!(text.iter().any(|r| r.contains("[Jimin +2]")), "{text:#?}");
    let (text, _) = run(7, 24, false, &three, true);
    assert!(text.iter().any(|r| r.contains("[Jimin]")), "{text:#?}");
    let (text, _) = run(6, 24, false, &three, true);
    assert!(text.iter().any(|r| r.contains("[Jim…]")), "{text:#?}");
    // a short panel keeps what it can: no panic, the tag only appears where a row is free
    for h in 1..=14u16 {
        let (text, _) = run(30, h, true, &members, true);
        assert_eq!(text.len(), h as usize);
    }
}

#[test]
fn focus_layout_puts_chord_rows_on_neighbours_without_hits() {
    use clap::Parser;
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let lines: Vec<String> = ["first line here", "second line here", "third line here"].map(String::from).to_vec();
    let aligned = chords::Aligned {
        per_line: vec![vec![(0, "Am".into())], vec![(0, "G".into())], vec![(0, "C".into())]],
        matched: 3,
        total: 3,
        flats: false,
        intro: vec![vec!["Dm".into()]],
        gaps: vec![vec![vec!["F".into(), "E".into()]], vec![], vec![]],
    };
    let fc = |current, in_break| ui::Focus {
        lines: &lines,
        current,
        gap: 0,
        scale: 1,
        chords: Some(&aligned),
        tr: 0,
        flats: false,
        context: 99,
        in_break,
        members: &[],
        reserve_tag: false,
        tag_below: false,
        progress: None,
    };
    let mut term = Terminal::new(TestBackend::new(40, 24)).unwrap();
    let area = Rect::new(0, 0, 40, 24);
    let mut hits = Vec::new();
    term.draw(|f| hits = ui::draw_focus(f, area, &theme, &fc(Some(1), false))).unwrap();
    let screen = |t: &Terminal<TestBackend>| -> Vec<String> {
        let b = t.backend().buffer();
        (0..24).map(|y| (0..40).map(|x| b[(x, y)].symbol()).collect::<String>()).collect()
    };
    let text = screen(&term);
    let row_of = |text: &[String], needle: &str| text.iter().position(|r| r.contains(needle)).unwrap();
    // the next line's chord sits directly above its text and maps to no lyric line
    let third = row_of(&text, "third line here");
    assert!(text[third - 1].contains('C'));
    assert_eq!(hits[third - 1], None);
    assert_eq!(hits[third], Some(2));
    let first = row_of(&text, "first line here");
    assert_eq!(hits[first], Some(0));
    assert_eq!(hits[first - 1], None);
    assert!(text[first - 1].contains("Am"));
    // break chords show under the stage only during a break
    assert!(!text.iter().any(|r| r.contains("F   E")));
    term.draw(|f| hits = ui::draw_focus(f, area, &theme, &fc(Some(0), true))).unwrap();
    let text = screen(&term);
    let brk = row_of(&text, "F   E");
    assert_eq!(hits[brk], None);
    // before the first line the intro chords take the stage, lines follow below
    term.draw(|f| hits = ui::draw_focus(f, area, &theme, &fc(None, false))).unwrap();
    let text = screen(&term);
    let intro = row_of(&text, "Dm");
    assert_eq!(hits[intro], None);
    assert!(row_of(&text, "first line here") > intro);
}

#[tokio::test]
async fn local_files_fall_back_to_their_own_tags() {
    use clap::Parser;
    assert_eq!(tags::title_from_url("file:///music/My%20Song.flac"), "My Song");
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
    let url = "file:///music/My%20Song.flac";
    app.local_tags.insert(
        url.into(),
        tags::Tags { title: "Real Title".into(), artist: "Band".into(), album: "LP".into(), year: "1999".into(), track: Some(3), ..Default::default() },
    );
    let mut s = snap("", "My Song", "");
    s.url = url.into();
    app.apply(AppEvent::Player(Some(s)));
    let shown = app.snap.as_ref().unwrap();
    assert_eq!((shown.title.as_str(), shown.artist.as_str(), shown.album.as_str()), ("Real Title", "Band", "LP"));
    assert_eq!((shown.content_created.as_str(), shown.track_no), ("1999", Some(3)));
}

#[test]
fn every_menu_row_saves_a_known_config_key() {
    for set in app::Setting::ALL {
        for k in set.config_keys() {
            // `ai.model` lives in the [ai] table
            let known = k.strip_prefix("ai.").map_or_else(
                || config::KEYS.iter().any(|c| c.name == *k),
                |sub| ["provider", "model", "endpoint", "api_key_env"].contains(&sub),
            );
            assert!(known, "{k}");
        }
        assert!(!set.label().is_empty());
    }
}

#[test]
fn base_title_strips_credits_and_editions() {
    let b = year::base_title;
    assert_eq!(b("Song (feat. Someone)"), "Song");
    assert_eq!(b("Song ft. Someone"), "Song");
    assert_eq!(b("Song - Demo"), "Song");
    assert_eq!(b("Song - Remastered 2011"), "Song");
    assert_eq!(b("Song [Live] (Remix)"), "Song");
    assert_eq!(b("(Intro)"), "(Intro)");
    assert_eq!(b("Plain"), "Plain");
}

#[test]
fn language_versions_are_their_own_track() {
    let k = |t, al| snap("TXT", t, al).key();
    let original = k("Cat & Dog", "Cat & Dog");
    let english = k("Cat & Dog", "Cat & Dog (English Version)");
    assert_ne!(original, english);
    assert_eq!(english, k("Cat & Dog (English Ver.)", "Anything"));
    assert_ne!(english, k("Cat & Dog", "Cat & Dog (Japanese Version)"));
    assert_ne!(original, k("Cat & Dog - Instrumental", "Cat & Dog"));
    // editions of the same recording still share one
    assert_eq!(original, k("Cat & Dog", "Cat & Dog (Deluxe Edition)"));
    assert_eq!(original, k("Cat & Dog - Remastered 2021", "Cat & Dog"));
    assert_eq!(util::version_marker("Song", "Album"), None);
}

#[test]
fn chords_follow_the_translation() {
    let cs = vec![(0, "Am".to_string()), (12, "G".to_string())];
    // "walking down the road" -> two chords at the start of word 1 and word 3
    let out = ui::remap_chords(&cs, "walking down the road", "gå ner för vägen idag");
    assert_eq!(out[0], (0, "Am".to_string()));
    assert!(out[1].0 >= 6 && out[1].0 <= 13, "{out:?}");
    // always lands on a word start of the translated line
    let starts = [0, 3, 7, 11, 18];
    assert!(out.iter().all(|c| starts.contains(&c.0)), "{out:?}");
}

#[test]
fn solo_chords_are_kept_for_the_break() {
    let sheet = chords::parse_chordpro(
        "[Am]walking down the [G]road tonight\n[C]  [G]  [Am]  [F]\n[Am]standing in the [G]rain again\n",
    );
    let lyrics = vec![
        (1.0, "walking down the road tonight".to_string()),
        (30.0, "standing in the rain again".to_string()),
    ];
    let a = chords::align(&sheet, &lyrics);
    assert_eq!(a.per_line[0].len(), 2);
    assert_eq!(a.gaps[0], vec![vec!["C", "G", "Am", "F"]]);
    assert!(a.gaps[1].is_empty());
}

fn test_app() -> app::App {
    util::enter_test_mode();
    use clap::Parser;
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx)
}

#[test]
fn spotify_pkce_helpers() {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(hex(&spotify::sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(hex(&spotify::sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    // two blocks
    let long = vec![b'a'; 1000];
    assert_eq!(hex(&spotify::sha256(&long)), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
    assert_eq!(spotify::b64url(b"hello"), "aGVsbG8");
    assert_eq!(spotify::b64url(&[0xfb, 0xff]), "-_8");
    // RFC 7636 appendix B
    let challenge = spotify::b64url(&spotify::sha256(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"));
    assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

#[test]
fn spotify_state_becomes_a_snapshot() {
    let p = serde_json::json!({
        "is_playing": true, "progress_ms": 61500, "shuffle_state": true, "repeat_state": "context",
        "device": {"name": "Pixel 8", "volume_percent": 40},
        "context": {"uri": "spotify:playlist:abc"},
        "item": {
            "uri": "spotify:track:t1", "name": "Song", "duration_ms": 200000,
            "track_number": 3, "disc_number": 1,
            "artists": [{"name": "A"}, {"name": "B"}],
            "album": {"name": "LP", "release_date": "2019-05-03", "images": [
                {"url": "big", "width": 640}, {"url": "mid", "width": 300}, {"url": "small", "width": 64}
            ]}
        }
    });
    let r = spotify::parse_state(&p, |n| n == "Living Room PC").unwrap();
    let s = &r.snap;
    assert!(!r.local && r.device == "Pixel 8");
    assert_eq!((s.title.as_str(), s.artist.as_str(), s.album.as_str()), ("Song", "A, B", "LP"));
    assert_eq!((s.position, s.length, s.track_no), (61.5, 200.0, Some(3)));
    assert_eq!((s.art_url.as_str(), s.volume, s.shuffle), ("mid", Some(0.4), Some(true)));
    assert_eq!((s.status, s.repeat), (player::Status::Playing, Some(player::Loop::Playlist)));
    assert_eq!(s.player, spotify::REMOTE);
    assert!(spotify::parse_state(&serde_json::Value::Null, |_| false).is_none());
    assert!(spotify::parse_state(&serde_json::json!({"item": null}), |_| false).is_none());
}

#[test]
fn spotify_track_uris_from_mpris_ids() {
    assert_eq!(spotify::track_uri("spotify:track:abc123").as_deref(), Some("spotify:track:abc123"));
    assert_eq!(spotify::track_uri("/com/spotify/track/abc123").as_deref(), Some("spotify:track:abc123"));
    assert_eq!(spotify::track_uri("/spotify/track/abc123").as_deref(), Some("spotify:track:abc123"));
    assert_eq!(spotify::track_uri("/org/mpris/MediaPlayer2/Track/1"), None);
    assert_eq!(spotify::track_uri(""), None);
}

#[test]
fn spotify_recent_contexts_are_distinct() {
    let p = serde_json::json!({"items": [
        {"track": {"album": {"uri": "spotify:album:x"}}, "context": {"uri": "spotify:playlist:p", "type": "playlist"}},
        {"track": {"album": {"uri": "spotify:album:x"}}, "context": {"uri": "spotify:playlist:p", "type": "playlist"}},
        {"track": {"album": {"uri": "spotify:album:y"}}, "context": null},
        {"track": {"album": {"uri": "spotify:album:z"}}, "context": {"uri": "spotify:user:me:collection", "type": "collection"}},
        {"track": {"album": {"uri": "spotify:album:y"}}},
    ]});
    let got: Vec<(String, String)> = spotify::recent_contexts(&p).into_iter().map(|(k, u, _)| (k, u)).collect();
    let want = [
        ("playlist", "spotify:playlist:p"),
        ("album", "spotify:album:y"),
        ("collection", "spotify:user:me:collection"),
    ];
    assert_eq!(got, want.map(|(k, u)| (k.to_string(), u.to_string())));
}

#[test]
fn spotify_artist_rows_newest_first() {
    let a = |n: &str, d: &str| serde_json::json!({"uri": format!("spotify:album:{n}"), "name": n, "release_date": d});
    let (summary, rows) = spotify::artist_rows(vec![a("old", "2001"), a("new", "2020-01-01")], vec![a("s", "2023")]);
    assert_eq!(summary, "2 albums · 1 single");
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["new", "old", "s"]);
    assert_eq!(rows[0].sub, "2020 · Album");
    assert_eq!(rows[2].sub, "2023 · Single");
}

#[test]
fn spotify_errors_read_like_spotify_wrote_them() {
    let e = spotify::api_error(404, r#"{"error": {"status": 404, "message": "Player command failed: No active device found", "reason": "NO_ACTIVE_DEVICE"}}"#);
    assert_eq!((e.msg.as_str(), e.status), ("NO_ACTIVE_DEVICE", 404));
    let e = spotify::api_error(400, r#"{"error": "invalid_grant", "error_description": "Refresh token revoked"}"#);
    assert_eq!(e.msg, "Refresh token revoked");
    assert_eq!(spotify::api_error(502, "").msg, "HTTP 502");
}

#[tokio::test]
async fn remote_playback_shows_only_without_an_mpris_player() {
    let mut app = test_app();
    app.args.spotify = true;
    app.start_spotify();
    let remote = |title: &str| {
        let p = serde_json::json!({"is_playing": true, "device": {"name": "Phone"},
            "item": {"uri": "spotify:track:r", "name": title, "duration_ms": 100000, "artists": [{"name": "X"}], "album": {"name": "Y"}}});
        AppEvent::Spotify(spotify::Reply::Remote(Ok(spotify::parse_state(&p, |_| false).map(Box::new))))
    };
    // no MPRIS player: the phone's song is shown and controls go to the API
    app.apply(AppEvent::Player(None));
    app.apply(remote("Phone Song"));
    assert_eq!(app.snap.as_ref().unwrap().title, "Phone Song");
    assert!(app.remote_active());
    // the next empty playerctl poll keeps it
    app.apply(AppEvent::Player(None));
    assert!(app.remote_active());
    // an MPRIS player wins
    let mut local = snap("Band", "Local Song", "LP");
    local.player = "spotify".into();
    app.apply(AppEvent::Player(Some(local)));
    assert_eq!(app.snap.as_ref().unwrap().title, "Local Song");
    assert!(!app.remote_active());
    app.apply(remote("Phone Song"));
    assert_eq!(app.snap.as_ref().unwrap().title, "Local Song");
}

#[tokio::test]
async fn versions_of_a_playing_song_never_restart_it() {
    let mut app = test_app();
    let at = |mut s: player::Snapshot, pos: f64| {
        s.position = pos;
        s.player = "spotify".into();
        s
    };
    app.apply(AppEvent::Player(Some(at(snap("Band", "Song", "Album"), 10.0))));
    let key = app.snap.as_ref().unwrap().key();
    // a new track resets the lyric offset, so a kept offset means nothing restarted
    app.offset = 0.7;
    // the player flips between releases the strings don't reveal as the same
    for (i, (t, al)) in [
        ("Song - Live at Wembley", "Live at Wembley"),
        ("Song", "Greatest Hits"),
        ("Song (feat. Guest)", "Album: Deluxe"),
        ("Totally Different Name", "Album Super Deluxe Edition"),
    ]
    .into_iter()
    .enumerate()
    {
        app.apply(AppEvent::Player(Some(at(snap("Band", t, al), 10.2 + i as f64 * 0.2))));
        let shown = app.snap.as_ref().unwrap();
        assert_eq!((shown.title.as_str(), shown.album.as_str()), ("Song", "Album"), "{t}");
        assert_eq!(app.offset, 0.7, "track restarted on {t}");
    }
    assert_eq!(app.snap.as_ref().unwrap().key(), key);
    // a real next track (position back near 0) switches at once
    app.apply(AppEvent::Player(Some(at(snap("Band", "Next One", "Album"), 0.3))));
    assert_eq!(app.snap.as_ref().unwrap().title, "Next One");
    assert_eq!(app.offset, 0.0);
    // another player that isn't playing doesn't take over the display
    let mut other = at(snap("Someone", "Video", ""), 50.0);
    other.player = "firefox".into();
    other.status = player::Status::Paused;
    app.apply(AppEvent::Player(Some(other)));
    assert_eq!(app.snap.as_ref().unwrap().title, "Next One");
}

enum R {
    Http(u16, &'static str, String),
    Drop,
}

/// Scripted server: each route plays its responses in order, repeating the last one.
fn scripted_site(routes: Vec<(&'static str, Vec<R>)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log2 = log.clone();
    let counts = std::sync::Mutex::new(vec![0usize; routes.len()]);
    std::thread::spawn(move || {
        for mut st in l.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = st.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = req.lines().next().unwrap_or("").to_string();
            log2.lock().unwrap().push(first.clone());
            let Some(ri) = routes.iter().position(|(f, _)| first.contains(f)) else {
                let _ = write!(st, "HTTP/1.1 404 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                continue;
            };
            let mut c = counts.lock().unwrap();
            let i = c[ri].min(routes[ri].1.len() - 1);
            c[ri] += 1;
            match &routes[ri].1[i] {
                R::Drop => drop(st), // socket disconnect
                R::Http(code, hdr, body) => {
                    let _ = write!(st, "HTTP/1.1 {code} X\r\n{hdr}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                }
            }
        }
    });
    (base, log)
}

fn fast() -> lyrics::Policy {
    lyrics::Policy { retries: 3, backoff: Duration::from_millis(1) }
}

fn run_lyrics(base: &str, policy: lyrics::Policy) -> lyrics::Lyrics {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let c = reqwest::Client::builder().timeout(Duration::from_millis(400)).build().unwrap();
    rt.block_on(lyrics::fetch_with(&c, base, "Metallica", "Enter Sandman - Remastered", "Metallica (Deluxe)", 331.0, policy, false))
}

const LRC_JSON: &str = r#"{"syncedLyrics":"[00:01.00]hello\n[00:02.00]world","plainLyrics":"hello\nworld","duration":331}"#;

#[test]
fn lyrics_recover_from_429_disconnect_and_5xx() {
    let (base, log) = scripted_site(vec![(
        "api/get",
        vec![
            R::Http(429, "Retry-After: 0\r\n", String::new()),
            R::Drop,
            R::Http(503, "", String::new()),
            R::Http(200, "", LRC_JSON.into()),
        ],
    )]);
    let l = run_lyrics(&base, fast());
    assert!(matches!(l, lyrics::Lyrics::Synced(ref v) if v.len() == 2), "{l:?}");
    assert_eq!(log.lock().unwrap().len(), 4, "3 failures then success on the 4th attempt");
}

#[test]
fn lyrics_timeout_is_retried() {
    // first reply never arrives within the client timeout (server just sits on the socket)
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for (i, st) in l.incoming().flatten().enumerate() {
            let mut st = st;
            let mut b = [0u8; 4096];
            let _ = st.read(&mut b);
            if i == 0 {
                std::thread::sleep(Duration::from_millis(900)); // > client timeout
            } else {
                let _ = write!(st, "HTTP/1.1 200 X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{LRC_JSON}", LRC_JSON.len());
            }
        }
    });
    let l = run_lyrics(&base, fast());
    assert!(matches!(l, lyrics::Lyrics::Synced(_)), "{l:?}");
}

#[test]
fn lyrics_fallback_chain_and_not_found_vs_error() {
    // (a) exact (has album_name) → 404; (b) without album → hit
    let (base, log) = scripted_site(vec![
        ("album_name", vec![R::Http(404, "", String::new())]),
        ("api/get", vec![R::Http(200, "", LRC_JSON.into())]),
    ]);
    assert!(matches!(run_lyrics(&base, fast()), lyrics::Lyrics::Synced(_)));
    let calls = log.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[1].contains("track_name=Enter+Sandman") && !calls[1].contains("Remastered"), "{calls:?}");

    // (c) both gets 404 → search finds it
    let (base, _) = scripted_site(vec![
        ("api/get", vec![R::Http(404, "", String::new())]),
        ("api/search", vec![R::Http(200, "", format!("[{LRC_JSON}]"))]),
    ]);
    assert!(matches!(run_lyrics(&base, fast()), lyrics::Lyrics::Synced(_)));

    // 404 everywhere = "no lyrics exist", not a network error
    let (base, log) = scripted_site(vec![
        ("api/get", vec![R::Http(404, "", String::new())]),
        ("api/search", vec![R::Http(200, "", "[]".into())]),
    ]);
    assert!(matches!(run_lyrics(&base, fast()), lyrics::Lyrics::NotFound));
    assert!(log.lock().unwrap().len() <= 6, "404 is never retried");

    // everything fails → network error, after bounded effort (4 + 1 + 1…)
    let (base, log) = scripted_site(vec![("api/", vec![R::Http(500, "", String::new())])]);
    let l = run_lyrics(&base, fast());
    assert!(matches!(&l, lyrics::Lyrics::Error(e) if e.contains("server error")), "{l:?}");
    let n = log.lock().unwrap().len();
    assert!((6..=9).contains(&n), "first step retried 3x, later steps once each: {n}");

    // rate limited for good → says so
    let (base, _) = scripted_site(vec![("api/", vec![R::Http(429, "Retry-After: 0\r\n", String::new())])]);
    let l = run_lyrics(&base, fast());
    assert!(matches!(&l, lyrics::Lyrics::Error(e) if e.contains("rate limited")), "{l:?}");
}

#[test]
fn keys_and_camelot() {
    let k = |x: &str| chords::parse_key(x).unwrap();
    // the full wheel, checked against the standard Camelot table
    let major = [("C", "8B"), ("Db", "3B"), ("D", "10B"), ("Eb", "5B"), ("E", "12B"), ("F", "7B"), ("F#", "2B"), ("G", "9B"), ("Ab", "4B"), ("A", "11B"), ("Bb", "6B"), ("B", "1B")];
    let minor = [("Am", "8A"), ("Bbm", "3A"), ("Bm", "10A"), ("Cm", "5A"), ("C#m", "12A"), ("Dm", "7A"), ("Ebm", "2A"), ("Em", "9A"), ("Fm", "4A"), ("F#m", "11A"), ("Gm", "6A"), ("G#m", "1A")];
    for (n, c) in major.iter().chain(minor.iter()) {
        assert_eq!(k(n).camelot(), *c, "{n}");
    }
    assert_eq!(k("Em").long(), "E Minor");
    assert_eq!(k("E minor").camelot(), "9A");
    assert_eq!(k("D#m").short(), "Ebm");
    assert_eq!(k("Am").shift(3).short(), "Cm");
    assert_eq!(k("Am").shift(-12).short(), "Am");
    assert_eq!(k("C#").shift(-3).short(), "Bb");
    assert!(chords::parse_key("Hello").is_none() && chords::parse_key("Asus4").is_none());
    assert!(k("Dm").prefers_flats() && k("F").prefers_flats() && !k("A").prefers_flats() && !k("Em").prefers_flats());
}

#[test]
fn sheet_metadata_and_dates() {
    let s = chords::parse_text("Key: Em\nBPM: 120\nCapo 3\n   Am   G\nhello world\n");
    assert_eq!((s.key.as_deref(), s.bpm, s.capo), (Some("Em"), Some(120), 3));
    let c = chords::parse_chordpro("{key: F#m}\n{tempo: 83}\n[Am]x [G]y");
    assert_eq!((c.key.as_deref(), c.bpm), (Some("F#m"), Some(83)));
    let back = chords::parse_chordpro(&chords::to_chordpro(&c));
    assert_eq!((back.key.as_deref(), back.bpm), (Some("F#m"), Some(83)));
    // a lyric that happens to contain a colon is untouched
    let l = chords::parse_text("   Am\nKey to my heart: you\n");
    assert_eq!(l.lines[0].text, "Key to my heart: you");
    assert_eq!(year::full_date("1969-09-26T00:00:00Z").as_deref(), Some("1969-09-26"));
    assert_eq!(year::full_date("1969-09-26").as_deref(), Some("1969-09-26"));
    assert_eq!(year::full_date("1969"), None);
    assert_eq!(year::full_date("1969-00-00"), None);
    assert_eq!(year::full_date(""), None);
}

#[test]
fn intro_chords_before_first_lyric() {
    let sheet = chords::parse_chordpro("[Am] [G] [C] [F]\n[Am] [G]\n[Am]Hello darkness my old [G]friend\n[C]I've come to talk with [F]you again\n[Am]Because a vision [G]softly creeping\n[C]Left its seeds while I was [F]sleeping\n");
    let a = chords::align(&sheet, &lrc(&["Hello darkness my old friend", "I've come to talk with you again", "Because a vision softly creeping", "Left its seeds while I was sleeping"]));
    assert_eq!(a.intro, vec![vec!["Am", "G", "C", "F"], vec!["Am", "G"]]);
    assert!(a.usable());
    // no chord-only lines before the first lyric → no intro
    let b = chords::parse_chordpro("[Am]Hello darkness my old [G]friend\n[C]I've come to talk with [F]you again\n");
    let ab = chords::align(&b, &lrc(&["Hello darkness my old friend", "I've come to talk with you again"]));
    assert!(ab.intro.is_empty());
}

#[test]
fn chord_diagrams() {
    let f = |n: &str| diagram::lookup(n).map(|s| s.frets);
    assert_eq!(f("C"), Some([-1, 3, 2, 0, 1, 0]));
    assert_eq!(f("Am"), Some([-1, 0, 2, 2, 1, 0]));
    assert_eq!(f("E"), Some([0, 2, 2, 1, 0, 0]));
    assert_eq!(f("F"), Some([1, 3, 3, 2, 1, 1]));
    assert_eq!(f("Bb"), Some([-1, 1, 3, 3, 3, 1]));
    assert_eq!(f("F#m"), Some([2, 4, 4, 2, 2, 2]));
    assert_eq!(f("G"), Some([3, 2, 0, 0, 0, 3]));
    assert_eq!(f("A7"), Some([-1, 0, 2, 0, 2, 0]));
    assert_eq!(f("Em7"), Some([0, 2, 0, 0, 0, 0]));
    assert_eq!(f("A5"), Some([-1, 0, 2, 2, -1, -1]));
    assert_eq!(f("Dsus4"), Some([-1, -1, 0, 2, 3, 3]));
    assert_eq!(f("Asus2"), Some([-1, 0, 2, 2, 0, 0]));
    // enharmonics resolve to the same shape
    assert_eq!(f("A#"), f("Bb"));
    assert_eq!(f("Db"), f("C#"));
    // everything we claim to know is playable: within a 4-fret span and at least 3 strings
    for root in ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"] {
        for q in ["", "m", "7", "m7", "maj7", "sus4", "sus2", "5"] {
            let name = format!("{root}{q}");
            let Some(sh) = diagram::lookup(&name) else {
                assert_eq!(q, "sus2", "only the high sus2 shapes are left out: {name}");
                continue;
            };
            let fr: Vec<i8> = sh.frets.iter().copied().filter(|x| *x > 0).collect();
            if let (Some(lo), Some(hi)) = (fr.iter().min(), fr.iter().max()) {
                assert!(hi - lo <= 3, "{name} {sh:?}");
            }
            assert!(sh.frets.iter().filter(|x| **x >= 0).count() >= 3, "{name}");
            assert!(sh.frets.iter().all(|x| *x <= 12), "{name}");
        }
    }
    // unknown / slash chords: no diagram rather than a wrong one
    for n in ["Cdim", "D/F#", "Gadd9", "Em9", "Hello", ""] {
        assert!(diagram::lookup(n).is_none(), "{n}");
    }
    let c = diagram::render("C", &diagram::lookup("C").unwrap());
    assert_eq!(c[0].trim(), "C");
    assert_eq!(c[1], "x     o   o");
    assert_eq!(c[2], "╒═╤═╤═╤═╤═╕");
    assert_eq!(c.len(), 7);
    let tab = diagram::tab(&[diagram::lookup("C"), diagram::lookup("Am"), diagram::lookup("Cdim")]);
    assert_eq!(tab[0], "e|-0---0---?--|");
    assert_eq!(tab[5], "E|-x---x---?--|");
}

#[test]
fn capo_and_transposition_math() {
    // higher capo → lower shapes; wrap-around both ways
    assert_eq!(chords::capo_shift(2, 5, 0), -3);
    assert_eq!(chords::capo_shift(0, 0, 0), 0);
    assert_eq!(chords::capo_shift(3, 0, 1), 4);
    let t = |c: &str, n: i32, flats: bool| chords::transpose(c, n, flats);
    assert_eq!(t("Dm", -3, false), "Bm");
    assert_eq!(t("B", 1, false), "C");
    assert_eq!(t("C", -1, false), "B");
    assert_eq!(t("C", -13, false), "B");
    assert_eq!(t("A", 25, false), "A#");
    assert_eq!(t("A", 25, true), "Bb");
    // slash chords move both notes, spelling follows the key
    assert_eq!(t("D/F#", 1, false), "D#/G");
    assert_eq!(t("D/F#", 1, true), "Eb/G");
    assert_eq!(t("D/F#", -2, true), "C/E");
    assert_eq!(t("C#m7/G#", 2, true), "Ebm7/Bb");
    assert_eq!(t("Bb/D", 12, true), "Bb/D");
    // sounding key stays put when only the capo changes; the shapes' key moves
    let k = chords::parse_key("Dm").unwrap();
    let (sheet_capo, new_capo) = (2u8, 4u8);
    let sounding = k.shift(sheet_capo as i32);
    let shapes = k.shift(chords::capo_shift(sheet_capo, new_capo, 0));
    assert_eq!(sounding.short(), "Em");
    assert_eq!(shapes.short(), "Cm"); // capo 4 → play Cm shapes → sounds Em
    assert_eq!(shapes.shift(new_capo as i32).short(), sounding.short());
    assert!(shapes.prefers_flats()); // Cm → flat spelling
}

fn cfg_run(file: &str, argv: &[&str]) -> (cli::Args, Vec<String>) {
    let path = tmp("cfg").join(format!("{}.toml", util::fnv1a(file)));
    std::fs::write(&path, file).unwrap();
    let mut full = vec!["x", "--config", path.to_str().unwrap()];
    full.extend_from_slice(argv);
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(full);
    let mut args = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
    let mut warns = Vec::new();
    config::apply(&mut args, &m, &path, true, &mut |w| warns.push(w));
    (args, warns)
}

#[test]
fn translate_mode_config_flag_and_menu() {
    use cli::TranslateMode::{Both, Off, Translated};
    // default off; the old `translate` switch still means "translated"
    assert_eq!(cfg_run("", &[]).0.start_translate_mode(), Off);
    assert_eq!(cfg_run("translate = true\n", &[]).0.start_translate_mode(), Translated);
    // the file's mode, then the flag beating the file
    assert_eq!(cfg_run("translate_mode = \"both\"\n", &[]).0.start_translate_mode(), Both);
    let (a, w) = cfg_run("translate_mode = \"both\"\n", &["--translate-mode", "off"]);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(a.start_translate_mode(), Off);
    // --translate on the command line beats a mode from the file
    assert_eq!(cfg_run("translate_mode = \"both\"\n", &["--translate"]).0.start_translate_mode(), Translated);
    // a bad word is reported and ignored
    let (a, w) = cfg_run("translate_mode = \"sideways\"\n", &[]);
    assert_eq!(a.start_translate_mode(), Off);
    assert!(w.iter().any(|x| x.contains("translate_mode") && x.contains("off, both, translated")), "{w:?}");
    // the settings row cycles all three and wraps
    assert_eq!(Off.step(1), Both);
    assert_eq!(Both.step(1), Translated);
    assert_eq!(Translated.step(1), Off);
    assert_eq!(Off.step(-1), Translated);
}

#[tokio::test]
async fn translate_mode_row_drives_the_lyrics_panel() {
    use cli::TranslateMode::{Both, Off, Translated};
    let mut app = test_app();
    assert_eq!(app.translate_mode(), Off);
    app.change_setting(app::Setting::TranslateMode, 1);
    assert_eq!((app.translate_mode(), app.translate_both), (Both, true));
    assert_eq!(app.setting_value(app::Setting::TranslateMode), "< both >");
    app.change_setting(app::Setting::TranslateMode, 1);
    assert_eq!((app.translate_mode(), app.translate_both), (Translated, false));
    app.change_setting(app::Setting::TranslateMode, 1);
    assert_eq!(app.translate_mode(), Off);
    // both keys are written, so the file never contradicts itself
    let saved: Vec<_> = app.settings_values().into_iter().map(|(k, _)| k).collect();
    assert!(saved.contains(&"translate") && saved.contains(&"translate_mode"));
}

#[test]
fn config_precedence_and_validation() {
    let file = "player = \"spotifyd\"\noffset = -0.5\ntint = 0.25\ncava = true\ncava_bars = 32\nvolume_step = 10\ninterlude_threshold_secs = 12\nlyrics_retries = 5\nminimal_view = true\nmouse = false\nchords = true\nchords_ug = true\nthirdpanel = true\n";
    // file beats built-in defaults
    let (a, w) = cfg_run(file, &[]);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!((a.player.as_str(), a.offset, a.tint, a.cava, a.cava_bars), ("spotifyd", -0.5, 0.25, true, 32));
    assert_eq!((a.volume_step, a.interlude_secs, a.lyrics_retries), (10, 12.0, 5));
    assert!(a.minimal && a.no_mouse && a.chords && a.chords_ug && a.thirdpanel);
    // flags beat the file
    let (a, _) = cfg_run(file, &["--player", "vlc", "--tint", "0.9", "--volume-step", "2", "--lyrics-retries", "0"]);
    assert_eq!((a.player.as_str(), a.tint, a.volume_step, a.lyrics_retries), ("vlc", 0.9, 2, 0));
    assert_eq!(a.cava_bars, 32, "settings not given as flags still come from the file");
    // defaults when nothing is set
    let (a, w) = cfg_run("", &[]);
    assert!(w.is_empty());
    assert_eq!((a.player.as_str(), a.volume_step, a.interlude_secs, a.lyrics_retries, a.minimal, a.no_mouse), ("spotify", 5, 8.0, 3, false, false));
    // unknown key warns (with its line), valid keys still apply
    let (a, w) = cfg_run("tint = 0.3\nfoo = 1\n", &[]);
    assert_eq!(a.tint, 0.3);
    assert!(w.len() == 1 && w[0].contains(":2:") && w[0].contains("unknown key 'foo'"), "{w:?}");
    // invalid values: key + line reported, default kept, the rest unaffected
    let (a, w) = cfg_run("# c\ntint = 7\nvolume_step = \"loud\"\nlyrics_retries = -1\ncava = 1\nplayer = \"\"\noffset = 1.5\n", &[]);
    assert_eq!((a.tint, a.volume_step, a.lyrics_retries, a.cava, a.player.as_str(), a.offset), (1.0, 5, 3, false, "spotify", 1.5));
    assert_eq!(w.len(), 5, "{w:?}");
    assert!(w[0].contains(":2:") && w[0].contains("'tint'"));
    assert!(w[1].contains(":3:") && w[1].contains("'volume_step'"));
    // broken TOML: warns, everything stays default
    let (a, w) = cfg_run("tint = = 3\n", &[]);
    assert_eq!(a.tint, 1.0);
    assert!(w.len() == 1 && w[0].contains("ignoring the file"), "{w:?}");
    // a missing default file is silent, a missing explicit one is not
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x"]);
    let mut a = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
    let mut w = Vec::new();
    let nowhere = tmp("cfg").join("nope.toml");
    assert!(!config::apply(&mut a, &m, &nowhere, false, &mut |x| w.push(x)));
    assert!(w.is_empty());
    config::apply(&mut a, &m, &nowhere, true, &mut |x| w.push(x));
    assert_eq!(w.len(), 1);
    // print-config round-trips through the parser
    let text = config::render(&a, &nowhere, false);
    let table: toml::Table = text.parse().unwrap();
    assert!(config::KEYS.iter().all(|k| table.contains_key(k.name) || k.name == "chords_dir" || k.name == "translate_fallback_key" || k.name == "genius_token" || k.name == "spotify_client_id"), "{text}");
    assert!(text.contains("# chords_dir"));
}

#[test]
fn keymap_has_no_conflicts_and_help_is_generated_from_it() {
    use app::{Action as A, BINDS, GROUPS, MOUSE};
    use crossterm::event::KeyCode;
    // no key is bound twice (m / M, t / T are different keys)
    let mut seen = std::collections::HashSet::new();
    for b in BINDS {
        for c in b.codes {
            assert!(seen.insert(format!("{c:?}")), "{c:?} bound twice");
        }
    }
    assert_eq!(app::action_for(KeyCode::Char('m')), Some(A::Mute));
    assert_eq!(app::action_for(KeyCode::Char('M')), Some(A::Minimal));
    assert_eq!(app::action_for(KeyCode::Char('?')), Some(A::Help));
    assert_eq!(app::action_for(KeyCode::Esc), Some(A::Quit));
    assert_eq!(app::action_for(KeyCode::Char('z')), Some(A::Shuffle));
    assert_eq!(app::action_for(KeyCode::Char('y')), None);
    assert_eq!(app::action_for(KeyCode::Char('x')), Some(A::BigScale));
    assert_eq!(app::action_for(KeyCode::Char('i')), Some(A::LineSpacing));
    assert_eq!(app::action_for(KeyCode::Char('u')), Some(A::LyricPad));
    assert_eq!(app::action_for(KeyCode::Char('o')), Some(A::Library));
    assert_eq!(app::action_for(KeyCode::Char('f')), Some(A::Like));
    assert_eq!(app::action_for(KeyCode::Char('H')), Some(A::PlayHere));
    assert_eq!(app::action_for(KeyCode::Char('d')), Some(A::Devices));
    assert_eq!(app::action_for(KeyCode::Char('D')), Some(A::Diagrams), "capital D is still the chord diagrams");
    // every action can be triggered by some key
    for a in [
        A::Quit, A::PlayPause, A::Next, A::Prev, A::Shuffle, A::Repeat, A::SeekBack, A::SeekFwd, A::SyncEarlier,
        A::SyncLater, A::VolUp, A::VolDown, A::Mute, A::ToggleLyrics, A::ToggleBig,
        A::ToggleCava, A::ToggleChords, A::TransposeUp, A::TransposeDown, A::CapoDown,
        A::CapoUp, A::CapoReset, A::Diagrams, A::Minimal, A::Help, A::Library, A::Like, A::PlayHere, A::Devices,
        A::BigScale, A::LineSpacing, A::LyricPad,
    ] {
        assert!(BINDS.iter().any(|b| b.action == a), "{a:?} has no key");
    }
    assert!(BINDS.iter().all(|b| GROUPS.contains(&b.group)));
    assert!(MOUSE.iter().all(|m| GROUPS.contains(&m.0)));
    // the overlay lists every row of both tables
    let th = theme_from(&["--terminal-colors"], None);
    let text: String = ui::help_blocks(&th)
        .iter()
        .flatten()
        .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>() + "\n")
        .collect();
    for b in BINDS {
        assert!(text.contains(b.keys) && text.contains(b.desc), "{} missing from help", b.keys);
    }
    for m in MOUSE {
        assert!(text.contains(m.1) && text.contains(m.2));
    }
}


#[test]
fn korean_song_replaces_romanized_hit_with_hangul() {
    let roman = r#"{"syncedLyrics":"[00:01.00]saranghae neomu\n[00:05.00]gaji ma","duration":200}"#;
    let hangul = r#"{"syncedLyrics":"[00:01.00]사랑해 너무\n[00:05.00]가지 마","duration":200}"#;
    let run = |artist: &str, title: &str, base: &str| {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let c = reqwest::Client::new();
        rt.block_on(lyrics::fetch_with(&c, base, artist, title, "", 200.0, fast(), false))
    };
    let site = || {
        scripted_site(vec![
            ("api/get", vec![R::Http(200, "", roman.into())]),
            ("api/search", vec![R::Http(200, "", format!("[{roman},{hangul}]"))]),
        ])
    };
    // Korean title: the romanized exact hit is not final, the search runs and Hangul wins
    let (base, log) = site();
    let l = run("방탄소년단", "사랑해", &base);
    assert!(matches!(&l, lyrics::Lyrics::Synced(v) if v[0].1.starts_with("사랑")), "{l:?}");
    assert!(log.lock().unwrap().iter().any(|r| r.contains("api/search")));
    // not Korean: the first synced hit is kept and no search happens
    let (base, log) = site();
    let l = run("Someone", "Saranghae", &base);
    assert!(matches!(&l, lyrics::Lyrics::Synced(v) if v[0].1.starts_with("sarang")), "{l:?}");
    assert!(!log.lock().unwrap().iter().any(|r| r.contains("api/search")));
}

#[test]
fn meta_grid_aligns_the_right_column() {
    let sp = |s: &str| vec![ratatui::text::Span::raw(s.to_string())];
    let text = |l: &ratatui::text::Line| l.spans.iter().map(|s| s.content.to_string()).collect::<String>();
    let left = || vec![sp("Track No  01-03"), sp("Release   1991-08-12")];
    let right = |n: usize| {
        ["Tuning: Drop D", "Capo: 2 (Am)", "BPM: 120", "Key: E Minor / 9A"]
            .iter()
            .take(n)
            .map(|s| sp(s))
            .collect::<Vec<_>>()
    };
    let rows: Vec<String> = ui::meta_grid(left(), right(4), 40).iter().map(text).collect();
    // start column = 40 - widest right item (17)
    assert_eq!(rows[0], format!("{:<23}Tuning: Drop D", "Track No  01-03"));
    assert_eq!(rows[1], format!("{:<23}Capo: 2 (Am)", "Release   1991-08-12"));
    assert_eq!(rows[2], format!("{:<23}BPM: 120", ""));
    assert_eq!(rows[3], format!("{:<23}Key: E Minor / 9A", ""));
    assert_eq!(rows.len(), 4);
    // only the key: one right row, left side untouched below it
    let rows: Vec<String> = ui::meta_grid(left(), vec![sp("Key: E Minor / 9A")], 40).iter().map(text).collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].ends_with("Key: E Minor / 9A") && rows[0].chars().count() == 40);
    assert_eq!(rows[1], "Release   1991-08-12");
    // nothing on the right
    assert_eq!(ui::meta_grid(left(), vec![], 40).len(), 2);
    // too narrow: the right items follow below, indented under the values
    let rows: Vec<String> = ui::meta_grid(left(), right(2), 30).iter().map(text).collect();
    assert_eq!(rows, ["Track No  01-03", "Release   1991-08-12", "             Tuning: Drop D", "             Capo: 2 (Am)"]);
}

#[test]
fn shuffle_and_loop_output_is_parsed() {
    use player::Loop;
    assert_eq!(player::parse_shuffle("On\n"), Some(true));
    assert_eq!(player::parse_shuffle("Off\n"), Some(false));
    assert_eq!(player::parse_shuffle("No players found\n"), None);
    assert_eq!(player::parse_shuffle(""), None);
    assert_eq!(player::parse_loop("None\n"), Some(Loop::None));
    assert_eq!(player::parse_loop("Track\n"), Some(Loop::Track));
    assert_eq!(player::parse_loop("Playlist\n"), Some(Loop::Playlist));
    assert_eq!(player::parse_loop("Some error"), None);
    // off -> playlist -> track -> off, with the argument playerctl expects
    assert_eq!(Loop::None.next(), Loop::Playlist);
    assert_eq!(Loop::Playlist.next(), Loop::Track);
    assert_eq!(Loop::Track.next(), Loop::None);
    assert_eq!(Loop::Playlist.arg(), "Playlist");
    // the metadata format carries neither; the poller stamps them on afterwards
    assert!(snap("A", "T", "Al").shuffle.is_none() && snap("A", "T", "Al").repeat.is_none());
}

#[test]
fn member_parts_keys_and_shuffle_binds_exist() {
    assert!(app::Setting::ALL.contains(&app::Setting::MemberParts));
    assert_eq!(app::Setting::MemberParts.config_keys(), ["member_parts"]);
    assert_eq!(app::action_for(crossterm::event::KeyCode::Char('z')), Some(app::Action::Shuffle));
    assert_eq!(app::action_for(crossterm::event::KeyCode::Char('r')), Some(app::Action::Repeat));
    let (a, _) = cfg_run("member_parts = true\n", &[]);
    assert!(a.member_parts);
    let (a, _) = cfg_run("member_parts = true\n", &["--member-parts"]);
    assert!(a.member_parts);
    let (a, _) = cfg_run("", &[]);
    assert!(!a.member_parts, "off by default");
}

#[test]
fn genius_headers_become_member_lists() {
    let h = |s: &str| genius::parse_header(s);
    assert_eq!(h("[Verse 1: Jimin]"), Some(vec!["Jimin".to_string()]));
    assert_eq!(h("[Chorus: Jungkook, V]"), Some(vec!["Jungkook".to_string(), "V".to_string()]));
    assert_eq!(h("[Pre-Chorus: RM & SUGA]"), Some(vec!["RM".to_string(), "SUGA".to_string()]));
    assert_eq!(h("[Bridge: Jin (진) and J-Hope]"), Some(vec!["Jin".to_string(), "J-Hope".to_string()]));
    assert_eq!(h("[Outro]"), Some(vec![]));
    assert_eq!(h("just a lyric"), None);
}

const GENIUS_PAGE: &str = r#"<html><body>
<div class="Lyrics__Container" data-lyrics-container="true" class="x"><div data-exclude-from-selection="true"><div>Contributors</div></div>[Intro]<br/>Ooh<br/><a href="/1"><span>[Verse 1: Jimin]</span></a><br/>I&#x27;m in love &amp; blind<br/>Walking down the road<br/><br/>[Chorus: Jungkook, V]<br/>Stay with me tonight<br/></div>
<div data-lyrics-container="true">[Verse 2: RM &amp; SUGA]<br>Rain on the window<br>Nothing left to say</div>
<div class="footer">[Verse 9: Nobody]<br>ignored</div></body></html>"#;

fn mp(members: &[&str], text: &str) -> genius::Part {
    genius::Part { members: members.iter().map(|m| m.to_string()).collect(), text: text.into() }
}

#[test]
fn genius_page_parses_sections_and_inherits_members() {
    let parts = genius::parse_page(GENIUS_PAGE);
    assert_eq!(
        parts,
        vec![
            mp(&[], "Ooh"),
            mp(&["Jimin"], "I'm in love & blind"),
            mp(&["Jimin"], "Walking down the road"),
            mp(&["Jungkook", "V"], "Stay with me tonight"),
            mp(&["RM", "SUGA"], "Rain on the window"),
            mp(&["RM", "SUGA"], "Nothing left to say"),
        ]
    );
}

#[test]
fn genius_lines_are_attributed_to_synced_lines_in_order() {
    let parts = genius::parse_page(GENIUS_PAGE);
    let lyrics: Vec<String> = [
        "Ooh",
        "I'm in love and blind", // wording differs slightly
        "something the page doesn't have",
        "Walking down the road",
        "Stay with me tonight",
        "Rain on the window",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let m = genius::attribute(&parts, &lyrics);
    assert!(m[0].is_empty(), "intro line has no member");
    assert_eq!(m[1], ["Jimin"]);
    assert!(m[2].is_empty());
    assert_eq!(m[3], ["Jimin"]);
    assert_eq!(m[4], ["Jungkook", "V"]);
    assert_eq!(m[5], ["RM", "SUGA"]);
    // stable colour per name, whatever the case
    assert_eq!(ui::member_color("Jimin"), ui::member_color("jimin"));
}

#[test]
fn genius_lookup_with_fake_site_and_cache() {
    let search = serde_json::json!({"meta":{"status":200},"response":{"sections":[
        {"type":"top_hit","hits":[{"type":"song","result":{"title":"Other Song","path":"/Bts-other-lyrics","primary_artist":{"name":"BTS"}}}]},
        {"type":"song","hits":[
            {"type":"song","result":{"title":"Dynamite (Türkçe Çeviri)","path":"/Genius-tr-dynamite-lyrics","primary_artist":{"name":"Genius Türkçe Çeviri"}}},
            {"type":"song","result":{"title":"Dynamite","url":"https://genius.com/Bts-dynamite-lyrics","primary_artist":{"name":"BTS"}}}
        ]}]}});
    let artist = "BTS";
    let title = "Dynamite GENIUSTEST";
    assert_eq!(genius::best_hit(&search, artist, "Dynamite").as_deref(), Some("/Bts-dynamite-lyrics"));
    assert_eq!(genius::best_hit(&search, artist, "Totally Different"), None);
    assert_eq!(genius::best_hit(&search, "Someone Else", "Dynamite"), None);

    let cache = util::cache_dir().join(format!("members-{:016x}.json", util::fnv1a(&format!("{artist}\0{title}"))));
    let _ = std::fs::remove_file(&cache);
    let (base, hits) = fake_site(vec![
        ("search/multi", 200, search.to_string()),
        ("Bts-dynamite-lyrics", 200, GENIUS_PAGE.into()),
    ]);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let c = reqwest::Client::new();
    // the fake title differs from the hit's ("Dynamite GENIUSTEST" vs "Dynamite"), so look up the plain title
    let title2 = "Dynamite";
    let cache2 = util::cache_dir().join(format!("members-{:016x}.json", util::fnv1a(&format!("{artist}\0{title2}"))));
    let _ = std::fs::remove_file(&cache2);
    let got = rt.block_on(genius::lookup(&c, &base, artist, title2)).unwrap().expect("parts");
    assert_eq!(got.len(), 6);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2, "search + song page");
    // second time: straight from the cache
    let again = rt.block_on(genius::lookup(&c, &base, artist, title2)).unwrap().expect("cached");
    assert_eq!(again, got);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    let _ = std::fs::remove_file(&cache2);

    // nothing found: remembered as a miss; a server error is an Err and not cached
    let _ = std::fs::remove_file(&cache);
    let (base2, hits2) = fake_site(vec![("search/multi", 200, r#"{"response":{"sections":[]}}"#.into())]);
    assert_eq!(rt.block_on(genius::lookup(&c, &base2, artist, title)).unwrap(), None);
    assert_eq!(rt.block_on(genius::lookup(&c, &base2, artist, title)).unwrap(), None);
    assert_eq!(hits2.load(std::sync::atomic::Ordering::SeqCst), 1, "negative cache");
    let _ = std::fs::remove_file(&cache);
    let (base3, _) = fake_site(vec![("search/multi", 500, "no".into())]);
    assert!(rt.block_on(genius::lookup(&c, &base3, artist, title)).is_err());
    assert!(!cache.exists());
}

#[test]
fn percent() {
    assert_eq!(util::percent_decode("/a%20b/c%C3%A9.jpg"), "/a b/cé.jpg");
}

#[test]
fn control_row_never_draws_half_a_button() {
    use ui::plan_controls;
    for width in 0..=160u16 {
        for shuffle in [None, Some(false), Some(true)] {
            for rep in [None, Some(false), Some(true)] {
                for (playing, volume, row_ok) in [(true, true, true), (false, true, false), (true, false, true), (false, false, false)] {
                    let p = plan_controls(width, shuffle, rep, playing, volume, row_ok);
                    let ctx = format!("w={width} sh={shuffle:?} rep={rep:?} play={playing} vol={volume} row={row_ok}");
                    // every button is whole, inside the row, and none touches the next
                    let mut end = 0u16;
                    for b in &p.buttons {
                        let w = b.label.chars().count() as u16;
                        assert!(b.x >= end && b.x + w <= width, "{ctx}: {b:?}");
                        end = b.x + w + 1;
                    }
                    if let Some((k, x)) = p.vol {
                        assert!(volume && x + k.width() <= width, "{ctx}: volume {k:?} at {x}");
                        if p.vol_row {
                            assert!(row_ok, "{ctx}");
                        } else {
                            assert!(x >= end, "{ctx}: volume overlaps the buttons");
                        }
                    } else {
                        assert!(!p.vol_row, "{ctx}");
                    }
                    // play stays longest; the whole transport group once it fits
                    let ids: Vec<u8> = p.buttons.iter().map(|b| b.id).collect();
                    if width >= 4 {
                        assert!(ids.contains(&3), "{ctx}: no play button");
                    }
                    if width >= 16 {
                        assert!(ids.contains(&2) && ids.contains(&4), "{ctx}");
                    }
                    // shuffle / repeat only exist for what the player reports
                    assert!(!ids.contains(&0) || shuffle.is_some(), "{ctx}");
                    assert!(!ids.contains(&1) || rep.is_some(), "{ctx}");
                }
            }
        }
    }
}

#[test]
fn control_row_gives_up_in_a_fixed_order() {
    use ui::{VolKind, plan_controls};
    let ids = |p: &ui::ControlPlan| p.buttons.iter().map(|b| b.id).collect::<Vec<_>>();
    let wide = plan_controls(120, Some(false), Some(false), true, true, true);
    assert_eq!(ids(&wide), [0, 1, 2, 3, 4]);
    assert_eq!(wide.vol.map(|v| v.0), Some(VolKind::Full));
    assert!(!wide.vol_row);
    assert!(wide.buttons[0].label.starts_with("[ shuf"));
    // three blank columns between buttons
    let (a, b) = (&wide.buttons[0], &wide.buttons[1]);
    assert_eq!(b.x - (a.x + a.label.chars().count() as u16), 3);
    // volume moves below the buttons before anything is shortened
    let mid = plan_controls(60, Some(false), Some(false), true, true, true);
    assert!(mid.vol_row && ids(&mid) == [0, 1, 2, 3, 4] && mid.buttons[2].label == "[ prev ]");
    // then compact labels
    let compact = plan_controls(40, Some(false), Some(false), true, true, true);
    assert_eq!(compact.buttons[2].label, "[<<]");
    // then shuffle / repeat go, the volume shrinks, and prev / play / next are kept longest
    let small = plan_controls(20, Some(false), Some(false), true, true, false);
    assert_eq!(ids(&small), [2, 3, 4]);
    assert_eq!(small.vol, None);
    assert_eq!(ids(&plan_controls(10, None, None, true, false, false)), [3, 4]);
    assert_eq!(ids(&plan_controls(9, None, None, true, false, false)), [3]);
    assert_eq!(ids(&plan_controls(4, None, None, false, false, false)), [3]);
    assert!(plan_controls(3, None, None, false, false, false).buttons.is_empty());
}

#[test]
fn left_panel_keeps_the_controls_when_short() {
    for h in 0..=60u16 {
        for meta in [0u16, 3, 7] {
            for vol_row in [false, true] {
                let l = ui::left_heights(h, meta, vol_row);
                let sum = l.cover + l.gap_cover + l.meta + l.gap_meta + l.prog + l.gap_prog + l.btn + l.vol;
                assert_eq!(sum, h, "h={h} meta={meta} {l:?}");
                if h >= 1 {
                    assert_eq!(l.btn, 1, "buttons first: h={h}");
                }
                // the cover is the first to go, then metadata
                if l.meta < meta && meta > 0 {
                    assert_eq!(l.cover, 0, "h={h} meta={meta} {l:?}");
                }
                if l.cover > 0 {
                    assert!(l.meta == meta || meta == 0, "h={h} meta={meta} {l:?}");
                }
                assert!(l.vol == 0 || vol_row);
            }
        }
    }
}

#[tokio::test]
async fn control_row_survives_every_window_size() {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    let mut s = snap("Band", "Song", "LP");
    s.shuffle = Some(true);
    s.repeat = Some(player::Loop::None);
    s.volume = Some(0.65);
    s.player = "spotify".into();
    for w in [10u16, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 100, 120] {
        for h in [4u16, 5, 6, 8, 10, 12, 16, 20, 30, 40] {
            let mut app = test_app();
            app.apply(AppEvent::Player(Some(s.clone())));
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| ui::draw(f, &mut app)).unwrap();
            let area = Rect::new(0, 0, w, h);
            let hits = &app.hits;
            let rects = [
                ("shuffle", hits.shuffle), ("repeat", hits.repeat), ("prev", hits.prev), ("play", hits.play),
                ("next", hits.next), ("vol-", hits.vol_minus), ("vol%", hits.vol_pct), ("vol+", hits.vol_plus),
            ];
            let shown: Vec<_> = rects.iter().filter(|r| r.1.width > 0).collect();
            for (name, r) in &shown {
                assert!(area.contains(ratatui::layout::Position::new(r.x, r.y)) && r.right() <= w && r.bottom() <= h, "{w}x{h} {name} {r:?}");
                // what is drawn under a hit area is a whole button: it starts with `[` and ends with `]`
                let b = term.backend().buffer();
                let text: String = (r.x..r.right()).map(|x| b[(x, r.y)].symbol()).collect();
                if *name != "vol%" || text.starts_with('[') {
                    assert!(text.starts_with('[') && text.ends_with(']'), "{w}x{h} {name}: {text:?}");
                }
            }
            for (i, (an, a)) in shown.iter().enumerate() {
                for (bn, b) in &shown[i + 1..] {
                    let apart = a.y != b.y || a.right() <= b.x || b.right() <= a.x;
                    assert!(apart, "{w}x{h}: {an} {a:?} overlaps {bn} {b:?}");
                }
            }
        }
    }
}

#[test]
fn key_hints_drop_whole_items_never_cut_one() {
    let items = [(3u8, "tab/1-5"), (1, "enter play"), (2, "→/← open/back"), (4, "a queue"), (5, "f like"), (1, "esc")];
    let full = "tab/1-5 · enter play · →/← open/back · a queue · f like · esc";
    assert_eq!(ui::fit_hints(&items, 200), full);
    for w in 0..=full.chars().count() {
        let got = ui::fit_hints(&items, w);
        assert!(util::width(&got) <= w, "{w}: {got:?}");
        // every shown piece is a whole item, in the original order
        let pieces: Vec<&str> = if got.is_empty() { vec![] } else { got.split(" · ").collect() };
        let mut at = 0;
        for p in pieces {
            at += items[at..].iter().position(|i| i.1 == p).expect(p) + 1;
        }
    }
    // the highest number goes first, then the next
    assert_eq!(ui::fit_hints(&items, 54), "tab/1-5 · enter play · →/← open/back · a queue · esc");
    assert_eq!(ui::fit_hints(&items, 12), "enter play");
    assert_eq!(ui::fit_hints(&items, 9), "esc");
    assert_eq!(ui::fit_hints(&items, 2), "");
    assert_eq!(ui::ellipsize("hello world", 8), "hello w…");
    assert_eq!(ui::ellipsize("hi", 8), "hi");
}

#[tokio::test]
async fn runtime_keys_cycle_size_spacing_and_padding() {
    use app::Action as A;
    let mut app = test_app();
    let mut after = |a: A, get: fn(&app::App) -> u16, n: usize| -> Vec<u16> {
        (0..n).map(|_| { app.run(a); get(&app) }).collect()
    };
    assert_eq!(after(A::BigScale, |a| a.args.big_scale, 4), [2, 3, 1, 2]);
    assert_eq!(after(A::LineSpacing, |a| a.args.lyric_spacing, 5), [2, 3, 0, 1, 2]);
    assert_eq!(after(A::LyricPad, |a| a.args.lyric_pad, 7), [2, 4, 6, 8, 10, 0, 2]);
    assert!(app.note_text().is_some_and(|n| n.contains("side padding")));
    // an odd value from the config can't step past the range
    app.args.lyric_pad = 9;
    app.run(A::LyricPad);
    assert_eq!(app.args.lyric_pad, 0);
    // the three keys are plain lowercase letters: no Shift, no AltGr on a Swedish layout
    for (k, a) in [('x', A::BigScale), ('i', A::LineSpacing), ('u', A::LyricPad)] {
        let b = app::BINDS.iter().find(|b| b.action == a).unwrap();
        assert_eq!(b.codes, [crossterm::event::KeyCode::Char(k)]);
        assert!(b.group == "Lyrics" && k.is_ascii_lowercase());
    }
}

#[test]
fn fine_hangul_composes_every_syllable() {
    use bigfont::{Font, dump};
    let mut seen = std::collections::HashSet::new();
    for u in 0xAC00u32..=0xD7A3 {
        let c = char::from_u32(u).unwrap();
        let g = dump(c, Font::Fine);
        assert_eq!((g.len(), g[0].chars().count()), (12, 12), "{c}");
        assert!(g.iter().any(|r| r.contains('#')), "{c} is empty");
        seen.insert(g);
    }
    // 12×12 tells syllables apart better than the 8×8 grid (10 049 distinct of 11 172)
    assert!(seen.len() >= 10_400, "{} distinct", seen.len());
    // golden: 가 = ㄱ with ㅏ, and 한 = ㅎ ㅏ ㄴ
    assert_eq!(
        dump('가', Font::Fine),
        [
            "#######.#...", "......#.#...", "......#.#...", "......#.#...", "......#.#...", "......#.####",
            "......#.#...", "......#.#...", "......#.#...", "......#.#...", "......#.#...", "......#.#...",
        ]
    );
    assert_eq!(
        dump('한', Font::Fine),
        [
            "...#....#...", "#######.#...", "..###...#...", ".#...#..####", ".#...#..#...", ".#...#..#...",
            "..###...#...", "............", ".#..........", ".#..........", ".#..........", ".##########.",
        ]
    );
    // Latin letters sit beside Hangul on the same 12 rows
    assert_eq!(dump('A', Font::Fine).iter().filter(|r| r.contains('#')).count(), 7);
}

#[test]
fn hangul_lines_use_the_fine_font_and_fall_back() {
    // "안녕": 2 × 13 − 1 columns, 6 text rows (12 pixel rows), against 17 columns × 4 rows
    let fine = ui::big_stage("안녕", 60, 20, 1, 0).unwrap();
    assert_eq!((fine.len(), fine[0].chars().count()), (6, 25));
    // a panel too short for it keeps the 8×8 font, one too narrow too (13 > 9 columns each)
    assert_eq!(ui::big_stage("안녕", 60, 7, 1, 0).unwrap().len(), 4);
    let narrow = ui::big_stage("안녕", 17, 20, 1, 0).unwrap();
    assert_eq!((narrow.len(), narrow[0].chars().count()), (4, 17));
    // Latin lines are unchanged, and mixed lines work
    assert_eq!(ui::big_stage("Hi there", 60, 20, 1, 0).unwrap().len(), 4);
    assert!(ui::big_stage("Hi 안녕", 60, 20, 1, 0).unwrap().len() == 6);
    // scale applies to the fine font too, and the compact font still can't draw Hangul
    assert_eq!(ui::big_stage("안녕", 80, 30, 2, 0).unwrap().len(), 12);
    assert!(bigfont::render_compact("안녕", 60, 3).is_none());
}

#[test]
fn chords_sit_above_their_glyphs_in_the_big_layout() {
    use clap::Parser;
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    // H(6) I(6) space(3) T(6): "T" starts at column 15, "I" at 6
    let lines: Vec<String> = vec!["HI TO".to_string()];
    let aligned = chords::Aligned {
        per_line: vec![vec![(0, "Am".into()), (1, "G".into()), (3, "C".into())]],
        matched: 1,
        total: 1,
        flats: false,
        intro: vec![],
        gaps: vec![vec![]],
    };
    let fc = ui::Focus {
        lines: &lines,
        current: Some(0),
        gap: 0,
        scale: 1,
        chords: Some(&aligned),
        tr: 0,
        flats: false,
        context: 99,
        in_break: false,
        members: &[],
        reserve_tag: false,
        tag_below: false,
        progress: None,
    };
    let (w, h) = (40u16, 12u16);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut hits = Vec::new();
    term.draw(|f| hits = ui::draw_focus(f, Rect::new(0, 0, w, h), &theme, &fc)).unwrap();
    let b = term.backend().buffer();
    let screen: Vec<String> = (0..h).map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect()).collect();
    let stage = screen.iter().position(|r| r.contains('█') || r.contains('▀') || r.contains('▄')).unwrap();
    // "HI TO" is 6+6+3+6+5 = 26 columns wide, centred in 40: starts at column 7
    let left = (40 - 26) / 2;
    let chord_row = &screen[stage - 1];
    assert_eq!(chord_row.find("Am"), Some(left), "{screen:#?}");
    // G sits over the I (6 columns on); C over the T (15 on)
    assert_eq!(chord_row.chars().position(|c| c == 'G'), Some(left + 6), "{chord_row:?}");
    assert_eq!(chord_row.chars().position(|c| c == 'C'), Some(left + 15), "{chord_row:?}");
    assert_eq!(hits[stage - 1], Some(0), "the chord row belongs to the active line");
    // a chord on the space goes over the next glyph (T), one past the end over the last
    let space = chords::Aligned { per_line: vec![vec![(2, "Em".into()), (9, "F".into())]], ..aligned.clone() };
    let fc2 = ui::Focus { chords: Some(&space), ..fc };
    term.draw(|f| { ui::draw_focus(f, Rect::new(0, 0, w, h), &theme, &fc2); }).unwrap();
    let b = term.backend().buffer();
    let row: String = (0..w).map(|x| b[(x, stage as u16 - 1)].symbol()).collect();
    assert_eq!(row.find("Em"), Some(left + 15), "{row:?}");
    assert!(row.contains('F'), "{row:?}");
    // wrapped pieces get their own chord row, in the blank row above each piece
    let wrap_lines: Vec<String> = vec!["HI TO GO".to_string()];
    let wrapped = chords::Aligned {
        per_line: vec![vec![(0, "Am".into()), (6, "G".into())]],
        matched: 1,
        total: 1,
        flats: false,
        intro: vec![],
        gaps: vec![vec![]],
    };
    let fc3 = ui::Focus { lines: &wrap_lines, chords: Some(&wrapped), ..fc2 };
    let (w, h) = (30u16, 16u16);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| { ui::draw_focus(f, Rect::new(0, 0, w, h), &theme, &fc3); }).unwrap();
    let b = term.backend().buffer();
    let screen: Vec<String> = (0..h).map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect()).collect();
    let am = screen.iter().position(|r| r.contains("Am")).unwrap();
    let g = screen.iter().position(|r| r.contains('G') && !r.contains('█')).unwrap();
    assert!(g > am + 4, "second piece, lower down: Am row {am}, G row {g}");
    assert!(screen[g].trim() == "G", "{:?}", screen[g]);
}

#[test]
fn hangul_chords_follow_the_wide_glyphs() {
    let r = bigfont::render_map("안녕 하", 60, 3, 1, bigfont::Font::Fine).unwrap();
    // 안 at 0, 녕 at 13, space 4, 하 at 13 + 13 + 4 = 30
    assert_eq!(r.chunks.len(), 1);
    assert_eq!(r.chunks[0].cols, [(0, 0), (1, 13), (3, 30)]);
    assert_eq!(r.chunks[0].width, 12 + 1 + 12 + 1 + 4 + 12);
    assert_eq!(r.column_of(1), Some((0, 13)));
    assert_eq!(r.column_of(2), Some((0, 30)), "a space goes to the next glyph");
    assert_eq!(r.column_of(99), Some((0, 30)), "past the end: the last glyph");
    // scaled: columns scale too; "…" maps all three dots to the one source character
    let r = bigfont::render_map("A…B", 60, 3, 2, bigfont::Font::Normal).unwrap();
    assert_eq!(r.chunks[0].cols, [(0, 0), (1, 12), (1, 24), (1, 36), (2, 48)]);
}

// `--setup` (spotifyd + Spotify login): src/tests/setup_tests.rs
mod setup_tests;

// ── Spotify in the settings menu ───────────────────────────────────────

fn key(code: crossterm::event::KeyCode) -> crossterm::event::Event {
    use crossterm::event::{Event, KeyEvent, KeyModifiers};
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// An app whose settings go to a scratch config file (never the real one).
fn settings_app(name: &str) -> (app::App, std::path::PathBuf) {
    let mut app = test_app();
    let cfg = tmp(name).join("config.toml");
    let _ = std::fs::remove_file(&cfg);
    app.args.config = Some(cfg.clone());
    (app, cfg)
}

fn remote_event(title: &str) -> AppEvent {
    let p = serde_json::json!({"is_playing": true, "device": {"name": "Phone"},
        "item": {"uri": "spotify:track:r", "name": title, "duration_ms": 100000, "artists": [{"name": "X"}], "album": {"name": "Y"}}});
    AppEvent::Spotify(spotify::Reply::Remote(Ok(spotify::parse_state(&p, |_| false).map(Box::new))))
}

#[test]
fn settings_menu_has_headers_and_a_status_line_between_the_rows() {
    use app::{Setting, SettingsRow as Row};
    let rows = Setting::rows();
    assert_eq!(rows[0], Row::Header("General"));
    let sp = rows.iter().position(|r| *r == Row::Header("Spotify")).unwrap();
    assert_eq!(rows[sp + 1], Row::Status);
    let on = Setting::ALL.iter().position(|s| *s == Setting::SpotifyOn).unwrap();
    assert_eq!(rows[sp + 2], Row::Item(on));
    // every setting is drawn exactly once, in order; headers and status are never selectable
    let items: Vec<usize> = rows.iter().filter_map(|r| if let Row::Item(i) = r { Some(*i) } else { None }).collect();
    assert_eq!(items, (0..Setting::ALL.len()).collect::<Vec<_>>());
    // the action rows own no config key; the toggle and the texts do
    assert!(Setting::SpotifyLogin.config_keys().is_empty() && Setting::SpotifyLogout.config_keys().is_empty());
    assert_eq!(Setting::SpotifyOn.config_keys(), ["spotify"]);
    assert_eq!(Setting::SpotifyClientId.config_keys(), ["spotify_client_id"]);
    assert_eq!(Setting::SpotifyDevice.config_keys(), ["spotify_device"]);
}

#[test]
fn text_input_edits_at_the_cursor() {
    use browser::{Edit, TextInput};
    use crossterm::event::KeyCode as K;
    let mut t = TextInput::with("abc");
    assert_eq!(t.cursor, 3);
    t.insert('d');
    t.key(K::Left);
    t.key(K::Backspace);
    assert_eq!(t.text, "abd");
    t.key(K::Home);
    t.key(K::Delete);
    assert_eq!(t.text, "bd");
    assert_eq!(t.shown(), "▏bd");
    t.key(K::End);
    t.key(K::Char('å'));
    t.key(K::Left);
    assert_eq!((t.text.as_str(), t.shown().as_str()), ("bdå", "bd▏å"));
    t.key(K::Backspace);
    assert_eq!(t.text, "bå");
    assert_eq!(t.key(K::Enter), Some(Edit::Submit));
    assert_eq!(t.key(K::Esc), Some(Edit::Cancel));
    // the empty field is fine
    let mut e = TextInput::default();
    e.key(K::Backspace);
    e.key(K::Delete);
    assert_eq!(e.shown(), "▏");
}

#[test]
fn hints_drop_whole_items_never_cut_one() {
    let items = ["tab/1-5", "enter play", "a queue", "esc"];
    assert_eq!(browser::fit_items(&items, 100), "tab/1-5 · enter play · a queue · esc");
    assert_eq!(browser::fit_items(&items, 25), "tab/1-5 · enter play");
    assert_eq!(browser::fit_items(&items, 19), "tab/1-5");
    // nothing joins: the one item that still fits alone is better than an empty line
    assert_eq!(browser::fit_items(&items, 3), "esc");
    assert_eq!(browser::fit_items(&items, 2), "");
}

#[tokio::test]
async fn client_id_is_typed_into_the_menu_and_saved() {
    use crossterm::event::KeyCode as K;
    let (mut app, cfg) = settings_app("cid");
    app.show_settings = true;
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::SpotifyClientId).unwrap();
    assert_eq!(app.setting_value(app::Setting::SpotifyClientId), "not set");
    app.handle(key(K::Enter));
    assert!(app.settings_edit.is_some());
    // q and s are letters now, they don't close the menu
    for c in "qs1AbC".chars() {
        app.handle(key(K::Char(c)));
    }
    assert!(app.show_settings);
    app.handle(key(K::Enter));
    assert!(app.settings_edit.is_none());
    assert_eq!(app.args.spotify_client_id.as_deref(), Some("qs1AbC"));
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["spotify_client_id"].as_str(), Some("qs1AbC"));
    // only the start of it is shown
    assert_eq!(app.setting_value(app::Setting::SpotifyClientId), "qs1AbC…");
    // something that can't be an ID is refused, Esc leaves it as it was
    app.handle(key(K::Enter));
    app.handle(key(K::Char('!')));
    app.handle(key(K::Enter));
    assert_eq!(app.args.spotify_client_id.as_deref(), Some("qs1AbC"));
    assert!(app.settings_note.as_deref().unwrap_or("").contains("Client ID"));
    app.settings_note = None;
    app.handle(key(K::Enter));
    app.handle(key(K::Char('x')));
    app.handle(key(K::Esc));
    assert!(app.settings_edit.is_none());
    assert_eq!(app.args.spotify_client_id.as_deref(), Some("qs1AbC"));
    // Esc with nothing being typed closes the menu again
    app.handle(key(K::Esc));
    assert!(!app.show_settings);
}

#[tokio::test]
async fn device_name_can_be_emptied() {
    use crossterm::event::KeyCode as K;
    let (mut app, cfg) = settings_app("devname");
    app.show_settings = true;
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::SpotifyDevice).unwrap();
    assert_eq!(app.args.spotify_device, "");
    assert_eq!(app.setting_value(app::Setting::SpotifyDevice), "none");
    app.handle(key(K::Enter));
    for c in "Desktop".chars() {
        app.handle(key(K::Char(c)));
    }
    app.handle(key(K::Enter));
    assert_eq!(app.args.spotify_device, "Desktop");
    app.handle(key(K::Enter));
    for _ in 0..7 {
        app.handle(key(K::Backspace));
    }
    app.handle(key(K::Enter));
    assert_eq!(app.args.spotify_device, "");
    // an empty name is a valid saved value
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["spotify_device"].as_str(), Some(""));
}

#[tokio::test]
async fn spotify_turns_on_and_off_from_settings_without_a_restart() {
    let (mut app, cfg) = settings_app("onoff");
    assert!(app.spot.is_none());
    app.change_setting(app::Setting::SpotifyOn, 1);
    assert!(app.spot.is_some() && app.args.spotify);
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["spotify"].as_bool(), Some(true));
    // not logged in (the test token dir is empty): the menu says so
    assert_eq!(app.spotify_status(), "not logged in");
    // a song playing on another device shows up ...
    app.apply(remote_event("Phone Song"));
    assert!(app.remote_active());
    // ... and goes away with the switch; a late answer of the stopped client is ignored
    app.change_setting(app::Setting::SpotifyOn, 1);
    assert!(app.spot.is_none() && !app.args.spotify && app.snap.is_none() && app.remote.is_none());
    app.apply(remote_event("Late"));
    assert!(app.snap.is_none());
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["spotify"].as_bool(), Some(false));
    assert_eq!(app.spotify_status(), "off · not logged in");
}

#[tokio::test]
async fn nothing_spotify_happens_while_it_is_off() {
    let mut app = test_app();
    app.start_spotify(); // `spotify` is false: no client, no task
    assert!(app.spot.is_none());
    // the Spotify keys only explain how to turn it on
    app.run(app::Action::Library);
    assert!(!app.lib.open);
    assert!(app.note_text().unwrap_or("").contains("Settings"));
}

#[tokio::test]
async fn login_progress_shows_in_the_status_line() {
    use spotify::{LoginMsg, LoginStep, Reply};
    let (mut app, _cfg) = settings_app("login");
    app.args.spotify = true;
    app.start_spotify();
    // without a Client ID the login doesn't even start
    app.change_setting(app::Setting::SpotifyLogin, 1);
    assert!(app.settings_note.as_deref().unwrap_or("").contains("Client ID"));
    assert_eq!(app.setting_value(app::Setting::SpotifyLogin), "");
    app.apply(AppEvent::Spotify(Reply::Login(LoginMsg::Step(LoginStep::Url("https://x".into())))));
    assert_eq!(app.spotify_status(), "not logged in");
    app.apply(AppEvent::Spotify(Reply::Login(LoginMsg::Step(LoginStep::Opened(false)))));
    assert!(app.spotify_status().contains("didn't open"));
    app.apply(AppEvent::Spotify(Reply::Login(LoginMsg::Done(Err("can't listen on 127.0.0.1:8897".into())))));
    assert_eq!(app.spotify_status(), "can't listen on 127.0.0.1:8897");
    app.apply(AppEvent::Spotify(Reply::Login(LoginMsg::Done(Ok(())))));
    assert_eq!(app.spotify_status(), "not logged in", "the new client looks for the token again");
    assert!(app.spot.is_some());
    // a refused refresh token reads as expired
    app.spot_status = Some("login failed: invalid_grant".into());
    assert_eq!(app.spotify_status(), "token expired: log in again");
    // a login that finishes while Spotify is off still counts
    app.stop_spotify();
    app.apply(AppEvent::Spotify(Reply::Login(LoginMsg::Done(Err("login timed out".into())))));
    assert_eq!(app.spotify_status(), "login timed out");
}

#[tokio::test]
async fn login_callback_needs_the_right_state() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let waiting = tokio::spawn(async move { spotify::wait_for_code(&l, "good").await });
    let get = |target: &'static str| async move {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    };
    assert!(get("/favicon.ico").await.starts_with("HTTP/1.1 404"));
    assert!(get("/callback?code=evil&state=bad").await.contains("Login failed"));
    assert!(!waiting.is_finished(), "a wrong state doesn't end the wait");
    assert!(get("/callback?code=abc&state=good").await.contains("Logged in"));
    assert_eq!(waiting.await.unwrap().unwrap(), "abc");
}

#[test]
fn an_empty_spotifyd_name_never_matches_a_device() {
    use serde_json::json;
    let http = reqwest::Client::new();
    let dev = |id: &str, name: &str, kind: &str| json!({"id": id, "name": name, "type": kind});
    // no spotifyd name set (the default): a device without a name is not "this PC" ...
    let api = spotify::Api::new(http.clone(), "id", "").with_host("pawtix");
    assert!(!api.is_local(""));
    assert!(api.is_local("pawtix") && api.is_local("PAWTIX"), "the Spotify app is named after the host");
    let unnamed = vec![dev("a", "", "Speaker")];
    assert!(api.local_device(&unnamed).is_none());
    // ... while the host name still finds the app, and "Computer" is still a fallback
    let devs = vec![dev("a", "", "Speaker"), dev("b", "Pawtix", "Computer"), dev("c", "Phone", "Smartphone")];
    assert_eq!(api.local_device(&devs).unwrap()["id"], "b");
    let other = vec![dev("a", "", "Speaker"), dev("d", "Something", "Computer")];
    assert_eq!(api.local_device(&other).unwrap()["id"], "d");
    // an unknown host name and no name set: nothing matches by name at all
    let api = spotify::Api::new(http.clone(), "id", "").with_host("");
    assert!(!api.is_local("") && !api.is_local("pawtix"));
    // with a name set, spotifyd counts, but the app still comes first
    let api = spotify::Api::new(http, "id", "kitchen").with_host("pawtix");
    assert!(api.is_local("kitchen"));
    let both = vec![dev("a", "kitchen", "Speaker"), dev("b", "pawtix", "Computer")];
    assert_eq!(api.local_device(&both).unwrap()["id"], "b");
    assert_eq!(api.local_device(&both[..1]).unwrap()["id"], "a");
}

#[tokio::test]
async fn the_client_only_talks_to_the_server_it_was_given() {
    let body = r#"{"is_playing":true,"progress_ms":5000,"device":{"name":"Phone","volume_percent":30},"item":{"uri":"spotify:track:z","name":"Zed","duration_ms":90000,"artists":[{"name":"Q"}],"album":{"name":"W"}}}"#;
    let (base, log) = scripted_site(vec![("/me/player", vec![R::Http(200, "Content-Type: application/json\r\n", body.into())])]);
    let api = spotify::Api::new(reqwest::Client::new(), "id", "").with_fake_server(&base);
    let r = api.state().await.unwrap().unwrap();
    assert_eq!((r.snap.title.as_str(), r.device.as_str(), r.local), ("Zed", "Phone", false));
    assert!(log.lock().unwrap()[0].starts_with("GET /me/player?additional_types=track"), "{:?}", log.lock().unwrap());
    // the polling loop hands the same answer to the app
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let poller = tokio::spawn(api.clone().remote_loop(tx));
    match rx.recv().await {
        Some(AppEvent::Spotify(spotify::Reply::Remote(Ok(Some(r))))) => assert_eq!(r.snap.title, "Zed"),
        _ => panic!("the poller sent something else"),
    }
    poller.abort();
}

// ── search and devices ─────────────────────────────────────────────────

const SEARCH_JSON: &str = r#"{"tracks":{"items":[{"uri":"spotify:track:t1","name":"Bohemian","duration_ms":354000,"artists":[{"name":"Queen"}]}]},
 "albums":{"items":[{"uri":"spotify:album:a1","name":"Opera","artists":[{"name":"Queen"}]}]},
 "artists":{"items":[{"uri":"spotify:artist:r1","name":"Queen"}]},
 "playlists":{"items":[null,{"uri":"spotify:playlist:p1","name":"Mine","owner":{"id":"me1","display_name":"Me"}},{"uri":"spotify:playlist:p2","name":"Theirs","owner":{"id":"zz","display_name":"Zed"}}]}}"#;

const DEVICES_JSON: &str = r#"{"devices":[{"id":"d1","name":"Phone","type":"Smartphone","is_active":true,"is_restricted":false},
 {"id":"d2","name":"Pawtix","type":"Computer","is_active":false},{"id":"d3","name":"Echo","type":"Speaker","is_restricted":true}]}"#;

#[test]
fn search_results_are_grouped_and_only_your_playlists_open() {
    let v: serde_json::Value = serde_json::from_str(SEARCH_JSON).unwrap();
    let items = spotify::parse_search(&v, "me1");
    let kinds: Vec<&str> = items.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(kinds, ["track", "album", "artist", "playlist", "playlist"], "the null playlist is skipped");
    assert_eq!((items[0].name.as_str(), items[0].sub.as_str(), items[0].duration), ("Bohemian", "Queen", 354.0));
    assert!(items[1].openable && items[2].openable);
    assert_eq!((items[3].openable, items[3].sub.as_str()), (true, "Playlist"));
    assert_eq!((items[4].openable, items[4].sub.as_str()), (false, "by Zed"));
    // without knowing who you are none of them counts as yours
    assert!(spotify::parse_search(&v, "").iter().all(|i| i.kind != "playlist" || !i.openable));
    assert!(spotify::parse_search(&serde_json::Value::Null, "me1").is_empty());
    // the browser draws a heading where the kind changes
    let level = browser::Level { items, search: true, ..Default::default() };
    use browser::ListRow::{Head, Item};
    assert_eq!(
        level.rows(),
        [Head("Songs"), Item(0), Head("Albums"), Item(1), Head("Artists"), Item(2), Head("Playlists"), Item(3), Item(4)]
    );
    let plain = browser::Level { items: level.items.clone(), ..Default::default() };
    assert_eq!(plain.rows().len(), 5, "other lists have no headings");
}

#[test]
fn scrolling_keeps_the_selection_and_its_heading_in_view() {
    let items: Vec<spotify::Item> = ["track", "track", "album", "album"]
        .iter()
        .enumerate()
        .map(|(i, k)| spotify::Item { kind: k.to_string(), uri: format!("u{i}"), name: format!("n{i}"), ..Default::default() })
        .collect();
    let mut l = browser::Level { items, search: true, ..Default::default() };
    // rows: 0 Head(Songs), 1 item0, 2 item1, 3 Head(Albums), 4 item2, 5 item3
    assert_eq!(l.scroll(3).len(), 6);
    l.sel = 3;
    l.scroll(3);
    assert_eq!(l.top, 3, "item 3 (row 5) is the last of the three rows shown");
    // scrolling back up to the first album brings its heading along
    l.top = 5;
    l.sel = 2;
    l.scroll(3);
    assert_eq!(l.top, 3);
    // and the first song shows the very first heading
    l.sel = 0;
    l.scroll(3);
    assert_eq!(l.top, 0);
    // a short list never scrolls
    l.sel = 3;
    l.scroll(10);
    assert_eq!(l.top, 0);
}

#[test]
fn devices_are_parsed_with_this_pc_marked() {
    let v: serde_json::Value = serde_json::from_str(DEVICES_JSON).unwrap();
    let d = spotify::parse_devices(&v, |n| n == "Pawtix");
    assert_eq!(d.len(), 3);
    assert_eq!((d[0].name.as_str(), d[0].kind.as_str(), d[0].active, d[0].local), ("Phone", "Smartphone", true, false));
    assert!(d[1].local && !d[1].active && !d[1].restricted);
    assert!(d[2].restricted);
    assert!(spotify::parse_devices(&serde_json::json!({}), |_| false).is_empty());
}

#[test]
fn failed_requests_are_explained() {
    assert_eq!(spotify::explain(429, "x", Some(30)), "Spotify is rate limiting: try again in 30 s");
    assert_eq!(spotify::explain(429, "x", None), "Spotify is rate limiting: try again in a moment");
    assert_eq!(spotify::explain(403, "PREMIUM_REQUIRED", None), "Spotify Premium is needed for this");
    assert_eq!(spotify::explain(403, "Forbidden", None), "Spotify refused it (403): Forbidden");
    assert!(spotify::explain(404, "NO_ACTIVE_DEVICE", None).contains("press d"));
    assert_eq!(spotify::explain(404, "Not found", None), "Not found");
    assert_eq!(spotify::explain(500, "boom", None), "boom");
}

#[tokio::test]
async fn the_client_searches_lists_devices_and_reports_limits() {
    let (base, log) = scripted_site(vec![
        ("GET /me HTTP", vec![R::Http(200, JSON_HDR, r#"{"id":"me1"}"#.into())]),
        ("/search", vec![R::Http(200, JSON_HDR, SEARCH_JSON.into())]),
        ("/me/player/devices", vec![R::Http(200, JSON_HDR, DEVICES_JSON.into())]),
        ("/me/player/pause", vec![R::Http(429, "Retry-After: 30\r\n", String::new()), R::Http(403, JSON_HDR, r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#.into())]),
        ("PUT /me/player HTTP", vec![R::Http(204, "", String::new())]),
    ]);
    let api = spotify::Api::new(reqwest::Client::new(), "id", "").with_fake_server(&base);
    // one /search call for the four kinds, ten of each
    let found = api.search("Queen & more").await.unwrap();
    assert_eq!(found.len(), 5);
    let req = log.lock().unwrap().iter().find(|l| l.contains("/search")).cloned().unwrap();
    assert!(req.contains("q=Queen") && req.contains("type=track%2Calbum%2Cartist%2Cplaylist") && req.contains("limit=10"), "{req}");
    assert!(req.contains("%26"), "the & of the query is escaped: {req}");
    assert!(found.iter().any(|i| i.kind == "playlist" && i.openable), "your playlist is recognised through /me");
    // devices
    let devs = api.devices_list().await.unwrap();
    assert_eq!(devs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Phone", "Pawtix", "Echo"]);
    // moving playback
    let ok = api.run(spotify::Cmd::TransferTo { id: "d2".into(), name: "Pawtix".into() }).await.unwrap();
    assert_eq!(ok, "Playing on Pawtix");
    assert!(log.lock().unwrap().iter().any(|l| l.starts_with("PUT /me/player HTTP")));
    // the limits Spotify sets come out as sentences
    let e = api.run(spotify::Cmd::Pause).await.unwrap_err();
    assert_eq!((e.msg.as_str(), e.status), ("Spotify is rate limiting: try again in 30 s", 429));
    // inside the Retry-After window nothing is sent at all
    let pauses = || log.lock().unwrap().iter().filter(|l| l.contains("/me/player/pause")).count();
    assert_eq!(pauses(), 1);
    let e = api.run(spotify::Cmd::Pause).await.unwrap_err();
    assert_eq!(e.status, 429);
    assert!(e.msg.contains("try again in"), "{}", e.msg);
    assert_eq!(pauses(), 1, "the second try never left the app");
    assert!(api.paused_for().is_some());
    // another client (no window of its own) gets the 403 the server answers next
    let other = spotify::Api::new(reqwest::Client::new(), "id", "").with_fake_server(&base);
    let e = other.run(spotify::Cmd::Pause).await.unwrap_err();
    assert_eq!((e.msg.as_str(), e.status), ("Spotify Premium is needed for this", 403));
}

/// An app with Spotify on that talks to a fake server, and the channel its answers come back on.
fn spotify_app(base: &str, name: &str) -> (app::App, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
    util::enter_test_mode();
    let args = cli::Args::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
    app.args.config = Some(tmp(name).join("config.toml"));
    app.args.spotify = true;
    app.start_spotify();
    app.spot = Some(spotify::Api::new(app.client.clone(), "id", "").with_fake_server(base));
    (app, rx)
}

async fn pump(
    app: &mut app::App,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    until: impl Fn(&app::App) -> bool,
) {
    for _ in 0..200 {
        app.preload_tick();
        if until(app) {
            return;
        }
        if let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            app.apply(ev);
        }
    }
    panic!("the app never got there");
}

fn type_text(app: &mut app::App, text: &str) {
    for c in text.chars() {
        app.handle(key(crossterm::event::KeyCode::Char(c)));
    }
}

#[tokio::test]
async fn searching_in_the_library_browser() {
    use crossterm::event::KeyCode as K;
    let album_tracks = r#"{"items":[{"uri":"spotify:track:t2","name":"Dreamer","duration_ms":1000,"artists":[{"name":"Queen"}]}],"next":null}"#;
    let (base, log) = scripted_site(vec![
        ("GET /me HTTP", vec![R::Http(200, JSON_HDR, r#"{"id":"me1"}"#.into())]),
        ("/search", vec![R::Http(200, JSON_HDR, SEARCH_JSON.into())]),
        ("/albums/a1/tracks", vec![R::Http(200, JSON_HDR, album_tracks.into())]),
        ("/me/player/recently-played", vec![R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())]),
        ("/me/player/play", vec![R::Http(204, "", String::new())]),
        ("/me/player/queue", vec![R::Http(204, "", String::new())]),
    ]);
    let (mut app, mut rx) = spotify_app(&base, "searchflow");
    app.run(app::Action::Library);
    assert!(app.lib.open);
    // `/` jumps to the Search tab with the field ready; q, o and d are letters there
    app.handle(key(K::Char('/')));
    assert!(app.lib.editing && app.lib.tab == browser::TAB_SEARCH);
    type_text(&mut app, "queen do");
    assert!(app.lib.open && app.lib.editing);
    app.handle(key(K::Enter));
    assert!(!app.lib.editing);
    pump(&mut app, &mut rx, |a| a.lib.tabs[browser::TAB_SEARCH].loaded).await;
    assert_eq!(app.lib.tabs[browser::TAB_SEARCH].items.len(), 5);
    assert_eq!(app.lib.tabs[browser::TAB_SEARCH].summary, "5 results");
    let req = log.lock().unwrap().iter().find(|l| l.contains("/search")).cloned().unwrap();
    assert!(req.contains("q=queen+do") || req.contains("q=queen%20do"), "{req}");
    // an older search that finishes late is dropped
    app.apply(AppEvent::Spotify(spotify::Reply::Search { q: "old".into(), result: Ok(vec![]) }));
    assert_eq!(app.lib.tabs[browser::TAB_SEARCH].items.len(), 5);
    // open the album (second item), see its song, go back to the results where they were
    app.handle(key(K::Down));
    app.handle(key(K::Right));
    pump(&mut app, &mut rx, |a| a.lib.stack.last().is_some_and(|l| l.loaded)).await;
    assert_eq!(app.lib.level().items[0].name, "Dreamer");
    app.handle(key(K::Left));
    assert!(app.lib.stack.is_empty() && app.lib.tabs[browser::TAB_SEARCH].sel == 1);
    // a on the song queues it, Enter plays it (alone: a search result has no context)
    app.handle(key(K::Home));
    app.handle(key(K::Char('a')));
    pump(&mut app, &mut rx, |a| a.note_text().is_some_and(|n| n.contains("queue"))).await;
    app.handle(key(K::Enter));
    assert!(!app.lib.open, "playing closes the browser");
    pump(&mut app, &mut rx, |_| log.lock().unwrap().iter().any(|l| l.starts_with("PUT /me/player/play"))).await;
    let calls = log.lock().unwrap().clone();
    assert!(calls.iter().any(|l| l.starts_with("POST /me/player/queue?uri=spotify%3Atrack%3At1")), "{calls:?}");
}

#[tokio::test]
async fn the_search_tab_asks_for_text_first_and_esc_leaves_the_field() {
    use crossterm::event::KeyCode as K;
    let (base, _log) = scripted_site(vec![("/me/player/recently-played", vec![R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())])]);
    let (mut app, _rx) = spotify_app(&base, "searchtab");
    app.run(app::Action::Library);
    app.handle(key(K::Char('6')));
    assert!(app.lib.editing, "nothing searched yet: the field is ready");
    app.handle(key(K::Esc));
    assert!(app.lib.open && !app.lib.editing, "Esc only leaves the field");
    app.handle(key(K::Esc));
    assert!(!app.lib.open);
    // an empty query runs nothing
    app.run(app::Action::Library);
    app.handle(key(K::Enter));
    assert!(!app.lib.tabs[browser::TAB_SEARCH].loading);
    // Tab walks through all seven tabs and wraps
    app.lib.editing = false;
    for _ in 0..7 {
        app.handle(key(K::Tab));
    }
    assert_eq!(app.lib.tab, browser::TAB_SEARCH);
}

#[tokio::test]
async fn the_device_picker_moves_playback_and_respects_restrictions() {
    use crossterm::event::KeyCode as K;
    let (base, log) = scripted_site(vec![
        ("/me/player/devices", vec![R::Http(200, JSON_HDR, DEVICES_JSON.into())]),
        ("PUT /me/player HTTP", vec![R::Http(204, "", String::new())]),
    ]);
    let (mut app, mut rx) = spotify_app(&base, "devices");
    assert_eq!(app::action_for(K::Char('d')), Some(app::Action::Devices));
    app.handle(key(K::Char('d')));
    assert!(app.devices.as_ref().is_some_and(|d| d.loading));
    pump(&mut app, &mut rx, |a| a.devices.as_ref().is_some_and(|d| !d.loading)).await;
    let picker = app.devices.as_ref().unwrap();
    assert_eq!(picker.items.len(), 3);
    assert_eq!(picker.sel, 0, "starts on the device that plays");
    // a restricted device (Echo) is refused with a word, nothing is sent
    app.handle(key(K::End));
    app.handle(key(K::Enter));
    assert!(app.devices.is_some());
    assert!(app.note_text().unwrap_or("").contains("restricted"));
    assert!(!log.lock().unwrap().iter().any(|l| l.starts_with("PUT")));
    // the second one takes playback
    app.handle(key(K::Up));
    app.handle(key(K::Enter));
    assert!(app.devices.is_none());
    pump(&mut app, &mut rx, |a| a.note_text().is_some_and(|n| n.contains("Playing on Pawtix"))).await;
    assert!(log.lock().unwrap().iter().any(|l| l.starts_with("PUT /me/player HTTP")));
    // Esc closes it, and it also opens from inside the browser
    app.handle(key(K::Char('d')));
    app.handle(key(K::Esc));
    assert!(app.devices.is_none());
    app.run(app::Action::Library);
    app.handle(key(K::Char('d')));
    assert!(app.devices.is_some() && app.lib.open);
}

/// Draw the app on a `w`×`h` terminal and return what is on screen, one string per row.
fn render(app: &mut app::App, w: u16, h: u16) -> Vec<String> {
    use ratatui::{Terminal, backend::TestBackend};
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect()
}

#[tokio::test]
async fn search_results_and_devices_are_drawn() {
    let (base, _log) = scripted_site(vec![("/me/player/recently-played", vec![R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())])]);
    let (mut app, _rx) = spotify_app(&base, "drawsearch");
    app.run(app::Action::Library);
    app.lib.tab = browser::TAB_SEARCH;
    let v: serde_json::Value = serde_json::from_str(SEARCH_JSON).unwrap();
    app.apply(AppEvent::Spotify(spotify::Reply::Search { q: String::new(), result: Ok(spotify::parse_search(&v, "me1")) }));
    for (w, h) in [(80u16, 24u16), (60, 12), (30, 8), (10, 4)] {
        let screen = render(&mut app, w, h).join("\n");
        if w >= 60 {
            assert!(screen.contains("Songs") && screen.contains("Bohemian"), "{w}x{h}\n{screen}");
            assert!(screen.contains("Search:"), "the field is shown: {w}x{h}");
        }
    }
    // loading a new search keeps the old results on screen
    app.lib.tabs[browser::TAB_SEARCH].loading = true;
    assert!(render(&mut app, 80, 24).join("\n").contains("Bohemian"));
    app.lib.tabs[browser::TAB_SEARCH].loading = false;
    // the device list
    let d: serde_json::Value = serde_json::from_str(DEVICES_JSON).unwrap();
    app.devices = Some(browser::DevicePicker { items: spotify::parse_devices(&d, |n| n == "Pawtix"), ..Default::default() });
    for (w, h) in [(80u16, 24u16), (40, 10), (20, 5), (10, 3)] {
        let screen = render(&mut app, w, h).join("\n");
        if w >= 40 {
            assert!(screen.contains("Phone") && screen.contains("this PC") && screen.contains("▶"), "{w}x{h}\n{screen}");
        }
    }
    app.devices = Some(browser::DevicePicker::default());
    assert!(render(&mut app, 80, 24).join("\n").contains("No Spotify devices"));
}

#[tokio::test]
async fn only_what_needs_a_word_reaches_the_footer_from_the_poller() {
    let (mut app, _rx) = settings_app("poller-notes");
    app.args.spotify = true;
    app.start_spotify();
    let err = |m: &str| AppEvent::Spotify(spotify::Reply::Remote(Err(m.into())));
    app.apply(err("no connection: dns"));
    assert!(app.note_text().is_none());
    app.apply(err("Spotify is rate limiting: try again in 30 s"));
    assert!(app.note_text().unwrap_or("").contains("30 s"));
    // a failed list shows its error in the list and in the footer
    app.apply(AppEvent::Spotify(spotify::Reply::Tab(1, Err("Spotify refused it (403): Forbidden".into()))));
    assert_eq!(app.lib.tabs[1].error.as_deref(), Some("Spotify refused it (403): Forbidden"));
    assert!(app.note_text().unwrap_or("").contains("403"));
}

// ── queue tab, cache, preload ──────────────────────────────────────────

const PLAYLISTS_JSON: &str = r#"{"items":[{"uri":"spotify:playlist:p1","name":"Roadtrip","owner":{"id":"me1","display_name":"Me"},"tracks":{"total":3}},
 {"uri":"spotify:playlist:p2","name":"Focus","owner":{"id":"me1"},"tracks":{"total":9}}],"next":null}"#;
const ALBUMS_JSON: &str = r#"{"items":[{"album":{"uri":"spotify:album:a1","name":"Opera","artists":[{"name":"Queen"}]}}],"next":null}"#;
const FOLLOWING_JSON: &str = r#"{"artists":{"items":[{"uri":"spotify:artist:r1","name":"Queen"}],"next":null}}"#;
const QUEUE_JSON: &str = r#"{"currently_playing":{"uri":"spotify:track:n1","name":"Now Song","duration_ms":1000,"artists":[{"name":"A"}]},
 "queue":[{"uri":"spotify:track:q1","name":"Next One","duration_ms":2000,"artists":[{"name":"B"}]},{"uri":"spotify:track:q2","name":"After That","duration_ms":3000,"artists":[{"name":"C"}]}]}"#;

/// A fake Spotify with every list the browser asks for.
fn lib_site() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    scripted_site(vec![
        ("GET /me HTTP", vec![R::Http(200, JSON_HDR, r#"{"id":"me1"}"#.into())]),
        ("/me/player/recently-played", vec![R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())]),
        ("/me/player/queue", vec![R::Http(200, JSON_HDR, QUEUE_JSON.into())]),
        ("/me/player/devices", vec![R::Http(200, JSON_HDR, DEVICES_JSON.into())]),
        ("/me/player/play", vec![R::Http(204, "", String::new())]),
        ("PUT /me/player HTTP", vec![R::Http(204, "", String::new())]),
        ("/me/playlists", vec![R::Http(200, JSON_HDR, PLAYLISTS_JSON.into())]),
        ("/me/albums", vec![R::Http(200, JSON_HDR, ALBUMS_JSON.into())]),
        ("/me/following", vec![R::Http(200, JSON_HDR, FOLLOWING_JSON.into())]),
        ("/me/tracks", vec![R::Http(200, JSON_HDR, r#"{"items":[],"next":null}"#.into())]),
    ])
}

#[test]
fn the_queue_is_now_playing_then_next_up() {
    let v: serde_json::Value = serde_json::from_str(QUEUE_JSON).unwrap();
    let (items, has_now) = spotify::parse_queue(&v);
    assert!(has_now);
    assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Now Song", "Next One", "After That"]);
    // nothing playing: only what is queued
    let v = serde_json::json!({"currently_playing": null, "queue": [{"uri": "spotify:track:q", "name": "Only", "artists": []}]});
    let (items, has_now) = spotify::parse_queue(&v);
    assert!(!has_now && items.len() == 1);
    assert_eq!(spotify::parse_queue(&serde_json::json!({})).0.len(), 0);
}

#[tokio::test]
async fn the_queue_tab_lists_and_plays_and_can_be_hidden() {
    use crossterm::event::KeyCode as K;
    let (base, log) = lib_site();
    let (mut app, mut rx) = spotify_app(&base, "queuetab");
    app.run(app::Action::Library);
    assert_eq!(app::action_for(K::Char('q')), Some(app::Action::Quit), "q is quit; the tab key is Q");
    assert_eq!(app::action_for(K::Char('Q')), None, "Q is free in the main view");
    app.handle(key(K::Char('Q')));
    assert_eq!(app.lib.tab, browser::TAB_QUEUE);
    pump(&mut app, &mut rx, |a| a.lib.tabs[browser::TAB_QUEUE].loaded).await;
    let l = &app.lib.tabs[browser::TAB_QUEUE];
    assert_eq!(l.items.len(), 3);
    assert_eq!(l.summary, "2 next");
    use browser::ListRow::{Head, Item};
    assert_eq!(l.rows(), [Head("Now playing"), Item(0), Head("Next up"), Item(1), Item(2)]);
    // Enter plays the one under the cursor
    app.handle(key(K::Down));
    app.handle(key(K::Enter));
    pump(&mut app, &mut rx, |_| hits(&log, "PUT /me/player/play") == 1).await;
    // 7 is the same tab, and the queue is asked for again soon (it changes all the time)
    app.lib.open = true;
    app.handle(key(K::Char('1')));
    app.handle(key(K::Char('7')));
    assert_eq!(app.lib.tab, browser::TAB_QUEUE);
    assert_eq!(hits(&log, "/me/player/queue"), 1, "just fetched: no new request");
    app.lib.tabs[browser::TAB_QUEUE].fetched = Some(Instant::now() - Duration::from_secs(6));
    app.handle(key(K::Char('1')));
    app.handle(key(K::Char('Q')));
    assert!(app.lib.tabs[browser::TAB_QUEUE].loading && !app.lib.tabs[browser::TAB_QUEUE].items.is_empty(), "old rows stay while it refreshes");
    pump(&mut app, &mut rx, |a| !a.lib.tabs[browser::TAB_QUEUE].loading).await;
    assert_eq!(hits(&log, "/me/player/queue"), 2);

    // switched off in the settings: gone from the tab row and from the keys, and saved
    let cfg = tmp("queuetab").join("config.toml");
    let _ = std::fs::remove_file(&cfg);
    app.args.config = Some(cfg.clone());
    app.change_setting(app::Setting::SpotifyQueueTab, 1);
    assert!(!app.lib.queue_tab && app.lib.visible_tabs() == [0, 1, 2, 3, 4, 5]);
    assert_eq!(app.lib.tab, 0, "the queue was on screen: back to the first tab");
    app.handle(key(K::Char('Q')));
    app.handle(key(K::Char('7')));
    assert_eq!(app.lib.tab, 0);
    for _ in 0..6 {
        app.handle(key(K::Tab));
    }
    assert_eq!(app.lib.tab, 0, "Tab wraps after six tabs");
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["spotify_queue_tab"].as_bool(), Some(false));
    let screen = render(&mut app, 100, 30).join("\n");
    assert!(screen.contains("Search") && !screen.contains("Queue"), "{screen}");
    app.change_setting(app::Setting::SpotifyQueueTab, 1);
    assert!(app.lib.queue_tab);
}

#[tokio::test]
async fn a_queue_error_reads_queue_unavailable() {
    let (base, _log) = scripted_site(vec![
        ("/me/player/queue", vec![R::Http(403, JSON_HDR, r#"{"error":{"status":403,"message":"Forbidden"}}"#.into())]),
        ("/me/player/recently-played", vec![R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())]),
    ]);
    let (mut app, mut rx) = spotify_app(&base, "queueerr");
    app.run(app::Action::Library);
    app.handle(key(crossterm::event::KeyCode::Char('Q')));
    pump(&mut app, &mut rx, |a| a.lib.tabs[browser::TAB_QUEUE].error.is_some()).await;
    let e = app.lib.tabs[browser::TAB_QUEUE].error.clone().unwrap();
    assert_eq!(e, "Queue unavailable: Spotify refused it (403): Forbidden");
    assert!(app.note_text().unwrap_or("").starts_with("Queue unavailable"));
    assert!(render(&mut app, 100, 30).join("\n").contains("Queue unavailable"));
}

#[tokio::test]
async fn an_open_browser_costs_no_request_while_the_cache_is_fresh() {
    use crossterm::event::KeyCode as K;
    let (base, log) = lib_site();
    let (mut app, mut rx) = spotify_app(&base, "cache");
    app.run(app::Action::Library);
    app.handle(key(K::Char('2')));
    pump(&mut app, &mut rx, |a| a.lib.tabs[browser::TAB_PLAYLISTS].loaded).await;
    assert_eq!(hits(&log, "/me/playlists"), 1);
    // closing and opening again, and walking through the tabs, shows the cache and asks for nothing
    for _ in 0..3 {
        app.handle(key(K::Esc));
        app.run(app::Action::Library);
        app.handle(key(K::Char('2')));
    }
    assert!(!app.lib.tabs[browser::TAB_PLAYLISTS].loading);
    assert_eq!(app.lib.tabs[browser::TAB_PLAYLISTS].items.len(), 2, "shown at once");
    assert_eq!(hits(&log, "/me/playlists"), 1, "fresh cache: no extra request");
    // a minute later it is refreshed in the background, the old rows stay until the new ones arrive
    app.lib.tabs[browser::TAB_PLAYLISTS].fetched = Some(Instant::now() - Duration::from_secs(61));
    app.handle(key(K::Char('1')));
    app.handle(key(K::Char('2')));
    assert!(app.lib.tabs[browser::TAB_PLAYLISTS].loading);
    assert_eq!(app.lib.tabs[browser::TAB_PLAYLISTS].items.len(), 2);
    assert!(render(&mut app, 100, 30).join("\n").contains("Roadtrip"), "drawn from the cache while refreshing");
    // and asking again while it is on its way doesn't pile up requests
    app.handle(key(K::Char('1')));
    app.handle(key(K::Char('2')));
    pump(&mut app, &mut rx, |a| !a.lib.tabs[browser::TAB_PLAYLISTS].loading).await;
    assert_eq!(hits(&log, "/me/playlists"), 2);
    // nothing is kept on disk
    assert!(!spotify::dir().join("library.json").exists());
}

#[tokio::test]
async fn the_lists_preload_in_the_background_one_after_the_other() {
    let (base, log) = lib_site();
    let (mut app, mut rx) = spotify_app(&base, "preload");
    app.start_preload();
    assert_eq!(log.lock().unwrap().len(), 0, "nothing leaves before the first tick");
    pump(&mut app, &mut rx, |a| [0usize, 1, 3, 4].iter().all(|i| a.lib.tabs[*i].loaded)).await;
    assert_eq!(hits(&log, "/me/playlists") + hits(&log, "/me/albums") + hits(&log, "/me/following") + hits(&log, "/me/player/recently-played"), 4);
    assert!(!app.lib.tabs[browser::TAB_LIKED].loaded, "Liked Songs (up to 1000 songs) waits until it is opened");
    // when the browser opens later everything is there, nothing is asked for
    let before = log.lock().unwrap().len();
    app.run(app::Action::Library);
    app.handle(key(crossterm::event::KeyCode::Char('2')));
    app.handle(key(crossterm::event::KeyCode::Char('4')));
    assert_eq!(app.lib.tabs[3].items[0].name, "Opera");
    assert_eq!(log.lock().unwrap().len(), before, "no request on open");
    // preloading again right away does nothing new
    app.start_preload();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(log.lock().unwrap().len(), before);
    // it only runs when Spotify is on and there is a token
    assert!(!app.preload_wanted(), "no token in the test token folder");
    std::fs::create_dir_all(spotify::dir()).unwrap();
    std::fs::write(spotify::dir().join("token.json"), r#"{"access_token":"x","refresh_token":"y","expires_at":1}"#).unwrap();
    assert!(app.preload_wanted());
    app.stop_spotify();
    assert!(!app.preload_wanted(), "off again");
    let _ = std::fs::remove_file(spotify::dir().join("token.json"));
}

#[tokio::test]
async fn retry_after_holds_back_the_next_request() {
    use crossterm::event::KeyCode as K;
    let (base, log) = scripted_site(vec![(
        "/me/player/recently-played",
        vec![R::Http(429, "Retry-After: 1\r\n", String::new()), R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())],
    )]);
    let (mut app, mut rx) = spotify_app(&base, "retryafter");
    app.run(app::Action::Library);
    pump(&mut app, &mut rx, |a| a.lib.tabs[0].error.is_some()).await;
    assert_eq!(hits(&log, "recently-played"), 1);
    assert!(app.note_text().unwrap_or("").contains("rate limiting"));
    // reloading right away doesn't leave the app
    app.handle(key(K::Char('r')));
    pump(&mut app, &mut rx, |a| !a.lib.tabs[0].loading).await;
    assert_eq!(hits(&log, "recently-played"), 1, "no request inside the window");
    assert!(app.lib.tabs[0].error.as_deref().unwrap_or("").contains("try again in"));
    // once it has run out the request goes through
    tokio::time::sleep(Duration::from_millis(1100)).await;
    app.handle(key(K::Char('r')));
    pump(&mut app, &mut rx, |a| a.lib.tabs[0].loaded).await;
    assert_eq!(hits(&log, "recently-played"), 2);
}

#[tokio::test]
async fn a_rate_limit_ends_the_preload() {
    let (base, log) = scripted_site(vec![
        ("GET /me HTTP", vec![R::Http(200, JSON_HDR, r#"{"id":"me1"}"#.into())]),
        ("/me/player/recently-played", vec![R::Http(429, "Retry-After: 1\r\n", String::new()), R::Http(200, JSON_HDR, r#"{"items":[]}"#.into())]),
        ("/me/playlists", vec![R::Http(200, JSON_HDR, PLAYLISTS_JSON.into())]),
        ("/me/albums", vec![R::Http(200, JSON_HDR, ALBUMS_JSON.into())]),
        ("/me/following", vec![R::Http(200, JSON_HDR, FOLLOWING_JSON.into())]),
    ]);
    let (mut app, mut rx) = spotify_app(&base, "preloadwait");
    app.start_preload();
    pump(&mut app, &mut rx, |a| a.lib.tabs[0].error.is_some()).await;
    // the first list was refused: the next tick inside the window drops the rest, so they
    // are not asked for even after it
    tokio::time::sleep(Duration::from_millis(200)).await;
    app.preload_tick();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    for _ in 0..5 {
        app.preload_tick();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(hits(&log, "recently-played"), 1);
    assert_eq!(hits(&log, "/me/playlists"), 0);
    // opening a tab asks again once the window has run out
    app.run(app::Action::Library);
    app.handle(key(crossterm::event::KeyCode::Char('2')));
    pump(&mut app, &mut rx, |a| a.lib.tabs[1].loaded).await;
    assert_eq!(hits(&log, "/me/playlists"), 1);
}

#[test]
fn rate_limit_waits_are_kept_in_full_and_said_in_words() {
    assert_eq!(spotify::explain(429, "", Some(69812)), "Spotify is rate limiting: try again in 19 h 24 min");
    assert_eq!(spotify::explain(429, "", Some(7200)), "Spotify is rate limiting: try again in 2 h");
    assert_eq!(spotify::explain(429, "", Some(300)), "Spotify is rate limiting: try again in 5 min");
    assert_eq!(spotify::explain(429, "", Some(30)), "Spotify is rate limiting: try again in 30 s");
}

#[tokio::test]
async fn a_long_retry_after_is_not_cut_short_and_no_header_backs_off() {
    let (base, _log) = scripted_site(vec![(
        "/me/player/pause",
        vec![
            R::Http(429, "Retry-After: 69812\r\n", String::new()),
        ],
    )]);
    let api = spotify::Api::new(reqwest::Client::new(), "cid", "").with_fake_server(&base);
    let _ = api.run(spotify::Cmd::Pause).await.unwrap_err();
    let left = api.paused_for().unwrap();
    assert!(left > Duration::from_secs(69000), "{left:?}");
    // without a header the wait doubles with every 429 in a row
    let (base, _log) = scripted_site(vec![("/me/player/pause", vec![R::Http(429, "", String::new())])]);
    let api = spotify::Api::new(reqwest::Client::new(), "cid", "").with_fake_server(&base);
    let _ = api.run(spotify::Cmd::Pause).await.unwrap_err();
    assert!(api.paused_for().unwrap() <= Duration::from_secs(5));
}

// ── side panel, overlay, button and the mouse ──────────────────────────

use crossterm::event::{Event, KeyCode as Kc, MouseButton, MouseEvent, MouseEventKind as Mk};
use ratatui::layout::Rect;

fn mouse(kind: Mk, x: u16, y: u16) -> Event {
    Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: crossterm::event::KeyModifiers::NONE })
}

fn click(app: &mut app::App, x: u16, y: u16) {
    app.handle(mouse(Mk::Down(MouseButton::Left), x, y));
}

fn text_at(screen: &[String], r: Rect) -> String {
    screen[r.y as usize].chars().skip(r.x as usize).take(r.width as usize).collect()
}

fn playlist_item(n: &str) -> spotify::Item {
    spotify::Item { kind: "playlist".into(), uri: format!("spotify:playlist:{n}"), name: format!("List {n}"), sub: "3 songs".into(), openable: true, duration: 0.0 }
}

fn track_item(n: &str) -> spotify::Item {
    spotify::Item { kind: "track".into(), uri: format!("spotify:track:{n}"), name: format!("Song {n}"), sub: "Band".into(), openable: false, duration: 61.0 }
}

/// An app on a fake Spotify with a few cached lists, the browser closed, nothing selected.
fn browser_app(base: &str, name: &str) -> (app::App, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
    use spotify::Reply;
    let (mut app, rx) = spotify_app(base, name);
    let lists: Vec<spotify::Item> = ["a", "b", "c", "d"].iter().map(|n| playlist_item(n)).collect();
    app.apply(AppEvent::Spotify(Reply::Tab(1, Ok(lists))));
    app.apply(AppEvent::Spotify(Reply::Tab(0, Ok(vec![playlist_item("r")]))));
    app.apply(AppEvent::Spotify(Reply::Tab(3, Ok(vec![playlist_item("alb")]))));
    app.apply(AppEvent::Spotify(Reply::Tab(2, Ok(vec![track_item("l1")]))));
    app.apply(AppEvent::Spotify(Reply::Tab(4, Ok(vec![]))));
    let found = vec![track_item("s1"), track_item("s2"), playlist_item("sp")];
    app.lib.query = "x".into();
    app.apply(AppEvent::Spotify(Reply::Search { q: "x".into(), result: Ok(found) }));
    app.apply(AppEvent::Spotify(Reply::Queue(Ok((vec![track_item("n"), track_item("q1")], true)))));
    (app, rx)
}

#[tokio::test]
async fn the_library_is_a_side_panel_when_wide_and_an_overlay_when_narrow() {
    let (base, _log) = lib_site();
    let (mut app, _rx) = browser_app(&base, "panelmode");
    // closed: only the button
    render(&mut app, 120, 30);
    assert_eq!(app.lib.hits.button, Rect { x: 0, y: 0, width: 11, height: 1 });
    assert!(!app.lib.hits.panel && app.lib.hits.area == Rect::default());
    assert_eq!(app.layout_key(), (false, true));
    app.run(app::Action::Library);
    app.handle(key(Kc::Char('2')));
    let screen = render(&mut app, 120, 30);
    assert!(app.lib.hits.panel);
    assert_eq!(app.lib.hits.area, Rect { x: 0, y: 1, width: ui::PANEL_WIDTH, height: 28 });
    assert!(screen[0].starts_with("[ library ]"), "{}", screen[0]);
    let first = screen.iter().position(|r| r.contains("List a")).expect("the first list is drawn");
    assert_eq!(first, 4, "border row 1, tabs 2, crumbs 3, then the list");
    assert!(screen.iter().all(|r| !r.contains("List a") || r.find("List a").unwrap() < ui::PANEL_WIDTH as usize));
    assert_eq!(app.layout_key(), (true, true), "the layout changed: the screen is cleared and drawn again");
    // the player gets the rest of the width, so it moved right of the panel
    assert!(screen.iter().any(|r| r.chars().skip(ui::PANEL_WIDTH as usize).any(|c| c != ' ')), "the player is drawn beside the panel");
    // 110 is the first width with a panel; 109 falls back to the overlay
    render(&mut app, 110, 30);
    assert!(app.lib.hits.panel);
    let screen = render(&mut app, 109, 30);
    assert!(!app.lib.hits.panel);
    assert_eq!(app.lib.hits.area, Rect { x: 18, y: 1, width: 72, height: 28 }, "centred, 72 wide");
    assert!(screen.join("\n").contains("List a"));
    assert_eq!(app.layout_key(), (false, true));
    // the minimal view has no room for either
    app.minimal = true;
    render(&mut app, 120, 30);
    assert!(!app.lib.hits.panel && app.lib.hits.button.width == 0 && app.lib.hits.area.width == 72);
    app.minimal = false;
    // no button when it is switched off, when the screen is short, or while Spotify is off
    app.args.no_spotify_button = true;
    render(&mut app, 120, 30);
    assert!(app.lib.hits.button.width == 0 && app.lib.hits.panel);
    assert_eq!(app.lib.hits.area, Rect { x: 0, y: 1, width: ui::PANEL_WIDTH, height: 28 }, "the strip stays: it holds [ settings ] and [ ? ]");
    app.args.no_spotify_button = false;
    render(&mut app, 120, 9);
    assert_eq!(app.lib.hits.button.width, 0);
    app.stop_spotify();
    let screen = render(&mut app, 120, 30).join("\n");
    assert!(!app.lib.open && app.lib.hits.button.width == 0 && !screen.contains("[ library ]"));
}

#[tokio::test]
async fn what_can_be_clicked_is_exactly_what_was_drawn() {
    let (base, _log) = lib_site();
    let (mut app, _rx) = browser_app(&base, "hitareas");
    let sizes = [(140u16, 40u16), (120, 30), (110, 12), (100, 30), (80, 24), (60, 16), (45, 12), (40, 10), (30, 8), (20, 6), (10, 4)];
    for open in [false, true] {
        for tab in [0usize, 1, 2, 3, 5, 6] {
            for (w, h) in sizes {
                app.lib.open = open;
                app.lib.tab = tab;
                app.lib.stack.clear();
                app.lib.editing = false;
                let screen = render(&mut app, w, h);
                let ctx = format!("open={open} tab={tab} {w}x{h}");
                let frame = Rect { x: 0, y: 0, width: w, height: h };
                let inside = |r: Rect, outer: Rect| r.x >= outer.x && r.y >= outer.y && r.right() <= outer.right() && r.bottom() <= outer.bottom();
                let hits = app.lib.hits.clone();
                // the button: whole text or nothing, in the top row
                if hits.button.width > 0 {
                    assert!(inside(hits.button, frame) && hits.button.y == 0 && hits.button.x == 0, "{ctx}");
                    let t = text_at(&screen, hits.button);
                    assert!(["[ library ]", "[ lib ]", "[≡]"].contains(&t.as_str()), "{ctx}: {t:?}");
                }
                if !open {
                    assert!(hits.area == Rect::default() && hits.rows.is_empty() && hits.tabs.is_empty(), "{ctx}");
                    continue;
                }
                assert!(inside(hits.area, frame), "{ctx}: {:?}", hits.area);
                // tabs: inside the box, apart from each other, and each one reads as a whole tab name
                let mut last_right = 0;
                for (r, i) in &hits.tabs {
                    assert!(inside(*r, hits.area) && r.x >= last_right, "{ctx}: tab {r:?}");
                    last_right = r.right();
                    let t = text_at(&screen, *r);
                    assert!(t == browser::TABS[*i] || t == browser::TABS_SHORT[*i], "{ctx}: {t:?} is not tab {i}");
                }
                // rows: one per line, inside the box, showing the item they say they are
                let mut last_y = None;
                for (r, i) in &hits.rows {
                    assert!(inside(*r, hits.area) && last_y.is_none_or(|y| r.y > y), "{ctx}: row {r:?}");
                    last_y = Some(r.y);
                    let t = text_at(&screen, *r);
                    let name = &app.lib.level().items[*i].name;
                    if r.width >= 24 {
                        assert!(t.contains(name.as_str()), "{ctx}: row shows {t:?}, expected {name}");
                    }
                }
                // (a box under 6 rows is too small to draw anything: no targets then)
                if hits.area.height < 6 {
                    assert!(hits.tabs.is_empty() && hits.rows.is_empty() && hits.field == Rect::default(), "{ctx}");
                    continue;
                }
                if tab == browser::TAB_SEARCH {
                    assert!(hits.field.width > 0 && inside(hits.field, hits.area), "{ctx}");
                    if hits.area.width >= 20 {
                        assert!(text_at(&screen, hits.field).starts_with("Search:"), "{ctx}: {:?}", text_at(&screen, hits.field));
                    }
                } else {
                    assert_eq!(hits.field, Rect::default(), "{ctx}");
                }
                // a click on a drawn row selects exactly that item
                if let Some((r, i)) = hits.rows.last() {
                    let before = app.lib.level().sel;
                    click(&mut app, r.x + r.width / 2, r.y);
                    let after = app.lib.level().sel;
                    // (a click on the selected row would play it: only look at the other case)
                    if *i != before {
                        assert_eq!(after, *i, "{ctx}");
                    }
                    app.lib.open = true;
                }
            }
        }
    }
}

#[tokio::test]
async fn the_mouse_works_in_the_side_panel_and_in_the_overlay() {
    for (w, h, panel) in [(120u16, 30u16, true), (90, 30, false)] {
        let (base, log) = lib_site();
        let (mut app, mut rx) = browser_app(&base, if panel { "mousepanel" } else { "mouseoverlay" });
        let ctx = format!("{w}x{h} panel={panel}");
        // the toggle button opens it
        render(&mut app, w, h);
        let b = app.lib.hits.button;
        click(&mut app, b.x + 2, b.y);
        assert!(app.lib.open, "{ctx}");
        render(&mut app, w, h);
        assert_eq!(app.lib.hits.panel, panel, "{ctx}");
        // tabs by click
        let albums = app.lib.hits.tabs.iter().find(|(_, i)| *i == 3).unwrap().0;
        click(&mut app, albums.x, albums.y);
        assert_eq!(app.lib.tab, 3, "{ctx}");
        render(&mut app, w, h);
        let lists = app.lib.hits.tabs.iter().find(|(_, i)| *i == 1).unwrap().0;
        click(&mut app, lists.x + 1, lists.y);
        assert_eq!(app.lib.tab, 1, "{ctx}");
        // a click on a row selects it, the wheel moves, nothing was asked of Spotify yet
        render(&mut app, w, h);
        let row = |app: &app::App, i: usize| app.lib.hits.rows.iter().find(|(_, n)| *n == i).unwrap().0;
        let r2 = row(&app, 2);
        click(&mut app, r2.x + 3, r2.y);
        assert_eq!(app.lib.level().sel, 2, "{ctx}");
        let r = row(&app, 2);
        app.handle(mouse(Mk::ScrollUp, r.x, r.y));
        assert_eq!(app.lib.level().sel, 0, "{ctx}: the wheel moves three rows");
        app.handle(mouse(Mk::ScrollDown, r.x, r.y));
        assert_eq!(app.lib.level().sel, 3, "{ctx}: and stops at the end");
        assert_eq!(hits(&log, "PUT"), 0, "{ctx}");
        // a click outside: the overlay closes, the panel leaves the player beside it alone
        let outside = if panel { (w - 5, h / 2) } else { (1, h / 2) };
        click(&mut app, outside.0, outside.1);
        assert_eq!(app.lib.open, panel, "{ctx}: outside click");
        app.run(app::Action::Library); // (a no-op beside the panel: it is still open)
        assert!(app.lib.open);
        render(&mut app, w, h);
        // a click on the selected row is Enter: it plays
        let sel = app.lib.level().sel;
        let r = row(&app, sel);
        click(&mut app, r.x + 2, r.y);
        pump(&mut app, &mut rx, |_| hits(&log, "PUT /me/player/play") == 1).await;
        assert_eq!(app.lib.open, panel, "{ctx}: the overlay gets out of the way, the panel stays");
        // Esc closes either
        app.lib.open = true;
        render(&mut app, w, h);
        app.handle(key(Kc::Esc));
        assert!(!app.lib.open, "{ctx}");
    }
}

#[tokio::test]
async fn beside_the_panel_the_players_keys_work_and_in_the_overlay_they_do_not() {
    for (w, panel) in [(120u16, true), (90, false)] {
        let (base, _log) = lib_site();
        let (mut app, _rx) = browser_app(&base, if panel { "keyspanel" } else { "keysoverlay" });
        app.run(app::Action::Library);
        render(&mut app, w, 30);
        assert_eq!(app.lib.hits.panel, panel);
        // keys the browser uses are used by it
        assert!(app.handle_library_key(Kc::Char('j')));
        assert!(app.handle_library_key(Kc::Tab));
        // M (minimal view) isn't one of them
        assert!(!app.handle_library_key(Kc::Char('M')));
        app.handle(key(Kc::Char('M')));
        assert_eq!(app.minimal, panel, "w={w}: only beside the panel does it reach the player");
        app.minimal = false;
        // Esc closes in both
        app.handle(key(Kc::Esc));
        assert!(!app.lib.open);
    }
}

#[tokio::test]
async fn the_device_list_takes_clicks_and_the_wheel() {
    let (base, log) = lib_site();
    let (mut app, mut rx) = spotify_app(&base, "devmouse");
    app.handle(key(Kc::Char('d')));
    pump(&mut app, &mut rx, |a| a.devices.as_ref().is_some_and(|d| !d.loading)).await;
    let screen = render(&mut app, 100, 30);
    let d = app.devices.as_ref().unwrap();
    assert_eq!(d.rows.len(), 3);
    for (r, i) in &d.rows {
        assert!(r.x >= d.area.x && r.right() <= d.area.right() && r.y >= d.area.y && r.bottom() <= d.area.bottom());
        assert!(text_at(&screen, *r).contains(&d.items[*i].name), "{:?}", text_at(&screen, *r));
    }
    let rows = d.rows.clone();
    // click selects, a second click on the same one chooses; a restricted device only says so
    click(&mut app, rows[2].0.x + 2, rows[2].0.y);
    assert_eq!(app.devices.as_ref().unwrap().sel, 2);
    click(&mut app, rows[2].0.x + 2, rows[2].0.y);
    assert!(app.devices.is_some() && app.note_text().unwrap_or("").contains("restricted"));
    assert_eq!(hits(&log, "PUT /me/player HTTP"), 0);
    app.handle(mouse(Mk::ScrollUp, rows[0].0.x, rows[0].0.y));
    assert_eq!(app.devices.as_ref().unwrap().sel, 1);
    click(&mut app, rows[1].0.x + 2, rows[1].0.y);
    assert!(app.devices.is_none());
    pump(&mut app, &mut rx, |_| hits(&log, "PUT /me/player HTTP") == 1).await;
    // a click outside closes it without choosing anything
    app.handle(key(Kc::Char('d')));
    pump(&mut app, &mut rx, |a| a.devices.as_ref().is_some_and(|d| !d.loading)).await;
    render(&mut app, 100, 30);
    click(&mut app, 0, 0);
    assert!(app.devices.is_none());
    assert_eq!(hits(&log, "PUT /me/player HTTP"), 1);
}

#[tokio::test]
async fn the_button_and_the_queue_tab_are_settings() {
    let (base, _log) = lib_site();
    let (mut app, _rx) = browser_app(&base, "btnsetting");
    let cfg = tmp("btnsetting").join("config.toml");
    let _ = std::fs::remove_file(&cfg);
    app.args.config = Some(cfg.clone());
    assert_eq!(app.setting_value(app::Setting::SpotifyButton), "[x]");
    assert_eq!(app.setting_value(app::Setting::SpotifyQueueTab), "[x]");
    render(&mut app, 100, 30);
    assert!(app.lib.hits.button.width > 0);
    app.change_setting(app::Setting::SpotifyButton, 1);
    assert!(app.args.no_spotify_button);
    render(&mut app, 100, 30);
    assert_eq!(app.lib.hits.button.width, 0, "gone: `o` still opens the library");
    app.run(app::Action::Library);
    assert!(app.lib.open);
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["spotify_button"].as_bool(), Some(false));
    // the file comes back as the same state on the next start
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x"]);
    let mut a = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
    assert!(!a.no_spotify_button && !a.no_spotify_queue_tab, "both are shown by default");
    config::apply(&mut a, &m, &cfg, true, &mut |_| {});
    assert!(a.no_spotify_button && !a.no_spotify_queue_tab);
    let out = config::render(&a, &cfg, true);
    assert!(out.contains("spotify_button = false") && out.contains("spotify_queue_tab = true"), "{out}");
    // the flags say the same
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x", "--no-spotify-button", "--no-spotify-queue-tab"]);
    let a = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
    assert!(a.no_spotify_button && a.no_spotify_queue_tab);
}

#[test]
fn spotify_device_has_no_default_and_empty_is_valid_in_the_file() {
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x"]);
    let mut a = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
    assert_eq!(a.spotify_device, "");
    assert!(!a.spotify && a.spotify_client_id.is_none());
    let path = tmp("emptydev").join("config.toml");
    std::fs::write(&path, "spotify_device = \"\"\n").unwrap();
    let mut w = Vec::new();
    config::apply(&mut a, &m, &path, true, &mut |x| w.push(x));
    assert!(w.is_empty(), "{w:?}");
    assert!(config::render(&a, &path, true).contains("spotify_device = \"\""));
    std::fs::write(&path, "spotify_device = \"My Speaker\"\n").unwrap();
    config::apply(&mut a, &m, &path, true, &mut |x| w.push(x));
    assert_eq!(a.spotify_device, "My Speaker");
}

#[tokio::test]
async fn use_spotifyd_is_a_setting_and_gates_the_device_name() {
    let (mut app, cfg) = settings_app("usespotifyd");
    app.args.spotify_device = "Living Room PC".into();
    assert_eq!(app.setting_value(app::Setting::UseSpotifyd), "[ ]");
    app.change_setting(app::Setting::UseSpotifyd, 1);
    assert!(app.args.use_spotifyd);
    assert_eq!(app.setting_value(app::Setting::UseSpotifyd), "[x]");
    let t: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(t["use_spotifyd"].as_bool(), Some(true));
    // the running client only knows the name while spotifyd is in use
    app.args.spotify = true;
    app.start_spotify();
    assert!(app.spot.as_ref().unwrap().is_local("Living Room PC"));
    app.change_setting(app::Setting::UseSpotifyd, 1);
    assert!(!app.args.use_spotifyd);
    assert!(!app.spot.as_ref().unwrap().is_local("Living Room PC"));
}

#[tokio::test]
async fn lyric_click_needs_the_text_not_the_blank_space_beside_it() {
    let mut app = test_app();
    app.lyrics = app::LyricsState::Ready(lyrics::Lyrics::Synced(vec![(10.0, "a".into()), (20.0, "line".into())]));
    app.lyric_hits = app::LyricHits {
        area: Rect::new(0, 0, 40, 4),
        top: 0,
        rows: vec![Some(0), Some(1), None, None],
        spans: vec![(19, 20), (18, 22), (0, 40), (0, 40)],
    };
    click(&mut app, 5, 1); // beside the text
    assert!(app.anchor.0 < 5.0, "{}", app.anchor.0);
    click(&mut app, 25, 1);
    assert!(app.anchor.0 < 5.0, "{}", app.anchor.0);
    click(&mut app, 20, 1); // on it
    assert!((app.anchor.0 - 20.0).abs() < 1.0, "{}", app.anchor.0);
    click(&mut app, 19, 0);
    assert!((app.anchor.0 - 10.0).abs() < 1.0, "{}", app.anchor.0);
}

// `[keys]` in config.toml: src/tests/keys_tests.rs
mod keys_tests;

// ── karaoke: word-by-word highlight ────────────────────────────────────

#[test]
fn karaoke_progress_is_a_guess_from_the_line_times() {
    // a 20-character line is sung in ~2 s (est_sung); the next line is 3 s away
    let text = "twenty characters!!!";
    assert_eq!(ui::line_progress(10.0, 10.0, Some(13.0), text), 0.0);
    assert_eq!(ui::line_progress(9.0, 10.0, Some(13.0), text), 0.0, "before it starts");
    let half = ui::line_progress(11.0, 10.0, Some(13.0), text);
    assert!(half > 0.3 && half < 0.45, "{half}");
    assert_eq!(ui::line_progress(12.8, 10.0, Some(13.0), text), 1.0, "lit before the next line");
    // a long silence after the line is not filled slowly
    assert_eq!(ui::line_progress(13.0, 10.0, Some(60.0), text), 1.0);
    // the last line has no next one
    assert_eq!(ui::line_progress(30.0, 10.0, None, text), 1.0);
}

#[test]
fn lyrics_with_word_times_lose_the_tags() {
    let l = lyrics::parse_lrc("[00:01.00]<00:01.00>Hello <00:01.50>world\n[00:03.00]a < b > c\n");
    assert_eq!(l[0].1, "Hello world");
    assert_eq!(l[1].1, "a < b > c", "not a time: kept");
}

#[test]
fn karaoke_colours_the_sung_part_of_the_big_line() {
    use ratatui::{Terminal, backend::TestBackend};
    let args = <cli::Args as clap::Parser>::parse_from(["music_display"]);
    let theme = theme::load(&args);
    let lines: Vec<String> = vec!["HI TO".to_string()];
    let fc = |progress| ui::Focus {
        lines: &lines,
        current: Some(0),
        gap: 0,
        scale: 1,
        chords: None,
        tr: 0,
        flats: false,
        context: 99,
        in_break: false,
        members: &[],
        reserve_tag: false,
        tag_below: false,
        progress,
    };
    let (w, h) = (40u16, 12u16);
    // the foreground colours of the glyph cells on the stage, left to right
    let colours = |progress| {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| { ui::draw_focus(f, Rect::new(0, 0, w, h), &theme, &fc(progress)); }).unwrap();
        let b = term.backend().buffer();
        let y = (0..h).find(|y| (0..w).any(|x| "█▀▄".contains(b[(x, *y)].symbol()))).unwrap();
        (0..w)
            .filter(|x| "█▀▄".contains(b[(*x, y)].symbol()))
            .map(|x| b[(x, y)].fg)
            .collect::<Vec<_>>()
    };
    let plain = colours(None);
    assert!(plain.iter().all(|c| *c == plain[0]), "one colour without karaoke");
    let half = colours(Some(0.5));
    assert_ne!(half.first(), half.last(), "sung part and the rest differ");
    assert_eq!(half.first(), plain.first(), "the sung part keeps the line colour");
    let none = colours(Some(0.0));
    assert!(none.iter().all(|c| *c == none[0]) && none[0] != plain[0], "nothing sung yet: all dim");
    let all = colours(Some(1.0));
    assert!(all.iter().all(|c| *c == plain[0]), "all sung: all lit");
}

#[test]
fn karaoke_splits_rows_by_characters_not_bytes() {
    // 안녕 하세요: 4 non-space characters, 2 lit -> cut after 녕 (multibyte)
    assert_eq!(ui::split_lit_for_test("안녕 하세요", 2), ("안녕", " 하세요"));
    assert_eq!(ui::split_lit_for_test("ab cd", 0), ("", "ab cd"));
    assert_eq!(ui::split_lit_for_test("ab cd", 4), ("ab cd", ""));
}

#[tokio::test]
async fn translations_round_trip_through_the_cache_file() {
    let path = std::env::temp_dir().join(format!("md-translation-{}.json", std::process::id()));
    let lines = vec!["hello".to_string(), "".to_string(), "tack".to_string()];
    translate::write_cache(&path, &lines, "libretranslate").await;
    assert_eq!(translate::read_cache(&path, 3).await, Some((lines.clone(), "libretranslate")));
    // a cache made for another number of lines (other lyrics version) is not used
    assert_eq!(translate::read_cache(&path, 4).await, None);
    assert_eq!(translate::read_cache(&path.with_extension("missing"), 3).await, None);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn window_title_follows_the_setting_and_drops_control_characters() {
    let mut app = test_app();
    app.snap = Some(snap("Band", "Song\x1b]0;evil\x07", ""));
    assert_eq!(app.window_title(), None, "off by default");
    app.args.window_title = "title".into();
    assert_eq!(app.window_title().as_deref(), Some("Song]0;evil"));
    app.args.window_title = "artist-title".into();
    assert_eq!(app.window_title().as_deref(), Some("Band – Song]0;evil"));
    app.snap = None;
    assert_eq!(app.window_title(), None, "nothing playing");
}

#[test]
fn chord_match_threshold_decides_which_sheets_show() {
    let sheet = chords::parse_chordpro("{title: T}\n[Am]Hello darkness my old friend\n[G]I've come to talk with you again\n[C]Because a vision softly creeping\n[F]Left its seeds while I was sleeping\n");
    // only two of the four sheet lines are in these lyrics: 50 %
    let l = vec![(1.0, "Hello darkness my old friend".to_string()), (5.0, "I've come to talk with you again".to_string()), (9.0, "something else entirely".to_string()), (12.0, "and something more".to_string())];
    let a = chords::align_with(&sheet, &l, 40);
    assert!(a.usable_at(40) && !a.usable_at(60));
    let strict = chords::align_with(&sheet, &l, 60);
    assert!(strict.per_line.iter().all(Vec::is_empty), "below the limit no chords are kept");
    assert!(!chords::align_with(&sheet, &l, 0).per_line.iter().all(Vec::is_empty));
}

#[test]
fn the_help_overlay_edits_keys_and_saves_them() {
    use crossterm::event::KeyCode as K;
    let dir = std::env::temp_dir().join(format!("md-help-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.toml");
    std::fs::write(&cfg, "player = \"any\"\n").unwrap();
    let mut app = test_app();
    app.args.config = Some(cfg.clone());
    app.handle(key(K::Char('?')));
    assert!(app.show_help);
    app.handle(key(K::Char('e')));
    assert!(app.help_edit.is_some(), "e opens the key editor");
    let row = app.keys.actions().iter().position(|a| *a == app::Action::Karaoke).unwrap();
    for _ in 0..row {
        app.handle(key(K::Down));
    }
    // Enter, then the new key
    app.handle(key(K::Enter));
    assert!(app.help_edit.as_ref().unwrap().capture);
    app.handle(key(K::Char('y')));
    assert_eq!(app.keys.action_for(K::Char('y')), Some(app::Action::Karaoke));
    assert_eq!(app.keys.action_for(K::Char('a')), None, "the old key is gone");
    let saved: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert_eq!(saved["keys"]["karaoke"].as_array().unwrap()[0].as_str(), Some("y"));
    assert_eq!(saved["player"].as_str(), Some("any"), "the other settings stay");
    // a key that another action has is refused and nothing changes
    app.handle(key(K::Enter));
    app.handle(key(K::Char('n')));
    assert!(app.help_edit.as_ref().unwrap().note.as_deref().unwrap().contains("already used"));
    assert_eq!(app.keys.action_for(K::Char('n')), Some(app::Action::Next));
    // Backspace goes back to the built-in key and drops the line
    app.handle(key(K::Backspace));
    assert_eq!(app.keys.action_for(K::Char('a')), Some(app::Action::Karaoke));
    let saved: toml::Table = std::fs::read_to_string(&cfg).unwrap().parse().unwrap();
    assert!(saved["keys"].as_table().unwrap().get("karaoke").is_none());
    // Esc leaves the editor, the help is still open; another key closes it
    app.handle(key(K::Esc));
    assert!(app.help_edit.is_none() && app.show_help);
    app.handle(key(K::Char('x')));
    assert!(!app.show_help);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_urls_round_trip_on_both_systems() {
    use crate::util::{file_url, path_from_url};
    use std::path::{Path, PathBuf};
    assert_eq!(file_url(Path::new("/home/me/a b.flac")), "file:///home/me/a b.flac");
    assert_eq!(file_url(Path::new("C:\\Music\\a.flac")), "file:///C:/Music/a.flac");
    assert_eq!(path_from_url("file:///home/me/My%20Song.flac"), Some(PathBuf::from("/home/me/My Song.flac")));
    // a drive letter loses the slash in front of it
    assert_eq!(path_from_url("file:///C:/Music/a.flac"), Some(PathBuf::from("C:/Music/a.flac")));
    assert_eq!(path_from_url("https://x/y"), None);
}

#[test]
fn changelog_keeps_an_unreleased_section_and_the_release_links() {
    let log = include_str!("../CHANGELOG.md");
    assert!(log.contains("\n## [Unreleased]\n"));
    assert!(log.contains("\n[Unreleased]: https://github.com/"));
    // the newest release heading is never older than the version in Cargo.toml
    let head = log.lines().find(|l| l.starts_with("## [") && !l.contains("Unreleased")).unwrap();
    let newest = head.trim_start_matches("## [").split(']').next().unwrap();
    let ver = |s: &str| s.split('.').map(|n| n.parse::<u32>().unwrap()).collect::<Vec<_>>();
    assert!(ver(newest) >= ver(env!("CARGO_PKG_VERSION")), "{newest} vs {}", env!("CARGO_PKG_VERSION"));
}

// ── Phase 2: layout, customize page, mouse editing ──────────────────────────

fn playing_app() -> app::App {
    let mut app = test_app();
    app.snap = Some(snap("Boards of Canada", "Roygbiv", "Music Has the Right to Children"));
    app
}

/// Screen column (in characters, not bytes) where `needle` starts in `row`.
fn col_of(row: &str, needle: &str) -> Option<usize> {
    row.find(needle).map(|b| row[..b].chars().count())
}

fn row_of(screen: &[String], needle: &str) -> Option<usize> {
    screen.iter().position(|r| r.contains(needle))
}

#[tokio::test]
async fn metadata_fields_follow_the_layout() {
    use layout::{Item, MetaField};
    let mut app = playing_app();
    let s = render(&mut app, 100, 30);
    let (t, a) = (row_of(&s, "Track ").unwrap(), row_of(&s, "Album ").unwrap());
    assert!(t < a, "Track above Album by default");
    // Album above Title
    app.layout.fields.shift(MetaField::Album, -2);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "Album ").unwrap() < row_of(&s, "Track ").unwrap());
    // hidden fields vanish, the rest keeps its place
    app.layout.fields.set(MetaField::Artist, false);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "Artist ").is_none() && row_of(&s, "Album ").is_some());
    assert!(app.hits.fields.iter().all(|(f, _)| *f != MetaField::Artist));
    // a field on the right-hand side can move above the first one: one row each, in order
    app.layout.fields.set(MetaField::Bpm, true);
    app.layout.fields.move_to(MetaField::Release, MetaField::Album);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "Release ").unwrap() < row_of(&s, "Album ").unwrap());
    let _ = MetaField::ALL;
}

#[tokio::test]
async fn cover_and_controls_can_be_switched_off() {
    use layout::{Button, PanelId};
    let mut app = playing_app();
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "[ pause ]").is_some() && app.hits.play.width > 0);
    app.layout.buttons.set(Button::Shuffle, false);
    app.layout.buttons.set(Button::Next, false);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "[ pause ]").is_some() && row_of(&s, "[ next ]").is_none());
    assert_eq!(app.hits.next.width, 0, "a hidden button is not clickable either");
    app.layout.panels.set(PanelId::Controls, false);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "[ pause ]").is_none() && app.hits.progress.width == 0);
    app.layout.panels.set(PanelId::Metadata, false);
    app.layout.panels.set(PanelId::Cover, false);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "Roygbiv").is_none(), "{s:#?}");
}

#[tokio::test]
async fn panels_follow_their_order_and_size() {
    use layout::PanelId;
    let mut app = playing_app();
    render(&mut app, 100, 30);
    let classic = app.hits.progress;
    assert!(classic.x < 50, "the player column is on the left");
    app.layout.panels.shift(PanelId::Lyrics, -3); // lyrics first
    render(&mut app, 100, 30);
    assert!(app.hits.progress.x >= 50, "now the player column is on the right: {:?}", app.hits.progress);
    // an explicit size
    app.layout = layout::Layout::default();
    app.layout.set_size(PanelId::Metadata, 70);
    render(&mut app, 100, 30);
    assert!(app.hits.progress.right() > 55, "{:?}", app.hits.progress);
    // vertical stack: the player column sits above the lyrics
    app.layout = layout::Layout::default();
    app.layout.direction = layout::Split::Vertical;
    render(&mut app, 100, 30);
    assert!(app.hits.progress.x < 50);
    assert!(app.hits.borders.is_empty(), "vertical splits are resized from the customize page");
}

#[tokio::test]
async fn top_buttons_open_settings_and_help() {
    let mut app = playing_app();
    let s = render(&mut app, 100, 30);
    let (sb, hb) = (app.hits.settings_btn, app.hits.help_btn);
    assert_eq!(text_at(&s, sb), "[ settings ]");
    assert_eq!(text_at(&s, hb), "[ ? ]");
    assert_eq!((sb.y, hb.y), (0, 0));
    click(&mut app, sb.x + 1, sb.y);
    assert!(app.show_settings);
    app.show_settings = false;
    click(&mut app, hb.x, hb.y);
    assert!(app.show_help);
    // no room: no buttons, no hit areas
    app.show_help = false;
    render(&mut app, 100, 8);
    assert_eq!(app.hits.settings_btn.width, 0);
}

#[tokio::test]
async fn clicking_bpm_key_capo_or_tuning_opens_the_chord_diagrams() {
    use layout::MetaField;
    let mut app = playing_app();
    assert!(!app.show_diagrams);
    assert!(!app.open_diagrams_for(MetaField::Album), "other fields do nothing");
    assert!(app.open_diagrams_for(MetaField::Bpm));
    assert!(app.show_diagrams && app.show_lyrics && app.layout.panels.shown(layout::PanelId::Chords));
    // through the mouse: a click on a drawn Release field does not
    app.show_diagrams = false;
    render(&mut app, 100, 30);
    let (_, r) = *app.hits.fields.iter().find(|(f, _)| *f == MetaField::Release).unwrap();
    click(&mut app, r.x, r.y);
    app.handle(mouse(Mk::Up(MouseButton::Left), r.x, r.y));
    assert!(!app.show_diagrams);
}

#[tokio::test]
async fn right_click_menu_hides_and_moves_a_field() {
    use layout::{Item, MetaField};
    let (mut app, cfg) = settings_app("fieldmenu");
    app.snap = Some(snap("Boards of Canada", "Roygbiv", "Music Has the Right to Children"));
    render(&mut app, 100, 30);
    let (_, r) = *app.hits.fields.iter().find(|(f, _)| *f == MetaField::Artist).unwrap();
    app.handle(mouse(Mk::Down(MouseButton::Right), r.x + 1, r.y));
    assert_eq!(app.field_menu.map(|m| m.field), Some(MetaField::Artist));
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "Move up").is_some() && row_of(&s, "Hide").is_some());
    // "Move down" by mouse: Artist swaps with Album
    let down = app.hits.menu_items[2];
    click(&mut app, down.x + 1, down.y);
    assert!(app.field_menu.is_none());
    assert!(app.layout.fields.position(MetaField::Artist) > app.layout.fields.position(MetaField::Album));
    // "Hide" by keyboard
    render(&mut app, 100, 30);
    let (_, r) = *app.hits.fields.iter().find(|(f, _)| *f == MetaField::Album).unwrap();
    app.handle(mouse(Mk::Down(MouseButton::Right), r.x, r.y));
    app.handle(key(crossterm::event::KeyCode::Enter));
    assert!(!app.layout.fields.shown(MetaField::Album));
    // everything was saved as [layout]
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("[layout.fields]") && saved.contains("album"), "{saved}");
    // a click elsewhere just closes the menu
    app.handle(mouse(Mk::Down(MouseButton::Right), r.x, r.y));
    app.handle(mouse(Mk::Down(MouseButton::Left), 0, 29));
    assert!(app.field_menu.is_none());
    let _ = MetaField::ALL;
}

#[tokio::test]
async fn dragging_a_field_reorders_and_dragging_a_border_resizes() {
    use layout::{MetaField, PanelId};
    let (mut app, cfg) = settings_app("dragging");
    app.snap = Some(snap("Boards of Canada", "Roygbiv", "Music Has the Right to Children"));
    render(&mut app, 100, 30);
    let find = |app: &app::App, f: MetaField| app.hits.fields.iter().find(|(x, _)| *x == f).unwrap().1;
    let (album, title) = (find(&app, MetaField::Album), find(&app, MetaField::Title));
    app.handle(mouse(Mk::Down(MouseButton::Left), album.x, album.y));
    app.handle(mouse(Mk::Drag(MouseButton::Left), title.x, title.y));
    app.handle(mouse(Mk::Up(MouseButton::Left), title.x, title.y));
    assert_eq!(app.layout.fields.items[0].0, MetaField::Album, "dropped on Track: Album takes its place");
    // the border between the player column and the lyrics
    render(&mut app, 100, 30);
    let b = app.hits.borders[0];
    assert_eq!(b.left, PanelId::Metadata);
    assert!(app.layout.is_automatic());
    app.handle(mouse(Mk::Down(MouseButton::Left), b.rect.x, b.rect.y + 2));
    app.handle(mouse(Mk::Drag(MouseButton::Left), 60, b.rect.y + 2));
    app.handle(mouse(Mk::Up(MouseButton::Left), 60, b.rect.y + 2));
    assert_eq!(app.layout.size_of(PanelId::Metadata), Some(60));
    assert!(!app.layout.is_automatic());
    render(&mut app, 100, 30);
    assert!(app.hits.borders[0].rect.x >= 58 && app.hits.borders[0].rect.x <= 60, "{:?}", app.hits.borders[0].rect);
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("metadata = 60"));
}

#[tokio::test]
async fn customize_page_toggles_moves_resizes_and_resets() {
    use crossterm::event::KeyCode as K;
    use layout::{MetaField, PanelId};
    let (mut app, cfg) = settings_app("customize");
    app.snap = Some(snap("A", "B", "C"));
    app.show_settings = true;
    let idx = app::Setting::ALL.iter().position(|s| *s == app::Setting::Customize).unwrap();
    app.settings_sel = idx;
    app.handle(key(K::Enter));
    assert!(app.customizing);
    let s = render(&mut app, 100, 40);
    assert!(row_of(&s, "customize layout").is_some() && row_of(&s, "Metadata fields").is_some());
    // first line: the Cover panel (it is on top). Down to Metadata, hide it
    assert_eq!(app.cust_items()[0], app::CustItem::Panel(PanelId::Cover));
    app.handle(key(K::Down));
    app.handle(key(K::Char(' ')));
    assert!(!app.layout.panels.shown(PanelId::Metadata));
    // move Lyrics up (reorder keeps the selection on it)
    while app.cust_items()[app.cust_sel] != app::CustItem::Panel(PanelId::Lyrics) {
        app.handle(key(K::Down));
    }
    app.handle(key(K::Char('u')));
    assert_eq!(app.cust_items()[app.cust_sel], app::CustItem::Panel(PanelId::Lyrics));
    assert!(app.layout.panels.position(PanelId::Lyrics) < app.layout.panels.position(PanelId::Controls));
    // resize it: + starts from its automatic share
    app.handle(key(K::Char('+')));
    assert_eq!(app.layout.size_of(PanelId::Lyrics), Some(65));
    app.handle(key(K::Char('-')));
    app.handle(key(K::Char('-')));
    assert_eq!(app.layout.size_of(PanelId::Lyrics), Some(55));
    // a field
    while app.cust_items()[app.cust_sel] != app::CustItem::Field(MetaField::Genre) {
        app.handle(key(K::Down));
    }
    app.handle(key(K::Enter));
    assert!(!app.layout.fields.shown(MetaField::Genre));
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("[layout]") && saved.contains("hidden"), "{saved}");
    // auto sizes, then reset everything
    app.handle(key(K::Char('a')));
    assert!(app.layout.sizes.is_empty());
    app.handle(key(K::Char('r')));
    assert_eq!(app.layout, layout::Layout::default());
    // Esc leaves the page, not the settings menu
    app.handle(key(K::Esc));
    assert!(!app.customizing && app.show_settings);
}

#[tokio::test]
async fn layout_loads_from_the_file_and_flags_win() {
    use clap::Parser;
    use layout::PanelId;
    let dir = tmp("layoutload");
    let cfg = dir.join("config.toml");
    std::fs::write(
        &cfg,
        "player = \"spotify\"\n[layout]\norder = [\"lyrics\", \"metadata\"]\nhidden = [\"cava\"]\n[layout.size]\nlyrics = 30\n[layout.fields]\nhidden = [\"track_no\"]\n[keys]\nquit = \"x\"\n",
    )
    .unwrap();
    let args = cli::Args::parse_from(["x"]);
    let mut warns = Vec::new();
    let l = config::load_layout(&args, &cfg, &mut |w| warns.push(w));
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!(l.panels.items[0].0, PanelId::Lyrics);
    assert_eq!(l.size_of(PanelId::Lyrics), Some(30));
    assert!(!l.fields.shown(layout::MetaField::TrackNo));
    // a flag beats the file; --layout-preset replaces it; --reset-layout ignores it
    let l = config::load_layout(&cli::Args::parse_from(["x", "--nolyrics"]), &cfg, &mut |_| {});
    assert!(!l.panels.shown(PanelId::Lyrics));
    let l = config::load_layout(&cli::Args::parse_from(["x", "--layout-preset", "under-lyrics"]), &cfg, &mut |_| {});
    assert!(l.cava_under_lyrics && l.is_automatic());
    let l = config::load_layout(&cli::Args::parse_from(["x", "--reset-layout"]), &cfg, &mut |_| {});
    assert!(l.is_automatic());
    let mut w = Vec::new();
    config::load_layout(&cli::Args::parse_from(["x", "--layout-preset", "wat"]), &cfg, &mut |m| w.push(m));
    assert!(w[0].contains("unknown preset"), "{w:?}");
    // the [layout] table does not trip the unknown-key warning, and [keys] survives a save
    let (_, warns) = cfg_run(&std::fs::read_to_string(&cfg).unwrap(), &[]);
    assert!(warns.is_empty(), "{warns:?}");
    config::save_layout(&cfg, &layout::Layout::default()).unwrap();
    let raw = std::fs::read_to_string(&cfg).unwrap();
    assert!(raw.contains("quit") && raw.contains("player"), "{raw}");
    // --reset-layout also removes the saved table
    config::forget_layout(&cfg).unwrap();
    assert!(!std::fs::read_to_string(&cfg).unwrap().contains("[layout"));
}

#[tokio::test]
async fn layout_sets_the_start_state() {
    use layout::PanelId;
    let mut app = test_app();
    let l = layout::Layout::default().with_flags(true, false, false, true);
    app.set_layout(l);
    assert!(!app.show_lyrics && app.show_cava);
    let mut app = test_app();
    app.set_layout(layout::Layout::preset("under-lyrics").unwrap());
    assert!(app.show_cava && app.placement == app::CavaPlacement::UnderLyrics);
    let mut app = test_app();
    let mut l = layout::Layout::default();
    l.panels.set(PanelId::Chords, true);
    app.set_layout(l);
    assert!(app.show_diagrams);
}

#[test]
fn every_panel_id_has_a_panel_with_a_section_and_a_minimum_size() {
    use layout::{Item, PanelId};
    for id in PanelId::ALL {
        let p = ui::panel_for(*id);
        assert_eq!(p.id(), *id);
        assert!(!p.title().is_empty() && p.config_section().starts_with("layout"));
        let (w, h) = p.min_size();
        assert!(w >= 8 && h >= 1);
    }
    assert_eq!(ui::panel_for(PanelId::Metadata).config_section(), "layout.fields");
}

// ── Phase 3: genre ──────────────────────────────────────────────────────────

#[tokio::test]
async fn genre_comes_from_the_player_then_the_tags_then_musicbrainz() {
    let mut app = test_app();
    app.args.genre_lookup = false;
    // the player's own xesam:genre
    let mut s = snap("Boards of Canada", "Roygbiv", "MHTRTC");
    s.genre = "electronic; ambient".into();
    app.on_snapshot(s);
    assert_eq!(app.genre.as_deref(), Some("Electronic / Ambient"));
    // a new song without one: nothing shown, and no lookup when switched off
    app.on_snapshot(snap("Other", "Song", "Album"));
    assert_eq!(app.genre, None);
    // MusicBrainz fills in, but only for the song it was asked about
    app.args.genre_lookup = true;
    app.on_snapshot(snap("Third", "Tune", "LP"));
    let key = app.track_key;
    app.apply(AppEvent::Genre { key: key ^ 1, genres: Some(vec!["Jazz".into()]) });
    assert_eq!(app.genre, None, "a stale answer is dropped");
    app.apply(AppEvent::Genre { key, genres: Some(vec!["Rock".into(), "Pop".into()]) });
    assert_eq!(app.genre.as_deref(), Some("Rock / Pop"));
    // the player wins over MusicBrainz, and the next track starts clean
    let mut s = snap("Third", "Tune", "LP");
    s.genre = "Folk".into();
    app.on_snapshot(s);
    assert_eq!(app.genre.as_deref(), Some("Folk"));
    app.on_snapshot(snap("Fourth", "Next", "LP2"));
    assert_eq!(app.genre, None);
}

#[tokio::test]
async fn the_genre_field_is_drawn_and_can_be_hidden() {
    use layout::MetaField;
    let mut app = playing_app();
    assert!(row_of(&render(&mut app, 100, 30), "Genre ").is_none(), "nothing known: no row");
    app.genre = Some("Electronic / Ambient".into());
    let s = render(&mut app, 100, 30);
    let r = row_of(&s, "Genre ").expect("shown");
    assert!(s[r].contains("Electronic / Ambient"));
    assert!(app.hits.fields.iter().any(|(f, _)| *f == MetaField::Genre));
    app.layout.fields.set(MetaField::Genre, false);
    assert!(row_of(&render(&mut app, 100, 30), "Genre ").is_none());
}

#[test]
fn mpris_genre_is_parsed_from_the_last_field() {
    let line = "Playing\x1f1000000\x1fT\x1fA\x1fAl\x1f200000000\x1f\x1f1\x1f1\x1f\x1f0.5\x1f\x1fid\x1fvlc\x1f[Rock, Pop]";
    let s = player::parse(line).unwrap();
    assert_eq!(s.genre, "Rock, Pop");
    // an older player line without the field still parses
    let s = player::parse(&line[..line.rfind('\x1f').unwrap()]).unwrap();
    assert_eq!(s.genre, "");
}

// ── Phase 4: AI translation ─────────────────────────────────────────────────

fn ai_app(site: &str, name: &str) -> (app::App, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
    util::enter_test_mode();
    let args = cli::Args::parse_from([
        "music_display",
        "--translate",
        "--translate-engine",
        "ai",
        "--ai-model",
        "tester",
        "--ai-endpoint",
        site,
        "--no-cache-translations",
    ]);
    let theme = theme::load(&args);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
    app.args.config = Some(tmp(name).join("config.toml"));
    app.on_snapshot(snap("Artist", "Hej", "Album"));
    app.lyrics = app::LyricsState::Ready(lyrics::Lyrics::Plain(vec!["Jag gillar bär".into(), "Solen går ner".into()]));
    (app, rx)
}

fn chat_reply(content: &str) -> R {
    let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
    R::Http(200, "Content-Type: application/json\r\n", body)
}

#[tokio::test]
async fn ai_translates_the_whole_song_in_one_request() {
    let (site, log) = scripted_site(vec![("/chat/completions", vec![chat_reply(r#"["I like berries","The sun goes down"]"#)])]);
    let (mut app, mut rx) = ai_app(&site, "ai_ok");
    app.ensure_translation();
    pump(&mut app, &mut rx, |a| a.translation().is_some()).await;
    assert_eq!(app.translation().unwrap(), &vec!["I like berries".to_string(), "The sun goes down".to_string()]);
    assert_eq!(log.lock().unwrap().iter().filter(|l| l.contains("/chat/completions")).count(), 1, "one request for the song");
    // the title names the model
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "translated by tester (en)").is_some(), "{s:#?}");
}

#[tokio::test]
async fn ai_that_keeps_failing_falls_back_to_the_standard_engine_and_says_so() {
    // wrong line count twice (the retry too); Google answers
    let (site, log) = scripted_site(vec![
        ("/chat/completions", vec![chat_reply(r#"["only one"]"#)]),
        ("/g/dict", vec![R::Http(429, "", String::new())]),
        ("/g?", vec![R::Http(429, "", String::new())]),
        ("/libre", vec![R::Http(200, "Content-Type: application/json\r\n", r#"{"translatedText":"berries"}"#.into())]),
    ]);
    let (mut app, mut rx) = ai_app(&site, "ai_fail");
    app.translate_base = Some(format!("{site}/g"));
    app.args.translate_fallback_url = format!("{site}/libre");
    app.args.translate_fallback_key = Some("k".into());
    app.ensure_translation();
    pump(&mut app, &mut rx, |a| a.translation().is_some()).await;
    assert!(app.translate_backend.starts_with(app::FALLBACK_TAG), "{}", app.translate_backend);
    assert_eq!(log.lock().unwrap().iter().filter(|l| l.contains("/chat/completions")).count(), 2, "one retry, then give up");
    let s = render(&mut app, 120, 30);
    assert!(row_of(&s, "AI failed").is_some(), "the title says the AI failed: {s:#?}");
}

#[tokio::test]
async fn ai_without_a_model_uses_the_standard_engine_with_a_note() {
    let (site, _log) = scripted_site(vec![("/g", vec![R::Http(200, "Content-Type: application/json\r\n", r#"[[["x","y"]],null,"sv"]"#.into())])]);
    let (mut app, _rx) = ai_app(&site, "ai_nomodel");
    app.args.ai_model.clear();
    app.translate_base = Some(format!("{site}/g"));
    app.ensure_translation();
    assert!(app.note_text().unwrap_or("").contains("needs a model"));
}

#[tokio::test]
async fn the_privacy_notice_is_for_remote_providers_only_and_needs_a_second_press() {
    let (mut app, _rx) = ai_app("http://localhost:11434/v1", "ai_notice_local");
    let _ = std::fs::remove_file(spotify::dir().with_file_name("ai-notice-ack"));
    assert!(!app.ai_notice_needed(), "a local Ollama endpoint needs no notice");
    // a remote provider: the first press only explains
    app.args.translate_engine = cli::TranslateEngine::Standard;
    app.args.ai_provider = "openai".into();
    app.args.ai_endpoint = String::new();
    assert!(app.ai_notice_needed());
    app.change_setting(app::Setting::TranslateEngine, 1);
    assert_eq!(app.args.translate_engine, cli::TranslateEngine::Standard);
    let note = app.settings_note.clone().unwrap();
    assert!(note.contains("sends the lyrics") && note.contains("api.openai.com"), "{note}");
    // the second press turns it on, and the notice is not shown again
    app.change_setting(app::Setting::TranslateEngine, 1);
    assert_eq!(app.args.translate_engine, cli::TranslateEngine::Ai);
    assert!(!app.ai_notice_needed());
    let _ = std::fs::remove_file(spotify::dir().with_file_name("ai-notice-ack"));
}

#[tokio::test]
async fn ai_settings_are_saved_under_the_ai_table_and_the_key_is_never_printed() {
    use crossterm::event::KeyCode as K;
    let (mut app, _rx) = ai_app("http://localhost:11434/v1", "ai_save");
    let cfg = app.args.config.clone().unwrap();
    let _ = std::fs::remove_file(&cfg);
    app.args.translate_engine = cli::TranslateEngine::Standard;
    app.show_settings = true;
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::AiModel).unwrap();
    app.handle(key(K::Enter));
    for _ in 0..16 {
        app.handle(key(K::Backspace));
    }
    for c in "llama3.1".chars() {
        app.handle(key(K::Char(c)));
    }
    app.handle(key(K::Enter));
    assert_eq!(app.args.ai_model, "llama3.1");
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("[ai]") && saved.contains("model = \"llama3.1\""), "{saved}");
    // config round trip; a bad provider is reported
    let (a, w) = cfg_run("translate_engine = \"ai\"\n[ai]\nprovider = \"claude\"\nmodel = \"m\"\napi_key_env = \"MY_KEY\"\n", &[]);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!((a.translate_engine, a.ai_provider.as_str(), a.ai_model.as_str(), a.ai_key_env.as_str()), (cli::TranslateEngine::Ai, "claude", "m", "MY_KEY"));
    let (_, w) = cfg_run("[ai]\nprovider = \"skynet\"\nbogus = 1\n", &[]);
    assert!(w.iter().any(|m| m.contains("provider")) && w.iter().any(|m| m.contains("bogus")), "{w:?}");
    // a flag beats the file
    let (a, _) = cfg_run("[ai]\nmodel = \"file\"\n", &["--ai-model", "flag"]);
    assert_eq!(a.ai_model, "flag");
    // --print-config shows the variable's name, never a key
    unsafe { std::env::set_var("MY_SECRET_KEY_FOR_TEST", "sk-super-secret") };
    let (a, _) = cfg_run("[ai]\napi_key_env = \"MY_SECRET_KEY_FOR_TEST\"\n", &[]);
    let printed = config::render(&a, std::path::Path::new("x"), true);
    assert!(printed.contains("MY_SECRET_KEY_FOR_TEST") && !printed.contains("sk-super-secret"));
}

// ── Phase 5: annotations ────────────────────────────────────────────────────

const GENIUS_JSON: &str = "Content-Type: application/json\r\n";

fn genius_site(artist: &str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let search = serde_json::json!({"response": {"hits": [{"result": {"id": 77, "title": "Roygbiv", "primary_artist": {"name": artist}}}]}});
    let referents = serde_json::json!({"response": {"referents": [
        {"fragment": "We build castles in the sand", "annotations": [{"url": "https://genius.com/1", "body": {"plain": "A metaphor for fragile things people work hard on."}}]}
    ]}});
    scripted_site(vec![
        ("/search", vec![R::Http(200, GENIUS_JSON, search.to_string())]),
        ("/referents", vec![R::Http(200, GENIUS_JSON, referents.to_string())]),
    ])
}

fn notes_app(artist: &str, site: &str) -> (app::App, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
    util::enter_test_mode();
    let args = cli::Args::parse_from(["music_display", "--annotations", "--genius-token", "tok123"]);
    let theme = theme::load(&args);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = app::App::new(args, theme, ratatui_image::picker::Picker::halfblocks(), tx);
    app.set_layout(layout::Layout::default());
    app.annotations_base = Some(site.to_string());
    app.args.config = Some(tmp("notes").join("config.toml"));
    app.on_snapshot(snap(artist, "Roygbiv", "Album"));
    app.lyrics = app::LyricsState::Ready(lyrics::Lyrics::Synced(vec![
        (0.0, "Intro line goes here".into()),
        (10.0, "We build castles in the sand".into()),
    ]));
    (app, rx)
}

#[tokio::test]
async fn genius_annotations_follow_the_active_line() {
    let artist = "NotesTest Genius Band";
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
    annotations::reset_breaker();
    let (site, log) = genius_site(artist);
    let (mut app, mut rx) = notes_app(artist, &site);
    assert!(app.annotations_shown());
    app.ensure_annotations();
    pump(&mut app, &mut rx, |a| matches!(a.ann_state, app::AnnState::Ready)).await;
    assert_eq!(app.ann_match, vec![None, Some(0)]);
    // before the matching line: nothing; on it: the excerpt, the quoted passage and the source
    app.set_position_for_test(2.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("no annotation for"), "{s}");
    app.set_position_for_test(11.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("fragile") && s.contains("— Genius") && s.contains("castles in the"), "{s}");
    assert!(s.contains("annotations"));
    // the token went out as a bearer header only
    assert!(log.lock().unwrap().iter().all(|l| !l.contains("tok123")));
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
}

#[tokio::test]
async fn annotation_misses_are_cached_and_errors_never() {
    annotations::reset_breaker();
    // a miss: the search finds nothing; the second ask costs no request
    let artist = "NotesTest Missing Band";
    let path = annotations::cache_path(artist, "Roygbiv");
    let _ = std::fs::remove_file(&path);
    let empty = serde_json::json!({"response": {"hits": []}});
    let (site, log) = scripted_site(vec![("/search", vec![R::Http(200, GENIUS_JSON, empty.to_string())])]);
    let c = reqwest::Client::new();
    assert_eq!(annotations::fetch_from(&c, &site, "t", artist, "Roygbiv").await, Ok(vec![]));
    assert!(std::fs::read_to_string(&path).unwrap().contains("\"none\""));
    assert_eq!(annotations::fetch_from(&c, &site, "t", artist, "Roygbiv").await, Ok(vec![]));
    assert_eq!(log.lock().unwrap().len(), 1);
    let _ = std::fs::remove_file(&path);
    // an error is reported, never cached, and never contains the token
    let artist = "NotesTest Error Band";
    let path = annotations::cache_path(artist, "Roygbiv");
    let _ = std::fs::remove_file(&path);
    let (site, _log) = scripted_site(vec![("/search", vec![R::Http(500, "", String::new())])]);
    let e = annotations::fetch_from(&c, &site, "secret-token-xyz", artist, "Roygbiv").await.unwrap_err();
    assert!(e.contains("500") && !e.contains("secret-token-xyz"), "{e}");
    assert!(!path.exists());
    annotations::reset_breaker();
    // a refused token says so
    let (site, _log) = scripted_site(vec![("/search", vec![R::Http(401, "", String::new())])]);
    let e = annotations::fetch_from(&c, &site, "secret-token-xyz", artist, "Roygbiv").await.unwrap_err();
    assert!(e.contains("refused the token") && !e.contains("secret-token-xyz"));
    annotations::reset_breaker();
}

#[tokio::test]
async fn without_a_token_the_panel_says_how_to_get_one_and_asks_nothing() {
    let (mut app, _rx) = notes_app("NotesTest NoToken", "http://127.0.0.1:1");
    app.args.genius_token.clear();
    // (the environment may hold one on a developer machine)
    if app.genius_token().is_some() {
        return;
    }
    app.ensure_annotations();
    assert!(matches!(&app.ann_state, app::AnnState::Failed(e) if e.contains("genius.com/api-clients")));
    let s = render(&mut app, 120, 30).join("\n");
    // (the column is narrow, so the message wraps)
    assert!(s.contains("no token") && s.contains("genius.com"), "{s}");
}

#[tokio::test]
async fn ai_notes_are_labelled_and_per_line() {
    let reply = chat_reply(r#"[null, "A nod to Hamlet's 'to be or not to be'."]"#);
    let (site, _log) = scripted_site(vec![("/chat/completions", vec![reply])]);
    let artist = "NotesTest AI Band";
    let (mut app, mut rx) = notes_app(artist, "http://127.0.0.1:1");
    app.args.annotations_mode = cli::AnnotationMode::Ai;
    app.args.ai_model = "tester".into();
    app.args.ai_endpoint = site;
    app.ensure_annotations();
    pump(&mut app, &mut rx, |a| matches!(a.ann_state, app::AnnState::Ready)).await;
    app.set_position_for_test(11.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("Hamlet") && s.contains("AI-generated"), "{s}");
    assert!(s.contains("annotations"));
    let _ = std::fs::remove_file(annotations::ai_cache_for_test(&app.ai_cfg().unwrap(), artist, "Roygbiv", &app.lyric_lines().unwrap()));
}

#[tokio::test]
async fn the_annotations_key_and_settings_rows_work_and_persist() {
    use crossterm::event::KeyCode as K;
    let (mut app, cfg) = settings_app("notes_settings");
    assert!(!app.annotations_shown(), "off by default");
    app.handle(key(K::Char('N')));
    assert!(app.annotations_shown());
    app.handle(key(K::Char('N')));
    assert!(!app.annotations_shown());
    app.show_settings = true;
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::Annotations).unwrap();
    app.handle(key(K::Enter));
    assert!(app.annotations_shown());
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("annotations = true"));
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::AnnotationsMode).unwrap();
    app.handle(key(K::Enter));
    assert_eq!(app.args.annotations_mode, cli::AnnotationMode::Ai);
    // the token row never shows the token
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::GeniusToken).unwrap();
    app.handle(key(K::Enter));
    for c in "abc-secret".chars() {
        app.handle(key(K::Char(c)));
    }
    app.handle(key(K::Enter));
    assert_eq!(app.args.genius_token, "abc-secret");
    assert_eq!(app.setting_value(app::Setting::GeniusToken), "set (hidden)");
    let printed = config::render(&app.args, std::path::Path::new("x"), true);
    assert!(!printed.contains("abc-secret"));
    // the config file round-trips the new keys
    let (a, w) = cfg_run("annotations = true\nannotations_mode = \"ai\"\ngenius_token = \"t\"\n", &[]);
    assert!(w.is_empty(), "{w:?}");
    assert!(a.annotations && a.annotations_mode == cli::AnnotationMode::Ai && a.genius_token == "t");
}

#[tokio::test]
async fn genius_ai_stacks_both_and_asks_each_service_once_per_song() {
    let artist = "NotesTest GeniusAI Band";
    let lines = vec!["Intro line goes here".to_string(), "We build castles in the sand".to_string()];
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
    annotations::reset_breaker();
    let (gsite, glog) = genius_site(artist);
    // line 2 repeats Genius and must be dropped; line 1 is new
    let reply = chat_reply(r#"["The intro was recorded in one take.", "A metaphor for fragile things people work hard on."]"#);
    let (asite, alog) = scripted_site(vec![("/chat/completions", vec![reply])]);
    let (mut app, mut rx) = notes_app(artist, &gsite);
    app.args.annotations_mode = cli::AnnotationMode::GeniusAi;
    app.args.ai_model = "tester".into();
    app.args.ai_endpoint = asite.clone();
    let cfg = app.ai_cfg().unwrap();
    let genius = vec![None, Some("A metaphor for fragile things people work hard on.".to_string())];
    let extra_cache = annotations::ai_extra_cache_for_test(&cfg, artist, "Roygbiv", &lines, &genius);
    let _ = std::fs::remove_file(&extra_cache);
    app.ensure_annotations();
    pump(&mut app, &mut rx, |a| matches!(a.ai_state, app::AnnState::Ready)).await;
    assert!(matches!(app.ann_state, app::AnnState::Ready));
    // one Genius search, one referents call, one AI request for the whole song
    assert_eq!(glog.lock().unwrap().len(), 2, "{:?}", glog.lock().unwrap());
    assert_eq!(alog.lock().unwrap().len(), 1, "{:?}", alog.lock().unwrap());
    assert_eq!(app.ai_notes, vec![Some("The intro was recorded in one take.".to_string()), None]);
    // the line Genius covers: Genius only (the AI repeat was dropped)
    app.set_position_for_test(11.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("fragile") && s.contains("— Genius") && !s.contains("AI extra"), "{s}");
    // the intro: the AI note alone, labelled as AI
    app.set_position_for_test(2.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("recorded in one") && s.contains("AI-generated"), "{s}");
    // the next play of the song asks nobody
    let (mut again, mut rx2) = notes_app(artist, &gsite);
    again.args.annotations_mode = cli::AnnotationMode::GeniusAi;
    again.args.ai_model = "tester".into();
    again.args.ai_endpoint = asite;
    again.ensure_annotations();
    pump(&mut again, &mut rx2, |a| matches!(a.ai_state, app::AnnState::Ready)).await;
    assert_eq!(glog.lock().unwrap().len(), 2);
    assert_eq!(alog.lock().unwrap().len(), 1);
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
    let _ = std::fs::remove_file(&extra_cache);
}

#[tokio::test]
async fn genius_ai_shows_the_ai_extra_under_the_genius_note() {
    let artist = "NotesTest GeniusAI Stack";
    let lines = vec!["Intro line goes here".to_string(), "We build castles in the sand".to_string()];
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
    annotations::reset_breaker();
    let (gsite, _glog) = genius_site(artist);
    let reply = chat_reply(r#"[null, "Jimi Hendrix released a song with the same title in 1967."]"#);
    let (asite, _alog) = scripted_site(vec![("/chat/completions", vec![reply])]);
    let (mut app, mut rx) = notes_app(artist, &gsite);
    app.args.annotations_mode = cli::AnnotationMode::GeniusAi;
    app.args.ai_model = "tester".into();
    app.args.ai_endpoint = asite;
    let genius = vec![None, Some("A metaphor for fragile things people work hard on.".to_string())];
    let extra_cache = annotations::ai_extra_cache_for_test(&app.ai_cfg().unwrap(), artist, "Roygbiv", &lines, &genius);
    let _ = std::fs::remove_file(&extra_cache);
    app.ensure_annotations();
    pump(&mut app, &mut rx, |a| matches!(a.ai_state, app::AnnState::Ready)).await;
    app.set_position_for_test(11.0);
    let rows = render(&mut app, 120, 40);
    let at = |t: &str| rows.iter().position(|r| r.contains(t)).unwrap_or_else(|| panic!("{t}: {}", rows.join("\n")));
    assert!(at("fragile") < at("AI extra") && at("AI extra") < at("Hendrix"));
    assert!(at("Hendrix") < at("AI-generated"));
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
    let _ = std::fs::remove_file(&extra_cache);
}

#[tokio::test]
async fn genius_ai_without_a_token_still_asks_the_ai() {
    let artist = "NotesTest GeniusAI NoToken";
    let reply = chat_reply(r#"[null, "Sand castles are a common image for impermanence."]"#);
    let (asite, alog) = scripted_site(vec![("/chat/completions", vec![reply])]);
    let (mut app, mut rx) = notes_app(artist, "http://127.0.0.1:1");
    app.args.genius_token.clear();
    if app.genius_token().is_some() {
        return; // (a developer machine with GENIUS_TOKEN set)
    }
    app.args.annotations_mode = cli::AnnotationMode::GeniusAi;
    app.args.ai_model = "tester".into();
    app.args.ai_endpoint = asite;
    let lines = app.lyric_lines().unwrap();
    let extra_cache = annotations::ai_extra_cache_for_test(&app.ai_cfg().unwrap(), artist, "Roygbiv", &lines, &[None, None]);
    let _ = std::fs::remove_file(&extra_cache);
    app.ensure_annotations();
    pump(&mut app, &mut rx, |a| matches!(a.ai_state, app::AnnState::Ready)).await;
    assert_eq!(alog.lock().unwrap().len(), 1);
    app.set_position_for_test(11.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("impermanence") && s.contains("no token"), "{s}");
    let _ = std::fs::remove_file(&extra_cache);
}

#[tokio::test]
async fn a_long_annotation_is_whole_scrolls_and_can_be_cut() {
    let artist = "NotesTest Long Band";
    let long = format!("Start of the note. {} End of the note.", "More context about the line. ".repeat(40));
    let referents = serde_json::json!({"response": {"referents": [
        {"fragment": "We build castles in the sand", "annotations": [{"body": {"plain": long}}]}
    ]}});
    let search = serde_json::json!({"response": {"hits": [{"result": {"id": 77, "title": "Roygbiv", "primary_artist": {"name": artist}}}]}});
    let (site, _log) = scripted_site(vec![
        ("/search", vec![R::Http(200, GENIUS_JSON, search.to_string())]),
        ("/referents", vec![R::Http(200, GENIUS_JSON, referents.to_string())]),
    ]);
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
    annotations::reset_breaker();
    let (mut app, mut rx) = notes_app(artist, &site);
    app.ensure_annotations();
    pump(&mut app, &mut rx, |a| matches!(a.ann_state, app::AnnState::Ready)).await;
    assert!(app.anns[0].text.ends_with("End of the note."), "stored whole");
    app.set_position_for_test(11.0);
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("Start of the") && s.contains("more") && !s.contains("End of the note"), "{s}");
    // e scrolls down to the end, E back up
    use crossterm::event::KeyCode as K;
    for _ in 0..200 {
        app.handle(key(K::Char('e')));
    }
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("End of the") && !s.contains("Start of the"), "{s}");
    for _ in 0..200 {
        app.handle(key(K::Char('E')));
    }
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains("Start of the"), "{s}");
    // a configured limit cuts it with …
    app.args.annotation_max_chars = 60;
    let s = render(&mut app, 120, 30).join("\n");
    assert!(s.contains('…') && !s.contains("more"), "{s}");
    let _ = std::fs::remove_file(annotations::cache_path(artist, "Roygbiv"));
}

#[tokio::test]
async fn the_annotation_settings_rows_cycle_and_persist() {
    use crossterm::event::KeyCode as K;
    let (mut app, cfg) = settings_app("notes_settings2");
    app.show_settings = true;
    let row = |s| app::Setting::ALL.iter().position(|x| *x == s).unwrap();
    app.settings_sel = row(app::Setting::AnnotationsMode);
    for want in [cli::AnnotationMode::Ai, cli::AnnotationMode::GeniusAi, cli::AnnotationMode::Genius] {
        app.handle(key(K::Enter));
        assert_eq!(app.args.annotations_mode, want);
    }
    app.handle(key(K::Left));
    assert_eq!(app.args.annotations_mode, cli::AnnotationMode::GeniusAi);
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("annotations_mode = \"genius+ai\""));
    app.settings_sel = row(app::Setting::AnnotationMaxChars);
    app.handle(key(K::Right));
    assert_eq!(app.args.annotation_max_chars, 200);
    app.settings_sel = row(app::Setting::AnnotationScroll);
    app.handle(key(K::Enter));
    assert_eq!(app.args.annotation_scroll, cli::AnnotationScroll::Auto);
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("annotation_max_chars = 200") && saved.contains("annotation_scroll = \"auto\""), "{saved}");
    let (a, w) = cfg_run("annotations_mode = \"genius+ai\"\nannotation_max_chars = 500\nannotation_scroll = \"auto\"\n", &[]);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!((a.annotations_mode, a.annotation_max_chars, a.annotation_scroll), (cli::AnnotationMode::GeniusAi, 500, cli::AnnotationScroll::Auto));
    let a = cli::Args::parse_from(["music_display", "--annotations-mode", "genius+ai"]);
    assert_eq!(a.annotations_mode, cli::AnnotationMode::GeniusAi);
    for n in cli::AnnotationMode::NAMES {
        assert_eq!(cli::AnnotationMode::parse(n).unwrap().name(), *n);
    }
}

// ── fixes: cava placement, cover above the track info, annotations next to the diagrams ──

#[tokio::test]
async fn the_cava_placement_toggle_is_saved_and_survives_a_restart() {
    use crossterm::event::KeyCode as K;
    use layout::PanelId;
    let (mut app, cfg) = settings_app("cavadock");
    assert_eq!(app.placement, app::CavaPlacement::UnderLyrics, "below the lyrics by default");
    app.show_settings = true;
    app.settings_sel = app::Setting::ALL.iter().position(|s| *s == app::Setting::ThirdPanel).unwrap();
    // "Third panel" on: a column of its own, written to [layout] (the toggle used to write only `thirdpanel`)
    app.handle(key(K::Enter));
    assert_eq!(app.placement, app::CavaPlacement::ThirdPanel);
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("cava_under_lyrics = false") && saved.contains("thirdpanel = true"), "{saved}");
    // ... and back to the bottom
    app.handle(key(K::Enter));
    assert_eq!(app.placement, app::CavaPlacement::UnderLyrics);
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("cava_under_lyrics = true") && saved.contains("thirdpanel = false"), "{saved}");
    // a restart with cava on: the saved placement is kept (it used to flip to a side column)
    let mut args = cli::Args::parse_from(["x", "--config", cfg.to_str().unwrap()]);
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x", "--config", cfg.to_str().unwrap()]);
    config::apply(&mut args, &m, &cfg, true, &mut |_| {});
    let mut l = config::load_layout(&args, &cfg, &mut |_| {});
    l.panels.set(PanelId::Cava, true);
    let mut app = test_app();
    app.set_layout(l);
    assert!(app.show_cava);
    assert_eq!(app.placement, app::CavaPlacement::UnderLyrics);
    // `--thirdpanel` still asks for the column
    let args = cli::Args::parse_from(["x", "--thirdpanel"]);
    let l = config::load_layout(&args, &cfg, &mut |_| {});
    assert!(!l.cava_under_lyrics && l.panels.shown(PanelId::Cava));
}

#[tokio::test]
async fn the_customize_page_switches_cava_between_bottom_and_side() {
    use crossterm::event::KeyCode as K;
    use layout::PanelId;
    let (mut app, cfg) = settings_app("cavadock_page");
    app.snap = Some(snap("A", "B", "C"));
    app.show_settings = true;
    app.open_customize();
    while app.cust_items()[app.cust_sel] != app::CustItem::Panel(PanelId::Cava) {
        app.handle(key(K::Down));
    }
    let s = render(&mut app, 100, 40);
    assert!(row_of(&s, "bottom").is_some(), "{s:#?}");
    app.handle(key(K::Char('p')));
    assert_eq!(app.placement, app::CavaPlacement::ThirdPanel);
    assert!(app.show_cava, "a column of its own is shown");
    assert!(row_of(&render(&mut app, 100, 40), "side").is_some());
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("cava_under_lyrics = false"));
    app.handle(key(K::Char('p')));
    assert_eq!(app.placement, app::CavaPlacement::UnderLyrics);
}

#[tokio::test]
async fn the_cover_is_above_the_track_information() {
    use layout::PanelId;
    let mut app = playing_app();
    let s = render(&mut app, 100, 30);
    let track = row_of(&s, "Track ").unwrap();
    assert!(track > 8, "the cover takes the top rows, the track information is under it (row {track})");
    // the other way round is still a choice
    app.layout.panels.shift(PanelId::Metadata, -1);
    assert_eq!(app.layout.now_playing()[0], PanelId::Metadata);
    let s = render(&mut app, 100, 30);
    assert!(row_of(&s, "Track ").unwrap() < 5, "{s:#?}");
}

#[tokio::test]
async fn annotations_and_diagrams_share_one_column() {
    use layout::PanelId;
    let mut app = playing_app();
    app.set_layout(layout::Layout::default());
    // the notes alone: a column of their own, narrower than before
    app.toggle_annotations();
    let s = render(&mut app, 120, 40);
    let x = s.iter().find_map(|r| col_of(r, "annotations")).unwrap();
    assert!(!app.hits.stack_diagrams);
    assert!(x >= 94, "the notes column starts at {x} of 120");
    // the diagrams alone dock inside the lyrics column, as before
    app.toggle_annotations();
    app.show_diagrams = true;
    let s = render(&mut app, 120, 40);
    assert!(!app.hits.stack_diagrams && s.iter().any(|r| r.contains("chords")));
    // both: one column, the diagrams on top and the notes under them, half each
    app.toggle_annotations();
    let s = render(&mut app, 120, 40);
    assert!(app.hits.stack_diagrams);
    let chords_row = s.iter().position(|r| r.contains("╭ chords")).unwrap();
    let notes_row = s.iter().position(|r| r.contains("╭ annotations")).unwrap();
    assert!(chords_row < notes_row, "diagrams above notes");
    let (cx, nx) = (col_of(&s[chords_row], "╭ chords").unwrap(), col_of(&s[notes_row], "╭ annotations").unwrap());
    assert_eq!(cx, nx, "the same column");
    assert_eq!(s.iter().filter(|r| r.contains("╭ chords")).count(), 1, "the lyrics column no longer holds its own copy");
    // roughly half the height each (the column is 40 - top row - footer rows tall)
    assert!(notes_row.abs_diff(chords_row + (40 - 2) / 2) <= 3, "{chords_row} {notes_row}");
    let _ = PanelId::Chords;
}

#[test]
fn metadata_links_use_uris_only_for_the_same_song() {
    use browser::{library_link, LibLink};
    use layout::MetaField as F;
    let mut s = snap("Coldplay, BTS", "My Universe", "Music of the Spheres");
    s.track_id = "spotify:track:abc".into();
    let remote = |id: &str| spotify::Remote {
        snap: {
            let mut r = s.clone();
            r.track_id = id.into();
            r
        },
        local: true,
        device: String::new(),
        album_uri: "spotify:album:al".into(),
        artist_uris: vec!["spotify:artist:co".into(), "spotify:artist:bts".into()],
    };
    // same song: the first artist and the album open by URI
    let r = remote("spotify:track:abc");
    let Some(LibLink::Open(a)) = library_link(F::Artist, &s, "", Some(&r)) else { panic!("artist") };
    assert_eq!((a.kind.as_str(), a.uri.as_str(), a.name.as_str()), ("artist", "spotify:artist:co", "Coldplay"));
    let Some(LibLink::Open(al)) = library_link(F::Album, &s, "", Some(&r)) else { panic!("album") };
    assert_eq!(al.uri, "spotify:album:al");
    // another song in the API state (stale): never its URIs
    let old = remote("spotify:track:other");
    assert_eq!(library_link(F::Artist, &s, "", Some(&old)), Some(LibLink::Search("Coldplay".into())));
    // MPRIS only: searches
    assert_eq!(library_link(F::Title, &s, "", None), Some(LibLink::Search("My Universe Coldplay".into())));
    assert_eq!(library_link(F::Album, &s, "", None), Some(LibLink::Search("Music of the Spheres Coldplay".into())));
    assert_eq!(library_link(F::Genre, &s, "Pop", None), Some(LibLink::Search("Pop".into())));
    assert_eq!(library_link(F::Genre, &s, "", None), None);
    assert_eq!(library_link(F::Key, &s, "Pop", None), None);
}

#[tokio::test]
async fn auto_zero_capo_in_the_terminal_ui() {
    let mut app = test_app();
    app.apply(AppEvent::Player(Some(snap("Band", "Song", "Album"))));
    let key = app.track_key;
    let sheet = chords::parse_chordpro("{capo: 2}\n[Am]hello\n");
    app.apply(AppEvent::Chords { key, sheet: Ok(Some(sheet)), capo: None });
    assert_eq!(app.effective_capo(), 2);
    assert_eq!(app.chord_shift(), 0);
    app.args.auto_zero_capo = true;
    assert_eq!((app.effective_capo(), app.chord_shift()), (0, 2));
    // a capo remembered for the track wins
    let sheet = chords::parse_chordpro("{capo: 2}\n[Am]hello\n");
    app.apply(AppEvent::Chords { key, sheet: Ok(Some(sheet)), capo: Some(1) });
    assert_eq!((app.effective_capo(), app.chord_shift()), (1, 1));
}

#[tokio::test]
async fn translation_and_member_tag_go_where_the_settings_say() {
    use cli::{MemberPosition as MP, TranslationPosition as TP};
    let mut app = test_app();
    app.apply(AppEvent::Player(Some(snap("Band", "Song", "Album"))));
    let key = app.track_key;
    app.apply(AppEvent::Lyrics { key, lyrics: lyrics::Lyrics::Synced(vec![(0.0, "hello there".into()), (30.0, "second line".into())]) });
    let lang = app.args.translate_lang.clone();
    app.apply(AppEvent::Translated { key, lang, lines: Ok((vec!["hej da".into(), "andra raden".into()], "test".into())) });
    app.translate_on = true;
    app.translate_both = true;
    app.members = vec![vec!["Jimin".into()], vec![]];
    // the rows (top to bottom) of the first line: original, translation, tag
    let order = |app: &mut app::App| -> Vec<&'static str> {
        let screen = render(app, 60, 20);
        let at = |needle: &str| screen.iter().position(|r| r.contains(needle));
        let mut found: Vec<(usize, &'static str)> =
            [("hello there", "original"), ("hej da", "translation"), ("[Jimin]", "tag")].into_iter().filter_map(|(n, w)| at(n).map(|y| (y, w))).collect();
        found.sort();
        found.into_iter().map(|(_, w)| w).collect()
    };
    app.args.member_position = MP::Below;
    assert_eq!(order(&mut app), ["original", "translation", "tag"]);
    app.args.translation_position = TP::Above;
    assert_eq!(order(&mut app), ["translation", "original", "tag"]);
    app.args.member_position = MP::Above;
    assert_eq!(order(&mut app), ["tag", "translation", "original"]);
    app.args.translation_position = TP::Below;
    assert_eq!(order(&mut app), ["tag", "original", "translation"]);
    // inline: the tag shares the row of the original
    app.args.member_position = MP::Inline;
    let screen = render(&mut app, 60, 20);
    assert!(screen.iter().any(|r| r.contains("[Jimin") && r.contains("hello there")), "{screen:#?}");
    // and with the translation above the line the tag still sits on the original's row
    app.args.translation_position = TP::Above;
    let screen = render(&mut app, 60, 20);
    assert!(screen.iter().any(|r| r.contains("[Jimin") && r.contains("hello there")), "{screen:#?}");
}
