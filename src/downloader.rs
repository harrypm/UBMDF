//! BMD catalog client and signed-URL downloader for Desktop Video installers.
//!
//! Flow (community-documented, confirmed by live fetch this session):
//! 1. GET https://www.blackmagicdesign.com/api/support/us/downloads.json
//! 2. Find the version's `downloadId`
//! 3. POST https://www.blackmagicdesign.com/api/register/us/download/<id>
//!    with `{country,platform,product}` JSON + browser-ish headers
//!    → response body = signed CDN URL (expires in hours)
//! 4. Download the zip from the signed URL

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

const CATALOG_URL: &str = "https://www.blackmagicdesign.com/api/support/us/downloads.json";
const REGISTER_URL: &str = "https://www.blackmagicdesign.com/api/register/us/download";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    pub downloads: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    #[allow(dead_code)] // parsed for API completeness, not yet used
    pub desc: Option<String>,
    #[serde(default)]
    pub urls: std::collections::HashMap<String, Vec<CatalogUrl>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogUrl {
    #[serde(rename = "downloadId")]
    pub download_id: String,
    #[serde(rename = "releaseId")]
    #[allow(dead_code)] // parsed for API completeness, not yet used
    pub release_id: Option<String>,
    #[serde(rename = "downloadTitle")]
    #[allow(dead_code)] // parsed for API completeness, not yet used
    pub download_title: Option<String>,
}

/// A single Desktop Video version with its Windows downloadId.
#[derive(Debug, Clone)]
pub struct DesktopVideoVersion {
    pub version: String,
    pub download_id: String,
    #[allow(dead_code)] // parsed for API completeness, not yet used
    pub release_id: Option<String>,
}

fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .build()
}

/// Fetch the full BMD downloads catalog.
pub fn fetch_catalog() -> Result<Catalog> {
    let agent = http();
    let resp = agent
        .get(CATALOG_URL)
        .call()
        .map_err(|e| anyhow::anyhow!("catalog fetch failed: {e:#}"))?;
    let body: Catalog = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("catalog parse failed: {e:#}"))?;
    log::info!("catalog fetched: {} entries", body.downloads.len());
    Ok(body)
}

/// Filter the catalog for Desktop Video (non-SDK) versions with a Windows download.
pub fn desktop_video_versions(catalog: &Catalog) -> Vec<DesktopVideoVersion> {
    let mut out = Vec::new();
    for entry in &catalog.downloads {
        let name = entry.name.trim();
        if !name.starts_with("Desktop Video ") || name.contains("SDK") {
            continue;
        }
        let version = name
            .strip_prefix("Desktop Video ")
            .unwrap_or(name)
            .to_string();
        let Some(win_urls) = entry.urls.get("Windows") else {
            continue;
        };
        let Some(first) = win_urls.first() else {
            continue;
        };
        if first.download_id.is_empty() {
            continue;
        }
        out.push(DesktopVideoVersion {
            version,
            download_id: first.download_id.clone(),
            release_id: first.release_id.clone(),
        });
    }
    out
}

/// Get a fresh signed download URL for a specific downloadId.
pub fn get_signed_url(download_id: &str) -> Result<String> {
    let agent = http();
    let body = serde_json::json!({
        "country": "us",
        "platform": "Windows",
        "product": "Desktop Video",
        "policy": true,
    });
    let url = format!("{}/{}", REGISTER_URL, download_id);
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json;charset=UTF-8")
        .set("Origin", "https://www.blackmagicdesign.com")
        .send_json(&body)
        .map_err(|e| anyhow::anyhow!("register call failed: {e:#}"))?;
    let signed_url = resp
        .into_string()
        .map_err(|e| anyhow::anyhow!("signed URL read failed: {e:#}"))?;
    let signed_url = signed_url.trim().to_string();
    if signed_url.is_empty() || !signed_url.starts_with("http") {
        bail!("unexpected signed URL response: {}...", &signed_url[..64.min(signed_url.len())]);
    }
    log::info!("signed URL obtained (expires in hours)");
    Ok(signed_url)
}

/// Download a zip from a signed URL to a local path, returning bytes written.
pub fn download_zip(signed_url: &str, dest: &Path, on_progress: &mut dyn FnMut(u64, u64)) -> Result<u64> {
    let agent = http();
    let resp = agent
        .get(signed_url)
        .call()
        .map_err(|e| anyhow::anyhow!("zip download failed: {e:#}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut file = std::fs::File::create(dest)
        .with_context(|| format!("failed to create {}", dest.display()))?;
    let mut written = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| anyhow::anyhow!("download read failed: {e:#}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n])?;
        written += n as u64;
        on_progress(written, total);
    }
    log::info!("downloaded {} bytes to {}", written, dest.display());
    Ok(written)
}

/// Find a specific Desktop Video version by version string (e.g. "16.5").
pub fn find_version(catalog: &Catalog, version: &str) -> Result<DesktopVideoVersion> {
    let versions = desktop_video_versions(catalog);
    versions
        .into_iter()
        .find(|v| v.version == version)
        .with_context(|| format!("Desktop Video {version} not found in catalog (non-SDK, Windows)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_catalog_entry() {
        let json = r#"{
            "name": "Desktop Video 16.5",
            "urls": {
                "Windows": [{"downloadId": "abc123", "releaseId": "rel456"}]
            }
        }"#;
        let entry: CatalogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.name, "Desktop Video 16.5");
        let win = entry.urls.get("Windows").unwrap();
        assert_eq!(win[0].download_id, "abc123");
    }

    #[test]
    fn filter_desktop_video() {
        let cat = Catalog {
            downloads: vec![
                CatalogEntry {
                    name: "Desktop Video 16.5".to_string(),
                    desc: None,
                    urls: {
                        let mut m = std::collections::HashMap::new();
                        m.insert("Windows".to_string(), vec![CatalogUrl {
                            download_id: "id1".to_string(),
                            release_id: None,
                            download_title: None,
                        }]);
                        m
                    },
                },
                CatalogEntry {
                    name: "Desktop Video 16.5 SDK".to_string(),
                    desc: None,
                    urls: std::collections::HashMap::new(),
                },
                CatalogEntry {
                    name: "DaVinci Resolve 20".to_string(),
                    desc: None,
                    urls: std::collections::HashMap::new(),
                },
            ],
        };
        let v = desktop_video_versions(&cat);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].version, "16.5");
        assert_eq!(v[0].download_id, "id1");
    }
}
