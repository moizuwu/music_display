//! `--setup`: a fake spotifyd, a fake HTTP site for the device list and a scratch home.

use super::*;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

const CANARY: &str = "CLIENTIDCANARY123";

/// A scripted outside world: nothing real is started, asked or fetched.
#[derive(Default)]
struct Fake {
    spotifyd: Option<PathBuf>,
    answers: RefCell<VecDeque<String>>,
    asked: RefCell<Vec<String>>,
    ran: RefCell<Vec<String>>,
    auth_fails: bool,
    running: bool,
    login_err: Option<String>,
    logins: RefCell<Vec<String>>,
    /// Base of the fake site that answers `GET /v1/me/player/devices`; `None` = the call fails.
    devices_url: Option<String>,
    devices_calls: Cell<u32>,
    waits: Cell<u32>,
}

impl setup::Steps for Fake {
    fn which(&self, name: &str) -> Option<PathBuf> {
        assert_eq!(name, "spotifyd");
        self.spotifyd.clone()
    }
    fn run_interactive(&self, prog: &Path, args: &[&str]) -> std::io::Result<bool> {
        self.ran.borrow_mut().push(format!("{} {}", prog.display(), args.join(" ")));
        Ok(!self.auth_fails)
    }
    fn ask(&self, question: &str, default: &str) -> String {
        self.asked.borrow_mut().push(question.to_string());
        // like the real prompt: an empty answer is the default
        match self.answers.borrow_mut().pop_front() {
            Some(a) if !a.is_empty() => a,
            _ => default.to_string(),
        }
    }
    fn spotifyd_running(&self) -> bool {
        self.running
    }
    async fn login(&self, client_id: &str) -> anyhow::Result<()> {
        self.logins.borrow_mut().push(client_id.to_string());
        match &self.login_err {
            Some(e) => Err(anyhow::anyhow!(e.clone())),
            None => Ok(()),
        }
    }
    async fn devices(&self, _client_id: &str) -> Result<Vec<(String, bool)>, String> {
        self.devices_calls.set(self.devices_calls.get() + 1);
        let url = self.devices_url.clone().ok_or("no connection")?;
        let r = reqwest::Client::new()
            .get(format!("{url}/v1/me/player/devices"))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("HTTP {}", r.status().as_u16()));
        }
        Ok(setup::parse_devices(&r.json().await.map_err(|e| e.to_string())?))
    }
    async fn wait(&self, _secs: u64) {
        self.waits.set(self.waits.get() + 1);
    }
}

fn fake(answers: &[&str]) -> Fake {
    Fake {
        spotifyd: Some(PathBuf::from("/fake/bin/spotifyd")),
        answers: RefCell::new(answers.iter().map(|a| a.to_string()).collect()),
        running: true,
        ..Default::default()
    }
}

/// An empty scratch home; the returned env points into it.
fn scratch(name: &str) -> (setup::Env, PathBuf) {
    let d = tmp(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let env = setup::Env {
        config_path: d.join("cfg/music_display/config.toml"),
        spotifyd_conf: d.join("cfg/spotifyd/spotifyd.conf"),
        os_release: "ID=arch\n".into(),
        now: 1_700_000_000,
    };
    (env, d)
}

fn inputs(client_id: Option<&str>) -> setup::Inputs {
    setup::Inputs { client_id: client_id.map(String::from), device: "Den".into() }
}

fn devices_site(names: &[&str]) -> String {
    let list: Vec<String> = names
        .iter()
        .map(|n| format!(r#"{{"id":"x{n}","name":"{n}","is_active":false,"type":"Computer"}}"#))
        .collect();
    fake_site(vec![("/v1/me/player/devices", 200, format!(r#"{{"devices":[{}]}}"#, list.join(",")))]).0
}

async fn go(env: &setup::Env, f: &Fake, i: &setup::Inputs) -> (setup::Outcome, String) {
    let mut out = Vec::new();
    let o = setup::run(env, f, i, &mut out).await.unwrap();
    (o, String::from_utf8(out).unwrap())
}

fn files_in(d: &Path) -> Vec<String> {
    fn walk(d: &Path, out: &mut Vec<String>, root: &Path) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out, root);
            } else {
                out.push(p.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    let mut v = Vec::new();
    walk(d, &mut v, d);
    v.sort();
    v
}

#[tokio::test]
async fn missing_spotifyd_prints_a_hint_and_changes_nothing() {
    for (os, hint) in [
        ("ID=arch\n", "pacman -S spotifyd"),
        ("ID=cachyos\nID_LIKE=\"arch\"\n", "pacman -S spotifyd"),
        ("ID=artix\n", "pacman -S spotifyd"),
        ("ID=ubuntu\nID_LIKE=debian\n", "apt install spotifyd"),
        ("ID=fedora\n", "dnf install spotifyd"),
        ("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n", "zypper install spotifyd"),
        ("ID=voidlinux\n", "https://github.com/Spotifyd/spotifyd"),
    ] {
        let (mut env, d) = scratch("setup-missing");
        env.os_release = os.into();
        let mut f = fake(&[]);
        f.spotifyd = None;
        let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
        assert_eq!(o, setup::Outcome::Missing, "{os}");
        assert!(out.contains(hint), "{os}: {out}");
        assert!(files_in(&d).is_empty(), "nothing written");
        assert!(f.asked.borrow().is_empty() && f.ran.borrow().is_empty() && f.logins.borrow().is_empty());
    }
}

#[tokio::test]
async fn setup_writes_the_files_and_confirms_readiness() {
    let (env, d) = scratch("setup-happy");
    let f = Fake { devices_url: Some(devices_site(&["Phone", "Den"])), ..fake(&[]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Done { ready: true }, "{out}");
    assert_eq!(f.ran.borrow().as_slice(), ["/fake/bin/spotifyd authenticate"]);
    assert_eq!(f.logins.borrow().as_slice(), [CANARY]);
    assert_eq!(std::fs::read_to_string(&env.spotifyd_conf).unwrap(), "[global]\ndevice_name = \"Den\"\n");
    let cfg: toml::Table = std::fs::read_to_string(&env.config_path).unwrap().parse().unwrap();
    assert_eq!(cfg["spotify"].as_bool(), Some(true));
    assert_eq!(cfg["use_spotifyd"].as_bool(), Some(true));
    assert_eq!(cfg["spotify_device"].as_str(), Some("Den"));
    assert_eq!(cfg["spotify_client_id"].as_str(), Some(CANARY));
    assert!(out.contains("OK   spotifyd is running") && out.contains("\"Den\" is listed"), "{out}");
    assert!(files_in(&d).iter().all(|n| !n.contains(".bak-")), "no backups when nothing existed");
    // the Client ID is never printed, and spotifyd.conf holds no secrets
    assert!(!out.contains(CANARY), "{out}");
    assert!(!std::fs::read_to_string(&env.spotifyd_conf).unwrap().contains(CANARY));
    // the device name asked for was the suggestion from the config
    assert!(f.asked.borrow()[0].contains("[Den]"));
}

#[tokio::test]
async fn asked_for_the_client_id_and_device_name_when_missing() {
    let (env, _d) = scratch("setup-ask");
    let f = Fake { devices_url: Some(devices_site(&["Kitchen"])), ..fake(&["Kitchen", CANARY]) };
    let i = setup::Inputs { client_id: None, device: String::new() };
    let (o, out) = go(&env, &f, &i).await;
    assert_eq!(o, setup::Outcome::Done { ready: true }, "{out}");
    assert_eq!(f.logins.borrow().as_slice(), [CANARY]);
    assert!(f.asked.borrow()[0].contains(&format!("[{}]", setup::DEFAULT_DEVICE)), "suggests music_display");
    assert!(std::fs::read_to_string(&env.spotifyd_conf).unwrap().contains("device_name = \"Kitchen\""));
    assert!(!out.contains(CANARY));
}

#[tokio::test]
async fn existing_files_are_kept_when_the_answer_is_no() {
    let (env, d) = scratch("setup-no");
    std::fs::create_dir_all(env.spotifyd_conf.parent().unwrap()).unwrap();
    std::fs::create_dir_all(env.config_path.parent().unwrap()).unwrap();
    std::fs::write(&env.spotifyd_conf, "[global]\nbackend = \"alsa\"\n").unwrap();
    std::fs::write(&env.config_path, "volume_step = 9\n").unwrap();
    // device name (default), then "no" for spotifyd.conf and an empty answer (= no) for config.toml
    let f = Fake { devices_url: Some(devices_site(&["Den"])), ..fake(&["", "n", ""]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Done { ready: true }, "{out}");
    assert_eq!(std::fs::read_to_string(&env.spotifyd_conf).unwrap(), "[global]\nbackend = \"alsa\"\n");
    assert_eq!(std::fs::read_to_string(&env.config_path).unwrap(), "volume_step = 9\n");
    assert!(files_in(&d).iter().all(|n| !n.contains(".bak-")), "{:?}", files_in(&d));
    // what to add by hand is printed, without the Client ID
    assert!(out.contains("device_name = \"Den\"") && out.contains("use_spotifyd = true"), "{out}");
    assert!(out.contains("<your Client ID>") && !out.contains(CANARY), "{out}");
}

#[tokio::test]
async fn existing_files_are_backed_up_before_they_are_replaced() {
    let (env, d) = scratch("setup-yes");
    std::fs::create_dir_all(env.spotifyd_conf.parent().unwrap()).unwrap();
    std::fs::create_dir_all(env.config_path.parent().unwrap()).unwrap();
    std::fs::write(&env.spotifyd_conf, "[global]\nbackend = \"alsa\"\n").unwrap();
    std::fs::write(&env.config_path, "volume_step = 9\n").unwrap();
    // a backup of the same second already exists: it must survive
    let taken = env.spotifyd_conf.with_file_name("spotifyd.conf.bak-1700000000");
    std::fs::write(&taken, "older backup").unwrap();
    let f = Fake { devices_url: Some(devices_site(&["Den"])), ..fake(&["", "y", "yes"]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Done { ready: true }, "{out}");
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "older backup");
    let second = env.spotifyd_conf.with_file_name("spotifyd.conf.bak-1700000000-1");
    assert_eq!(std::fs::read_to_string(second).unwrap(), "[global]\nbackend = \"alsa\"\n");
    let cfg_bak = env.config_path.with_file_name("config.toml.bak-1700000000");
    assert_eq!(std::fs::read_to_string(cfg_bak).unwrap(), "volume_step = 9\n");
    assert_eq!(std::fs::read_to_string(&env.spotifyd_conf).unwrap(), "[global]\ndevice_name = \"Den\"\n");
    // the config keeps what was in it and gains the new entries
    let cfg: toml::Table = std::fs::read_to_string(&env.config_path).unwrap().parse().unwrap();
    assert_eq!(cfg["volume_step"].as_integer(), Some(9));
    assert_eq!(cfg["use_spotifyd"].as_bool(), Some(true));
    assert!(out.contains("backed up to"), "{out}");
    assert_eq!(files_in(&d).len(), 5, "{:?}", files_in(&d));
}

#[tokio::test]
async fn readiness_says_what_is_missing() {
    // spotifyd not running: no device lookup at all
    let (env, _d) = scratch("setup-ready1");
    let f = Fake { running: false, devices_url: Some(devices_site(&["Den"])), ..fake(&[]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Done { ready: false });
    assert!(out.contains("MISSING   spotifyd is not running"), "{out}");
    assert_eq!(f.devices_calls.get(), 0);

    // running, but Spotify does not list the device: asked a few times, with waits between
    let (env, _d) = scratch("setup-ready2");
    let f = Fake { devices_url: Some(devices_site(&["Phone"])), ..fake(&[]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Done { ready: false });
    assert!(out.contains("\"Den\" is not in your Spotify devices"), "{out}");
    assert_eq!((f.devices_calls.get(), f.waits.get()), (5, 4), "no tight loop");

    // the API call itself fails
    let (env, _d) = scratch("setup-ready3");
    let (base, _) = fake_site(vec![("/v1/me/player/devices", 403, r#"{"error":{"message":"nope"}}"#.into())]);
    let f = Fake { devices_url: Some(base), ..fake(&[]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Done { ready: false });
    assert!(out.contains("could not read your devices: HTTP 403"), "{out}");
}

#[tokio::test]
async fn a_failed_step_stops_before_anything_is_written() {
    // spotifyd authenticate fails: the login is never started
    let (env, d) = scratch("setup-fail1");
    let f = Fake { auth_fails: true, ..fake(&[]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Stopped, "{out}");
    assert!(f.logins.borrow().is_empty() && files_in(&d).is_empty());

    // the browser login fails: still nothing written
    let (env, d) = scratch("setup-fail2");
    let f = Fake { login_err: Some("login timed out".into()), ..fake(&[]) };
    let (o, out) = go(&env, &f, &inputs(Some(CANARY))).await;
    assert_eq!(o, setup::Outcome::Stopped);
    assert!(out.contains("login timed out") && !out.contains(CANARY), "{out}");
    assert!(files_in(&d).is_empty());
}

#[tokio::test]
async fn unusable_answers_are_refused_before_anything_runs() {
    for (answers, id) in [(&["bad\"name"][..], CANARY), (&["Den", "not an id!"][..], "")] {
        let (env, d) = scratch("setup-bad");
        let f = fake(answers);
        let i = setup::Inputs { client_id: (!id.is_empty()).then(|| id.to_string()), device: "x".into() };
        let (o, out) = go(&env, &f, &i).await;
        assert_eq!(o, setup::Outcome::Stopped, "{out}");
        assert!(f.ran.borrow().is_empty() && f.logins.borrow().is_empty() && files_in(&d).is_empty());
    }
}

#[test]
fn the_pieces() {
    assert_eq!(setup::spotifyd_conf("My PC"), "[global]\ndevice_name = \"My PC\"\n");
    let v: serde_json::Value = serde_json::from_str(r#"{"devices":[{"name":"A","is_active":true},{"name":"B"}]}"#).unwrap();
    assert_eq!(setup::parse_devices(&v), vec![("A".to_string(), true), ("B".to_string(), false)]);
    assert!(setup::parse_devices(&serde_json::Value::Null).is_empty());
    let (_, d) = scratch("setup-backup");
    let f = d.join("a.conf");
    std::fs::write(&f, "x").unwrap();
    assert_eq!(setup::backup(&f, 5).unwrap(), d.join("a.conf.bak-5"));
    assert_eq!(setup::backup(&f, 5).unwrap(), d.join("a.conf.bak-5-1"));
}

#[test]
fn use_spotifyd_and_setup_are_known_to_flags_and_config() {
    use clap::Parser;
    let a = cli::Args::parse_from(["music_display", "--setup", "--use-spotifyd"]);
    assert!(a.setup && a.use_spotifyd);
    let a = cli::Args::parse_from(["music_display"]);
    assert!(!a.setup && !a.use_spotifyd, "off by default");
    // the config file sets it
    let path = tmp("setup-cfg").join("config.toml");
    std::fs::write(&path, "use_spotifyd = true\n").unwrap();
    let m = <cli::Args as clap::CommandFactory>::command().get_matches_from(["x"]);
    let mut a = <cli::Args as clap::FromArgMatches>::from_arg_matches(&m).unwrap();
    config::apply(&mut a, &m, &path, true, &mut |w| panic!("{w}"));
    assert!(a.use_spotifyd);
    assert!(config::render(&a, &path, true).contains("use_spotifyd = true"));
}
