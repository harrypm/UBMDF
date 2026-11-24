//! Flashable firmware bundles embedded in the binary (feature
//! `bundled-firmware`, packed by build.rs). On start-up any bundle missing
//! from the firmware root is unpacked (sha256-verified against its manifest)
//! and the index is rebuilt, so a lone .exe / AppImage / .app ships with
//! every flashable version ready to use. An installed bundle's images are never
//! touched; its manifest is refreshed when the bundled one has the same images
//! but newer metadata (e.g. corrected erase geometry).
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::Read as _;

static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundled-firmware.zip"));

/// Install bundles that are not present yet. Returns how many were installed.
pub fn install_missing() -> Result<usize> {
    let root = crate::index::firmware_root();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(ARCHIVE)).context("bundled archive")?;
    let manifests: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.to_string()))
        .filter(|n| n.starts_with("bundles/") && n.ends_with("/manifest.json"))
        .collect();

    let mut installed = 0;
    // sha256 -> first file written in this run. Later bundles with the same
    // image get a hardlink to it, so 43 bundles over 4 distinct images take
    // ~30 MB instead of ~470 MB. Falls back to a copy where hardlinks are not
    // supported (FAT/exFAT, cross-volume).
    let mut written: std::collections::HashMap<String, std::path::PathBuf> = Default::default();
    let mut linked = 0usize;
    let mut refreshed = 0usize;
    for name in manifests {
        let rel = name.trim_start_matches("bundles/").trim_end_matches("/manifest.json");
        let dir = root.join(rel);
        let mut text = String::new();
        zip.by_name(&name)?.read_to_string(&mut text)?;
        let manifest: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("parse {name}"))?;
        let installed_path = dir.join("manifest.json");
        if installed_path.exists() {
            // Refresh metadata only (erase geometry, notes) when the installed bundle
            // carries the exact same images; never touch a bundle whose images differ.
            if refresh_manifest(&installed_path, &manifest, &text)? {
                refreshed += 1;
            }
            continue;
        }
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        for c in manifest["components"].as_array().context("manifest components")? {
            let file = c["file"].as_str().context("component file")?;
            let sha = c["sha256"].as_str().context("component sha256")?;
            let dst = dir.join(file);
            if let Some(src) = written.get(sha) {
                let _ = std::fs::remove_file(&dst);
                if std::fs::hard_link(src, &dst).is_ok() {
                    linked += 1;
                    continue;
                }
            }
            let mut data = Vec::new();
            zip.by_name(&format!("blobs/{sha}"))
                .with_context(|| format!("{rel}: blob {sha} missing from bundled archive"))?
                .read_to_end(&mut data)?;
            let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
            if got != sha {
                bail!("{rel}/{file}: bundled image failed sha256 check");
            }
            std::fs::write(&dst, &data)?;
            written.entry(sha.to_string()).or_insert(dst);
        }
        // Manifest last: a bundle only counts as present once complete.
        std::fs::write(dir.join("manifest.json"), text)?;
        installed += 1;
    }
    if refreshed > 0 {
        log::info!("bundled firmware: refreshed {refreshed} installed manifests (same images, updated metadata)");
    }
    if installed > 0 || refreshed > 0 {
        crate::index::Index::rebuild()?;
        if installed > 0 {
            log::info!(
                "bundled firmware: installed {installed} flashable bundles into {} ({} images written, {linked} hardlinked)",
                root.display(),
                written.len()
            );
        }
    }
    Ok(installed)
}

/// Component (file, sha256) pairs of a manifest, in order.
fn component_ids(m: &serde_json::Value) -> Option<Vec<(String, String)>> {
    m["components"]
        .as_array()?
        .iter()
        .map(|c| Some((c["file"].as_str()?.to_string(), c["sha256"].as_str()?.to_string())))
        .collect()
}

/// Overwrite an installed manifest with the bundled one when both describe the
/// same images and the installed files still match their sha256. Returns true
/// when the file was rewritten.
fn refresh_manifest(installed: &std::path::Path, bundled: &serde_json::Value, text: &str) -> Result<bool> {
    let current_text = std::fs::read_to_string(installed)
        .with_context(|| format!("read {}", installed.display()))?;
    let Ok(current) = serde_json::from_str::<serde_json::Value>(&current_text) else {
        log::warn!("{}: unreadable manifest left untouched", installed.display());
        return Ok(false);
    };
    if &current == bundled {
        return Ok(false);
    }
    let (Some(a), Some(b)) = (component_ids(&current), component_ids(bundled)) else {
        return Ok(false);
    };
    if a != b {
        log::warn!("{}: installed images differ from the bundled ones — left untouched", installed.display());
        return Ok(false);
    }
    let dir = installed.parent().context("manifest has no parent dir")?;
    for (file, sha) in &b {
        let data = std::fs::read(dir.join(file)).with_context(|| format!("read {file}"))?;
        let got: String = Sha256::digest(&data).iter().map(|x| format!("{x:02x}")).collect();
        if &got != sha {
            log::warn!("{}: {file} fails sha256 — manifest left untouched", dir.display());
            return Ok(false);
        }
    }
    std::fs::write(installed, text).with_context(|| format!("write {}", installed.display()))?;
    log::info!("refreshed manifest {}", installed.display());
    Ok(true)
}
