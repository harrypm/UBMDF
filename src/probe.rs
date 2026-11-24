//! `ubmdf probe`: look for a flash read-back command on the service port.
//!
//! Sends ONLY vendor control IN requests (device -> host; the host sends no data
//! stage) over a bRequest range, to both the device and interface recipients,
//! plus optionally one bulk IN read on EP 0x82. Every request is logged before
//! it is sent and its result after, so if the device ever stops answering, the
//! log names the last request. Nothing is erased or written; with `--session`
//! the session is opened with 0x31 and closed with 0x12 exactly like `info`.
//! Results go to a JSON file for later comparison.

use crate::protocol::{self, ProbeResult};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

pub struct ProbeOptions {
    pub session: bool,
    pub from: u8,
    pub to: u8,
    pub value: u16,
    pub index: u16,
    /// wLength values to try for every request (the device STALLs any request whose
    /// wLength is not exactly the expected size — verified on 0x10 and 0x30).
    pub lengths: Vec<u16>,
    pub bulk_in: bool,
    pub out: Option<PathBuf>,
}

#[derive(Serialize)]
struct ProbeEntry {
    bm_request_type: String,
    request: String,
    value: u16,
    index: u16,
    length: u16,
    known_as: &'static str,
    #[serde(flatten)]
    result: ProbeResult,
}

#[derive(Serialize)]
struct ProbeDoc {
    timestamp_unix: u64,
    session: bool,
    nonce: String,
    state_before: String,
    state_in_session: Option<String>,
    state_after: String,
    entries: Vec<ProbeEntry>,
    bulk_in_ep82: Option<ProbeResult>,
}

fn state_hex(iface: &nusb::Interface) -> String {
    protocol::read_state(iface)
        .map(|s| protocol::hex(&s))
        .unwrap_or_else(|e| format!("error: {e:#}"))
}

pub fn run(o: &ProbeOptions) -> Result<PathBuf> {
    if o.from > o.to {
        bail!("--from must be <= --to");
    }
    let mut dev = protocol::open()?;
    let (nonce, id_byte) = protocol::read_id(&dev.iface)?;
    log::info!(
        "probe: device nonce {} id byte {id_byte:#04x}",
        protocol::hex(&nonce)
    );
    let state_before = state_hex(&dev.iface);
    log::info!("probe: 0x30 state before: {state_before}");

    let mut state_in_session = None;
    if o.session {
        protocol::begin_session(&mut dev)?;
        let s = state_hex(&dev.iface);
        log::info!("probe: 0x31 begin OK, state {s}");
        state_in_session = Some(s);
    }

    let mut entries = Vec::new();
    let mut timeouts_in_row = 0;
    let mut aborted: Option<String> = None;
    if o.lengths.is_empty() {
        bail!("no wLength values given");
    }
    'sweep: for request in o.from..=o.to {
        for device in [true, false] {
            for &length in &o.lengths {
                let rt = if device { 0xc0u8 } else { 0xc1 };
                log::info!(
                    "probe: -> IN {rt:02x}/0x{request:02x} value {:#06x} index {:#06x} len {length}",
                    o.value,
                    o.index
                );
                let result =
                    protocol::probe_in(&dev.iface, device, request, o.value, o.index, length);
                log::info!("probe: <- IN {rt:02x}/0x{request:02x} len {length}: {result:?}");
                let stop = match &result {
                    ProbeResult::Disconnected => Some("device disconnected".to_string()),
                    ProbeResult::Timeout => {
                        timeouts_in_row += 1;
                        (timeouts_in_row >= 3)
                            .then(|| "3 timeouts in a row — device not answering".to_string())
                    }
                    _ => {
                        timeouts_in_row = 0;
                        None
                    }
                };
                entries.push(ProbeEntry {
                    bm_request_type: format!("{rt:02x}"),
                    request: format!("{request:02x}"),
                    value: o.value,
                    index: o.index,
                    length,
                    known_as: protocol::request_name(request),
                    result,
                });
                if let Some(why) = stop {
                    log::warn!("probe: stopping sweep at 0x{request:02x}: {why}");
                    aborted = Some(why);
                    break 'sweep;
                }
            }
        }
    }

    let bulk_in_ep82 = if o.bulk_in && aborted.is_none() {
        log::info!("probe: -> bulk IN ep 0x82, 512 B, 2 s timeout");
        let r = protocol::probe_bulk_in(&dev.iface, 512, Duration::from_secs(2))?;
        log::info!("probe: <- bulk IN ep 0x82: {r:?}");
        Some(r)
    } else {
        None
    };

    let state_after = state_hex(&dev.iface);
    log::info!("probe: 0x30 state after sweep: {state_after}");
    if o.session && aborted.is_none() {
        crate::ops::close_session(&dev)?;
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let answered: Vec<&ProbeEntry> = entries
        .iter()
        .filter(|e| matches!(e.result, ProbeResult::Data(_)))
        .collect();
    let doc = ProbeDoc {
        timestamp_unix: ts,
        session: o.session,
        nonce: protocol::hex(&nonce),
        state_before: state_before.clone(),
        state_in_session,
        state_after: state_after.clone(),
        entries: Vec::new(),
        bulk_in_ep82,
    };
    let path = o
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("ubmdf-probe-{ts}.json")));

    println!(
        "probed bRequest 0x{:02x}..=0x{:02x} (value {:#06x}, index {:#06x}, {} wLength values), {} requests{}",
        o.from,
        o.to,
        o.value,
        o.index,
        o.lengths.len(),
        entries.len(),
        if o.session {
            ", inside a 0x31 session"
        } else {
            ", no session"
        }
    );
    println!("requests the device answered ({}):", answered.len());
    for e in &answered {
        if let ProbeResult::Data(d) = &e.result {
            println!(
                "  {}/0x{} len {} ({}) -> {} B {}",
                e.bm_request_type,
                e.request,
                e.length,
                e.known_as,
                d.len() / 2,
                d
            );
        }
    }
    let count = |f: fn(&ProbeResult) -> bool| entries.iter().filter(|e| f(&e.result)).count();
    println!(
        "stalled: {}, timed out: {}, other errors: {}",
        count(|r| matches!(r, ProbeResult::Stall)),
        count(|r| matches!(r, ProbeResult::Timeout)),
        count(|r| matches!(r, ProbeResult::Error(_)))
    );
    if let Some(b) = &doc.bulk_in_ep82 {
        println!("bulk IN ep 0x82: {b:?}");
    }
    println!(
        "0x30 state: before {state_before}, after {state_after}{}",
        if state_before == state_after {
            ""
        } else {
            "  (CHANGED)"
        }
    );
    if let Some(why) = &aborted {
        println!("sweep stopped early: {why}");
    }

    let doc = ProbeDoc { entries, ..doc };
    let json = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    println!("results: {}", path.display());
    log::info!("probe results written: {}", path.display());
    if let Some(why) = aborted {
        bail!(
            "probe stopped early: {why} (session not closed — run `ubmdf reset` once the device is back)"
        );
    }
    Ok(path)
}
