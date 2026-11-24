# BMD USB Service-Port Flash Protocol — UltraStudio 4K Mini (Desktop Video 12.8.1)

Source of truth: USBPcap capture of official `DesktopVideoUpdateTool.exe -a -u` (2026-11-19, `captures/flash_run_USBPcap3.pcap`, 1688 packets, 72.9 s) plus handshake-only capture `list_run_USBPcap3.pcap`. All findings below are byte-verified against that trace.

## Device / transport
- USB VID `0x1EDB` PID `0xBE13`, bcdDevice `0x0101`, class 0x00 — enumerates as "UltraStudio 4K Mini Service Port"
- One vendor interface: EP `0x01` bulk OUT (image data) + EP `0x82` bulk IN (unused during flash)
- Windows access via BMD's `Drivers\Update\DesktopVideoUpdate.inf` (service `DesktopVideoUpdate`) — the MSI does not bind it automatically; bind with `pnputil /add-driver ... /install`
- All commands are vendor control transfers on EP0: `bmRequestType 0xC1` = vendor IN (interface), `0xC0` = vendor IN (device), `0x41` = vendor OUT (interface)

## Command set
| bRequest | Dir | Data | Meaning (verified) |
|---|---|---|---|
| 0x10 | IN 0xC0 | 16 B | device nonce/ID (e.g. `0c 4e 54 15 23 25 4d 53 ba f8 ee 57 4d 1f 3b fe`); also 0xC1 variant, 1 B → `00`. No client response written — no crypto handshake over the wire |
| 0x11 | IN 0xC1 | 8 B | **found by `ubmdf probe` 2026-11-24**, not used by the official tool: `890bb734 890bb734` (same u32 twice) on the test unit, in and out of a session. Not a CRC32/Adler-32/sum of any component image. Meaning unknown |
| 0x20 | IN 0xC1 | 2 B | **probe-found**: `01 01`, in and out of a session. Possibly a version |
| 0x33 | IN 0xC1 | 2 B | **probe-found**: `05 00`, answered **only inside a 0x31 session** |
| 0x30 | IN 0xC1 | 6 B | state check → `00 00 00 00 ff ff` (semantics partly unknown) |
| 0x35 | IN 0xC1 | 16 B | erase-context info: `00000002 00000000 00000100 00010000` (BE u32s) — 2 components/slots, block size 0x10000 = 64 K. **STALLs outside an active erase session** (hardware-verified); the official tool reads it only after the 0x34 erase command and mid/post-write |
| 0x31 | OUT 0x41 | none | begin session |
| 0x34 | OUT 0x41 | none | start erase |
| 0x37 | OUT 0x41 | 8 B | select region: data = `{u32 LE addr in 64K blocks, u32 LE block count}` — A: `00 00 00 00 91 00 00 00` = (0, 145); B: `c0 00 00 00 1e 00 00 00` = (192, 30) → byte address 192×65536 = 0x00C00000 |
| 0x38 | IN 0xC1 | 15 B | progress poll: `{u32 LE region, u32 LE expected_blocks, u32 LE progress, u8,u8,u8}`. `progress` is the absolute block reached, not a count: region 192 × 30 ends at 222 (UBMDF flash log 2026-11-24) — trailing `01 00 01` = busy, `00 00 00` = phase done; polled ~2/s during erase only |
| 0x39 | OUT 0x41 | 4 B | set write address: data = `{u32 BE byte address}` — A: `00 00 00 00`; B: `00 c0 00 00` |
| (bulk) | OUT EP1 | ≤32,768 B | image bytes, sequential chunks; **each component ends with its final short packet followed by an explicit zero-length transfer (ZLP terminator — frames 1359/1645); omitting the ZLP leaves the region write unfinalized and the next 0x39 stalls (hardware-verified)** |
| 0x32 | OUT 0x41 | none | commit |
| 0x12 | OUT 0x41 | none | reset/reboot (tool then says "unplug the USB cable") |

## Full session sequence (as captured)
1. standard GET DESCRIPTORs
2. IN 0xC0/0x10 (16 B nonce) ×2, IN 0xC1/0x10 (1 B)
3. OUT 0x31 (begin)
4. IN 0x30 ×2, IN 0x35 (geometry)
5. OUT 0x34 (erase) → IN 0x35 (erase context — stalls without the active erase session)
6. Component A: OUT 0x37 (0, 145) → poll 0x38 until erase done → OUT 0x39 (0x00000000) → IN 0x35 (write latch) → 290 bulk writes (289×32,768 + 21,476 short) = 9,491,428 B → zero-length terminator
7. IN 0x30 / 0x35 checks
8. Component B: OUT 0x37 (192, 30) → poll 0x38 until erase done → OUT 0x39 (0x00C00000) → IN 0x35 (write latch) → 59 bulk writes (58×32,768 + 27,576 short) = 1,928,120 B → zero-length terminator
9. OUT 0x32 (commit) → IN 0x30 checks → OUT 0x12 (reboot)

Hardware-verified session details (2026-11-19, bmdflash on live device):
- 0x31 flips the state byte: 0x30 reads `00000000ffff` before, `01000000ffff` after
- 0x38 reads idle `{region=0, expected=0, done=0, busy=false}` once a session has begun
- 0x10 nonce stable across sessions: `0c4e541523254d53baf8ee574d1f3bfe`

## Key findings
- **The shipped `Firmware\data-rNN.bin` files are encrypted containers; the tool decrypts in software and the USB wire stream is plaintext.** Evidence: component A contains the Xilinx bitstream sync word `AA 99 55 66` at offset 112 and structured headers, while every large bin is uniform high-entropy with no sync word and no shared 16-byte subsequences with the stream.
- Consequence: a byte-exact replay of the captured plaintext stream is a valid flash; version archiving is capture-based (extract components from an official-tool capture per version). No need to replicate BMD's container decryption.
- Component A (~9.5 MB) = FPGA bitstream image (sync at offset 112); component B (~1.9 MB) = second image (likely MCU/boot). Erase is per-region (progress via 0x38); writes are unpollled; verification is internal + commit (0x32).
- No authentication/signature exchange over USB was observed.

## Captured artifacts (`captures/`)
- `flash_run_USBPcap3.pcap` (11,504,100 B) — authoritative protocol trace
- `list_run_USBPcap3.pcap` (1,852 B) — handshake-only trace
- `flash_out_stream.bin` — 11,419,548 B, sha256 `2A25E74FA0C9A1BA77B5461898D2CA6B454A4FA0A658FE094C854588B3630DB1` (full plaintext stream, both components concatenated)
- `component_A.bin` — 9,491,428 B, sha256 `6D2D050003929BD9B52A9A648179D53ED628205B9C672210A27DAD58FC4F19C5` (region 0, 145×64K)
- `component_B.bin` — 1,928,120 B, sha256 `1C8D4F7FAD154BC1406A4684112B7531B2A5A11282165C3CF2A694AE55974B05` (region 192×64K, 30×64K)
- `list_console.txt`, `flash_console.txt` — official tool console output
- `flash_run_USBPcap1.pcap` (181 MB) — NOT BMD: an isochronous audio device on root hub 1 (noise, kept for provenance)

## DV 16.5 updater captures (`firmware/installers/flash165_USBPcap{1,2,3}.pcap`)
Checked 2026-11-24 (LFS objects sha256-verified, parsed record by record):
- `USBPcap1` (30.6 MB): 18 other devices on root hub 1; **no Blackmagic traffic**.
- `USBPcap2` (708 B): two USB hubs only.
- `USBPcap3` (3.8 KB): `1edb:be13` re-enumeration, then 25 × `0x38` polls 0.5 s apart: region 0, `expected=145`, `done` 93 → 138, flags `01 00 01`.
  The capture starts mid-erase and ends before the write. **No 0x39, no bulk data, no 0x32 commit.**

So there is no capture of an official 16.x *write* in these files. The `expected=145` it shows does **not** match what the official macOS 16.5 updater sends: `(0, 144)` (see the trace below).

## Official macOS 16.5 updater, traced (2026-11-24)
`DesktopVideoUpdateTool -u -d <id>` from Desktop Video 16.5 for macOS flashes the 4K Mini over the service port. It was traced with `tools/usbtrace`, an IOUSBLib interposer injected into an ad-hoc re-signed copy of the tool, which writes a USBPcap-format pcap plus a text log. The full sequence:

1. `ClearPipeStallBothEnds` on both bulk pipes (EP 0x01 and 0x82: CLEAR_FEATURE(ENDPOINT_HALT) + host data-toggle reset)
2. IN C1/0x10 (1 B) → `00`; IN C0/0x10 (16 B) **twice** → nonce
3. ~2 s pause, then IN 0x30 ×2 → `00000000ffff`; OUT 0x31; IN 0x30 ×2 → `01000000ffff`
4. OUT 0x34; IN 0x35 → `00000002 00000000 00000100 00010000`
5. A: OUT 0x37 `(0, 144)` → 380 × 0x38 polls ~0.1 s apart (≈38.8 s; `done` counts 0 → 144) → OUT 0x39 `00000000` → IN 0x35 → 287 bulk OUT (286 × 32,768 + 22,060) = 9,399,532 B, byte-identical to `dv-16.5/component_A.bin` → ZLP
6. IN 0x30 → `010101000000` (**write busy**); ~105 ms later IN 0x30 → `010000000000`; IN 0x30 again; IN 0x35
7. B: OUT 0x37 `(192, 30)` → 81 polls (`done` ends at 222 = 192 + 30) → OUT 0x39 `00c00000` → IN 0x35 → 59 bulk OUT = 1,928,120 B (byte-identical) → ZLP
8. IN 0x30 → `010101000000`; ~104 ms later IN 0x30 → `010000000000`; IN 0x30; OUT 0x32 (commit); IN 0x30 → `00000000ffff`; OUT 0x12
9. ~5 s later IN 0x10 fails (device re-enumerating), so the tool finishes with "All updates complete"

UBMDF's `planned_ops` follows this exactly since this trace: pipe clears, ID order, 144-block erase, and the `0x30` busy-wait after each ZLP. `replay-check` against the trace leaves only step 9's post-reboot read.

Outcome on the test unit: same as UBMDF's flashes. Backlight on, black screen; Thunderbolt link training times out after a power-cycle (10:10:10). The flash procedure is not what keeps Thunderbolt down.

## What a "complete" flash proves
The device acknowledges every control and bulk transfer, but no read-back command is known (EP 0x82 never carries data in the captures). A completed UBMDF flash therefore means *every transfer was accepted*, not *the flash holds the image*. The flash log records each step and every device response (0x30 state, 0x35 context, each 0x38 poll, bulk byte counts with the image sha256, ZLP, commit) so a run can be compared with a capture.

## Read-back probe (`ubmdf probe`, 2026-11-24, test unit on macOS)
- **The device STALLs any request whose wLength is not exactly the expected size** (0x10 answers only at 16 B to the device or 1 B to the interface, 0x30 only at 6 B). A sweep at one length finds nothing.
- Swept with read-only vendor IN only: bRequest 0x00–0xff × recipients 0xC0/0xC1 × wLength 1–64, 128, 256, 512, 1024, 2048, 4096, wValue = wIndex = 0, both outside and inside a 0x31 session. That is 35,840 requests per pass, about 12 s. Answers: 0x10, 0x11, 0x20, 0x30, 0x38, plus 0x33 inside a session. Nothing else.
- 0x11, 0x20 and 0x33 STALL for any non-zero wValue or wIndex (tried 1, 2, 0x10, 0x100, 0x8000, 0xffff × index 0/1). They are fixed queries, not address-based reads.
- Bulk IN EP 0x82 (512 B, 2 s): times out, outside and inside a session.
- 0x30 state unchanged by the sweeps (`00000000ffff` outside, `01000000ffff` in a session). `info` afterwards is identical to before.
- **Conclusion: no flash read-back is reachable with wValue = wIndex = 0.** A read that needs an address in wValue/wIndex would only show up in a (very large) value sweep, or in a capture of a Blackmagic tool that reads.
- Raw results: the JSON written by each `probe` run.

## Open questions
- Exact semantics of the 0x30 response fields
- 0x35 field #3 (0x100 = 256) — possibly sectors per block
- Whether EP 0x82 (bulk IN) is ever used (zero reads observed)
- Which encrypted `data-rNN.bin` container maps to this unit (irrelevant for replay)

## Analysis notes (tshark on USBPcap captures)
- Control-transfer data payloads: use field `usb.control.Response` (not `usb.capdata`)
- Filter USB devices with `usb.idVendor` (not `usb.vendor_id`); device address from `usb.device_address`
- `tshark -c N -Y` reads only N packets before filtering — do not combine
- USBPcap logs submit and complete as separate frames; bulk data_len>0 marks the data-carrying frames
