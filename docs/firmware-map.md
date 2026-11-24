# Firmware map — plaintext updater images (DV 10.9.10 – 11.4.1)

Desktop Video 10.9.10 – 11.4.1 ship every device image as an unnamed PE `RCDATA`
resource of `CheckVersionPCI.exe`, and the images are plaintext.
`tools/extract-updater-firmware.ps1` extracts them to
`firmware/installers/dv-<v>-extract/Firmware/CheckVersionPCI-rcdata/rc-<id>.bin`.
This page records which resource belongs to which device, as far as hard
evidence allows.

## UltraStudio 4K Mini

| DV versions | FPGA image (component A) | ARM ELF (component B) | Status |
|---|---|---|---|
| ≤ 10.11.2 | — | — | no 4K Mini resources: support first appears in 10.11.4 |
| 10.11.4 – 11.2 | `rc-200`, 2,161,946 B, sha256 `199ec0cf94a7…`, IDCODE `03824093` | `rc-202`, 1,375,860 B, sha256 `aacc0c454375…` | **probable**: same resource slot and FPGA IDCODE as the confirmed pair, but no byte match to a known image and the sizes differ a lot, so the flash layout is unproven. **No bundle made.** |
| 11.3 – 11.4.1 | `rc-200`, 9,491,428 B, sha256 `1f9b20834c93…` | `rc-202`, 1,928,120 B, sha256 `1c8d4f7fad15…` | **high confidence**, bundled as `firmware/ultrastudio4kmini/dv-11.4.1` (see below) |

The images are byte-identical across all versions within a group. Evidence for the 11.3 – 11.4.1 pair:

- **`rc-202`**: byte-identical (sha256) to `component_B.bin` of the official-tool 12.8.1 and 16.0 capture bundles.
- **`rc-200`**:
  - Same size as the verified 12.8.1 `component_A.bin` (9,491,428 B).
  - Same Xilinx IDCODE `03824093`.
  - Same 112-byte vendor header before the `AA995566` sync word.
  - Differs in only **153 bytes**:
    - Header bytes 4–7 look like a build timestamp: `11 15 12 42` (12.8.1) against `06 12 14 05` (11.4.1).
    - The remainder are scattered words, consistent with a rebuilt bitstream's stamp/CRC.
- **Geometry**: copied from the 12.8.1 capture (region 0 / 145 blocks, region 192 / 30 blocks, write 0x0 and 0xC00000). The images are the same sizes.
- **Not yet flashed on hardware.** The manifest `notes` say so; the GUI shows the note in amber and repeats it in the Flash confirmation box, and the CLI prints it.

Vendor header (first 112 bytes, both versions):
`01 01 01 13 | <4-byte build stamp> | 00 | FF × 87 | 00 00 00 BB 11 22 00 44 | FF × 8`
The `000000BB 11220044` part is the standard Xilinx bus-width detection pattern.

## UltraStudio 4K Mini — every version (all builds)

From 11.5 onward the 4K Mini images are in `Firmware/data-r90.bin`, a zlib container whose records target USB `1edb:be13` (see README, *Installer firmware format*). There are four distinct builds across all of Desktop Video:

| DV versions | component_A sha256 | component_B sha256 | Bundles | Status |
|---|---|---|---|---|
| 10.11.4 – 11.2 | `199ec0cf…` (2.16 MB) | `aacc0c45…` (1.38 MB) | none | probable; layout unproven |
| 11.3 – 11.4.1 | `1f9b2083…` | `1c8d4f7f…` | dv-11.4.1 | derived; not yet flashed |
| 11.5 – 12.8.1 (20 versions) | `6d2d0500…` = 12.8.1 official-tool capture | `1c8d4f7f…` | dv-11.5 … dv-12.8.1 | **byte-identical to the official-tool capture**; erase 145 blocks |
| 12.9 – 16.5 (22 versions) | `2cd5104b…` = 16.0 official-tool capture | `1c8d4f7f…` | dv-12.9 … dv-16.5 | **byte-identical to the official-tool capture**; erase 144 blocks |

`component_B` (a MicroBlaze ELF that runs inside the FPGA and drives the front-panel display) has not changed since 11.3.

**What "capture" proves:** the images are byte-identical to what the official updater sent over USB, and the device accepted every transfer. It does not prove the unit boots afterwards: as of 2026-11-24 the test unit has stayed dark (blank front panel, no PCIe device) after the official 16.5 updater and after UBMDF flashes of 12.8.1 and 16.5. The service port has no known read-back command, so the flash contents have never been read back.

**Erase size:** the image size rounded up to 64 KiB blocks: 145 for the 12.8.1 image, **144** for the 12.9 – 16.5 image. The official macOS 16.5 updater sends `0x37` (0, 144), traced on 2026-11-24 (`ubmdf` repo `tools/usbtrace`). The 145 that the Windows `flash165_USBPcap3.pcap` reports in its `0x38` polls does not match that trace; the capture starts mid-erase, so whatever it was erasing is unknown. a143c67 briefly sent 145; that was reverted.

DV 12.9 is flagged KNOWN-BAD from forum reports of bricked units, but its 4K Mini images are byte-identical to 14.0 – 16.5. So the failures were not caused by the image files; they could have come from the 12.9 updater tool or the update process.

## FPGA families in DV 11.4.1 (69 resources)

Grouped by `tools/bitstream_info.py`. The IDCODE comes from the type-1 packet write to the IDCODE register after the sync word.

| IDCODE / type | Part (if known) | Count | Resource ids |
|---|---|---|---|
| bitstream, IDCODE not parsed | older 16-bit-packet families (parser is 32-bit only) | 21 | 142, 146, 149, 150, 153–156, 158, 160, 161, 164–169, 171, 172, 180, 182 |
| `binary` (no sync word) | headers like `0004d7e6 00004aa5 … 0100ff7f` | 14 | 133–136, 138–140, 143–145, 147, 148, 151, 162 |
| `03822093` | Kintex UltraScale (unconfirmed) | 9 | 183, 184, 186–188, 195, 196, 198, 199 |
| `03631093` | Artix-7 XC7A100T | 5 | 189–193 |
| `elf` | ARM firmware | 4 | 179, 194, **202 (4K Mini)**, 203 |
| `02e9a093` | — | 3 | 152, 157, 163 |
| `04244093` | — | 3 | 170, 173, 174 |
| `03647093` | Kintex-7 XC7K70T | 3 | 175, 176, 181 |
| `03651093` | Kintex-7 XC7K325T | 3 | 177, 178, 185 |
| `03824093` | Kintex UltraScale (unconfirmed) | 2 | **200 (4K Mini)**, 201 |
| `02a6e093` | — | 1 | 141 |
| `0362c093` | Artix-7 XC7A50T | 1 | 197 |

`rc-201` (9,304,676 B, unchanged from 10.11.4 to 11.4.1) is a second `03824093` image, so some other device uses the same FPGA as the 4K Mini.

## Tools

- `tools/bitstream_info.py <file|dir>` (`--csv out.csv`): reports the sync offset, IDCODE and part.
- `tools/bitstream_packets.py <component_A.bin>`: dumps the configuration packets up to the first frame-data write (quad-SPI setup, WBSTAR/TIMER fallback settings, IDCODE). The 4K Mini images have WBSTAR=0 and TIMER=0, so there is no fallback image.
- `tools/make_rcdata_bundle.py`: builds a flashable bundle from RCDATA ids using a verified bundle's geometry. It refuses if an image needs more blocks than the proven region, if the resources differ between the `--also` versions, or if the bundle already exists.

## Open

- Map the other resources to devices. Leads:
  - device product IDs / names in `CheckVersionPCI.exe` `.rdata` near the resource-loading code;
  - USB captures of the official updater on other units.
- The 10.11.4 – 11.2 4K Mini pair needs a capture (or another proof of its flash layout) before it can be bundled.
