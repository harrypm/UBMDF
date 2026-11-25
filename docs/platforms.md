# Platform notes

## Windows
The service port needs the **WinUSB** driver (install it with Zadig) for UBMDF to open it. Blackmagic's own `DesktopVideoUpdateTool` needs Blackmagic's `DesktopVideoUpdate` driver instead. To switch, re-run Zadig or bind Blackmagic's INF with `pnputil /add-driver … /install`. The Devices tab shows the bound driver.

## Linux
No driver is needed. A udev rule gives access without root:

```
SUBSYSTEM=="usb", ATTRS{idVendor}=="1edb", ATTRS{idProduct}=="be13", MODE="0660", TAG+="uaccess"
```

## macOS
No driver install is needed, because IOKit gives user-space access. Blackmagic's macOS `DesktopVideoUpdateTool` can flash the service port too. [tracing.md](tracing.md) shows how to trace it.
