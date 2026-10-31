use crate::AppEvent;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

pub const MAX: u16 = 1000;

/// Headless config: raw ASCII frames on stdout. `channels = mono` makes cava average
/// left+right into one set of bars (stereo output is mirrored: left reversed, then right).
pub fn config(bars: u16) -> String {
    format!(
        "[general]\nbars = {bars}\nframerate = 60\n\n\
             [output]\nmethod = raw\nchannels = mono\nmono_option = average\n\
             raw_target = /dev/stdout\ndata_format = ascii\nascii_max_range = {MAX}\n\
             bar_delimiter = 59\nframe_delimiter = 10\n"
    )
}

pub async fn run(bars: u16, tx: UnboundedSender<AppEvent>) {
    let fail = |msg: String| {
        let _ = tx.send(AppEvent::CavaError(msg));
    };

    // cava's raw output goes to /dev/stdout and it has no Windows capture setup here
    if cfg!(windows) {
        return fail("the visualizer is not available on Windows".into());
    }

    let path = crate::util::tmp_dir().join(format!("cava-{bars}.conf"));
    if crate::util::write_safely(&path, config(bars).as_bytes()).is_err() {
        return fail("couldn't write the cava config in /tmp".into());
    }
    let child = Command::new("cava")
        .arg("-p")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true) // aborting this task kills cava
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return fail("cava not found in PATH (pacman -S cava)".into());
        }
        Err(e) => return fail(format!("couldn't start cava: {e}")),
    };
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return fail("couldn't attach to cava's output".into());
    };

    // Remember cava's last complaint so a failure can say *why*.
    let last_err = Arc::new(Mutex::new(String::new()));
    let le = last_err.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(err).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if !l.trim().is_empty() {
                *le.lock().unwrap() = l.trim().to_string();
            }
        }
    });

    let mut lines = BufReader::new(out).lines();
    let mut got_data = false;
    loop {
        // If nothing arrives at first, say so instead of showing "starting…" forever.
        let next = if got_data {
            lines.next_line().await
        } else {
            match tokio::time::timeout(Duration::from_secs(4), lines.next_line()).await {
                Ok(r) => r,
                Err(_) => {
                    let _ = tx.send(AppEvent::CavaNote(
                        "cava is running but sent no data — check [input] method in your cava config"
                            .into(),
                    ));
                    got_data = true; // stop timing out; keep waiting normally
                    continue;
                }
            }
        };
        match next {
            Ok(Some(line)) => {
                let bars: Vec<u16> = line
                    .split(';')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect();
                if !bars.is_empty() {
                    got_data = true;
                    if tx.send(AppEvent::Cava(bars)).is_err() {
                        return;
                    }
                }
            }
            _ => break,
        }
    }

    let status = child.wait().await.ok();
    let _ = tokio::time::timeout(Duration::from_millis(300), err_task).await;
    let why = last_err.lock().unwrap().clone();
    fail(match (why.is_empty(), status) {
        (false, _) => format!("cava failed: {why}"),
        (true, Some(s)) if !s.success() => format!("cava exited ({s})"),
        _ => "cava exited".into(),
    });
}
