//! Device operations shared by the CLI and the GUI: read-only snapshot/dump,
//! clean session close, and manifest → component-spec loading (sha256-verified).
//! Keeping these in one place means the GUI's Dump/Reset/Flash buttons run the
//! exact code paths the CLI commands were hardware-validated with.
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::Manifest;
use crate::protocol::{self, BmdDevice, ComponentSpec};

/// Shown before every flash (CLI prompt, CLI --yes, GUI confirm dialog).
/// Full text: DISCLAIMER.md.
pub const FLASH_DISCLAIMER: &str = "UBMDF is unofficial and not affiliated with or endorsed by Blackmagic Design. Flashing firmware can permanently brick the device and may void its warranty or support. The software comes with NO WARRANTY of any kind; you flash at your own risk. See DISCLAIMER.md.";

#[derive(Serialize)]
pub struct ProgressSnapshot {
    pub region: u32,
    pub expected: u32,
    pub done: u32,
    pub busy: bool,
}

#[derive(Serialize)]
pub struct DeviceSnapshot {
    pub nonce: String,
    pub id_byte: u8,
    pub state: String,
    pub geometry: Option<[u32; 4]>,
    pub progress: Option<ProgressSnapshot>,
}

#[derive(Serialize)]
pub struct DumpDoc {
    pub timestamp_unix: u64,
    pub device: DeviceSnapshot,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Everything readable from the device in one read-only pass.
pub fn read_device_snapshot(dev: &mut BmdDevice) -> Result<DeviceSnapshot> {
    let (nonce, id_byte) = protocol::read_id(&dev.iface)?;
    let nonce_hex = protocol::hex(&nonce);

    protocol::begin_session(dev)?;
    let state = protocol::read_state(&dev.iface)?;
    let state_hex = protocol::hex(&state);

    // 0x35 stalls outside an active erase session and 0x38 is idle outside one —
    // both tolerated (documented in protocol.md).
    let geometry = protocol::read_geometry(&dev.iface).ok();
    let progress = protocol::read_progress(&dev.iface).ok();

    Ok(DeviceSnapshot {
        nonce: nonce_hex,
        id_byte,
        state: state_hex,
        geometry: geometry.map(|g| g.fields),
        progress: progress.map(|p| ProgressSnapshot {
            region: p.region,
            expected: p.expected,
            done: p.done,
            busy: p.busy,
        }),
    })
}

/// Close a read session cleanly (0x12) so no lingering state remains — an
/// unterminated session makes the device refuse the next 0x31 (hardware-verified).
pub fn close_session(dev: &BmdDevice) -> Result<()> {
    match protocol::reboot(&dev.iface) {
        Ok(()) => {
            log::info!("session closed cleanly (0x12)");
            Ok(())
        }
        Err(e) => {
            log::warn!("session close (0x12) failed: {e:#}; run `ubmdf reset` to clear");
            Err(e).context("session close (0x12) failed; run `ubmdf reset` to clear")
        }
    }
}

/// Default folder for GUI dumps: `dumps/` beside the resolved firmware root.
#[cfg(feature = "gui")]
pub fn dumps_dir() -> PathBuf {
    crate::index::firmware_root()
        .parent()
        .map(|p| p.join("dumps"))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("dumps"))
}

/// Read-only device dump → pretty JSON file. Returns the written path.
/// The session is closed (0x12) even when the write fails.
pub fn dump_device(out: Option<PathBuf>) -> Result<PathBuf> {
    let mut dev = protocol::open()?;
    let snap = read_device_snapshot(&mut dev);
    let close = close_session(&dev);
    let snap = snap?;

    let ts = unix_now();
    let doc = DumpDoc { timestamp_unix: ts, device: snap };
    let path = out.unwrap_or_else(|| PathBuf::from(format!("ubmdf-dump-{ts}.json")));
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write dump {}", path.display()))?;
    log::info!("device dump written: {}", path.display());
    if let Err(e) = close {
        log::warn!("dump written but {e:#}");
    }
    Ok(path)
}

/// Load a manifest, verify every component's sha256, and read the images.
pub fn load_specs(manifest: &Path) -> Result<(Manifest, Vec<ComponentSpec>)> {
    let (m, base) = Manifest::load(manifest)?;
    let files = m.verify_files(&base)?;
    let mut specs = Vec::with_capacity(m.components.len());
    for (c, p) in m.components.iter().zip(files) {
        let data =
            std::fs::read(&p).with_context(|| format!("failed to read {}", p.display()))?;
        specs.push(ComponentSpec {
            region_block_addr: c.region_block_addr,
            block_count: c.block_count,
            write_addr: c.write_addr,
            data,
        });
    }
    Ok((m, specs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_doc_serializes() {
        let doc = DumpDoc {
            timestamp_unix: 1763600000,
            device: DeviceSnapshot {
                nonce: "0c4e541523254d53baf8ee574d1f3bfe".into(),
                id_byte: 0,
                state: "01000000ffff".into(),
                geometry: Some([2, 0, 256, 65536]),
                progress: Some(ProgressSnapshot {
                    region: 192,
                    expected: 30,
                    done: 222,
                    busy: false,
                }),
            },
        };
        let json = serde_json::to_string_pretty(&doc).unwrap();
        assert!(json.contains("\"nonce\": \"0c4e541523254d53baf8ee574d1f3bfe\""));
        // round-trip
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["device"]["id_byte"], 0);
    }
}
