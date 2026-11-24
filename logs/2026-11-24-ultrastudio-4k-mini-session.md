# 2026-11-24 — UltraStudio 4K Mini not working with local apps over TB3

## User input
> Interact with my ultrastudio 4k mini figure out why its not working with local apps via its TB3 connection

## Environment (verified)
- Mac mini 2018 (Macmini8,1, Intel, T2), macOS 15.8.2 (24H232), uptime 4d
- Desktop Video 16.5 installed: DExt `com.blackmagic-design.BlackmagicIO.DExt` 16.5 [activated enabled],
  Network DExt, CMIO camera extension, DeckLinkAPI.framework 16.5, DesktopVideoHelper running.
- Desktop Video Updater (UIElement) running since 2026-11-20 06:01, ~47 min after boot (= device arrival time).

## Commands run and results
1. `system_profiler SPThunderboltDataType` → UltraStudio 4K Mini present on TB Bus 1 (receptacle 4),
   Device ID 0xA14F, FW 68.3, LC FW 1.45.0, Mode Thunderbolt 3, Status: Device connected.
2. `systemextensionsctl list` → all 3 Blackmagic extensions activated/enabled.
3. `system_profiler SPPCIDataType` → EMPTY (no tunnelled PCIe devices).
4. `ioreg -p IOService -t` → host bridge PEG2@1,2/UPSB@0/DSB1@1 (the PCIe down-adapter for this TB bus)
   has NO child device. No BlackmagicIO / IOUserService instance bound anywhere.
5. ioreg TB adapter properties:
   - Device PCIe Up adapter (Port@8): `Hop Table = ()`
   - Host PCIe Down adapter (bus 1 Port@8, PCI Path ...PEG2@1,2/.../DSB1@1): `Hop Table = ()`
   - => no PCIe path ever programmed across the TB link.
   - Link ports both ends: Current Link Speed 4 / Target 12, Current Link Width 1 (meaning of
     encoding NOT verified, may or may not indicate a degraded link).
6. `ffmpeg -f avfoundation -list_devices true` → only Hagibis + screen; no Blackmagic device.
   ffmpeg build has no decklink support.
7. Custom DeckLink API enumerator (`dlenum.c`, calls CreateDeckLinkIteratorInstance_0004 from
   /Library/Frameworks/DeckLinkAPI.framework) → `Total DeckLink devices visible to API: 0`.
8. `log show` (2h and 4d windows, TB/PCI/UltraStudio predicates) → nothing retained.

## Conclusion (verified)
Thunderbolt control link is up, but the PCIe tunnel was never set up, so no PCIe endpoint exists for the
Blackmagic DExt to bind to. All apps (DeckLink API, AVFoundation/CMIO) therefore see zero devices.
Drivers/install are NOT the fault.

## Next steps (need user physical action; no sudo available)
See chat. After each step re-run: `/Users/harry/prompt-logs/2026-11-24-ultrastudio-4k-mini-tb3/dlenum`
and `system_profiler SPPCIDataType`.

## Session 2 — 2026-11-24 ~08:00 (resumed after reboot)
User: "I restarted and am using a reference cable", then "replugged".
- Pre-replug (uptime 8 min): SPThunderboltDataType listed Bus 1 with NO ports; ioreg showed Bus 1
  IOThunderboltController stuck busy ~400 s (since boot), and remote IOThunderboltPort children on
  ports 1/2 were stuck !registered/!matched, so enumeration of the attached device was hanging.
- Post-replug 08:01:55: Bus 1 ports populated again and the controller is no longer busy.
  Receptacle 4 Link Status 0x1, Receptacle 3 Link Status 0x7 (differs from the idle 0x101 seen on Bus 0).
  Both report "No device connected". SPPCIDataType is empty. dlenum = 0 devices.

## Session 2 cont. — UBMDF (user's flasher, github.com/harrypm/UBMDF, cloned to ~/UBMDF-dev @ 02041cc)
User: "I have UBMDF installed to overwrite its firmware ... blackscreen (no power) despite reporting flashed"
- Installed app data: ~/Library/Application Support/UBDMF (old name). Log shows 3 flashes, all "Flash complete":
  08:09:25 DV16.5, 08:16:16 DV12.8.1, 08:22:24 DV16.5. Device re-enumerates on USB 1-2 s after each one (reboot 0x12 accepted).
- Now: USB service port present: 1edb:be13 "UltraStudio 4K Mini", Full Speed 12 Mb/s, needs 100 mA (bus-powered MCU).
- TB: device absent. Note: TB Bus 1 was already stuck at boot (~07:52), BEFORE the first UBMDF flash (08:09).
- Repo prompt-log already records that after the OFFICIAL BMD 16.5 updater the front panel was blank and the unit was not
  recognised by DV, with only the TB controller alive. So the blank panel predates UBMDF.
- Code review src/protocol.rs flash(): no readback/verify step. "Flash complete" = all USB transfers accepted, not
  content verified. The 16.5 bundle sha (2cd5104b...) = the 16.0 official capture per docs/firmware-map.md.

## Session 2 cont. — firmware analysis (user: "There is no power issues this is 100% firmware")
- component_B = ELF 32-bit MicroBlaze (soft CPU inside the FPGA); strings: UDisplay/FrontDisplay/MainDisplay, SpiEeprom,
  "Settings storage fail", "FPGA version incompatible with this SW version". => the front LCD is drawn by B running in the FPGA.
- component_A config packets (tools/bitcfg.py): sync@112, BSPI=0x66c + BSPI_READ (quad SPI), TIMER=0, WBSTAR=0,
  IDCODE 03824093, compressed (MFW). => single image at flash 0x0, no multiboot/golden fallback.
- Flash geometry (0x35) = 256 blocks x 64 KiB = 16 MiB. Written: blocks 0-144 (A), 192-221 (B). Never written by any
  tool: 145-191 and 222-255.
- Official 16.5 captures (repo LFS, sha256 verified; tools/usbpcap_bmd.py):
  hub1 = other devices only; hub2 = hubs only; hub3 = be13, but only 25x 0x38 polls, erase region 0 expected=145
  done 93->138. NO write/commit captured. => no capture of an official 16.x write exists in the repo.
  The official tool erases 145 blocks for 16.5 component A; the UBMDF dv-16.5 manifest says block_count 144.
- `ubdmf info` (installed 1.0.0, /Applications/UBDMF.app) 08:35: nonce 0c4e5415..., state after 0x31 = 01000000ffff
  (same as the 2026-11-19 reference), progress idle. The service MCU responds exactly as in the reference capture.

## Session 2 cont. — UBMDF items 1-5 (user: "1-5 go ahead")
Work in ~/UBMDF-dev (clone of harrypm/UBMDF @ 02041cc, current with origin/master). Full detail is in the repo's
logs/prompt-log.md "Execution log (24 Nov 2026, macOS ...)". Summary:
- 145-block erase for 16.x, step-by-step flash log, docs corrected, `replay-check`, `probe`.
- probe/ subfolder here: probe-*.json + logs. Found 0x11/0x20/0x33 fixed queries; no read-back; exact-wLength rule.
- Downloaded data (LFS, sha-verified) and test trees: ~/UBMDF-dev-captures (captures, blobs, r90, build-tree, sandbox,
  target-bundled). Can be deleted when no longer needed.
- Not committed, not flashed.
