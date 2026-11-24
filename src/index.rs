//! Firmware index: the master registry of all firmware bundles UBDMF knows about.
//!
//! `firmware/index.json` is the on-disk index; `rebuild` scans the firmware
//! tree and regenerates it. Two kinds of entries:
//! - `flashable`: plaintext capture-derived bundles (manifest + components)
//! - `encrypted-inventory` (historical name): sha256 inventories of the
//!   installers' `data-rNN.bin` zlib containers / updater RCDATA images
//!   (not encrypted — see tools/datar.py and docs/firmware-map.md)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn firmware_root() -> PathBuf {
    // 1. Explicit override always wins.
    if let Ok(p) = std::env::var("UBDMF_FIRMWARE") {
        return PathBuf::from(p);
    }
    // 2. Relative to the current directory (running from the repo root).
    let cwd = PathBuf::from("firmware");
    if cwd.is_dir() {
        return cwd;
    }
    // 3. Walk up from the executable's location (target/release -> repo
    //    root; UBDMF.app/Contents/MacOS -> folder holding the .app). For an
    //    AppImage the exe lives in a temporary mount, so start from the
    //    .AppImage file itself ($APPIMAGE) instead.
    let start = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    if let Some(exe) = start {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..5 {
            let Some(d) = dir else { break; };
            let candidate = d.join("firmware");
            if candidate.is_dir() {
                log::debug!("firmware root resolved to {}", candidate.display());
                return candidate;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    // 4. Fallback: a writable per-user data folder, so portable builds
    //    (.app launched with CWD "/", read-only AppImage mount) can still keep
    //    their log, settings, dumps and downloads. Created on demand.
    user_data_dir().map(|d| d.join("firmware")).unwrap_or_else(|| PathBuf::from("firmware"))
}

/// Per-user application data folder for UBDMF, per platform conventions.
fn user_data_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").or_else(|| env("APPDATA"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local").join("share")))
    }?;
    Some(base.join("UBDMF"))
}

pub fn index_path() -> PathBuf {
    firmware_root().join("index.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Index {
    pub version: u32,
    #[serde(default)]
    pub bundles: Vec<BundleEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntry {
    /// Unique id: "<device>/<dv-version>" e.g. "ultrastudio4kmini/dv-12.8.1"
    pub id: String,
    pub device: String,
    pub dv_version: String,
    pub kind: BundleKind,
    /// Path to the manifest.json (flashable) relative to firmware/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    /// Path to the inventory CSV (encrypted-inventory) relative to firmware/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<String>,
    /// Path to the extracted installer zip (for provenance) relative to firmware/
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installer_zip: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub components: Vec<IndexComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BundleKind {
    Flashable,
    EncryptedInventory,
}

impl std::fmt::Display for BundleKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleKind::Flashable => write!(f, "flashable"),
            BundleKind::EncryptedInventory => write!(f, "encrypted-inventory"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexComponent {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_block_addr: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_addr: Option<u32>,
}

impl Index {
    /// Load the index from disk (or return an empty one if it doesn't exist).
    pub fn load() -> Result<Index> {
        let path = index_path();
        if !path.exists() {
            return Ok(Index {
                version: 1,
                bundles: Vec::new(),
            });
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read index {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("failed to parse index {}", path.display()))
    }

    /// Save the index to disk.
    pub fn save(&self) -> Result<()> {
        let path = index_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)
            .with_context(|| format!("failed to write index {}", path.display()))?;
        Ok(())
    }

    /// Find a bundle by id (exact match or unique prefix).
    pub fn find(&self, id: &str) -> Option<&BundleEntry> {
        // exact
        if let Some(b) = self.bundles.iter().find(|b| b.id == id) {
            return Some(b);
        }
        // unique prefix
        let matches: Vec<_> = self.bundles.iter().filter(|b| b.id.starts_with(id)).collect();
        if matches.len() == 1 {
            return Some(matches[0]);
        }
        None
    }

    /// Scan the firmware tree and rebuild the index from what's on disk.
    /// Flashable bundles: any dir containing manifest.json + component files.
    /// Encrypted inventories: any dir containing an inventory CSV.
    pub fn rebuild() -> Result<Index> {
        let root = firmware_root();
        let mut bundles = Vec::new();

        scan_flashable(&root, &mut bundles)?;
        scan_inventories(&root, &mut bundles)?;

        bundles.sort_by(|a, b| a.id.cmp(&b.id));

        let idx = Index {
            version: 1,
            bundles,
        };
        idx.save()?;
        Ok(idx)
    }
}

/// Scan for flashable bundles: dirs under firmware/<device>/dv-<version>/ containing manifest.json.
fn scan_flashable(root: &Path, bundles: &mut Vec<BundleEntry>) -> Result<()> {
    let Ok(device_dirs) = std::fs::read_dir(root) else {
        return Ok(());
    };
    for device_dir in device_dirs.flatten() {
        let Ok(version_dirs) = std::fs::read_dir(device_dir.path()) else {
            continue;
        };
        for version_dir in version_dirs.flatten() {
            let manifest_path = version_dir.path().join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }
            let version_path = version_dir.path();
            let rel = match version_path.strip_prefix(root) {
                Ok(r) => r.to_path_buf(),
                Err(_) => version_path.to_path_buf(),
            };
            let device = device_dir.file_name().to_string_lossy().to_string();
            let dir_name = version_dir.file_name().to_string_lossy().to_string();
            // dir_name is "dv-<version>"; extract version
            let dv_version = dir_name
                .strip_prefix("dv-")
                .unwrap_or(&dir_name)
                .to_string();

            // Read the manifest for component info
            let manifest_text = std::fs::read_to_string(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?;
            let manifest: crate::manifest::Manifest = serde_json::from_str(&manifest_text)
                .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

            let components: Vec<IndexComponent> = manifest
                .components
                .iter()
                .map(|c| IndexComponent {
                    file: c.file.clone(),
                    sha256: c.sha256.clone(),
                    bytes: c.bytes,
                    region_block_addr: Some(c.region_block_addr),
                    block_count: Some(c.block_count),
                    write_addr: Some(c.write_addr),
                })
                .collect();

            let is_known_bad = dv_version == "12.9";
            bundles.push(BundleEntry {
                id: format!("{}/{}", device, dir_name),
                device: device.clone(),
                dv_version,
                kind: BundleKind::Flashable,
                manifest: Some(rel.join("manifest.json").to_string_lossy().to_string()),
                inventory: None,
                installer_zip: None,
                components,
                notes: if is_known_bad {
                    Some(format!(
                        "KNOWN-BAD for UltraStudio 4K Mini: the official 12.9 update bricked multiple units (see forum t=199366). \
                         The 12.9 installer's 4K Mini images are byte-identical to DV 14.0-16.5 and the verified 16.0 capture, \
                         so the cause was not these image files; prefer a later version anyway. {}",
                        manifest.image.notes.clone().unwrap_or_default()
                    ))
                } else {
                    manifest.image.notes.clone()
                },
            });
        }
    }
    Ok(())
}

/// Scan for encrypted inventories: firmware/installers/dv-<version>-inventory.csv files.
fn scan_inventories(root: &Path, bundles: &mut Vec<BundleEntry>) -> Result<()> {
    let inv_dir = root.join("installers");
    let Ok(entries) = std::fs::read_dir(&inv_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("dv-") || !name.ends_with("-inventory.csv") {
            continue;
        }
        let dv_version = name
            .trim_start_matches("dv-")
            .trim_end_matches("-inventory.csv")
            .to_string();
        let id = format!("encrypted/dv-{}", dv_version);
        let zip_name = format!("Blackmagic_Desktop_Video_Windows_{}.zip", dv_version);
        let zip = inv_dir.join(&zip_name);
        // Updater-embedded layout (DV 10.9.10 - 11.4.1): the device images are
        // PE RCDATA resources inside CheckVersionPCI.exe, extracted by
        // tools/extract-updater-firmware.ps1 (plaintext, unlike data-r*.bin).
        let rcdata_manifest = inv_dir
            .join(format!("dv-{dv_version}-extract"))
            .join("Firmware")
            .join("CheckVersionPCI-rcdata")
            .join("resources.csv");
        let rcdata_images = std::fs::read_to_string(&rcdata_manifest)
            .ok()
            .map(|t| t.lines().skip(1).filter(|l| !l.trim().is_empty()).count());
        // Legacy-layout inventories (DV <= 10.9.7): the file column carries
        // relative paths ("Blackmagic Design\...\decklink.sys") instead of bare
        // data-*.bin names. Detected via a path separator in the first data row.
        let legacy = rcdata_images.is_none()
            && std::fs::read_to_string(inv_dir.join(&name))
                .ok()
                .and_then(|t| t.lines().nth(1).map(|l| l.contains('\\') || l.contains('/')))
                .unwrap_or(false);
        bundles.push(BundleEntry {
            id,
            device: "(all Desktop Video devices)".to_string(),
            dv_version,
            kind: BundleKind::EncryptedInventory,
            manifest: None,
            inventory: Some(format!("installers/{}", name)),
            installer_zip: if zip.exists() {
                Some(format!("installers/{}", zip_name))
            } else {
                None
            },
            components: Vec::new(),
            notes: if let Some(n) = rcdata_images {
                Some(format!(
                    "updater-embedded layout (DV 10.9.10 - 11.4.1): {n} plaintext firmware images extracted from CheckVersionPCI.exe RCDATA resources (Firmware/CheckVersionPCI-rcdata, see resources.csv)"
                ))
            } else if legacy {
                Some("legacy layout (DV <= 10.9.7): no separate Firmware dir - firmware ships inside drivers; full installer payload inventoried, zip = authoritative copy".to_string())
            } else {
                None
            },
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_round_trips() {
        let idx = Index {
            version: 1,
            bundles: vec![BundleEntry {
                id: "ultrastudio4kmini/dv-12.8.1".to_string(),
                device: "UltraStudio 4K Mini".to_string(),
                dv_version: "12.8.1".to_string(),
                kind: BundleKind::Flashable,
                manifest: Some("ultrastudio4kmini/dv-12.8.1/manifest.json".to_string()),
                inventory: None,
                installer_zip: None,
                components: vec![IndexComponent {
                    file: "component_A.bin".to_string(),
                    sha256: "abc123".to_string(),
                    bytes: 9491428,
                    region_block_addr: Some(0),
                    block_count: Some(145),
                    write_addr: Some(0),
                }],
                notes: None,
            }],
        };
        let json = serde_json::to_string_pretty(&idx).unwrap();
        let back: Index = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bundles.len(), 1);
        assert_eq!(back.bundles[0].kind, BundleKind::Flashable);
        assert_eq!(back.bundles[0].components[0].bytes, 9491428);
    }

    #[test]
    fn find_by_exact_and_prefix() {
        let idx = Index {
            version: 1,
            bundles: vec![
                BundleEntry {
                    id: "a/dv-1.0".to_string(),
                    device: "a".to_string(),
                    dv_version: "1.0".to_string(),
                    kind: BundleKind::Flashable,
                    manifest: None,
                    inventory: None,
                    installer_zip: None,
                    components: Vec::new(),
                    notes: None,
                },
                BundleEntry {
                    id: "a/dv-2.0".to_string(),
                    device: "a".to_string(),
                    dv_version: "2.0".to_string(),
                    kind: BundleKind::Flashable,
                    manifest: None,
                    inventory: None,
                    installer_zip: None,
                    components: Vec::new(),
                    notes: None,
                },
            ],
        };
        assert!(idx.find("a/dv-1.0").is_some());
        assert!(idx.find("a/dv-2").is_some()); // unique prefix
        assert!(idx.find("a/dv-").is_none()); // ambiguous prefix
        assert!(idx.find("nonexistent").is_none());
    }
}
