//! Logs to stderr and `$XDG_STATE_HOME/launchpad-desktop/companion.log`, every line through `redact` first. The file
//! is kept under 1 MB (one older file is kept as `companion.log.1`).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

use crate::redact::redact;

const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
struct Redacting {
    file: Arc<Mutex<Option<File>>>,
}

pub struct Line {
    file: Arc<Mutex<Option<File>>>,
}

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = redact(&String::from_utf8_lossy(buf));
        let _ = std::io::stderr().write_all(text.as_bytes());
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            let _ = file.write_all(text.as_bytes());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Redacting {
    type Writer = Line;
    fn make_writer(&'a self) -> Line {
        Line { file: self.file.clone() }
    }
}

pub fn init() {
    let dir = crate::settings::state_dir();
    let path = dir.join("companion.log");
    let file = std::fs::create_dir_all(&dir).ok().and_then(|_| {
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
            let _ = std::fs::rename(&path, dir.join("companion.log.1"));
        }
        // Only the owner reads it (lines are redacted, but they name Homes and agents).
        std::os::unix::fs::OpenOptionsExt::mode(OpenOptions::new().create(true).append(true), 0o600).open(&path).ok()
    });
    let filter = EnvFilter::try_from_env("LAUNCHPAD_DESKTOP_LOG").unwrap_or_else(|_| EnvFilter::new("info,zbus=warn,ksni=warn,reqwest=warn,hyper=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_target(false).with_writer(Redacting { file: Arc::new(Mutex::new(file)) }).init();
}
