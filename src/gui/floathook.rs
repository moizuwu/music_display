//! Asking the compositor to float the extra windows. On a tiling window manager (Hyprland, sway,
//! niri) a new window is tiled next to the main one, which splits and shrinks it; a toolkit
//! cannot say "float" on Wayland (egui and winit cannot set a parent for the window), so after
//! the window has appeared the compositor is asked through its own command line tool. Which one
//! is decided by the environment variables those compositors set, anything else does nothing, and
//! every failure is silent. The window is found by this program's process id and the exact
//! title, so other windows are never touched. Not tried on a real compositor.
use serde_json::Value;
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    Hyprland,
    Sway,
    Niri,
}

/// The compositor we are running under, from its environment variables (`env` looks one up).
pub fn backend(env: &dyn Fn(&str) -> Option<String>) -> Option<Backend> {
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if set("HYPRLAND_INSTANCE_SIGNATURE") {
        Some(Backend::Hyprland)
    } else if set("SWAYSOCK") {
        Some(Backend::Sway)
    } else if set("NIRI_SOCKET") {
        Some(Backend::Niri)
    } else {
        None
    }
}

/// Hyprland's `clients -j`: the address and whether it floats, of our window.
pub fn hypr_find(json: &str, pid: u32, title: &str) -> Option<(String, bool)> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.as_array()?.iter().find_map(|c| {
        let mine = c.get("pid")?.as_u64()? == pid as u64 && c.get("title")?.as_str()? == title;
        let address = c.get("address")?.as_str().filter(|a| !a.is_empty())?;
        mine.then(|| (address.to_string(), c.get("floating").and_then(Value::as_bool).unwrap_or(false)))
    })
}

/// The `hyprctl` calls for Hyprland 0.55 and later, whose dispatchers are Lua calls: float the
/// window, give it its size and put it in the middle.
pub fn hypr_lua_commands(address: &str, size: [f32; 2]) -> Vec<Vec<String>> {
    let (w, h) = (size[0].round() as i32, size[1].round() as i32);
    let call = |code: String| vec!["dispatch".to_string(), code];
    vec![
        call(format!("hl.dsp.window.float({{ action = \"enable\", window = \"address:{address}\" }})")),
        call(format!("hl.dsp.window.resize({{ x = {w}, y = {h}, relative = false, window = \"address:{address}\" }})")),
        call(format!("hl.dsp.window.center({{ window = \"address:{address}\" }})")),
    ]
}

/// The same for the older Hyprland (`hyprctl dispatch setfloating address:…`).
pub fn hypr_commands(address: &str, size: [f32; 2]) -> Vec<Vec<String>> {
    let (w, h) = (size[0].round() as i32, size[1].round() as i32);
    vec![
        vec!["dispatch".into(), "setfloating".into(), format!("address:{address}")],
        vec!["dispatch".into(), "resizewindowpixel".into(), format!("exact {w} {h},address:{address}")],
        vec!["dispatch".into(), "focuswindow".into(), format!("address:{address}")],
        vec!["dispatch".into(), "centerwindow".into()],
    ]
}

/// The `swaymsg` command for our window (sway matches `title` as a regular expression).
pub fn sway_command(pid: u32, title: &str, size: [f32; 2]) -> String {
    let mut escaped = String::new();
    for c in title.chars() {
        if ".^$|()[]{}*+?\\\"".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!(
        "[pid={pid} title=\"^{escaped}$\"] floating enable, resize set {} {}, move position center",
        size[0].round() as i32,
        size[1].round() as i32
    )
}

/// niri's `msg --json windows`: the id and whether it floats, of our window.
pub fn niri_find(json: &str, pid: u32, title: &str) -> Option<(u64, bool)> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.as_array()?.iter().find_map(|w| {
        let mine = w.get("pid")?.as_u64()? == pid as u64 && w.get("title")?.as_str()? == title;
        mine.then(|| Some((w.get("id")?.as_u64()?, w.get("is_floating").and_then(Value::as_bool).unwrap_or(false))))?
    })
}

fn run(program: &str, args: &[String]) -> Option<String> {
    let out = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One try: has the window appeared, and is it floating now (or already was)?
fn try_once(backend: Backend, pid: u32, title: &str, size: [f32; 2]) -> bool {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match backend {
        Backend::Hyprland => {
            let Some(json) = run("hyprctl", &s(&["clients", "-j"])) else { return false };
            let Some((address, floating)) = hypr_find(&json, pid, title) else { return false };
            if !floating {
                // the new (Lua) syntax first; a Hyprland that does not know it says "error"
                let lua = hypr_lua_commands(&address, size);
                if run("hyprctl", &lua[0]).is_some_and(|out| out.trim() == "ok") {
                    for args in &lua[1..] {
                        run("hyprctl", args);
                    }
                } else {
                    for args in hypr_commands(&address, size) {
                        run("hyprctl", &args);
                    }
                }
            }
            true
        }
        Backend::Sway => {
            let cmd = sway_command(pid, title, size);
            run("swaymsg", &[cmd]).is_some_and(|out| out.contains("\"success\": true") && !out.contains("\"success\": false"))
        }
        Backend::Niri => {
            let Some(json) = run("niri", &s(&["msg", "--json", "windows"])) else { return false };
            let Some((id, floating)) = niri_find(&json, pid, title) else { return false };
            if !floating {
                run("niri", &s(&["msg", "action", "move-window-to-floating", "--id", &id.to_string()]));
            }
            true
        }
    }
}

/// Float the window called `title`, `size` big, once it has appeared (looked for every 80 ms for
/// about two seconds, off the UI thread). Does nothing outside the supported compositors.
pub fn float_once(title: &str, size: [f32; 2]) {
    let Some(backend) = backend(&|k| std::env::var(k).ok()) else { return };
    let title = title.to_string();
    std::thread::spawn(move || {
        let pid = std::process::id();
        for _ in 0..25 {
            std::thread::sleep(Duration::from_millis(80));
            if try_once(backend, pid, &title, size) {
                return;
            }
        }
    });
}

fn seen_id() -> egui::Id {
    egui::Id::unique("floated-windows")
}

/// True the first time it is asked for a window (until [`forget`]): the window has just opened.
pub fn first_frame(ctx: &egui::Context, title: &str) -> bool {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<HashSet<String>>(seen_id()).insert(title.to_string()))
}

/// The window was closed: the next one with that title is new again.
pub fn forget(ctx: &egui::Context, title: &str) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashSet<String>>(seen_id()).remove(title);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const HYPR: &str = r#"[
        {"address":"0xaaa","pid":100,"title":"music_display - Settings","class":"music_display.floating","floating":false},
        {"address":"0xbbb","pid":200,"title":"music_display - Settings","class":"music_display.floating","floating":false},
        {"address":"0xccc","pid":100,"title":"music_display","class":"music_display","floating":false},
        {"address":"0xddd","pid":100,"title":"music_display - Keys","class":"music_display.floating","floating":true}
    ]"#;

    #[test]
    fn the_compositor_is_told_by_its_environment() {
        let env = |set: &'static [&'static str]| move |k: &str| set.contains(&k).then(|| "x".to_string());
        assert_eq!(backend(&env(&["HYPRLAND_INSTANCE_SIGNATURE"])), Some(Backend::Hyprland));
        assert_eq!(backend(&env(&["SWAYSOCK"])), Some(Backend::Sway));
        assert_eq!(backend(&env(&["NIRI_SOCKET"])), Some(Backend::Niri));
        assert_eq!(backend(&env(&["HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK"])), Some(Backend::Hyprland));
        assert_eq!(backend(&env(&[])), None);
        // an empty variable is not set
        assert_eq!(backend(&|k: &str| (k == "SWAYSOCK").then(String::new)), None);
    }

    #[test]
    fn only_our_window_with_the_exact_title_is_found() {
        assert_eq!(hypr_find(HYPR, 100, "music_display - Settings"), Some(("0xaaa".into(), false)));
        assert_eq!(hypr_find(HYPR, 200, "music_display - Settings"), Some(("0xbbb".into(), false)));
        assert_eq!(hypr_find(HYPR, 100, "music_display - Keys"), Some(("0xddd".into(), true)), "already floating");
        assert_eq!(hypr_find(HYPR, 100, "music_display - Library"), None);
        assert_eq!(hypr_find(HYPR, 999, "music_display - Settings"), None);
        assert_eq!(hypr_find("not json", 100, "x"), None);
        assert_eq!(hypr_find("{}", 100, "x"), None);
    }

    #[test]
    fn the_lua_calls_of_new_hyprland_float_size_and_centre_that_window() {
        let c = hypr_lua_commands("0xaaa", [480.0, 600.4]);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0], ["dispatch", "hl.dsp.window.float({ action = \"enable\", window = \"address:0xaaa\" })"]);
        assert_eq!(c[1], ["dispatch", "hl.dsp.window.resize({ x = 480, y = 600, relative = false, window = \"address:0xaaa\" })"]);
        assert_eq!(c[2], ["dispatch", "hl.dsp.window.center({ window = \"address:0xaaa\" })"]);
    }

    #[test]
    fn the_hyprland_calls_float_size_and_centre_that_window() {
        let c = hypr_commands("0xaaa", [480.0, 600.4]);
        assert_eq!(c[0], ["dispatch", "setfloating", "address:0xaaa"]);
        assert_eq!(c[1], ["dispatch", "resizewindowpixel", "exact 480 600,address:0xaaa"]);
        assert_eq!(c[2], ["dispatch", "focuswindow", "address:0xaaa"]);
        assert_eq!(c[3], ["dispatch", "centerwindow"]);
    }

    #[test]
    fn the_sway_command_names_the_window_by_pid_and_an_escaped_title() {
        let c = sway_command(42, "music_display - Settings (1)", [480.0, 600.0]);
        assert_eq!(c, "[pid=42 title=\"^music_display - Settings \\(1\\)$\"] floating enable, resize set 480 600, move position center");
    }

    #[test]
    fn niri_windows_are_found_by_pid_and_title() {
        let json = r#"[{"id":7,"title":"music_display - Keys","app_id":"music_display.floating","pid":100,"is_floating":false},
                       {"id":8,"title":"music_display - Keys","app_id":"x","pid":300,"is_floating":false}]"#;
        assert_eq!(niri_find(json, 100, "music_display - Keys"), Some((7, false)));
        assert_eq!(niri_find(json, 400, "music_display - Keys"), None);
    }

    #[test]
    fn a_window_counts_as_new_once_until_it_is_forgotten() {
        let ctx = egui::Context::default();
        assert!(first_frame(&ctx, "a"));
        assert!(!first_frame(&ctx, "a"));
        assert!(first_frame(&ctx, "b"));
        forget(&ctx, "a");
        assert!(first_frame(&ctx, "a"));
    }
}
