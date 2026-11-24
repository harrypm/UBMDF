//! Reverse-engineered Blackmagic service-port flash protocol.
//! See docs/protocol.md for the full byte-level analysis this is based on.
//!
//! Device: Blackmagic UltraStudio 4K Mini service port (USB VID 0x1EDB PID 0xBE13)
//! Protocol as captured from DesktopVideoUpdateTool.exe 12.8.1 on 2026-11-19.

use anyhow::{bail, Context, Result};
use nusb::transfer::{Buffer, Bulk, ControlIn, ControlOut, ControlType, Out, Recipient};
use nusb::{Device, Interface, MaybeFuture};
use std::io::Write as _;
use std::time::{Duration, Instant};

pub const VID: u16 = 0x1edb;
pub const PID: u16 = 0xbe13;

const CTRL_TIMEOUT: Duration = Duration::from_millis(1000);

// Vendor bRequest values (control transfers on EP0).
const REQ_ID: u8 = 0x10; // IN 0xC0 (16B nonce) / IN 0xC1 (1B)
const REQ_STATE: u8 = 0x30; // IN 0xC1, 6B
const REQ_GEOMETRY: u8 = 0x35; // IN 0xC1, 16B
const REQ_BEGIN: u8 = 0x31; // OUT 0x41, no data
const REQ_ERASE: u8 = 0x34; // OUT 0x41, no data
const REQ_REGION: u8 = 0x37; // OUT 0x41, 8B: {LE u32 addr in 64K blocks, LE u32 block count}
const REQ_PROGRESS: u8 = 0x38; // IN 0xC1, 15B
const REQ_WRITE_ADDR: u8 = 0x39; // OUT 0x41, 4B: {BE u32 byte address}
const REQ_COMMIT: u8 = 0x32; // OUT 0x41, no data
const REQ_REBOOT: u8 = 0x12; // OUT 0x41, no data

/// Bulk OUT endpoint carrying image data.
const BULK_OUT_EP: u8 = 0x01;
/// Transfer chunk size used by the official tool (max). The final chunk of each
/// component is short, which terminates the component's data stream on the wire.
const CHUNK: usize = 32768;

fn ctrl_in(iface: &Interface, recipient: Recipient, request: u8, len: u16) -> Result<Vec<u8>> {
    iface
        .control_in(
            ControlIn {
                control_type: ControlType::Vendor,
                recipient,
                request,
                value: 0,
                index: 0,
                length: len,
            },
            CTRL_TIMEOUT,
        )
        .wait()
        .with_context(|| format!("control IN request 0x{request:02x} failed"))
}

fn ctrl_out(iface: &Interface, request: u8, data: &[u8]) -> Result<()> {
    iface
        .control_out(
            ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Interface,
                request,
                value: 0,
                index: 0,
                data,
            },
            CTRL_TIMEOUT,
        )
        .wait()
        .with_context(|| format!("control OUT request 0x{request:02x} failed"))?;
    Ok(())
}

/// List connected Blackmagic service-port devices (human-readable strings).
pub fn list_bmd() -> Result<Vec<String>> {
    let mut out = Vec::new();
    for d in nusb::list_devices().wait()? {
        if d.vendor_id() == VID && d.product_id() == PID {
            out.push(format!(
                "product={:?} manufacturer={:?} serial={:?}",
                d.product_string(),
                d.manufacturer_string(),
                d.serial_number()
            ));
        }
    }
    Ok(out)
}

/// A service-port device found on the bus (enumeration only — nothing is
/// opened or claimed). Used by the GUI's live device list (polled every 2s).
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "gui"), allow(dead_code))] // GUI device list
pub struct DeviceEntry {
    /// iProduct string, if cached by the OS.
    pub product: Option<String>,
    /// iSerial string, if cached by the OS.
    pub serial: Option<String>,
    /// Bus / host controller identifier (platform-specific string).
    pub bus_id: String,
    /// Device address on the bus.
    pub address: u8,
    /// Physical port path (hub chain), stable across replugs.
    pub port_chain: Vec<u8>,
    /// Connection speed (e.g. Super, High), if known.
    pub speed: Option<String>,
    /// Driver bound to the device (Windows only; always None elsewhere).
    /// "WinUSB" = flashable by UBMDF; anything else (e.g. BMD
    /// DesktopVideoUpdate) needs a Zadig swap first.
    pub driver: Option<String>,
}

/// Enumerate connected service-port devices with structured details.
/// No handles are opened and no interfaces claimed — safe to call repeatedly.
#[cfg_attr(not(feature = "gui"), allow(dead_code))] // GUI device poller
pub fn list_bmd_entries() -> Result<Vec<DeviceEntry>> {
    let mut out = Vec::new();
    for d in nusb::list_devices().wait()? {
        if d.vendor_id() != VID || d.product_id() != PID {
            continue;
        }
        out.push(DeviceEntry {
            product: d.product_string().map(str::to_string),
            serial: d.serial_number().map(str::to_string),
            bus_id: d.bus_id().to_string(),
            address: d.device_address(),
            port_chain: d.port_chain().to_vec(),
            speed: d.speed().map(|s| format!("{s:?}")),
            #[cfg(windows)]
            driver: d.driver().map(str::to_string),
            #[cfg(not(windows))]
            driver: None,
        });
    }
    Ok(out)
}

/// An opened service-port device: USB device handle plus the claimed interface.
/// The handle is kept so a USB port reset can be issued for session recovery.
pub struct BmdDevice {
    pub device: Device,
    pub iface: Interface,
}

/// Open and claim the first service-port device.
pub fn open() -> Result<BmdDevice> {
    let di = nusb::list_devices()
        .wait()?
        .find(|d| d.vendor_id() == VID && d.product_id() == PID)
        .context("no Blackmagic service-port device found (USB VID 1edb PID be13)")?;
    let device = di.open().wait().context(
        "failed to open USB device — on Windows this usually means the interface is bound to \
         Blackmagic's DesktopVideoUpdate driver instead of WinUSB; see README (Windows driver)",
    )?;
    let iface = device
        .claim_interface(0)
        .wait()
        .context("failed to claim interface 0")?;
    Ok(BmdDevice { device, iface })
}

/// Device identity: (16-byte nonce from 0xC0/0x10, 1-byte value from 0xC1/0x10).
pub fn read_id(iface: &Interface) -> Result<(Vec<u8>, u8)> {
    let nonce = ctrl_in(iface, Recipient::Device, REQ_ID, 16)?;
    let one = ctrl_in(iface, Recipient::Interface, REQ_ID, 1)?;
    Ok((nonce, one.first().copied().unwrap_or(0)))
}

/// Begin a session (0x31). The official tool always sends this before reading
/// 0x35/0x38 — without it the device STALLs those requests (verified on hardware).
pub fn begin(iface: &Interface) -> Result<()> {
    ctrl_out(iface, REQ_BEGIN, &[])
}

/// Send the reboot command (0x12) — the official session terminator. Used by the
/// `reset` subcommand to clear a lingering session (the device refuses a new
/// 0x31 while an old session is active; session state survives USB re-enumeration,
/// hardware-verified).
pub fn reboot(iface: &Interface) -> Result<()> {
    ctrl_out(iface, REQ_REBOOT, &[])
}

/// Begin a session with recovery: if 0x31 stalls (device left in a lingering
/// session by an aborted run — it refuses a third begin without an intervening
/// reset/commit; hardware-verified), issue a USB port reset, re-claim the
/// interface and retry once.
pub fn begin_session(dev: &mut BmdDevice) -> Result<()> {
    if begin(&dev.iface).is_ok() {
        return Ok(());
    }
    dev.device.reset().wait().context("USB port reset failed")?;
    dev.iface = dev
        .device
        .claim_interface(0)
        .wait()
        .context("re-claim after USB reset failed")?;
    begin(&dev.iface)
}

/// 6-byte state response (0xC1/0x30). Field semantics partly unknown; see protocol.md.
pub fn read_state(iface: &Interface) -> Result<Vec<u8>> {
    ctrl_in(iface, Recipient::Interface, REQ_STATE, 6)
}

/// Flash geometry (0xC1/0x35): four big-endian u32s. [3] is the flash block size in bytes.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub fields: [u32; 4],
}

impl Geometry {
    #[allow(dead_code)] // used by the GUI (U4)
    pub fn block_size(&self) -> u32 {
        self.fields[3]
    }
}

pub fn read_geometry(iface: &Interface) -> Result<Geometry> {
    let d = ctrl_in(iface, Recipient::Interface, REQ_GEOMETRY, 16)?;
    if d.len() < 16 {
        bail!("short geometry response: {} bytes", d.len());
    }
    let mut fields = [0u32; 4];
    for i in 0..4 {
        fields[i] = u32::from_be_bytes([d[i * 4], d[i * 4 + 1], d[i * 4 + 2], d[i * 4 + 3]]);
    }
    Ok(Geometry { fields })
}

/// Erase/write progress (0xC1/0x38), 15 bytes:
/// {LE u32 region, LE u32 expected, LE u32 progress, 3 flag bytes}.
/// Trailing flags `01 00 01` = busy, `00 00 00` = phase complete.
#[derive(Debug, Clone, Copy)]
pub struct ProgressView {
    pub region: u32,
    pub expected: u32,
    pub done: u32,
    pub busy: bool,
}

pub fn read_progress(iface: &Interface) -> Result<ProgressView> {
    let d = ctrl_in(iface, Recipient::Interface, REQ_PROGRESS, 15)?;
    if d.len() < 15 {
        bail!("short progress response: {} bytes", d.len());
    }
    Ok(ProgressView {
        region: u32::from_le_bytes([d[0], d[1], d[2], d[3]]),
        expected: u32::from_le_bytes([d[4], d[5], d[6], d[7]]),
        done: u32::from_le_bytes([d[8], d[9], d[10], d[11]]),
        busy: !(d[12] == 0 && d[13] == 0 && d[14] == 0),
    })
}

/// One firmware component to write: region metadata plus the plaintext image bytes.
pub struct ComponentSpec {
    pub region_block_addr: u32,
    pub block_count: u32,
    pub write_addr: u32,
    pub data: Vec<u8>,
}

/// Events emitted during flashing so the CLI can render progress.
pub enum FlashEvent {
    /// Erase-phase poll: (region block address, blocks done, blocks expected)
    ErasePoll { region: u32, done: u32, expected: u32 },
    /// Image bytes written for component `index` (chunk completed).
    WriteChunk { component: usize, bytes: usize },
}

/// Execute the full flash session exactly as the official tool does.
///
/// Sequence (see docs/protocol.md):
/// begin(0x31, with reset recovery) → state(0x30)×2 → erase(0x34) → context(0x35) →
/// for each component: region(0x37) → poll(0x38) until idle → write addr(0x39) →
/// bulk OUT chunks → state/geometry checks → commit(0x32) → state(0x30) → reboot(0x12).
pub fn flash(
    dev: &mut BmdDevice,
    components: &[ComponentSpec],
    mut on_event: impl FnMut(&FlashEvent),
) -> Result<()> {
    if components.is_empty() {
        bail!("no components to flash");
    }

    // Official prologue (capture frames 35-37): two state reads BEFORE begin too.
    let _ = read_state(&dev.iface)?;
    let _ = read_state(&dev.iface)?;
    begin_session(dev)?;
    let iface = &dev.iface;
    let _ = read_state(iface)?;
    let _ = read_state(iface)?;

    // Official order (verified against the capture): the erase command (0x34) is
    // sent BEFORE reading 0x35 — 0x35 only responds during an active erase session
    // and STALLs otherwise (verified on hardware).
    ctrl_out(iface, REQ_ERASE, &[])?;
    let geo = read_geometry(iface)?;

    for (index, c) in components.iter().enumerate() {
        let mut region = [0u8; 8];
        region[0..4].copy_from_slice(&c.region_block_addr.to_le_bytes());
        region[4..8].copy_from_slice(&c.block_count.to_le_bytes());
        ctrl_out(iface, REQ_REGION, &region)?;

        // Give the device a moment to arm the erase after the 0x37 region select.
        std::thread::sleep(Duration::from_millis(100));
        let deadline = Instant::now() + Duration::from_secs(600);
        let arm_deadline = Instant::now() + Duration::from_secs(15);
        // Completion requires the erase to be ARMED first (0x38 expected > 0): the
        // pre-arm idle reply {expected=0, busy=false} is indistinguishable from a
        // finished erase otherwise (hardware-verified the hard way).
        let mut idle = 0u32;
        let mut armed = false;
        #[allow(unused_assignments)]
        let mut last: Option<ProgressView> = None;
        loop {
            let p = read_progress(iface)?;
            last = Some(p);
            on_event(&FlashEvent::ErasePoll {
                region: p.region,
                done: p.done,
                expected: p.expected,
            });
            if p.expected > 0 {
                armed = true;
            }
            if armed && !p.busy {
                idle += 1;
                if idle >= 2 {
                    break;
                }
            } else {
                idle = 0;
            }
            if !armed && Instant::now() > arm_deadline {
                bail!("erase did not arm (0x38 expected stayed 0 for 15 s)");
            }
            if Instant::now() > deadline {
                bail!(
                    "erase poll timed out (region {} blocks {}/{})",
                    p.region,
                    p.done,
                    p.expected
                );
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let last = last.context("no progress poll completed")?;
        eprintln!(
            "[diag] erase phase ended: region={} expected={} done={} busy={}",
            last.region, last.expected, last.done, last.busy
        );

        ctrl_out(iface, REQ_WRITE_ADDR, &c.write_addr.to_be_bytes())?;
        // Official flow reads 0x35 immediately after arming the write address
        // (reference-capture frames 777 and 1525) — required to latch write mode;
        // omitting it makes the second 0x39 stall (hardware-verified).
        let _ = read_geometry(iface)?;

        let mut writer = iface.endpoint::<Bulk, Out>(BULK_OUT_EP)?.writer(CHUNK);
        for chunk in c.data.chunks(CHUNK) {
            writer
                .write_all(chunk)
                .context("bulk OUT write failed")?;
            on_event(&FlashEvent::WriteChunk {
                component: index,
                bytes: chunk.len(),
            });
        }
        writer.flush().context("bulk OUT flush failed")?;
        drop(writer);

        // Explicit zero-length terminator: the official tool submits a genuine
        // zero-length bulk OUT after each component's final short packet
        // (reference-capture frames 1359 and 1645). Without it the device does
        // not finalize the region write and the next 0x39 stalls (verified).
        let mut ep = iface.endpoint::<Bulk, Out>(BULK_OUT_EP)?;
        ep.transfer_blocking(Buffer::new(0), Duration::from_secs(10))
            .into_result()
            .context("zero-length terminator failed")?;

        // Post-write checks, matching the official trace exactly:
        // non-final component: 0x30 + 0x35; final component: 0x30 twice.
        let _ = read_state(iface)?;
        if index + 1 < components.len() {
            let _ = read_geometry(iface)?;
        } else {
            let _ = read_state(iface)?;
        }
    }

    let _ = geo; // geometry was read for the session record
    ctrl_out(iface, REQ_COMMIT, &[])?;
    let _ = read_state(iface)?;
    ctrl_out(iface, REQ_REBOOT, &[])?;
    Ok(())
}

/// Format a byte slice as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_lowercases() {
        assert_eq!(hex(&[0x0a, 0xff, 0x10]), "0aff10");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn region_params_match_capture() {
        // 0x37 payload layout verified against the captured official-tool values.
        let region_block_addr: u32 = 192;
        let block_count: u32 = 30;
        let mut region = [0u8; 8];
        region[0..4].copy_from_slice(&region_block_addr.to_le_bytes());
        region[4..8].copy_from_slice(&block_count.to_le_bytes());
        assert_eq!(region, [0xc0, 0, 0, 0, 0x1e, 0, 0, 0]);
    }

    #[test]
    fn write_addr_be_matches_capture() {
        assert_eq!(12582912u32.to_be_bytes(), [0x00, 0xc0, 0x00, 0x00]);
        assert_eq!(0u32.to_be_bytes(), [0, 0, 0, 0]);
    }
}
