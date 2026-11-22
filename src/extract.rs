//! Extractor: unzip Desktop Video installers, sandbox-extract the MSI via
//! `msiexec /a` (administrative extract — no system changes), and inventory
//! the encrypted Firmware/*.bin files with SHA-256 hashes.
//!
//! Windows-only (msiexec); the inventory output is a CSV consumed by the index.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Result of a successful extraction + inventory pass.
pub struct ExtractResult {
    #[allow(dead_code)] // informational, not directly consumed by callers
    pub firmware_dir: PathBuf,
    #[allow(dead_code)] // informational, not directly consumed by callers
    pub inventory_csv: PathBuf,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Extract the MSI from a Desktop Video installer zip, then admin-extract the MSI.
/// Returns the path to the extracted `Firmware/` directory.
pub fn extract_installer(zip_path: &Path, out_dir: &Path, version: &str) -> Result<ExtractResult> {
    let extract_dir = out_dir.join(format!("dv-{version}-extract"));
    if extract_dir.exists() {
        std::fs::remove_dir_all(&extract_dir)
            .with_context(|| format!("failed to clean {}", extract_dir.display()))?;
    }
    std::fs::create_dir_all(&extract_dir)?;

    // Step 1: extract the MSI from the zip
    let msi_path = extract_msi_from_zip(zip_path, &extract_dir)?;
    log::info!("MSI extracted: {}", msi_path.display());

    // Step 2: msiexec /a (administrative/sandbox extract)
    let target_dir = extract_dir.join("sandbox");
    std::fs::create_dir_all(&target_dir)?;
    admin_extract_msi(&msi_path, &target_dir)?;
    log::info!("sandbox extract complete: {}", target_dir.display());

    // Step 3: find the Firmware directory (16.0+ nests it, older versions may not)
    let firmware_dir = find_firmware_dir(&target_dir)
        .context("Firmware directory not found in sandbox extract")?;
    log::info!("firmware dir: {}", firmware_dir.display());

    // Step 4: inventory the data-*.bin files
    let inventory_path = out_dir.join(format!("dv-{version}-inventory.csv"));
    let (count, total) = inventory_firmware(&firmware_dir, &inventory_path, version)?;
    log::info!("inventory: {count} files, {total} bytes → {}", inventory_path.display());

    // Step 5: remove the sandbox extract (keep only the firmware + inventory)
    std::fs::remove_dir_all(&target_dir)
        .with_context(|| format!("failed to clean sandbox {}", target_dir.display()))?;

    Ok(ExtractResult {
        firmware_dir,
        inventory_csv: inventory_path,
        file_count: count,
        total_bytes: total,
    })
}

/// Find and extract the MSI from a Desktop Video zip.
fn extract_msi_from_zip(zip_path: &Path, out_dir: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("failed to open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to parse zip {}", zip_path.display()))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("failed to read zip entry {i}"))?;
        let name = entry
            .name()
            .map(|n| n.to_string())
            .unwrap_or_default();
        if !name.ends_with(".msi") {
            continue;
        }
        let msi_name = Path::new(&name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "installer.msi".to_string());
        let msi_path = out_dir.join(&msi_name);
        let mut msi_file = std::fs::File::create(&msi_path)?;
        std::io::copy(&mut entry, &mut msi_file)?;
        return Ok(msi_path);
    }
    anyhow::bail!("no .msi file found in {}", zip_path.display())
}

/// Run `msiexec /a <msi> /qn TARGETDIR=<dir>` (sandbox extract, no system changes).
fn admin_extract_msi(msi_path: &Path, target_dir: &Path) -> Result<()> {
    let status = std::process::Command::new("msiexec.exe")
        .arg("/a")
        .arg(msi_path)
        .arg("/qn")
        .arg(format!("TARGETDIR={}", target_dir.display()))
        .output()
        .context("failed to launch msiexec")?;
    if !status.status.success() {
        anyhow::bail!(
            "msiexec /a exited with {} — {}",
            status.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&status.stderr)
        );
    }
    Ok(())
}

/// Find the Firmware directory within the sandbox extract (handles nesting).
fn find_firmware_dir(root: &Path) -> Option<PathBuf> {
    // Direct: <root>/Firmware
    let direct = root.join("Firmware");
    if direct.is_dir() {
        return Some(direct);
    }
    // Nested: <root>/Blackmagic Design/Blackmagic Desktop Video/Firmware
    let nested = root
        .join("Blackmagic Design")
        .join("Blackmagic Desktop Video")
        .join("Firmware");
    if nested.is_dir() {
        return Some(nested);
    }
    // Search recursively (fallback for older layouts)
    find_dir_recursive(root, "Firmware")
}

fn find_dir_recursive(root: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        if entry.file_name() == name && entry.path().is_dir() {
            return Some(entry.path());
        }
        if entry.path().is_dir()
            && let Some(found) = find_dir_recursive(&entry.path(), name)
        {
            return Some(found);
        }
    }
    None
}

/// SHA-256 all `data-*.bin` files and write a CSV inventory.
/// Returns (file_count, total_bytes).
fn inventory_firmware(firmware_dir: &Path, csv_path: &Path, version: &str) -> Result<(usize, u64)> {
    let mut entries: Vec<_> = std::fs::read_dir(firmware_dir)
        .with_context(|| format!("failed to read {}", firmware_dir.display()))?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("data-") && n.ends_with(".bin")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut csv = String::from("version,file,bytes,sha256\n");
    let mut total = 0u64;
    for entry in &entries {
        let data = std::fs::read(entry.path())?;
        let mut h = Sha256::new();
        h.update(&data);
        let sha = crate::protocol::hex(&h.finalize());
        let name = entry.file_name().to_string_lossy().to_string();
        total += data.len() as u64;
        csv.push_str(&format!("{version},{name},{},{sha}\n", data.len()));
    }
    std::fs::write(csv_path, csv)?;
    Ok((entries.len(), total))
}

/// Import a local installer zip (bypasses the download).
pub fn import_local_zip(zip_path: &Path, out_dir: &Path) -> Result<ExtractResult> {
    let filename = zip_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Extract version from the filename: Blackmagic_Desktop_Video_Windows_16.0.zip
    let version = filename
        .trim_start_matches("Blackmagic_Desktop_Video_Windows_")
        .trim_end_matches(".zip")
        .to_string();
    if version.is_empty() || version == filename {
        anyhow::bail!("could not parse version from filename: {filename}");
    }
    log::info!("importing local zip for DV {version}: {}", zip_path.display());
    extract_installer(zip_path, out_dir, &version)
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_parse_from_filename() {
        let f = "Blackmagic_Desktop_Video_Windows_16.0.zip";
        let v = f
            .trim_start_matches("Blackmagic_Desktop_Video_Windows_")
            .trim_end_matches(".zip");
        assert_eq!(v, "16.0");
    }
}
