"""Build flashable bundles for every Desktop Video version (11.5+) straight from
the official installer's Firmware/data-rNN.bin containers (see tools/datar.py).

For each version, the container whose records target the given USB device
(default 1edb:be13 = UltraStudio 4K Mini service port) is unpacked. Geometry
comes from the record headers (write address -> region block, size -> block
count) and must agree with the official-tool capture bundles' layout; the erase
count is always the full captured region (145 blocks for component A).
If the images are byte-identical to an official-tool capture bundle, the
manifest says so; otherwise it is marked "NOT yet flashed on hardware".

Identical component files are hardlinked to the verified bundle's copy (NTFS /
ext4 / APFS) to avoid duplicating ~11 MB per version; falls back to copying.

Usage:
  python -I tools/make_datar_bundles.py [--device ultrastudio4kmini] [--pid be13]
      [--reference dv-12.8.1 dv-16.0] [--dry-run]
"""
import argparse
import glob
import hashlib
import json
import math
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import datar  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FW = os.path.join(ROOT, "firmware")
INST = os.path.join(FW, "installers")
BLOCK = 65536


def vkey(v):
    return [int(x) for x in v.split(".")]


def sha(b):
    return hashlib.sha256(b).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--device", default="ultrastudio4kmini")
    ap.add_argument("--vid", default="1edb")
    ap.add_argument("--pid", default="be13")
    ap.add_argument("--reference", nargs="+", default=["dv-12.8.1", "dv-16.0"],
                    help="official-tool capture bundles (geometry + identity check)")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    vid, pid = int(a.vid, 16), int(a.pid, 16)
    dev_dir = os.path.join(FW, a.device)

    # Verified references: component sha256 -> (bundle, file path); layout per component index.
    verified = {}
    layout = {}
    for r in a.reference:
        m = json.load(open(os.path.join(dev_dir, r, "manifest.json"), encoding="utf-8"))
        for i, c in enumerate(m["components"]):
            verified.setdefault(c["sha256"], (r, os.path.join(dev_dir, r, c["file"])))
            lo = layout.setdefault(i, dict(file=c["file"], region=c["region_block_addr"],
                                           write=c["write_addr"], max_blocks=c["block_count"]))
            if (lo["region"], lo["write"]) != (c["region_block_addr"], c["write_addr"]):
                sys.exit(f"reference bundles disagree on component {i} layout")
            lo["max_blocks"] = max(lo["max_blocks"], c["block_count"])

    versions = sorted(
        (p.split("dv-")[1].split("-extract")[0] for p in glob.glob(os.path.join(INST, "dv-*-extract"))),
        key=vkey)
    made = skipped = 0
    for v in versions:
        fwdir = os.path.join(INST, f"dv-{v}-extract", "Firmware")
        recs, src = None, None
        for f in sorted(os.listdir(fwdir)) if os.path.isdir(fwdir) else []:
            if not f.startswith("data-r"):
                continue
            rs = list(datar.records(open(os.path.join(fwdir, f), "rb").read()))
            if rs and all((r["vid"], r["pid"]) == (vid, pid) for r in rs):
                recs, src = rs, f
                break
        if recs is None:
            continue
        out = os.path.join(dev_dir, f"dv-{v}")
        comps, matches = [], []
        for r in sorted(recs, key=lambda r: r["index"]):
            lo = layout.get(r["index"])
            if lo is None:
                sys.exit(f"{v}: component {r['index']} has no verified layout")
            region = r["write_addr"] // BLOCK
            needed = math.ceil(r["size"] / BLOCK)
            if r["write_addr"] != lo["write"] or region != lo["region"] or needed > lo["max_blocks"] \
                    or not r["size_ok"]:
                sys.exit(f"{v}: component {r['index']} layout {r['write_addr']:#x}/{needed} blocks "
                         f"differs from the verified layout — refusing")
            # Erase the whole captured region, not just the blocks this image fills: the
            # official 16.5 updater erases 145 blocks for the 9,399,532 B (144-block)
            # component A (flash165_USBPcap3.pcap, 0x38 expected=145).
            blocks = lo["max_blocks"]
            h = sha(r["data"])
            matches.append(verified.get(h, (None,))[0])
            comps.append((r["data"], h, {
                "file": lo["file"], "sha256": h, "bytes": r["size"],
                "region_block_addr": region, "block_count": blocks, "write_addr": r["write_addr"],
                "source_resource": f"Firmware/{src} record {r['index']} (DV {v} installer)",
            }))
        all_verified = all(matches)
        if all_verified:
            notes = (f"Extracted from the official DV {v} installer (Firmware/{src}, zlib container). "
                     + " ".join(f"{c[2]['file']} = official-tool capture image {m}." for c, m in zip(comps, matches))
                     + f" Erase region = full captured region ({', '.join(str(c[2]['block_count']) for c in comps)} blocks).")
        else:
            notes = (f"Extracted from the official DV {v} installer (Firmware/{src}, zlib container); "
                     f"geometry from its own record headers, matching the verified layout. "
                     f"NOT yet flashed on hardware.")
        if os.path.exists(os.path.join(out, "manifest.json")):
            existing = json.load(open(os.path.join(out, "manifest.json"), encoding="utf-8"))
            same = [c["sha256"] for c in existing["components"]] == [c[1] for c in comps]
            print(f"{v:>8}: exists ({'identical images' if same else 'DIFFERENT images — left untouched'})")
            skipped += 1
            continue
        print(f"{v:>8}: {src} -> dv-{v}  " + ", ".join(
            f"{c[2]['file']} {c[2]['bytes']} B ({m or 'new'})" for c, m in zip(comps, matches)))
        if a.dry_run:
            continue
        os.makedirs(out)
        for data, h, c in comps:
            dst = os.path.join(out, c["file"])
            ref = verified.get(h)
            try:
                if ref:
                    os.link(ref[1], dst)
                else:
                    raise OSError
            except OSError:
                with open(dst, "wb") as fh:
                    fh.write(data)
            if sha(open(dst, "rb").read()) != h:
                sys.exit(f"{v}: written {c['file']} failed sha256 check")
        manifest = {
            "image": {
                "name": f"UltraStudio 4K Mini - Desktop Video {v}",
                "source": f"official DV {v} installer, Firmware/{src} (unpacked with tools/datar.py)",
                "protocol": "docs/protocol.md",
                "notes": notes,
            },
            "components": [c for _, _, c in comps],
        }
        with open(os.path.join(out, "manifest.json"), "w", encoding="utf-8") as fh:
            json.dump(manifest, fh, indent=2)
            fh.write("\n")
        made += 1
    print(f"\nbundles created: {made}, existing left untouched: {skipped}"
          + (" (dry run)" if a.dry_run else ""))


if __name__ == "__main__":
    main()
