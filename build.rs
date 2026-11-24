// Build script:
// 1. Windows targets: embed the app icon (Explorer / taskbar / shortcut).
// 2. Feature `bundled-firmware`: pack every flashable bundle under
//    firmware/<device>/dv-*/ (manifest.json + component images) into
//    $OUT_DIR/bundled-firmware.zip. Images are stored once per sha256
//    ("blobs/<sha256>"), so 43 bundles over 4 distinct images stay small.
//    The binary installs missing bundles on first run (src/bundled.rs).
use std::io::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/ubdmf.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/ubdmf.ico");
        if let Err(e) = res.compile() {
            // Never fail the build over an icon (e.g. no Windows SDK rc.exe).
            println!("cargo:warning=icon not embedded: {e}");
        }
    }

    if std::env::var_os("CARGO_FEATURE_BUNDLED_FIRMWARE").is_some() {
        pack_firmware();
    }
}

fn pack_firmware() {
    use sha2::{Digest, Sha256};
    let root = PathBuf::from("firmware");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bundled-firmware.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&out).expect("create bundled-firmware.zip"));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9));
    let mut blobs = std::collections::HashSet::new();
    let mut bundles = 0;

    let mut devices: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("bundled-firmware: cannot read {}: {e}", root.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "installers"))
        .collect();
    devices.sort();
    for dev in devices {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(&dev).unwrap().flatten().map(|e| e.path())
            .filter(|p| p.join("manifest.json").is_file()).collect();
        versions.sort();
        for ver in versions {
            let rel = |p: &Path| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            println!("cargo:rerun-if-changed={}", ver.display());
            let manifest = ver.join("manifest.json");
            zip.start_file(format!("bundles/{}", rel(&manifest)), opts).unwrap();
            zip.write_all(&std::fs::read(&manifest).unwrap()).unwrap();
            for f in std::fs::read_dir(&ver).unwrap().flatten().map(|e| e.path()) {
                if f.extension().is_none_or(|x| x != "bin") {
                    continue;
                }
                let data = std::fs::read(&f).unwrap();
                if data.starts_with(b"version https://git-lfs") {
                    panic!(
                        "bundled-firmware: {} is a Git LFS pointer — run `git lfs pull --include=\"firmware/*/dv-*/**\"` first",
                        f.display()
                    );
                }
                let sha: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
                if blobs.insert(sha.clone()) {
                    zip.start_file(format!("blobs/{sha}"), opts).unwrap();
                    zip.write_all(&data).unwrap();
                }
            }
            bundles += 1;
        }
    }
    zip.finish().unwrap();
    if bundles == 0 {
        panic!("bundled-firmware: no flashable bundles found under firmware/<device>/dv-*/");
    }
    println!(
        "cargo:warning=bundled-firmware: {bundles} bundles, {} unique images, {:.1} MB packed",
        blobs.len(),
        std::fs::metadata(&out).unwrap().len() as f64 / 1e6
    );
}
