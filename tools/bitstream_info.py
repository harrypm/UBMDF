"""Xilinx bitstream inspector for UBDMF firmware images (read-only).

Finds the sync word (AA 99 55 66) and walks type-1/type-2 configuration packets
to report the IDCODE register write (identifies the exact FPGA part), the
offset of the sync word (= size of any vendor header in front of it), and
whether the bitstream is compressed/encrypted-looking.

Usage:
  python -I tools/bitstream_info.py <file-or-dir> [...]   # prints one line per file
  python -I tools/bitstream_info.py --csv out.csv <dirs...>
"""
import csv
import os
import struct
import sys

SYNC = b"\xaa\x99\x55\x66"

# A few common Xilinx IDCODEs (revision nibble masked off) for readability.
PARTS = {
    0x03651093: "Kintex-7 XC7K325T", 0x03656093: "Kintex-7 XC7K410T",
    0x03647093: "Kintex-7 XC7K70T", 0x0364C093: "Kintex-7 XC7K160T",
    0x03631093: "Artix-7 XC7A100T", 0x03636093: "Artix-7 XC7A200T",
    0x0362D093: "Artix-7 XC7A35T", 0x0362C093: "Artix-7 XC7A50T",
    0x03727093: "Zynq XC7Z020", 0x03722093: "Zynq XC7Z010",
    0x0372C093: "Zynq XC7Z030", 0x03731093: "Zynq XC7Z045",
    0x04004093: "Spartan-6 XC6SLX4", 0x04008093: "Spartan-6 XC6SLX9",
    0x04011093: "Spartan-6 XC6SLX16", 0x0401D093: "Spartan-6 XC6SLX25",
    0x0402E093: "Spartan-6 XC6SLX45", 0x04034093: "Spartan-6 XC6SLX75",
    0x04042093: "Spartan-6 XC6SLX100", 0x0403D093: "Spartan-6 XC6SLX150",
    0x04001093: "Spartan-6 XC6SLX25T", 0x04024093: "Spartan-6 XC6SLX45T",
    0x0424A093: "Virtex-6 XC6VLX75T", 0x04250093: "Virtex-6 XC6VLX130T",
    0x04252093: "Virtex-6 XC6VLX195T", 0x04256093: "Virtex-6 XC6VLX240T",
    0x03823093: "Kintex UltraScale XCKU040", 0x0484A093: "Kintex UltraScale+ XCKU5P",
    0x0362F093: "Artix-7 XC7A15T", 0x037C4093: "Spartan-7 XC7S25",
    0x03622093: "Spartan-7 XC7S6/15", 0x037C8093: "Spartan-7 XC7S50",
}


def parse(data: bytes):
    off = data.find(SYNC, 0, min(len(data), 1 << 16))
    if off < 0:
        return None
    pos = off + 4
    idcode = None
    compressed = False
    n = 0
    while pos + 4 <= len(data) and n < 400:
        n += 1
        (w,) = struct.unpack_from(">I", data, pos)
        pos += 4
        hdr = w >> 29
        if hdr == 1:  # type 1
            op = (w >> 27) & 3
            reg = (w >> 13) & 0x3FFF
            cnt = w & 0x7FF
            if op == 2 and reg == 0x0C and cnt >= 1 and pos + 4 <= len(data):  # IDCODE write
                (idcode,) = struct.unpack_from(">I", data, pos)
            if op == 2 and reg == 0x02 and cnt == 0:  # FDRI with type-2 following
                pass
            if op == 2 and reg == 0x14:  # MFWR / compression hints (7-series)
                compressed = True
            pos += 4 * cnt
        elif hdr == 2:  # type 2 (large data, FDRI) — stop: config data follows
            break
        else:
            if w in (0xFFFFFFFF, 0x20000000, 0x00000000):
                continue
            break
    return off, idcode, compressed


def info(path):
    data = open(path, "rb").read()
    r = parse(data)
    if r is None:
        return dict(file=path, bytes=len(data), sync_off="", idcode="", part="", head=data[:16].hex())
    off, idcode, comp = r
    part = PARTS.get((idcode or 0) & 0x0FFFFFFF, "") if idcode is not None else ""
    return dict(
        file=path, bytes=len(data), sync_off=off,
        idcode=f"{idcode:08x}" if idcode is not None else "",
        part=part, head=data[:16].hex(),
    )


def files(args):
    for a in args:
        if os.path.isdir(a):
            for f in sorted(os.listdir(a)):
                if f.endswith(".bin"):
                    yield os.path.join(a, f)
        else:
            yield a


def main():
    args = sys.argv[1:]
    out = None
    if args[:1] == ["--csv"]:
        out, args = args[1], args[2:]
    rows = [info(f) for f in files(args)]
    if out:
        with open(out, "w", newline="", encoding="utf-8") as fh:
            w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
            w.writeheader()
            w.writerows(rows)
    for r in rows:
        print(f"{os.path.basename(r['file']):>18} {r['bytes']:>9} sync@{str(r['sync_off']):>6} id={r['idcode']:>8} {r['part']}")


if __name__ == "__main__":
    main()
