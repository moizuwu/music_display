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
    let r = theme::readable((30, 20, 90), dark_bg);
    assert!(
        theme::luminance(r) - theme::luminance((16, 16, 16)) >= 0.4 - 1e-3,
        "{r:?}"
    );
    assert!(theme::luminance(theme::readable((10, 10, 40), None)) >= 0.35);
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
    let (base4, _) = fake_site(vec![("search.php", 200, empty)]);
    ug::reset_breaker();
    let o = rt.block_on(ug::lookup(&client, &base4, "Nobody", "Truly Unknown Song"));
    assert!(matches!(o, ug::Outcome::NotFound));
    let _ = std::fs::remove_file(util::cache_dir().join(format!("chords-ug-{:016x}.json", util::fnv1a("Nobody\0Truly Unknown Song"))));
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
            assert!(config::KEYS.iter().any(|c| c.name == *k), "{k}");
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
    let r = spotify::parse_state(&p, |n| n == "Desktop (spotifyd)").unwrap();
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
    assert!(config::KEYS.iter().all(|k| table.contains_key(k.name) || k.name == "chords_dir" || k.name == "translate_fallback_key" || k.name == "spotify_client_id"), "{text}");
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
    // every action can be triggered by some key
    for a in [
        A::Quit, A::PlayPause, A::Next, A::Prev, A::Shuffle, A::Repeat, A::SeekBack, A::SeekFwd, A::SyncEarlier,
        A::SyncLater, A::VolUp, A::VolDown, A::Mute, A::ToggleLyrics, A::ToggleBig,
        A::ToggleCava, A::ToggleChords, A::TransposeUp, A::TransposeDown, A::CapoDown,
        A::CapoUp, A::CapoReset, A::Diagrams, A::Minimal, A::Help, A::Library, A::Like, A::PlayHere,
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
    assert_eq!(genius::color("Jimin"), genius::color("jimin"));
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
