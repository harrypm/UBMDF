//! UBDMF integrated log system.
//!
//! One `log` facade, three sinks:
//! - terminal readout (stderr)
//! - file sink (`UBDMF_LOG` env override, else `logs/ubdmf.log` anchored to the
//!   resolved firmware root's parent — the repo root — so it is CWD-independent)
//! - in-memory ring buffer (capped) that the GUI renders live
//!
//! Init is idempotent so tests can call it freely.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const RING_CAPACITY: usize = 5000;

fn ring() -> &'static Mutex<VecDeque<String>> {
    static RING: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(RING_CAPACITY)))
}

/// Full snapshot of the in-memory log ring (oldest first) for the GUI.
#[allow(dead_code)] // used by the GUI (U4)
pub fn snapshot() -> Vec<String> {
    ring()
        .lock()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default()
}

/// The most recent `n` log lines (oldest first within the slice).
#[allow(dead_code)] // used by the GUI (U4)
pub fn tail(n: usize) -> Vec<String> {
    let r = ring().lock();
    let r = match r {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let start = r.len().saturating_sub(n);
    r.iter().skip(start).cloned().collect()
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

struct UbdmfLogger {
    file: Mutex<Option<File>>,
}

impl log::Log for UbdmfLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // raylib's TraceLog messages are forwarded by the log bridge with
        // target "raylib" — tag them so the integrated console is readable.
        let src = if record.target() == "raylib" { "[raylib] " } else { "" };
        let line = format!("[{}] {:5} {src}{}", timestamp(), record.level(), record.args());
        // Non-panicking stderr sink: GUI-subsystem runs may have no console at all.
        let _ = writeln!(std::io::stderr(), "{line}");

        if let Ok(mut r) = ring().lock() {
            if r.len() >= RING_CAPACITY {
                r.pop_front();
            }
            r.push_back(line.clone());
        }

        if let Ok(mut guard) = self.file.lock()
            && let Some(f) = guard.as_mut()
        {
            let _ = writeln!(f, "{line}");
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock()
            && let Some(f) = guard.as_mut()
        {
            let _ = f.flush();
        }
    }
}

pub fn log_path() -> PathBuf {
    if let Ok(p) = std::env::var("UBDMF_LOG") {
        return PathBuf::from(p);
    }
    // Anchor the log beside the firmware root (repo root) so launches from any
    // directory — double-click, exe dir, terminal — keep writing the same file
    // instead of scattering logs/ folders. Falls back to CWD-relative when no
    // firmware root is found.
    crate::index::firmware_root()
        .parent()
        .map(|p| p.join("logs").join("ubdmf.log"))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("logs").join("ubdmf.log"))
}

/// Initialise the logger (idempotent — a second call is a no-op).
pub fn init() {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();

    let logger = UbdmfLogger {
        file: Mutex::new(file),
    };
    // Ignore the "already set" error so tests / re-init are harmless.
    let _ = log::set_boxed_logger(Box::new(logger));
    // Info for the terminal/file sinks — Debug would forward nusb's per-device
    // probe spam into every sink.
    log::set_max_level(log::LevelFilter::Info);
    log::info!("UBDMF logging initialised (file: {})", path.display());
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_never_panics_without_init() {
        // snapshot/tail must be safe pre-init (GUI may poll before any log line).
        let _ = super::snapshot();
        let _ = super::tail(4);
    }
}
