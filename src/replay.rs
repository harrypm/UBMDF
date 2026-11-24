//! `ubmdf replay-check`: compare the wire sequence UBMDF would send for a
//! manifest (`protocol::planned_ops`, the same list `protocol::flash` executes)
//! with a USB capture of the official Blackmagic updater.
//!
//! Supported captures (classic pcap, either byte order):
//! - USBPcap on Windows (LINKTYPE_USBPCAP = 249)
//! - Linux usbmon (LINKTYPE_USB_LINUX_MMAPPED = 220, 64-byte header)
//!
//! pcapng is not read: convert with `editcap -F pcap in.pcapng out.pcap`.

use crate::protocol::{self, ComponentSpec, WireOp};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::Path;

const LINKTYPE_USB_LINUX_MMAPPED: u32 = 220;
const LINKTYPE_USBPCAP: u32 = 249;

/// Transfer type, normalised across capture formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Xfer {
    Iso,
    Interrupt,
    Control,
    Bulk,
}

/// One USB event from a capture, normalised across formats.
#[derive(Debug, Clone)]
struct UsbEvent {
    frame: usize,
    time: f64,
    bus: u16,
    dev: u16,
    /// Endpoint address including the direction bit (0x80 = IN).
    ep: u8,
    xfer: Xfer,
    /// true = completion (device -> host side of the URB/IRP), false = submit.
    completion: bool,
    /// Control transfers only: the 8-byte setup packet, if this record carries it.
    setup: Option<[u8; 8]>,
    /// Payload carried by this record (OUT data on submit, IN data on completion).
    data: Vec<u8>,
    /// Transfer length the host asked for / reported (0 for a genuine ZLP).
    length: u32,
    status: i64,
}

/// (timestamp, record bytes) per pcap record.
type PcapRecords = Vec<(f64, Vec<u8>)>;

fn read_pcap(path: &Path) -> Result<(u32, PcapRecords)> {
    let b = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if b.len() < 24 {
        bail!("{}: too short for a pcap file", path.display());
    }
    let magic = u32::from_le_bytes(b[0..4].try_into().unwrap());
    let (le, nanos) = match magic {
        0xa1b2c3d4 => (true, false),
        0xa1b23c4d => (true, true),
        0xd4c3b2a1 => (false, false),
        0x4d3cb2a1 => (false, true),
        0x0a0d0d0a => bail!(
            "{}: pcapng is not supported — convert with `editcap -F pcap <in> <out>`",
            path.display()
        ),
        m => bail!("{}: not a pcap file (magic {m:#010x})", path.display()),
    };
    let u32at = |o: usize| -> u32 {
        let a: [u8; 4] = b[o..o + 4].try_into().unwrap();
        if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        }
    };
    let linktype = u32at(20);
    let mut out = Vec::new();
    let mut p = 24;
    while p + 16 <= b.len() {
        let ts = u32at(p) as f64;
        let frac = u32at(p + 4) as f64;
        let incl = u32at(p + 8) as usize;
        p += 16;
        if p + incl > b.len() {
            log::warn!("{}: truncated last record", path.display());
            break;
        }
        let t = ts + if nanos { frac / 1e9 } else { frac / 1e6 };
        out.push((t, b[p..p + incl].to_vec()));
        p += incl;
    }
    Ok((linktype, out))
}

fn parse_usbpcap(frame: usize, time: f64, d: &[u8]) -> Option<UsbEvent> {
    // USBPCAP_BUFFER_PACKET_HEADER (packed, little endian), 27 bytes.
    if d.len() < 27 {
        return None;
    }
    let hl = u16::from_le_bytes([d[0], d[1]]) as usize;
    if hl < 27 || hl > d.len() {
        return None;
    }
    let status = u32::from_le_bytes(d[10..14].try_into().unwrap()) as i64;
    let info = d[16];
    let bus = u16::from_le_bytes([d[17], d[18]]);
    let dev = u16::from_le_bytes([d[19], d[20]]);
    let ep = d[21];
    let xfer = match d[22] {
        0 => Xfer::Iso,
        1 => Xfer::Interrupt,
        2 => Xfer::Control,
        3 => Xfer::Bulk,
        _ => return None,
    };
    let length = u32::from_le_bytes(d[23..27].try_into().unwrap());
    let completion = info & 1 == 1;
    let mut data = d[hl..].to_vec();
    let mut setup = None;
    if xfer == Xfer::Control && hl >= 28 && d[27] == 0 && !completion && data.len() >= 8 {
        // Setup stage: 8-byte setup packet, then (for OUT) the data stage bytes.
        setup = Some(data[..8].try_into().unwrap());
        data = data[8..].to_vec();
    }
    Some(UsbEvent {
        frame,
        time,
        bus,
        dev,
        ep,
        xfer,
        completion,
        setup,
        data,
        length,
        status,
    })
}

fn parse_usbmon(frame: usize, time: f64, d: &[u8]) -> Option<UsbEvent> {
    // struct usbmon_packet (mmapped, 64 bytes, host byte order = little endian on all
    // usual capture hosts).
    if d.len() < 64 {
        return None;
    }
    let kind = d[8]; // 'S' submit, 'C' complete, 'E' error
    let xfer = match d[9] {
        0 => Xfer::Iso,
        1 => Xfer::Interrupt,
        2 => Xfer::Control,
        3 => Xfer::Bulk,
        _ => return None,
    };
    let ep = d[10];
    let dev = d[11] as u16;
    let bus = u16::from_le_bytes([d[12], d[13]]);
    let flag_setup = d[14];
    let status = i32::from_le_bytes(d[28..32].try_into().unwrap()) as i64;
    let length = u32::from_le_bytes(d[32..36].try_into().unwrap());
    let setup = (xfer == Xfer::Control && kind == b'S' && flag_setup == 0)
        .then(|| d[40..48].try_into().unwrap());
    Some(UsbEvent {
        frame,
        time,
        bus,
        dev,
        ep,
        xfer,
        completion: kind != b'S',
        setup,
        data: d[64..].to_vec(),
        length,
        status,
    })
}

fn load_events(path: &Path) -> Result<Vec<UsbEvent>> {
    let (linktype, recs) = read_pcap(path)?;
    let parse: fn(usize, f64, &[u8]) -> Option<UsbEvent> = match linktype {
        LINKTYPE_USBPCAP => parse_usbpcap,
        LINKTYPE_USB_LINUX_MMAPPED => parse_usbmon,
        other => bail!(
            "{}: link type {other} is not supported (USBPcap = 249, Linux usbmon mmapped = 220)",
            path.display()
        ),
    };
    Ok(recs
        .iter()
        .enumerate()
        .filter_map(|(i, (t, d))| parse(i + 1, *t, d))
        .collect())
}

/// One vendor operation recovered from the capture, with what the device answered.
#[derive(Debug, Clone)]
struct CapturedOp {
    op: WireOp,
    frame: usize,
    time: f64,
    /// Number of wire transfers folded into this op (0x38 polls, bulk chunks).
    count: usize,
    /// Last IN response / last 0x38 reply, if any.
    response: Option<Vec<u8>>,
    /// Non-zero completion status seen for this op.
    error: Option<i64>,
    /// Bulk stream bytes (only for `WireOp::Bulk`).
    stream: Vec<u8>,
}

/// Device addresses that look like the Blackmagic service port, in order of first use.
fn find_targets(ev: &[UsbEvent]) -> Vec<(u16, u16)> {
    let mut found: Vec<(u16, u16)> = Vec::new();
    // Device descriptor replies carrying the VID/PID.
    for e in ev {
        if e.xfer == Xfer::Control
            && e.completion
            && e.data.len() >= 18
            && e.data[0] == 18
            && e.data[1] == 1
        {
            let vid = u16::from_le_bytes([e.data[8], e.data[9]]);
            let pid = u16::from_le_bytes([e.data[10], e.data[11]]);
            if vid == protocol::VID && pid == protocol::PID && !found.contains(&(e.bus, e.dev)) {
                found.push((e.bus, e.dev));
            }
        }
    }
    // Capture started after enumeration: any device issuing this protocol's vendor requests.
    for e in ev {
        if let Some(s) = e.setup
            && s[0] & 0x60 == 0x40
            && matches!(
                s[1],
                0x10 | 0x30 | 0x31 | 0x34 | 0x35 | 0x37 | 0x38 | 0x39 | 0x32 | 0x12
            )
            && !found.contains(&(e.bus, e.dev))
        {
            found.push((e.bus, e.dev));
        }
    }
    found
}

/// Fold one device's events into protocol ops: vendor control transfers, 0x38 poll
/// runs, bulk OUT EP1 streams split at zero-length packets.
fn extract_ops(
    ev: &[UsbEvent],
    target: (u16, u16),
    components: &[ComponentSpec],
) -> Vec<CapturedOp> {
    let mut ops: Vec<CapturedOp> = Vec::new();
    // Pending control transfer per endpoint-0 (USBPcap/usbmon complete in order on EP0).
    let mut pending: Option<usize> = None;
    let mut component = 0usize;
    let mut bulk_open = false;
    for e in ev.iter().filter(|e| (e.bus, e.dev) == target) {
        match e.xfer {
            Xfer::Control => {
                if let Some(s) = e.setup {
                    let (rt, rq) = (s[0], s[1]);
                    let wlen = u16::from_le_bytes([s[6], s[7]]);
                    if rt & 0x60 != 0x40 {
                        pending = None; // standard/class request: not part of the protocol
                        continue;
                    }
                    bulk_open = false;
                    let op = if rt & 0x80 != 0 {
                        if rq == 0x38 {
                            WireOp::ErasePoll {
                                component: component.min(components.len().saturating_sub(1)),
                            }
                        } else {
                            WireOp::CtrlIn {
                                device: rt & 0x1f == 0,
                                request: rq,
                                length: wlen,
                            }
                        }
                    } else {
                        WireOp::CtrlOut {
                            request: rq,
                            data: e.data.clone(),
                        }
                    };
                    // Collapse consecutive 0x38 polls into one ErasePoll.
                    if let (WireOp::ErasePoll { .. }, Some(last)) = (&op, ops.last_mut())
                        && matches!(last.op, WireOp::ErasePoll { .. })
                    {
                        last.count += 1;
                        pending = Some(ops.len() - 1);
                        continue;
                    }
                    if let WireOp::CtrlOut { request: 0x37, .. } = op
                        && ops
                            .iter()
                            .any(|o| matches!(o.op, WireOp::CtrlOut { request: 0x37, .. }))
                    {
                        component += 1;
                    }
                    ops.push(CapturedOp {
                        op,
                        frame: e.frame,
                        time: e.time,
                        count: 1,
                        response: None,
                        error: None,
                        stream: Vec::new(),
                    });
                    pending = Some(ops.len() - 1);
                } else if e.completion {
                    if let Some(i) = pending {
                        let o = &mut ops[i];
                        if e.status != 0 {
                            o.error = Some(e.status);
                        }
                        if !e.data.is_empty() {
                            o.response = Some(e.data.clone());
                        }
                    }
                } else if let Some(i) = pending {
                    // Separate data-stage record (OUT data not in the setup record).
                    if let WireOp::CtrlOut { data, .. } = &mut ops[i].op
                        && data.is_empty()
                    {
                        *data = e.data.clone();
                    }
                }
            }
            Xfer::Bulk if e.ep == 0x01 => {
                if e.completion {
                    if e.status != 0
                        && let Some(o) = ops.last_mut()
                    {
                        o.error = Some(e.status);
                    }
                    continue;
                }
                if e.length == 0 && e.data.is_empty() {
                    ops.push(CapturedOp {
                        op: WireOp::Zlp {
                            component: component.min(components.len().saturating_sub(1)),
                        },
                        frame: e.frame,
                        time: e.time,
                        count: 1,
                        response: None,
                        error: None,
                        stream: Vec::new(),
                    });
                    bulk_open = false;
                    continue;
                }
                if !bulk_open {
                    ops.push(CapturedOp {
                        op: WireOp::Bulk {
                            component,
                            bytes: 0,
                            sha256: String::new(),
                        },
                        frame: e.frame,
                        time: e.time,
                        count: 0,
                        response: None,
                        error: None,
                        stream: Vec::new(),
                    });
                    bulk_open = true;
                }
                let o = ops.last_mut().unwrap();
                o.stream.extend_from_slice(&e.data);
                o.count += 1;
            }
            _ => {}
        }
    }
    for o in &mut ops {
        if let WireOp::Bulk { bytes, sha256, .. } = &mut o.op {
            *bytes = o.stream.len();
            *sha256 = protocol::sha256_hex(&o.stream);
        }
    }
    ops
}

/// Key used to align planned and captured ops. Component indices are left out so a
/// partial capture still aligns; bulk streams compare by length + sha256.
fn key(op: &WireOp) -> String {
    match op {
        WireOp::CtrlIn {
            device,
            request,
            length,
        } => format!("in:{device}:{request:02x}:{length}"),
        WireOp::CtrlOut { request, data } => format!("out:{request:02x}:{}", protocol::hex(data)),
        WireOp::ErasePoll { .. } => "poll".into(),
        WireOp::Bulk { bytes, sha256, .. } => format!("bulk:{bytes}:{sha256}"),
        WireOp::Zlp { .. } => "zlp".into(),
    }
}

/// Longest-common-subsequence alignment. Returns (planned idx, captured idx) pairs,
/// None on one side for an insertion/deletion.
fn align(a: &[String], b: &[String]) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (a.len(), b.len());
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            out.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if j < m && (i == n || t[i][j + 1] >= t[i + 1][j]) {
            out.push((None, Some(j)));
            j += 1;
        } else {
            out.push((Some(i), None));
            i += 1;
        }
    }
    out
}

/// Summary returned to the caller (and used by tests).
#[derive(Debug, Default)]
pub struct ReplayReport {
    pub matched: usize,
    pub only_planned: usize,
    pub only_captured: usize,
    pub bulk_mismatches: usize,
    /// Distinct `expected` block counts the device reported in 0x38 replies.
    pub erase_expected: Vec<u32>,
}

fn describe_captured(o: &CapturedOp) -> String {
    let mut s = o.op.describe();
    if let WireOp::Bulk { .. } = o.op {
        s = format!("{s} in {} transfers", o.count);
    }
    if let WireOp::ErasePoll { .. } = o.op {
        s = format!("{s} x{}", o.count);
        if let Some(r) = &o.response
            && r.len() >= 15
        {
            let u = |k: usize| u32::from_le_bytes(r[k..k + 4].try_into().unwrap());
            s = format!(
                "{s}, last: region={} expected={} done={} flags={}",
                u(0),
                u(4),
                u(8),
                protocol::hex(&r[12..15])
            );
        }
    } else if let Some(r) = &o.response {
        s = format!("{s} -> {}", protocol::hex(r));
    }
    if let Some(e) = o.error {
        s = format!("{s} [status {e:#x}]");
    }
    s
}

/// Run the comparison and print a report. `device` selects `bus:addr` when the
/// capture holds more than one candidate.
pub fn check(
    capture: &Path,
    components: &[ComponentSpec],
    device: Option<(u16, u16)>,
) -> Result<ReplayReport> {
    let ev = load_events(capture)?;
    println!("capture: {} ({} USB records)", capture.display(), ev.len());
    let targets = find_targets(&ev);
    if targets.is_empty() {
        bail!(
            "no Blackmagic service-port traffic (VID {:04x} PID {:04x} or its vendor requests) in this capture",
            protocol::VID,
            protocol::PID
        );
    }
    let mut per_target: BTreeMap<(u16, u16), Vec<CapturedOp>> = BTreeMap::new();
    for t in &targets {
        per_target.insert(*t, extract_ops(&ev, *t, components));
    }
    for (t, ops) in &per_target {
        println!(
            "  candidate bus {} addr {}: {} protocol ops",
            t.0,
            t.1,
            ops.len()
        );
    }
    let target = match device {
        Some(d) => {
            if !per_target.contains_key(&d) {
                bail!(
                    "device bus {} addr {} not found among the candidates",
                    d.0,
                    d.1
                );
            }
            d
        }
        // The flash session is the address with the most protocol ops (the unit
        // re-enumerates with a new address after the 0x12 reboot).
        None => *per_target.iter().max_by_key(|(_, o)| o.len()).unwrap().0,
    };
    let captured = &per_target[&target];
    println!("comparing against bus {} addr {}", target.0, target.1);

    let planned = protocol::planned_ops(components);
    let pk: Vec<String> = planned.iter().map(key).collect();
    let ck: Vec<String> = captured.iter().map(|o| key(&o.op)).collect();
    let mut rep = ReplayReport::default();
    for o in captured {
        if let (WireOp::ErasePoll { .. }, Some(r)) = (&o.op, &o.response)
            && r.len() >= 15
        {
            let e = u32::from_le_bytes(r[4..8].try_into().unwrap());
            if !rep.erase_expected.contains(&e) {
                rep.erase_expected.push(e);
            }
        }
    }

    println!("\n  {:<5} {:<8} {:>9}  op", "", "frame", "t (s)");
    let t0 = captured.first().map(|o| o.time).unwrap_or(0.0);
    for (pi, ci) in align(&pk, &ck) {
        match (pi, ci) {
            (Some(_), Some(c)) => {
                rep.matched += 1;
                let o = &captured[c];
                println!(
                    "  {:<5} {:<8} {:>9.3}  {}",
                    "ok",
                    o.frame,
                    o.time - t0,
                    describe_captured(o)
                );
            }
            (None, Some(c)) => {
                rep.only_captured += 1;
                let o = &captured[c];
                println!(
                    "  {:<5} {:<8} {:>9.3}  {}",
                    "+OFF",
                    o.frame,
                    o.time - t0,
                    describe_captured(o)
                );
            }
            (Some(p), None) => {
                rep.only_planned += 1;
                println!(
                    "  {:<5} {:<8} {:>9}  {}",
                    "-UBMDF",
                    "",
                    "",
                    planned[p].describe()
                );
            }
            (None, None) => {}
        }
    }

    // Bulk streams: compare each captured stream with the component images directly,
    // so a mismatch says which bytes differ instead of only "not aligned".
    println!();
    for o in captured
        .iter()
        .filter(|o| matches!(o.op, WireOp::Bulk { .. }))
    {
        let hit = components.iter().position(|c| c.data == o.stream);
        match hit {
            Some(i) => println!(
                "  bulk stream at frame {}: {} B = component {i} image (byte-exact)",
                o.frame,
                o.stream.len()
            ),
            None => {
                rep.bulk_mismatches += 1;
                let mut line = format!(
                    "  bulk stream at frame {}: {} B matches NO component",
                    o.frame,
                    o.stream.len()
                );
                for (i, c) in components.iter().enumerate() {
                    let common = c
                        .data
                        .iter()
                        .zip(&o.stream)
                        .take_while(|(a, b)| a == b)
                        .count();
                    line += &format!(
                        "; component {i} ({} B) first differs at byte {common}",
                        c.data.len()
                    );
                }
                println!("{line}");
            }
        }
    }
    if !rep.erase_expected.is_empty() {
        let requested: Vec<u32> = components.iter().map(|c| c.block_count).collect();
        println!(
            "  device-reported erase sizes (0x38 expected): {:?}; manifest requests {:?}",
            rep.erase_expected, requested
        );
    }
    println!(
        "\nresult: {} matched, {} only in the official capture (+OFF), {} only in UBMDF's plan (-UBMDF), {} bulk stream mismatch(es)",
        rep.matched, rep.only_captured, rep.only_planned, rep.bulk_mismatches
    );
    if captured
        .first()
        .is_some_and(|o| !matches!(o.op, WireOp::CtrlIn { request: 0x10, .. }))
    {
        println!(
            "note: the capture does not start at the 0x10 id read — it began mid-session, so the -UBMDF lines before its first op are expected."
        );
    }
    log::info!(
        "replay-check {}: {} matched, {} +OFF, {} -UBMDF, {} bulk mismatches",
        capture.display(),
        rep.matched,
        rep.only_captured,
        rep.only_planned,
        rep.bulk_mismatches
    );
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(n: usize, fill: u8, region: u32, blocks: u32, addr: u32) -> ComponentSpec {
        ComponentSpec {
            region_block_addr: region,
            block_count: blocks,
            write_addr: addr,
            data: vec![fill; n],
        }
    }

    /// Build a synthetic USBPcap capture that replays `ops` the way USBPcap logs them.
    fn usbpcap(ops: &[WireOp], comps: &[ComponentSpec], polls: usize) -> Vec<u8> {
        let mut recs: Vec<Vec<u8>> = Vec::new();
        let rec = |info: u8, ep: u8, xfer: u8, stage: Option<u8>, payload: &[u8], len: u32| {
            let hl: u16 = if stage.is_some() { 28 } else { 27 };
            let mut d = Vec::new();
            d.extend_from_slice(&hl.to_le_bytes());
            d.extend_from_slice(&0u64.to_le_bytes()); // irp id
            d.extend_from_slice(&0u32.to_le_bytes()); // status
            d.extend_from_slice(&0u16.to_le_bytes()); // function
            d.push(info);
            d.extend_from_slice(&1u16.to_le_bytes()); // bus
            d.extend_from_slice(&5u16.to_le_bytes()); // device
            d.push(ep);
            d.push(xfer);
            d.extend_from_slice(&len.to_le_bytes());
            if let Some(s) = stage {
                d.push(s);
            }
            d.extend_from_slice(payload);
            d
        };
        for op in ops {
            match op {
                WireOp::CtrlIn {
                    device,
                    request,
                    length,
                } => {
                    let rt = if *device { 0xc0 } else { 0xc1 };
                    let n = if *request == 0x38 { polls } else { 1 };
                    for _ in 0..n {
                        let setup = [
                            rt,
                            *request,
                            0,
                            0,
                            0,
                            0,
                            *length as u8,
                            (*length >> 8) as u8,
                        ];
                        recs.push(rec(0, 0x80, 2, Some(0), &setup, 8));
                        recs.push(rec(
                            1,
                            0x80,
                            2,
                            Some(3),
                            &vec![0u8; *length as usize],
                            *length as u32,
                        ));
                    }
                }
                WireOp::ErasePoll { .. } => {
                    for k in 0..polls {
                        let setup = [0xc1, 0x38, 0, 0, 0, 0, 15, 0];
                        recs.push(rec(0, 0x80, 2, Some(0), &setup, 8));
                        let mut r = vec![0u8; 15];
                        r[4..8].copy_from_slice(&145u32.to_le_bytes());
                        r[8..12].copy_from_slice(&(k as u32).to_le_bytes());
                        recs.push(rec(1, 0x80, 2, Some(3), &r, 15));
                    }
                }
                WireOp::CtrlOut { request, data } => {
                    let mut p = vec![0x41, *request, 0, 0, 0, 0, data.len() as u8, 0];
                    p.extend_from_slice(data);
                    recs.push(rec(0, 0x00, 2, Some(0), &p, 8 + data.len() as u32));
                    recs.push(rec(1, 0x00, 2, Some(3), &[], 0));
                }
                WireOp::Bulk { component, .. } => {
                    for ch in comps[*component].data.chunks(32768) {
                        recs.push(rec(0, 0x01, 3, None, ch, ch.len() as u32));
                        recs.push(rec(1, 0x01, 3, None, &[], ch.len() as u32));
                    }
                }
                WireOp::Zlp { .. } => {
                    recs.push(rec(0, 0x01, 3, None, &[], 0));
                    recs.push(rec(1, 0x01, 3, None, &[], 0));
                }
            }
        }
        let mut f = Vec::new();
        f.extend_from_slice(&0xa1b2c3d4u32.to_le_bytes());
        f.extend_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&4u16.to_le_bytes());
        f.extend_from_slice(&[0u8; 8]);
        f.extend_from_slice(&65535u32.to_le_bytes());
        f.extend_from_slice(&LINKTYPE_USBPCAP.to_le_bytes());
        for (i, r) in recs.iter().enumerate() {
            f.extend_from_slice(&(i as u32).to_le_bytes());
            f.extend_from_slice(&0u32.to_le_bytes());
            f.extend_from_slice(&(r.len() as u32).to_le_bytes());
            f.extend_from_slice(&(r.len() as u32).to_le_bytes());
            f.extend_from_slice(r);
        }
        f
    }

    fn write_tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("ubmdf-test-{}-{name}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn identical_capture_matches_plan() {
        let comps = vec![
            spec(70000, 0xaa, 0, 145, 0),
            spec(40000, 0xbb, 192, 30, 0x00c0_0000),
        ];
        let plan = protocol::planned_ops(&comps);
        let p = write_tmp("same.pcap", &usbpcap(&plan, &comps, 3));
        let rep = check(&p, &comps, None).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(rep.matched, plan.len());
        assert_eq!(
            (rep.only_planned, rep.only_captured, rep.bulk_mismatches),
            (0, 0, 0)
        );
        assert_eq!(rep.erase_expected, vec![145]);
    }

    #[test]
    fn detects_block_count_and_data_differences() {
        let comps = vec![
            spec(70000, 0xaa, 0, 145, 0),
            spec(40000, 0xbb, 192, 30, 0x00c0_0000),
        ];
        let plan = protocol::planned_ops(&comps);
        let p = write_tmp("diff.pcap", &usbpcap(&plan, &comps, 2));
        // Same capture, but UBMDF would request 144 blocks and send different data.
        let mut other = vec![
            spec(70000, 0xaa, 0, 144, 0),
            spec(40000, 0xbb, 192, 30, 0x00c0_0000),
        ];
        other[1].data[100] = 0;
        let rep = check(&p, &other, None).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(rep.only_planned, 2); // 0x37 with 144 blocks + the changed bulk stream
        assert_eq!(rep.only_captured, 2);
        assert_eq!(rep.bulk_mismatches, 1);
    }

    #[test]
    fn rejects_pcapng() {
        let p = write_tmp(
            "ng.pcap",
            &[
                0x0a, 0x0d, 0x0d, 0x0a, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        );
        let e = check(&p, &[], None).unwrap_err().to_string();
        std::fs::remove_file(&p).ok();
        assert!(e.contains("pcapng"), "{e}");
    }
}
