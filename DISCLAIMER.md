# Disclaimer and flashing warnings

> [!CAUTION]
> **Read this before flashing anything with UBMDF.** Flashing can permanently brick your device, and UBMDF comes with no warranty.

## Not an official tool
UBMDF is an independent, community-made tool. It is **not affiliated with, endorsed by, or supported by Blackmagic Design Pty Ltd**. "Blackmagic Design", "UltraStudio", "DeckLink" and "Desktop Video" are trademarks of Blackmagic Design. They are used here only to say which hardware and software the tool works with.

The service-port protocol was worked out by observing the official `DesktopVideoUpdateTool`. It is not based on any Blackmagic documentation.

## Flashing can brick your device
Writing firmware erases and rewrites the device's flash memory. If anything goes wrong, the unit can stop working, **possibly permanently**. Things that can go wrong:

- an interrupted write: power loss, cable pulled, computer sleeping or crashing;
- a wrong or damaged image;
- an untested device or firmware combination.

Using UBMDF may also void your warranty or your right to support from Blackmagic Design.

## No warranty
UBMDF is provided **"AS IS", without warranty of any kind**, express or implied, under the MIT license. The authors are not liable for any damage to hardware, lost data, lost work time or any other loss from using it. **You flash at your own risk.**

## What has and has not been verified
- **Tested on one model only.** Flashing has been tested on a single **UltraStudio 4K Mini**. Other Blackmagic models are not supported by the flash command.
- **No read-back.** The service port has no known way to read the flash back. A "successful" flash means every transfer was accepted by the device, **not** that the written contents were verified.
- **11.4.1 bundle never flashed.** The `dv-11.4.1` bundle was derived from installer data and has **not** been flashed on hardware. UBMDF marks it with a caution.
- **12.9 known-bad.** Desktop Video **12.9** is flagged KNOWN-BAD because the official 12.9 update bricked multiple 4K Mini units. Prefer a later version.
- **Image provenance.** Images are byte-identical to the ones in Blackmagic's official installers, checked by sha256. The flash sequence matches a USB trace of the official updater.

## Before you flash
1. **Power and connection:** use a reliable power source, and plug the USB service-port cable straight into the computer, not through a hub.
2. **Keep the computer awake:** turn off sleep and hibernation, and don't run other heavy USB or system tasks during the write.
3. **Close Blackmagic software:** close Desktop Video Setup, Media Express and other Blackmagic programs.
4. **Plan your recovery:** keep the official Desktop Video installer for your preferred version at hand. Blackmagic's own updater (`DesktopVideoUpdateTool`) and Blackmagic support are the recovery path.
5. **Never interrupt a flash:** don't unplug or power off the device while it runs. UBMDF ignores Esc and keeps the window open until the flash finishes.
6. **After the flash:** unplug the USB cable, then power-cycle the unit.

## Firmware ownership
The firmware images are Blackmagic Design property. Archive and flash only images obtained through Blackmagic's official installers, for devices you own.
