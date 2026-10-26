//! The `[keys]` section of config.toml: overrides, warnings, quit / help, help overlay,
//! `--print-config` and the settings-save round trip.

use super::*;
use crate::app::BINDS;

fn keymap_from(text: &str) -> (keymap::Keymap, Vec<String>) {
    let table: toml::Table = text.parse().unwrap();
    keymap::Keymap::from_table(&table["keys"], text, std::path::Path::new("c.toml"))
}

fn key(c: char) -> crossterm::event::KeyCode {
    crossterm::event::KeyCode::Char(c)
}

#[test]
fn keys_override_replaces_the_default_key() {
    use app::Action as A;
    let (km, warns) = keymap_from("[keys]\nnext = \"j\"\n");
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!(km.action_for(key('j')), Some(A::Next));
    assert_eq!(km.action_for(key('n')), None, "the old key no longer fires");
    assert_eq!(km.action_for(key('p')), Some(A::Prev), "everything else keeps its keys");
    assert_eq!(km.label(A::Next), "j");
    assert_eq!(km.label(A::Prev), "p");
}

#[test]
fn keys_accept_a_list_names_and_case() {
    use crossterm::event::KeyCode as K;
    let (km, warns) = keymap_from("[keys]\nmute = [\"w\", \"F5\", \"home\"]\nplay_pause = \"enter\"\ncapo_up = \"W\"\n");
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!(km.codes(app::Action::Mute), [key('w'), K::F(5), K::Home]);
    assert_eq!(km.action_for(K::Enter), Some(app::Action::PlayPause));
    // case matters: "W" is a capital, "w" is another key
    assert_eq!(km.action_for(key('W')), Some(app::Action::CapoUp));
    assert_eq!(km.action_for(key('w')), Some(app::Action::Mute));
}

#[test]
fn keys_unknown_action_and_invalid_keys_warn_with_the_line_and_keep_the_default() {
    use app::Action as A;
    let text = "translate = true\n[keys]\ntranslate = \"J\"\nbogus = \"x\"\nnext = \"ctrl+n\"\nprev = \"banana\"\nmute = 5\nlike = []\n";
    let (km, warns) = keymap_from(text);
    // a top-level key with the same name does not confuse the line lookup
    assert_eq!(km.action_for(key('J')), Some(A::Translate));
    assert_eq!(km.action_for(key('n')), Some(A::Next), "a modifier is refused");
    assert_eq!(km.action_for(key('p')), Some(A::Prev));
    assert_eq!(km.action_for(key('m')), Some(A::Mute));
    assert_eq!(km.action_for(key('f')), Some(A::Like));
    assert_eq!(warns.len(), 5, "{warns:?}");
    assert!(warns[0].starts_with("c.toml:4:") && warns[0].contains("bogus") && warns[0].contains("unknown action"), "{warns:?}");
    assert!(warns[1].starts_with("c.toml:5:") && warns[1].contains("modifiers"), "{warns:?}");
    assert!(warns[2].starts_with("c.toml:6:") && warns[2].contains("not a key"), "{warns:?}");
    assert!(warns[3].starts_with("c.toml:7:") && warns[3].contains("list of keys"), "{warns:?}");
    assert!(warns[4].starts_with("c.toml:8:") && warns[4].contains("no key"), "{warns:?}");
    assert!(warns.iter().all(|w| w.contains("[keys]")));
}

#[test]
fn keys_conflicts_keep_the_defaults_and_a_swap_works() {
    use app::Action as A;
    // p is prev's key
    let (km, warns) = keymap_from("[keys]\nnext = \"p\"\n");
    assert_eq!((km.action_for(key('n')), km.action_for(key('p'))), (Some(A::Next), Some(A::Prev)));
    assert!(warns.len() == 1 && warns[0].starts_with("c.toml:2:") && warns[0].contains("already bound to prev"), "{warns:?}");
    // the same key twice in one list
    let (km, warns) = keymap_from("[keys]\nmute = [\"w\", \"w\"]\n");
    assert_eq!(km.action_for(key('m')), Some(A::Mute));
    assert!(warns.len() == 1 && warns[0].contains("same key twice"), "{warns:?}");
    // two overrides on one key: the earlier line wins
    let (km, warns) = keymap_from("[keys]\nmute = \"w\"\nlike = \"w\"\n");
    assert_eq!((km.action_for(key('w')), km.action_for(key('f'))), (Some(A::Mute), Some(A::Like)));
    assert!(warns.len() == 1 && warns[0].starts_with("c.toml:3:") && warns[0].contains("like"), "{warns:?}");
    // swapping two defaults is fine: the final table has no clash
    let (km, warns) = keymap_from("[keys]\nnext = \"p\"\nprev = \"n\"\n");
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!((km.action_for(key('p')), km.action_for(key('n'))), (Some(A::Next), Some(A::Prev)));
}

#[test]
fn quit_and_help_never_lose_their_keys() {
    use app::Action as A;
    use crossterm::event::KeyCode as K;
    // an override of quit / help only adds keys
    let (km, warns) = keymap_from("[keys]\nquit = \"Q\"\nhelp = \"h\"\n");
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!(km.codes(A::Quit), [key('Q'), key('q'), K::Esc]);
    assert_eq!(km.codes(A::Help), [key('h'), key('?')]);
    // nobody else can take them
    let (km, warns) = keymap_from("[keys]\nmute = \"q\"\nlike = \"esc\"\nnext = \"?\"\n");
    assert_eq!(warns.len(), 3, "{warns:?}");
    assert_eq!(km.action_for(key('q')), Some(A::Quit));
    assert_eq!(km.action_for(K::Esc), Some(A::Quit));
    assert_eq!(km.action_for(key('?')), Some(A::Help));
    assert_eq!(km.action_for(key('m')), Some(A::Mute));
    // a key of another action can't be added to quit either (x is big_scale's)
    let (km, warns) = keymap_from("[keys]\nquit = \"x\"\n");
    assert!(warns.len() == 1 && warns[0].contains("already bound to big_scale"), "{warns:?}");
    assert_eq!(km.action_for(key('x')), Some(A::BigScale));
    assert_eq!(km.action_for(key('q')), Some(A::Quit));
}

#[test]
fn key_names_round_trip_and_every_action_has_a_unique_name() {
    let mut seen = std::collections::HashSet::new();
    for b in BINDS {
        assert!(seen.insert(b.action.name()), "{} used twice", b.action.name());
        assert_eq!(app::Action::from_name(b.action.name()), Some(b.action));
        for c in b.codes {
            assert_eq!(keymap::parse_key(&keymap::key_name(*c)), Ok(*c), "{c:?}");
        }
    }
    assert!(keymap::parse_key("shift+t").is_err() && keymap::parse_key("Alt-x").is_err());
    assert!(keymap::parse_key("f13").is_err() && keymap::parse_key("").is_err());
    assert_eq!(keymap::parse_key("+"), Ok(key('+')));
    assert_eq!(keymap::parse_key("-"), Ok(key('-')));
}

#[test]
fn a_keys_override_drives_the_real_input_handler() {
    use crossterm::event::{Event, KeyCode as K, KeyEvent, KeyModifiers};
    let mut app = test_app();
    let (km, warns) = keymap_from("[keys]\nminimal = \"w\"\n");
    assert!(warns.is_empty(), "{warns:?}");
    app.keys = km;
    let press = |c: char| Event::Key(KeyEvent::new(K::Char(c), KeyModifiers::NONE));
    assert!(!app.minimal);
    app.handle(press('M'));
    assert!(!app.minimal, "the old key does nothing");
    app.handle(press('w'));
    assert!(app.minimal);
    // Ctrl+C still quits, whatever is configured
    assert!(app.handle(Event::Key(KeyEvent::new(K::Char('c'), KeyModifiers::CONTROL))));
}

#[tokio::test]
async fn help_overlay_and_footer_show_the_effective_bindings() {
    use app::Action as A;
    let th = theme_from(&["--terminal-colors"], None);
    let text_of = |km: &keymap::Keymap| -> String {
        ui::help_blocks_with(&th, km)
            .iter()
            .flatten()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>() + "\n")
            .collect()
    };
    let (km, warns) = keymap_from("[keys]\nnext = \"j\"\nmute = [\"w\", \"F5\"]\n");
    assert!(warns.is_empty(), "{warns:?}");
    let text = text_of(&km);
    let row = |desc: &str| text.lines().find(|l| l.contains(desc)).unwrap_or("").to_string();
    assert!(row("next track").starts_with("j "), "{}", row("next track"));
    assert!(row("mute / unmute").starts_with("w / F5 "), "{}", row("mute / unmute"));
    assert!(row("previous track").starts_with("p "), "untouched rows keep their text");
    // the defaults produce exactly the table's own text
    let dflt = keymap::Keymap::default();
    assert_eq!(dflt.label(A::Settings), "s / S");
    assert_eq!(dflt.first(A::Next), "n");
    assert_eq!(km.first(A::Next), "j");
    // the footer hints follow
    let mut app = test_app();
    app.keys = km;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 30)).unwrap();
    app.apply(AppEvent::Player(Some(snap("Band", "Song", "LP"))));
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let last: String = (0..140).map(|x| term.backend().buffer()[(x, 29)].symbol().to_string()).collect();
    assert!(last.contains("j/p"), "{last}");
}

#[test]
fn print_config_shows_the_effective_bindings_and_parses_back() {
    use clap::Parser;
    let (km, _) = keymap_from("[keys]\nnext = \"j\"\nmute = [\"w\", \"F5\"]\nquit = \"Q\"\n");
    let args = cli::Args::parse_from(["music_display"]);
    let text = config::render(&args, std::path::Path::new("c.toml"), false) + &km.render();
    assert!(text.contains("[keys]\n") && text.contains("next = [\"j\"]") && text.contains("mute = [\"w\", \"f5\"]"), "{text}");
    assert!(text.contains("quit = [\"Q\", \"q\", \"esc\"]"), "{text}");
    // the whole output is valid TOML, and the [keys] block gives the same keymap back
    let table: toml::Table = text.parse().unwrap();
    let (again, warns) = keymap::Keymap::from_table(&table["keys"], &text, std::path::Path::new("c.toml"));
    assert!(warns.is_empty(), "{warns:?}");
    for b in BINDS {
        assert_eq!(again.codes(b.action), km.codes(b.action), "{}", b.action.name());
    }
    // so do the defaults
    let dtext = keymap::Keymap::default().render();
    let t: toml::Table = dtext.parse().unwrap();
    let (d, warns) = keymap::Keymap::from_table(&t["keys"], &dtext, std::path::Path::new("c.toml"));
    assert!(warns.is_empty(), "{warns:?}");
    for b in BINDS {
        assert_eq!(d.codes(b.action), b.codes, "{}", b.action.name());
    }
}

#[test]
fn settings_save_keeps_the_keys_section() {
    use clap::Parser;
    let dir = tmp("keys-save");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "# my config\nvolume_step = 5\n\n[keys]\nnext = \"j\"\nmute = [\"w\", \"F5\"]\nquit = \"Q\"\n",
    )
    .unwrap();
    // the settings loader does not call [keys] an unknown key
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x"]);
    let mut args = cli::Args::parse_from(["x"]);
    let mut warns = Vec::new();
    config::apply(&mut args, &m, &path, true, &mut |w| warns.push(w));
    assert!(warns.is_empty(), "{warns:?}");
    assert_eq!(args.volume_step, 5);
    // a settings change re-writes the file
    config::save(&path, &[("volume_step", toml::Value::Integer(10)), ("player", toml::Value::String("vlc".into()))]).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    let t: toml::Table = saved.parse().unwrap();
    assert_eq!(t["volume_step"].as_integer(), Some(10));
    assert_eq!(t["keys"]["next"].as_str(), Some("j"));
    assert_eq!(t["keys"]["mute"].as_array().map(|a| a.len()), Some(2));
    // and the keymap read back from it is the same one
    let mut warns = Vec::new();
    let km = keymap::load(&path, &mut |w| warns.push(w));
    assert!(warns.is_empty(), "{warns:?}");
    let want = keymap_from("[keys]\nnext = \"j\"\nmute = [\"w\", \"F5\"]\nquit = \"Q\"\n").0;
    for b in BINDS {
        assert_eq!(km.codes(b.action), want.codes(b.action), "{}", b.action.name());
    }
    // no file, no section, broken file: the defaults, silently
    let mut warns = Vec::new();
    for body in [None, Some("player = \"vlc\"\n"), Some("not = [valid")] {
        let p = dir.join("other.toml");
        let _ = std::fs::remove_file(&p);
        if let Some(b) = body {
            std::fs::write(&p, b).unwrap();
        }
        let km = keymap::load(&p, &mut |w| warns.push(w));
        assert_eq!(km.action_for(key('n')), Some(app::Action::Next));
    }
    assert!(warns.is_empty());
}
