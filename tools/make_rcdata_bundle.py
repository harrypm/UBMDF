"""Build a flashable UBMDF bundle from plaintext CheckVersionPCI.exe RCDATA
images (DV 10.9.10 - 11.4.1), using a hardware-verified bundle as the flash
geometry reference.

Geometry rule (from the verified captures): each component keeps the reference
region_block_addr / write_addr, and block_count = ceil(bytes / 64 KiB). The
tool refuses to build if any image needs more blocks than the reference region
(i.e. if it would not fit the layout that was proven on hardware).

Usage:
  python -I tools/make_rcdata_bundle.py --device ultrastudio4kmini \
      --version 11.4.1 --rc 200 202 --reference dv-16.0 \
      --name "UltraStudio 4K Mini - Desktop Video 11.4.1" \
      --notes "..." [--also 11.3 11.4]
"""
import argparse
import hashlib
import json
import math
import os
import shutil
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FW = os.path.join(ROOT, "firmware")
BLOCK = 65536


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def rc_path(version, rc):
    return os.path.join(FW, "installers", f"dv-{version}-extract", "Firmware",
                        "CheckVersionPCI-rcdata", f"rc-{rc}.bin")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--device", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--rc", nargs="+", required=True, help="resource ids, in reference component order")
    ap.add_argument("--reference", required=True, help="verified bundle dir name, e.g. dv-16.0")
    ap.add_argument("--name", required=True)
    ap.add_argument("--notes", required=True)
    ap.add_argument("--also", nargs="*", default=[], help="other versions whose resources must be byte-identical")
    a = ap.parse_args()

    ref_dir = os.path.join(FW, a.device, a.reference)
    ref = json.load(open(os.path.join(ref_dir, "manifest.json"), encoding="utf-8"))
    if len(ref["components"]) != len(a.rc):
        sys.exit(f"reference has {len(ref['components'])} components, got {len(a.rc)} resource ids")

    out_dir = os.path.join(FW, a.device, f"dv-{a.version}")
    if os.path.exists(out_dir):
        sys.exit(f"refusing to overwrite existing bundle {out_dir}")

    comps = []
    for ref_c, rc in zip(ref["components"], a.rc):
        src = rc_path(a.version, rc)
        digest = sha256(src)
        for other in a.also:
            od = sha256(rc_path(other, rc))
            if od != digest:
                sys.exit(f"rc-{rc} differs between {a.version} and {other}")
        size = os.path.getsize(src)
        blocks = math.ceil(size / BLOCK)
        if blocks > ref_c["block_count"]:
            sys.exit(f"rc-{rc}: {blocks} blocks > reference region {ref_c['block_count']} — layout not proven")
        comps.append((src, {
            "file": ref_c["file"],
            "sha256": digest,
            "bytes": size,
            "region_block_addr": ref_c["region_block_addr"],
            "block_count": blocks,
            "write_addr": ref_c["write_addr"],
            "source_resource": f"CheckVersionPCI.exe RCDATA {rc} (DV {a.version})",
        }))

    os.makedirs(out_dir)
    for src, c in comps:
        shutil.copyfile(src, os.path.join(out_dir, c["file"]))
        assert sha256(os.path.join(out_dir, c["file"])) == c["sha256"]
    manifest = {
        "image": {
            "name": a.name,
            "source": f"plaintext CheckVersionPCI.exe RCDATA resources from the official DV {a.version} installer "
                      f"(tools/extract-updater-firmware.ps1); geometry from {a.device}/{a.reference}",
            "protocol": "docs/protocol.md",
            "notes": a.notes,
        },
        "components": [c for _, c in comps],
    }
    with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"bundle written: {out_dir}")
    for _, c in comps:
        print(f"  {c['file']}: {c['bytes']} B, {c['block_count']} blocks @ region {c['region_block_addr']}, "
              f"write {c['write_addr']:#010x}, sha256 {c['sha256'][:16]}…")


if __name__ == "__main__":
    main()
