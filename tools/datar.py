"""Unpack Blackmagic Desktop Video `Firmware/data-rNN.bin` containers (DV 11.5+).

Despite the long-standing assumption in this project, these files are NOT
encrypted: each is a 32-byte prefix (hash/signature) followed by one or more
records, each a big-endian header (magic BD BD 00 01; 72 bytes for USB
service-port devices, 48 bytes for PCIe cards) plus a zlib stream. Verified 2026-11-24: 12.8.1 and 16.0 data-r90.bin decompress to the
exact sha256 of the hardware-captured UltraStudio 4K Mini component_A/B.

Record header fields used here (offsets within the 72 bytes, big-endian):
  0  u16 magic 0xBDBD, 2 u16 version (1)
  4  u32 unknown, 8 u32 bytes to next record (= zlib length + 72)
 12  u16 header length (0x48), 14 u16 flags (0x0100)
 16  u32 uncompressed size, 20 u32 checksum (NOT crc32/adler32/mpeg2/sum32 of
     the data — unknown, not needed: zlib's own Adler-32 trailer is verified
     on decompression and the size field is checked)
 24  u16 tag 1, 26 u16 len 0x18, 28 u32 write address, 32 u16 component
     index, 34 u16 ?, 36 u16 USB VID, 38 u16 USB PID (1edb:be13 = service port)

Usage (read-only listing):
  python -I tools/datar.py <data-rNN.bin> [...]
"""
import struct
import sys
import zlib

SYNC = b"\xaa\x99\x55\x66"


def records(b: bytes):
    """Yield dicts for every record: header fields + decompressed data.

    Layout: 32-byte prefix, then records of [header (length at +12: 0x48 for
    USB service-port devices, 0x30 for PCIe cards)] + zlib stream. The
    `next` field (+8) = header length + compressed length.
    """
    pos = 32
    while pos + 16 <= len(b):
        if b[pos:pos + 2] != b"\xbd\xbd":
            raise ValueError(f"no record magic at {pos}")
        (nxt,) = struct.unpack_from(">I", b, pos + 8)
        hlen, flags = struct.unpack_from(">HH", b, pos + 12)
        h = b[pos:pos + hlen]
        d = zlib.decompressobj()
        data = d.decompress(b[pos + hlen:pos + nxt])
        if not d.eof:
            raise ValueError(f"truncated zlib stream in record at {pos}")
        size, checksum = struct.unpack_from(">II", h, 16)
        (write_addr,) = struct.unpack_from(">I", h, 28)
        index, _x, vid, pid = struct.unpack_from(">HHHH", h, 32)
        yield dict(
            offset=pos, hlen=hlen, flags=flags, size=size, checksum=checksum,
            write_addr=write_addr, index=index, vid=vid, pid=pid, data=data,
            size_ok=len(data) == size,
        )
        pos += nxt


def idcode(data: bytes):
    off = data.find(SYNC, 0, 1 << 16)
    if off < 0:
        return None
    p = off + 4
    for _ in range(400):
        if p + 8 > len(data):
            return None
        (w,) = struct.unpack_from(">I", data, p)
        p += 4
        if w >> 29 == 1:
            op, reg, cnt = (w >> 27) & 3, (w >> 13) & 0x3FFF, w & 0x7FF
            if op == 2 and reg == 0x0C:
                return struct.unpack_from(">I", data, p)[0]
            p += 4 * cnt
        elif w >> 29 == 2:
            return None
    return None


def kind(data: bytes):
    if data[:4] == b"\x7fELF":
        return "elf"
    i = idcode(data)
    if i is not None:
        return f"bitstream:{i:08x}"
    if SYNC in data[:1 << 16]:
        return "bitstream"
    return "binary"


if __name__ == "__main__":
    for path in sys.argv[1:]:
        b = open(path, "rb").read()
        print(path)
        for r in records(b):
            print(f"  @{r['offset']:>9} size {r['size']:>9} write {r['write_addr']:#010x} idx {r['index']} "
                  f"usb {r['vid']:04x}:{r['pid']:04x} size {'ok' if r['size_ok'] else 'BAD'} {kind(r['data'])}")
