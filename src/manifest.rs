//! Flash image bundle manifest: JSON describing the plaintext components
//! captured from an official DesktopVideoUpdateTool flash, with sha256 hashes.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub image: ImageMeta,
    pub components: Vec<Component>,
}

#[derive(Debug, Deserialize)]
pub struct ImageMeta {
    pub name: String,
    pub source: String,
    /// Provenance / verification status shown in the index and GUI
    /// (e.g. "derived from installer, not yet flashed on hardware").
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Component {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    /// Region address in 64 KiB blocks (0x37 parameter 1, LE).
    pub region_block_addr: u32,
    /// Region length in 64 KiB blocks (0x37 parameter 2, LE).
    pub block_count: u32,
    /// Write byte address (0x39 parameter, BE).
    pub write_addr: u32,
}

impl Manifest {
    /// Load a manifest; returns it plus the directory component paths are relative to.
    pub fn load(path: &Path) -> Result<(Manifest, PathBuf)> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest {}", path.display()))?;
        let m: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse manifest {}", path.display()))?;
        if m.components.is_empty() {
            bail!("manifest has no components");
        }
        let base = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Ok((m, base))
    }

    /// Verify each component file exists, matches the recorded size and sha256.
    /// Returns the absolute-ish paths in component order.
    pub fn verify_files(&self, base: &Path) -> Result<Vec<PathBuf>> {
        let mut out = Vec::with_capacity(self.components.len());
        for c in &self.components {
            let p = base.join(&c.file);
            let meta = std::fs::metadata(&p)
                .with_context(|| format!("component file missing: {}", p.display()))?;
            if meta.len() != c.bytes {
                bail!(
                    "component {} size {} != manifest {}",
                    p.display(),
                    meta.len(),
                    c.bytes
                );
            }
            let data = std::fs::read(&p)
                .with_context(|| format!("failed to read {}", p.display()))?;
            let mut h = Sha256::new();
            h.update(&data);
            let actual = crate::protocol::hex(&h.finalize());
            if actual != c.sha256.to_lowercase() {
                bail!(
                    "component {} sha256 mismatch: expected {}, got {}",
                    p.display(),
                    c.sha256,
                    actual
                );
            }
            out.push(p);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "image": { "name": "test", "source": "test" },
      "components": [
        { "file": "a.bin", "sha256": "AA", "bytes": 1, "region_block_addr": 0, "block_count": 145, "write_addr": 0 }
      ]
    }"#;

    #[test]
    fn parses_sample_manifest() {
        let m: Manifest = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.components[0].block_count, 145);
        assert_eq!(m.components[0].sha256.to_lowercase(), "aa");
    }
}
