//! Flashable firmware bundles embedded in the binary (feature
//! `bundled-firmware`, packed by build.rs). On start-up any bundle missing
//! from the firmware root is unpacked (sha256-verified against its manifest)
//! and the index is rebuilt, so a lone .exe / AppImage / .app ships with
//! every flashable version ready to use. Existing bundles are never touched.
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
    for name in manifests {
        let rel = name.trim_start_matches("bundles/").trim_end_matches("/manifest.json");
        let dir = root.join(rel);
        if dir.join("manifest.json").exists() {
            continue;
        }
        let mut text = String::new();
        zip.by_name(&name)?.read_to_string(&mut text)?;
        let manifest: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("parse {name}"))?;
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
    if installed > 0 {
        crate::index::Index::rebuild()?;
        log::info!(
            "bundled firmware: installed {installed} flashable bundles into {} ({} images written, {linked} hardlinked)",
            root.display(),
            written.len()
        );
    }
    Ok(installed)
}
