//! Termination signals that end the main loop like `q` does, so the terminal and
//! spotifyd are cleaned up. SIGTERM / SIGHUP on Unix, closing the console window or
//! logging off on Windows.

use anyhow::Result;

#[cfg(unix)]
pub struct Shutdown {
    term: tokio::signal::unix::Signal,
    hup: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl Shutdown {
    pub fn new() -> Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Shutdown {
            term: signal(SignalKind::terminate())?,
            hup: signal(SignalKind::hangup())?,
        })
    }

    pub async fn recv(&mut self) {
        tokio::select! {
            _ = self.term.recv() => {}
            _ = self.hup.recv() => {}
        }
    }
}

#[cfg(windows)]
pub struct Shutdown {
    close: tokio::signal::windows::CtrlClose,
    off: tokio::signal::windows::CtrlShutdown,
}

#[cfg(windows)]
impl Shutdown {
    pub fn new() -> Result<Self> {
        use tokio::signal::windows::{ctrl_close, ctrl_shutdown};
        Ok(Shutdown {
            close: ctrl_close()?,
            off: ctrl_shutdown()?,
        })
    }

    pub async fn recv(&mut self) {
        tokio::select! {
            _ = self.close.recv() => {}
            _ = self.off.recv() => {}
        }
    }
}
