# CLI reference

Running the exe with **no arguments** opens the GUI in GUI builds, so double-clicking works. CLI-only builds print help.

```
ubmdf                                          # no args → GUI (GUI builds)
ubmdf list                                     # find service-port devices
ubmdf info                                     # identity/state (read-only; session closed with 0x12)
ubmdf dump [--out file.json]                   # identity/state to JSON
ubmdf flash <manifest.json> [--yes] [--dry-run]
ubmdf verify <manifest.json>                   # bundle hashes + device state
ubmdf reset                                    # send 0x12 to clear a lingering session
ubmdf replay-check <cap.pcap> <manifest.json> [--device BUS:ADDR]
ubmdf probe [--session] [--bulk-in] [--from 0x00] [--to 0xff] [--length 1-64,128] [--value N] [--index N]
ubmdf index list | show <id> | rebuild
ubmdf update check | fetch <version> | import <zip>
ubmdf gui [--tab firmware|devices|update|log] [--select <dv-version>] [--check-catalog] [--flash <manifest> --yes]
```

## Flash log
Every flash logs each wire step with the device's answer:
- `0x30` state bytes and the `0x35` context;
- every `0x38` erase poll;
- the bulk byte count with the image sha256;
- the ZLP, the write-done wait and the commit.

A finished flash means every transfer was accepted. The service port has no known read-back, so the written contents are **not** verified.

Log file: `logs/ubmdf.log` beside the firmware folder, or `UBMDF_LOG`.

## `replay-check`
Builds the exact op list `flash` would send for a manifest and aligns it with the Blackmagic service-port traffic in a capture of the official updater. Nothing is sent to any device.

- **Input:** classic pcap from USBPcap (Windows, link type 249), Linux usbmon (link type 220), or `tools/usbtrace` on macOS (see [tracing.md](tracing.md)). For pcapng, convert first with `editcap -F pcap`.
- **Output:** each op is marked `ok`, `+OFF` (only the official tool sent it) or `-UBMDF` (only UBMDF sends it). Bulk streams are compared byte-for-byte with the component images, and the device-reported `0x38 expected` erase size is shown.
- **Capturing a complete official flash on Windows:** start USBPcap on the unit's root hub **before** launching the updater, and stop only after the unit has rebooted.

## `probe`
Looks for a read-back command. It sends only vendor control **IN** requests (no data from the host):
- bRequest `--from`..`--to`, to both the device (0xC0) and interface (0xC1) recipients;
- every `--length`, because the device STALLs any request whose wLength isn't exactly right;
- `--value` and `--index` as wValue and wIndex.

Options and safeguards:
- `--session` opens a `0x31` session first and closes it with `0x12`.
- `--bulk-in` adds one 512-byte read on EP 0x82.
- Each request is logged before it is sent, so a hang names its request.
- The sweep stops on disconnect or after 3 timeouts in a row.
- The 0x30 state is compared before and after.

Results go to `ubmdf-probe-<epoch>.json`; the findings so far are in [protocol.md](protocol.md#read-back-probe-ubmdf-probe-2026-11-24-test-unit-on-macos).

## Paths and environment
UBMDF works from any working directory (double-click, shortcut, script):
- **Firmware root:** `UBMDF_FIRMWARE` → `./firmware` → walking up from the executable → the per-user folder. See [firmware-data.md](firmware-data.md#where-ubmdf-keeps-its-data).
- **Log file:** `UBMDF_LOG`, otherwise `logs/ubmdf.log` beside the firmware root.
- **GUI settings:** `ubmdf-gui.conf` beside the firmware root.
