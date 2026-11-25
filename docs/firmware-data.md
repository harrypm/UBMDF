# Firmware data: index, harvest, LFS, installer formats

## Firmware index
`firmware/index.json` tracks two kinds of entries:
- **flashable:** bundles of manifest + component images, byte-identical to official-tool captures.
- **encrypted-inventory** (a historical name): SHA-256 inventories of the installers' `data-rNN.bin` containers. Despite the name, these are zlib-compressed, not encrypted ([see below](#installer-firmware-format-data-rnnbin-dv-115)).

Per-version image mapping: [firmware-map.md](firmware-map.md).

## Where UBMDF keeps its data
UBMDF looks for a `firmware/` folder in this order:
1. the `UBMDF_FIRMWARE` environment variable;
2. `./firmware` in the current directory;
3. walking up from the executable (for an AppImage, from the `.AppImage` file);
4. a per-user folder: `%LOCALAPPDATA%\UBMDF` on Windows, `~/Library/Application Support/UBMDF` on macOS, `$XDG_DATA_HOME/UBMDF` or `~/.local/share/UBMDF` on Linux.

`logs/`, `dumps/` and `ubmdf-gui.conf` sit beside that folder. Firmware images are Blackmagic Design property; release builds embed only the flashable bundles (see [building.md](building.md#bundled-firmware)).

## Raw firmware data via Git LFS
The repo tracks the whole extracted firmware tree. Binaries are in Git LFS, which stores each unique file once: 1.65 GB for 10.3 GB of extracted data. Installer zips (27 GB) are not tracked; they can be re-downloaded from Blackmagic's catalog.

```
git clone https://github.com/harrypm/UBMDF.git && cd UBMDF   # pointers only, fast
git lfs pull --include="firmware/*/dv-*/**" --exclude="firmware/installers/**"   # flashable bundles (~30 MB)
git lfs pull --include="firmware/installers/**"                                  # all raw extracted data (~1.65 GB)
ubmdf index rebuild
```

Linux and macOS need no msiexec or Windows step. These pure-Python 3 tools cover it:
- `tools/datar.py` unpacks the containers;
- `tools/bitstream_info.py` and `tools/bitstream_packets.py` identify FPGA images and dump their configuration packets;
- `tools/make_datar_bundles.py` rebuilds the flashable bundles.

## Harvest (batch firmware collection, Windows)
```
pwsh -File tools/harvest.ps1                      # all versions
pwsh -File tools/harvest.ps1 -From 16.5 -To 12.8  # version range
pwsh -File tools/harvest.ps1 -NoZips              # discard zips after inventory
```
The script self-elevates once (one UAC prompt). For each version it downloads, extracts, inventories and indexes, then writes `harvest-report.json`. Local installer zips are imported when available.

## Installer layouts by Desktop Video version
- **≤ 10.9.7:** firmware ships inside the drivers (legacy layout).
- **10.9.10 – 11.4.1:** every device image is a PE `RCDATA` resource inside `CheckVersionPCI.exe`, not encrypted. Each version holds 46–51 Xilinx bitstreams, 2–4 ELF images and 14 other binaries. Extract them with:
  ```
  pwsh -File tools/extract-updater-firmware.ps1   # 7-Zip, no admin/msiexec needed
  ubmdf index rebuild
  ```
  Output goes to `firmware/installers/dv-<v>-extract/Firmware/CheckVersionPCI-rcdata/`, as `rc-<id>.bin` files plus `resources.csv`.
- **11.5 onward:** zlib containers in `Firmware\data-rNN.bin` ([below](#installer-firmware-format-data-rnnbin-dv-115)).

## Installer firmware format (`data-rNN.bin`, DV 11.5+)
These files are **not encrypted**. Each one has:
- a 32-byte prefix;
- one or more records, each a big-endian header (magic `BD BD 00 01`; 72 bytes for USB service-port devices, 48 for PCIe cards) followed by a **zlib** stream.

The header gives the uncompressed size, the flash write address and the target device ID (USB `1edb:be13` = UltraStudio 4K Mini service port). All 2,151 `data-*` files in DV 11.5 – 16.5 unpack cleanly, covering 61 target devices. The macOS Desktop Video 16.5 updater carries the same `data-r90.bin`.

```
python -I tools/datar.py <data-rNN.bin>            # list records (size, write addr, device, type)
python -I tools/make_datar_bundles.py [--dry-run]  # 4K Mini bundle for every DV 11.5+ version
```

`make_datar_bundles.py` takes geometry from the record headers: erase = image size in 64 KiB blocks, as the official updater sends. It refuses anything that differs from the captured layout, and hardlinks identical images (43 bundles ≈ 33 MB).
