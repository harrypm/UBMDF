# Tracing the official updater on macOS (`tools/usbtrace`)

`tools/usbtrace/usbtrace.c` is a small IOUSBLib interposer. It logs every control and bulk transfer a process makes in two forms:
- a USBPcap-format pcap, readable by `ubmdf replay-check` and Wireshark;
- a text log.

No root is needed. The target must run without the hardened runtime, though, so trace an ad-hoc re-signed **copy** of Blackmagic's updater:

```
tools/usbtrace/build.sh
cp -R "/Library/Application Support/Blackmagic Design/Blackmagic DeckLink/Desktop Video Updater.app" /tmp/dvu/
codesign --force --deep --sign - "/tmp/dvu/Desktop Video Updater.app"   # whole bundle, or macOS reports it as damaged
T="/tmp/dvu/Desktop Video Updater.app/Contents/MacOS/DesktopVideoUpdateTool"
"$T" -l                                                     # macOS may ask once in Privacy & Security
USBTRACE_PCAP=official.pcap USBTRACE_LOG=official.txt \
  DYLD_INSERT_LIBRARIES=$PWD/tools/usbtrace/libusbtrace.dylib "$T" -u -d <device id>
ubmdf replay-check official.pcap <manifest.json>
```

How it works: it hooks `IOCreatePlugInInterfaceForService`, then the plug-in's `QueryInterface`, then swaps the vtable of each `IOUSBDeviceInterface` / `IOUSBInterfaceInterface` for a patched copy. These calls are wrapped:
- `DeviceRequest*`, `ControlRequest*`;
- `WritePipe*`, `ReadPipe*` (sync and async);
- `ClearPipeStall*`, `AbortPipe`, `ResetPipe`, `ResetDevice`, `USBDeviceReEnumerate`.

Since 2026-11-24, UBMDF's flash sequence is aligned to the trace of the official 16.5 updater: 32 of 32 ops match. The sequence is in [protocol.md](protocol.md#official-macos-165-updater-traced-2026-11-24), and the trace is in `logs/official-trace-2026-11-24/`.
