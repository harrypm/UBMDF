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
/// Bulk IN endpoint (never carries data in any capture; the official tool still
/// clears its halt when opening the device).
const BULK_IN_EP: u8 = 0x82;
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
    /// The 3 trailing flag bytes as received (`01 00 01` busy, `00 00 00` done).
    pub flags: [u8; 3],
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
        flags: [d[12], d[13], d[14]],
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
    /// Device identity read at the start of the session (0x10 nonce, 0x10 id byte).
    Identity { nonce: Vec<u8>, id_byte: u8 },
    /// Erase-phase poll: (region block address, blocks done, blocks expected)
    ErasePoll { region: u32, done: u32, expected: u32 },
    /// Image bytes written for component `index` (chunk completed).
    WriteChunk { component: usize, bytes: usize },
}

/// One step of the flash session as it appears on the wire. `planned_ops` builds
/// the list, `flash` executes exactly that list, and `replay::check` compares the
/// same list with a USB capture of the official updater — so what is checked is
/// what is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireOp {
    /// Vendor control IN (bmRequestType 0xC0 device / 0xC1 interface).
    CtrlIn { device: bool, request: u8, length: u16 },
    /// Vendor control OUT to the interface (bmRequestType 0x41).
    CtrlOut { request: u8, data: Vec<u8> },
    /// 0x38 progress polls until the component's erase finishes (one or more on the wire).
    ErasePoll { component: usize },
    /// The component's image as bulk OUT chunks on EP 0x01.
    Bulk { component: usize, bytes: usize, sha256: String },
    /// Zero-length bulk OUT terminator after a component.
    Zlp { component: usize },
    /// Clear the halt / data toggle of a bulk endpoint on both ends (standard
    /// CLEAR_FEATURE(ENDPOINT_HALT) + host toggle reset). The official tool does this
    /// for both pipes when it opens the device (traced 2026-11-24).
    ClearHalt { ep: u8 },
    /// Poll 0x30 until the device has finished committing the written region
    /// (bytes 1..3 go `01 01` busy -> `00 00` done, ~100 ms after the ZLP).
    WaitWriteDone { component: usize },
}

impl WireOp {
    /// Short human-readable form, used in logs and the replay report.
    pub fn describe(&self) -> String {
        match self {
            WireOp::CtrlIn { device, request, length } => format!(
                "IN  {:02x}/0x{request:02x} len {length} ({})",
                if *device { 0xc0 } else { 0xc1 },
                request_name(*request)
            ),
            WireOp::CtrlOut { request, data } => format!(
                "OUT 41/0x{request:02x} [{}] ({})",
                hex(data),
                request_name(*request)
            ),
            WireOp::ErasePoll { component } => format!("0x38 erase polls (component {component})"),
            WireOp::Bulk { component, bytes, sha256 } => {
                format!("BULK OUT ep01 {bytes} B sha256 {} (component {component})", &sha256[..16.min(sha256.len())])
            }
            WireOp::Zlp { component } => format!("BULK OUT ep01 zero-length (component {component})"),
            WireOp::ClearHalt { ep } => format!("CLEAR_HALT ep{ep:02x} (both ends)"),
            WireOp::WaitWriteDone { component } => {
                format!("IN  c1/0x30 len 6 (state) polled until write done (component {component})")
            }
        }
    }
}

/// Name of a known vendor request.
pub fn request_name(request: u8) -> &'static str {
    match request {
        REQ_ID => "id",
        REQ_STATE => "state",
        REQ_GEOMETRY => "erase context",
        REQ_BEGIN => "begin",
        REQ_ERASE => "erase",
        REQ_REGION => "region select",
        REQ_PROGRESS => "progress",
        REQ_WRITE_ADDR => "write addr",
        REQ_COMMIT => "commit",
        REQ_REBOOT => "reboot",
        _ => "unknown",
    }
}

fn op_in(device: bool, request: u8, length: u16) -> WireOp {
    WireOp::CtrlIn { device, request, length }
}

fn op_out(request: u8, data: &[u8]) -> WireOp {
    WireOp::CtrlOut { request, data: data.to_vec() }
}

/// The complete wire sequence UBMDF sends to flash `components`.
///
/// id(0x10 dev 16 B, 0x10 iface 1 B) → state(0x30)×2 → begin(0x31) → state×2 →
/// erase(0x34) → context(0x35) → per component: region(0x37) → 0x38 polls →
/// write addr(0x39) → context(0x35, write latch) → bulk OUT → ZLP → state(0x30) +
/// context(0x35) [non-final] or state×2 [final] → commit(0x32) → state → reboot(0x12).
pub fn planned_ops(components: &[ComponentSpec]) -> Vec<WireOp> {
    let mut ops = vec![
        // Official open sequence (macOS DesktopVideoUpdateTool 16.5, traced 2026-11-24):
        // clear both pipes, then id 1 B, then the 16 B nonce twice.
        WireOp::ClearHalt { ep: BULK_OUT_EP },
        WireOp::ClearHalt { ep: BULK_IN_EP },
        op_in(false, REQ_ID, 1),
        op_in(true, REQ_ID, 16),
        op_in(true, REQ_ID, 16),
        // Official prologue (capture frames 35-37): two state reads BEFORE begin too.
        op_in(false, REQ_STATE, 6),
        op_in(false, REQ_STATE, 6),
        op_out(REQ_BEGIN, &[]),
        op_in(false, REQ_STATE, 6),
        op_in(false, REQ_STATE, 6),
        // The erase command (0x34) goes BEFORE the 0x35 read — 0x35 only responds
        // during an active erase session and STALLs otherwise (verified on hardware).
        op_out(REQ_ERASE, &[]),
        op_in(false, REQ_GEOMETRY, 16),
    ];
    for (index, c) in components.iter().enumerate() {
        let mut region = [0u8; 8];
        region[0..4].copy_from_slice(&c.region_block_addr.to_le_bytes());
        region[4..8].copy_from_slice(&c.block_count.to_le_bytes());
        ops.push(op_out(REQ_REGION, &region));
        ops.push(WireOp::ErasePoll { component: index });
        ops.push(op_out(REQ_WRITE_ADDR, &c.write_addr.to_be_bytes()));
        // Official flow reads 0x35 immediately after arming the write address
        // (reference-capture frames 777 and 1525) — required to latch write mode;
        // omitting it makes the second 0x39 stall (hardware-verified).
        ops.push(op_in(false, REQ_GEOMETRY, 16));
        ops.push(WireOp::Bulk { component: index, bytes: c.data.len(), sha256: sha256_hex(&c.data) });
        // Explicit zero-length terminator (reference-capture frames 1359 and 1645).
        ops.push(WireOp::Zlp { component: index });
        // Post-write, as traced from the official tool: 0x30 (reads busy `010101..`),
        // poll 0x30 until done (`010000..`, ~100 ms), 0x30 once more, then 0x35 before
        // the next region (nothing extra before the final commit).
        ops.push(op_in(false, REQ_STATE, 6));
        ops.push(WireOp::WaitWriteDone { component: index });
        ops.push(op_in(false, REQ_STATE, 6));
        if index + 1 < components.len() {
            ops.push(op_in(false, REQ_GEOMETRY, 16));
        }
    }
    ops.push(op_out(REQ_COMMIT, &[]));
    ops.push(op_in(false, REQ_STATE, 6));
    ops.push(op_out(REQ_REBOOT, &[]));
    ops
}

/// Poll 0x38 until the component's erase is armed and then idle twice in a row.
fn wait_erase(
    iface: &Interface,
    index: usize,
    c: &ComponentSpec,
    on_event: &mut impl FnMut(&FlashEvent),
) -> Result<()> {
    let erase_start = Instant::now();
    // Give the device a moment to arm the erase after the 0x37 region select.
    std::thread::sleep(Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(600);
    let arm_deadline = Instant::now() + Duration::from_secs(15);
    // Completion requires the erase to be ARMED first (0x38 expected > 0): the
    // pre-arm idle reply {expected=0, busy=false} is indistinguishable from a
    // finished erase otherwise (hardware-verified the hard way).
    let mut idle = 0u32;
    let mut armed = false;
    let last = loop {
        let p = read_progress(iface)?;
        log::info!(
            "component {index}: 0x38 progress region={} expected={} done={} flags={}",
            p.region,
            p.expected,
            p.done,
            hex(&p.flags)
        );
        if p.expected > 0 && p.expected != c.block_count {
            log::warn!(
                "component {index}: device reports expected={} blocks but {} were requested",
                p.expected,
                c.block_count
            );
        }
        on_event(&FlashEvent::ErasePoll { region: p.region, done: p.done, expected: p.expected });
        if p.expected > 0 {
            armed = true;
        }
        if armed && !p.busy {
            idle += 1;
            if idle >= 2 {
                break p;
            }
        } else {
            idle = 0;
        }
        if !armed && Instant::now() > arm_deadline {
            bail!("erase did not arm (0x38 expected stayed 0 for 15 s)");
        }
        if Instant::now() > deadline {
            bail!("erase poll timed out (region {} blocks {}/{})", p.region, p.done, p.expected);
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    log::info!(
        "component {index}: erase done in {:.1} s: region={} expected={} done={} busy={}",
        erase_start.elapsed().as_secs_f32(),
        last.region,
        last.expected,
        last.done,
        last.busy
    );
    // `done` is the absolute block the erase has reached, not a count: region 192 x 30
    // blocks finishes at done=222 (hardware log 2026-11-24).
    let end = last.region + last.expected;
    if last.done != end {
        log::warn!(
            "component {index}: erase ended at block {} but region {} + {} blocks ends at {end}",
            last.done,
            last.region,
            last.expected
        );
    }
    Ok(())
}

/// Send one component's image as bulk OUT chunks.
fn write_component(
    iface: &Interface,
    index: usize,
    c: &ComponentSpec,
    on_event: &mut impl FnMut(&FlashEvent),
) -> Result<()> {
    let write_start = Instant::now();
    let mut sent = 0usize;
    let mut chunks = 0usize;
    let mut writer = iface.endpoint::<Bulk, Out>(BULK_OUT_EP)?.writer(CHUNK);
    for chunk in c.data.chunks(CHUNK) {
        writer
            .write_all(chunk)
            .with_context(|| format!("bulk OUT write failed after {sent} bytes"))?;
        sent += chunk.len();
        chunks += 1;
        if chunks.is_multiple_of(32) {
            log::info!("component {index}: {sent}/{} bytes queued", c.data.len());
        }
        on_event(&FlashEvent::WriteChunk { component: index, bytes: chunk.len() });
    }
    writer.flush().context("bulk OUT flush failed")?;
    drop(writer);
    log::info!(
        "component {index}: bulk OUT complete: {sent} bytes in {chunks} chunks ({:.1} s, sha256 {})",
        write_start.elapsed().as_secs_f32(),
        sha256_hex(&c.data)
    );
    Ok(())
}

/// Execute the flash session: exactly the `planned_ops` sequence (see docs/protocol.md),
/// logging every step and every device response.
pub fn flash(
    dev: &mut BmdDevice,
    components: &[ComponentSpec],
    mut on_event: impl FnMut(&FlashEvent),
) -> Result<()> {
    if components.is_empty() {
        bail!("no components to flash");
    }
    let total: usize = components.iter().map(|c| c.data.len()).sum();
    log::info!("flash: {} component(s), {total} bytes", components.len());
    for (i, c) in components.iter().enumerate() {
        log::info!(
            "flash: component {i}: {} B, region block {} x {} blocks (erase 0x{:08x}-0x{:08x}), write addr 0x{:08x}",
            c.data.len(),
            c.region_block_addr,
            c.block_count,
            c.region_block_addr as u64 * 0x10000,
            (c.region_block_addr as u64 + c.block_count as u64) * 0x10000 - 1,
            c.write_addr
        );
    }

    let ops = planned_ops(components);
    let mut nonce: Option<Vec<u8>> = None;
    for (step, op) in ops.iter().enumerate() {
        let n = step + 1;
        match op {
            WireOp::CtrlIn { device, request, length } => {
                let recipient = if *device { Recipient::Device } else { Recipient::Interface };
                let d = ctrl_in(&dev.iface, recipient, *request, *length)?;
                log::info!("[{n}/{}] {} -> {}", ops.len(), op.describe(), hex(&d));
                if *request == REQ_ID {
                    if *device {
                        nonce = Some(d);
                    } else if let Some(nv) = nonce.take() {
                        on_event(&FlashEvent::Identity { nonce: nv, id_byte: d.first().copied().unwrap_or(0) });
                    }
                }
            }
            WireOp::CtrlOut { request, data } => {
                if *request == REQ_BEGIN {
                    // Begin with lingering-session recovery (USB reset + retry once).
                    begin_session(dev)?;
                } else {
                    ctrl_out(&dev.iface, *request, data)?;
                }
                log::info!("[{n}/{}] {}: OK", ops.len(), op.describe());
            }
            WireOp::ErasePoll { component } => {
                log::info!("[{n}/{}] {}", ops.len(), op.describe());
                wait_erase(&dev.iface, *component, &components[*component], &mut on_event)?;
            }
            WireOp::Bulk { component, .. } => {
                log::info!("[{n}/{}] {}", ops.len(), op.describe());
                write_component(&dev.iface, *component, &components[*component], &mut on_event)?;
            }
            WireOp::ClearHalt { ep } => {
                let r = if *ep & 0x80 != 0 {
                    dev.iface.endpoint::<Bulk, nusb::transfer::In>(*ep)?.clear_halt().wait()
                } else {
                    dev.iface.endpoint::<Bulk, Out>(*ep)?.clear_halt().wait()
                };
                r.with_context(|| format!("clear halt on ep {ep:#04x} failed"))?;
                log::info!("[{n}/{}] {}: OK", ops.len(), op.describe());
            }
            WireOp::WaitWriteDone { component } => {
                let start = Instant::now();
                let mut polls = 0;
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    let st = read_state(&dev.iface)?;
                    polls += 1;
                    log::info!("[{n}/{}] 0x30 state -> {} (poll {polls})", ops.len(), hex(&st));
                    if st.len() >= 3 && st[1] == 0 && st[2] == 0 {
                        break;
                    }
                    if start.elapsed() > Duration::from_secs(30) {
                        bail!("component {component}: write not finished after 30 s (0x30 = {})", hex(&st));
                    }
                }
                log::info!(
                    "[{n}/{}] component {component}: write done after {polls} poll(s), {:.2} s",
                    ops.len(),
                    start.elapsed().as_secs_f32()
                );
            }
            WireOp::Zlp { component } => {
                let mut ep = dev.iface.endpoint::<Bulk, Out>(BULK_OUT_EP)?;
                ep.transfer_blocking(Buffer::new(0), Duration::from_secs(10))
                    .into_result()
                    .with_context(|| format!("zero-length terminator failed (component {component})"))?;
                log::info!("[{n}/{}] {}: OK", ops.len(), op.describe());
            }
        }
    }
    log::info!(
        "flash session finished: every transfer was accepted; the device offers no read-back, so the flash contents are NOT verified"
    );
    Ok(())
}

/// Outcome of one probe transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum ProbeResult {
    /// The device answered with these bytes (possibly empty).
    Data(String),
    /// The device STALLed the request (not supported in this state).
    Stall,
    /// No answer within the timeout.
    Timeout,
    /// The device left the bus.
    Disconnected,
    /// Any other transfer error.
    Error(String),
}

fn probe_result(r: std::result::Result<Vec<u8>, nusb::transfer::TransferError>) -> ProbeResult {
    use nusb::transfer::TransferError as E;
    match r {
        Ok(d) => ProbeResult::Data(hex(&d)),
        Err(E::Stall) => ProbeResult::Stall,
        Err(E::Cancelled) => ProbeResult::Timeout,
        Err(E::Disconnected) => ProbeResult::Disconnected,
        Err(e) => ProbeResult::Error(format!("{e:?}")),
    }
}

/// One vendor control IN request with arbitrary request/value/index (read
/// direction only: the host sends no data). Used by `ubmdf probe`.
pub fn probe_in(iface: &Interface, device: bool, request: u8, value: u16, index: u16, length: u16) -> ProbeResult {
    probe_result(
        iface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: if device { Recipient::Device } else { Recipient::Interface },
                    request,
                    value,
                    index,
                    length,
                },
                CTRL_TIMEOUT,
            )
            .wait(),
    )
}

/// Bulk IN EP 0x82 (never used by the official updater): one read attempt.
pub fn probe_bulk_in(iface: &Interface, length: usize, timeout: Duration) -> Result<ProbeResult> {
    let mut ep = iface.endpoint::<Bulk, nusb::transfer::In>(0x82)?;
    let c = ep.transfer_blocking(Buffer::new(length), timeout);
    let r = c.status.map(|_| c.buffer[..c.actual_len].to_vec());
    let stalled = r == Err(nusb::transfer::TransferError::Stall);
    let out = probe_result(r);
    if stalled {
        let _ = ep.clear_halt().wait();
    }
    Ok(out)
}

/// sha256 of a byte slice as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(data))
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
    fn planned_ops_follow_the_documented_session() {
        let comps = [
            ComponentSpec { region_block_addr: 0, block_count: 145, write_addr: 0, data: vec![1; 10] },
            ComponentSpec { region_block_addr: 192, block_count: 30, write_addr: 0x00c0_0000, data: vec![2; 5] },
        ];
        let ops = planned_ops(&comps);
        // 12 prologue ops, 10 for a non-final and 9 for the final component, 3 epilogue
        // ops — the official macOS 16.5 trace (2026-11-24).
        assert_eq!(ops.len(), 12 + 10 + 9 + 3);
        assert_eq!(ops[0], WireOp::ClearHalt { ep: 0x01 });
        assert_eq!(ops[2], WireOp::CtrlIn { device: false, request: 0x10, length: 1 });
        assert_eq!(ops[7], WireOp::CtrlOut { request: 0x31, data: vec![] });
        assert_eq!(ops[12], WireOp::CtrlOut { request: 0x37, data: vec![0, 0, 0, 0, 0x91, 0, 0, 0] });
        assert_eq!(ops[22], WireOp::CtrlOut { request: 0x37, data: vec![0xc0, 0, 0, 0, 0x1e, 0, 0, 0] });
        assert_eq!(ops[24], WireOp::CtrlOut { request: 0x39, data: vec![0, 0xc0, 0, 0] });
        assert_eq!(ops[ops.len() - 3], WireOp::CtrlOut { request: 0x32, data: vec![] });
        assert_eq!(ops[ops.len() - 1], WireOp::CtrlOut { request: 0x12, data: vec![] });
    }

    #[test]
    fn write_addr_be_matches_capture() {
        assert_eq!(12582912u32.to_be_bytes(), [0x00, 0xc0, 0x00, 0x00]);
        assert_eq!(0u32.to_be_bytes(), [0, 0, 0, 0]);
    }
}
