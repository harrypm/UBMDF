//! GUI view-model: index bundles with precomputed display strings, numeric
//! version ordering (flashable first, then newest → oldest) and inventory stats.
use std::path::PathBuf;

use crate::index::{BundleKind, Index};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Flashable,
    Inventory,
    Legacy,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Flashable => "Flashable",
            Kind::Inventory => "Inventory",
            Kind::Legacy => "Legacy inventory",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Flashable,
    Inventory,
}

pub struct Bundle {
    pub version: String,
    pub title: String,
    pub device: String,
    pub kind: Kind,
    /// Absolute manifest path (flashable only), separators normalised.
    pub manifest: Option<PathBuf>,
    /// Absolute inventory CSV path (inventories only).
    pub inventory: Option<PathBuf>,
    pub installer_zip: Option<PathBuf>,
    pub notes: Option<String>,
    pub stats: String,
    pub source: String,
}

/// "16.0.1" → [16, 0, 1]; non-numeric parts sort as 0.
pub fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

pub fn pretty_device(raw: &str) -> String {
    match raw {
        "ultrastudio4kmini" => "UltraStudio 4K Mini".into(),
        "(all Desktop Video devices)" => "All Desktop Video devices".into(),
        other => other.to_string(),
    }
}

pub fn human_bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.2} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1} MB", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.0} KB", n / 1e3)
    } else {
        format!("{n:.0} B")
    }
}

/// Index paths are stored with whichever separator the harvester used
/// (Windows writes '\'); normalise so they resolve on every platform.
fn resolve(rel: &str) -> PathBuf {
    let norm = rel.replace('\\', "/");
    let p = crate::index::firmware_root().join(norm);
    std::path::absolute(&p).unwrap_or(p)
}

/// (file count, total bytes) from an inventory CSV: `version,file,bytes,sha256`.
/// Parsed from the right so file paths containing commas stay harmless.
fn inventory_stats(path: &PathBuf) -> Option<(usize, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let mut it = line.rsplitn(3, ',');
        let _sha = it.next();
        if let Some(b) = it.next().and_then(|b| b.trim().parse::<u64>().ok()) {
            bytes += b;
        }
        files += 1;
    }
    Some((files, bytes))
}

pub fn load_bundles() -> (Vec<Bundle>, Option<String>) {
    let idx = match Index::load() {
        Ok(i) => i,
        Err(e) => return (Vec::new(), Some(format!("{e:#}"))),
    };
    let mut out: Vec<Bundle> = idx
        .bundles
        .iter()
        .map(|b| {
            let legacy = b.notes.as_deref().is_some_and(|n| n.contains("legacy layout"));
            let kind = match b.kind {
                BundleKind::Flashable => Kind::Flashable,
                BundleKind::EncryptedInventory if legacy => Kind::Legacy,
                BundleKind::EncryptedInventory => Kind::Inventory,
            };
            let manifest = b.manifest.as_deref().map(resolve);
            let inventory = b.inventory.as_deref().map(resolve);
            let stats = match kind {
                Kind::Flashable => {
                    let total: u64 = b.components.iter().map(|c| c.bytes).sum();
                    format!(
                        "{} component{} · {}",
                        b.components.len(),
                        if b.components.len() == 1 { "" } else { "s" },
                        human_bytes(total)
                    )
                }
                _ => match inventory.as_ref().and_then(inventory_stats) {
                    Some((n, bytes)) => format!("{n} files inventoried · {}", human_bytes(bytes)),
                    None => "inventory file missing".into(),
                },
            };
            let source = manifest
                .as_ref()
                .or(inventory.as_ref())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            Bundle {
                version: b.dv_version.clone(),
                title: format!("Desktop Video {}", b.dv_version),
                device: pretty_device(&b.device),
                kind,
                manifest,
                inventory,
                installer_zip: b.installer_zip.as_deref().map(resolve),
                notes: b.notes.clone(),
                stats,
                source,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        let fa = a.kind == Kind::Flashable;
        let fb = b.kind == Kind::Flashable;
        fb.cmp(&fa).then_with(|| version_key(&b.version).cmp(&version_key(&a.version)))
    });
    (out, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_numerically() {
        let mut v = vec!["10.2", "10.10", "9.5.3", "16.5", "10.11.4", "12.8.1"];
        v.sort_by_key(|x| std::cmp::Reverse(version_key(x)));
        assert_eq!(v, ["16.5", "12.8.1", "10.11.4", "10.10", "10.2", "9.5.3"]);
    }

    #[test]
    fn bytes_format() {
        assert_eq!(human_bytes(11_419_548), "11.4 MB");
        assert_eq!(human_bytes(251_900_000), "251.9 MB");
    }
}
