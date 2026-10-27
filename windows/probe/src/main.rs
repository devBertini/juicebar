// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! juicebar-probe: read-only HID++ probe of Logitech receivers and devices on Windows.
//!
//! Usage: juicebar-probe [--enumerate] [--pid <hex>] [--listen <seconds>] [--no-probe] [--trace <file>]

mod hid;
mod proto;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use hid::{Entry, Hub, Port};
use proto::*;

struct Trace {
    file: Option<BufWriter<File>>,
    t0: Instant,
}

impl Trace {
    fn line(&mut self, kind: char, text: &str) {
        let ms = self.t0.elapsed().as_millis();
        let line = format!("{ms:>6} {kind} {text}");
        println!("{line}");
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{line}");
        }
    }
    fn sent(&mut self, frame: &[u8]) {
        self.line('>', &hex(frame));
    }
    fn recv(&mut self, frame: &[u8]) {
        self.line('<', &hex(frame));
    }
    fn note(&mut self, text: &str) {
        self.line('#', text);
    }
}

enum Outcome {
    Answer(Vec<u8>),
    Error { v1: bool, code: u8 },
    Timeout,
    Failed(String),
}

fn exchange(hub: &mut Hub, tr: &mut Trace, req: &Request, timeout: Duration) -> Outcome {
    if !is_allowed(req) {
        return Outcome::Failed("refused: request is not on the probe's allowed list".into());
    }
    let frame = req.encode();
    if let Err(e) = hub.send(&frame) {
        return Outcome::Failed(e);
    }
    tr.sent(&frame);
    let deadline = Instant::now() + timeout;
    let mut foreign = 0u32;
    let result = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break Outcome::Timeout;
        }
        match hub.poll(left) {
            None => break Outcome::Timeout,
            Some((_, frame)) => match classify(req, &frame) {
                Reply::Answer(p) => {
                    tr.recv(&frame);
                    break Outcome::Answer(p);
                }
                Reply::Error { v1, code } => {
                    tr.recv(&frame);
                    break Outcome::Error { v1, code };
                }
                Reply::Ignore => foreign += 1,
            },
        }
    };
    if foreign > 0 {
        tr.note(&format!("ignored {foreign} frame(s) that were not the answer (other software's traffic or events)"));
    }
    result
}

fn describe_outcome(o: &Outcome) -> String {
    match o {
        Outcome::Answer(_) => "answer".into(),
        Outcome::Error { v1: true, code } => {
            format!("HID++ 1.0 error 0x{code:02x} ({})", error_name_v1(*code))
        }
        Outcome::Error { v1: false, code } => {
            format!("HID++ 2.0 error 0x{code:02x} ({})", error_name_v2(*code))
        }
        Outcome::Timeout => "no answer within the timeout".into(),
        Outcome::Failed(e) => format!("not sent: {e}"),
    }
}

#[derive(Default)]
struct Known {
    /// (device index, feature index) -> battery feature ID
    battery: HashMap<(u8, u8), u16>,
    online: Vec<u8>,
}

const TIMEOUT: Duration = Duration::from_millis(1500);
/// A sleeping keyboard needs about 1.5 s to answer the first request (measured), so the ping waits longer.
const PING_TIMEOUT: Duration = Duration::from_millis(4000);

/// True when no collection takes 7-byte reports: a direct device (device index 0xFF), long reports only.
fn long_only(hub: &Hub) -> bool {
    !hub.ports
        .iter()
        .any(|p| p.caps.output_len as usize == SHORT_LEN)
}

fn probe_receiver(hub: &mut Hub, tr: &mut Trace) -> Known {
    let mut known = Known::default();
    if long_only(hub) {
        tr.note(
            "direct device on a long-only collection: no receiver registers, device index 0xFF",
        );
        if probe_device(hub, tr, RECEIVER, &mut known) {
            known.online.push(RECEIVER);
        }
        return known;
    }
    tr.note("receiver: register 0x02 short read (connection state)");
    let o = exchange(
        hub,
        tr,
        &Request::register_read(RECEIVER, 0x02, [0; 3]),
        TIMEOUT,
    );
    match &o {
        Outcome::Answer(p) => tr.note(&format!("register 0x02: {}", decode_connection_state(p))),
        other => tr.note(&format!("register 0x02: {}", describe_outcome(other))),
    }
    for n in 1..=6u8 {
        let o = exchange(
            hub,
            tr,
            &Request::register_read_long(RECEIVER, 0xB5, [0x20 + n - 1, 0, 0]),
            TIMEOUT,
        );
        match &o {
            Outcome::Answer(p) => match decode_pairing(p) {
                Some((wpid, kind)) => tr.note(&format!(
                    "pairing slot {n}: wireless product ID {wpid:04x}, kind {} ({kind})",
                    kind_name(kind)
                )),
                None => tr.note(&format!("pairing slot {n}: unreadable answer")),
            },
            other => tr.note(&format!("pairing slot {n}: {}", describe_outcome(other))),
        }
        let o = exchange(
            hub,
            tr,
            &Request::register_read_long(RECEIVER, 0xB5, [0x40 + n - 1, 0, 0]),
            TIMEOUT,
        );
        match &o {
            Outcome::Answer(p) => tr.note(&format!(
                "slot {n} name register: \"{}\"",
                decode_register_name(p).unwrap_or_default()
            )),
            other => tr.note(&format!(
                "slot {n} name register: {}",
                describe_outcome(other)
            )),
        }
    }
    for idx in 1..=6u8 {
        tr.note(&format!("device index {idx}"));
        if probe_device(hub, tr, idx, &mut known) {
            known.online.push(idx);
        }
    }
    known
}

fn feature_request(hub: &Hub, idx: u8, fi: u8, func: u8, params: &[u8]) -> Request {
    let long = hub
        .ports
        .iter()
        .any(|p| p.caps.output_len as usize == LONG_LEN);
    Request::feature(idx, fi, func, params, long)
}

fn get_feature(hub: &mut Hub, tr: &mut Trace, idx: u8, id: u16) -> Option<(u8, u8, u8)> {
    let req = feature_request(hub, idx, 0, 0, &id.to_be_bytes());
    match exchange(hub, tr, &req, TIMEOUT) {
        Outcome::Answer(p) => {
            let (fi, t, v) = decode_get_feature(&p)?;
            if fi == 0 && id != 0 {
                tr.note(&format!(
                    "root.getFeature(0x{id:04x}): not supported (index 0)"
                ));
                None
            } else {
                tr.note(&format!(
                    "root.getFeature(0x{id:04x}): index {fi}, type 0x{t:02x}, version {v}"
                ));
                Some((fi, t, v))
            }
        }
        other => {
            tr.note(&format!(
                "root.getFeature(0x{id:04x}): {}",
                describe_outcome(&other)
            ));
            None
        }
    }
}

/// Returns true when the device answered the ping.
fn probe_device(hub: &mut Hub, tr: &mut Trace, idx: u8, known: &mut Known) -> bool {
    // A long-only collection (the G733 dongle, usage page 0xFF43) takes the ping as a long report.
    let ping = Request::feature(idx, 0, 1, &[0, 0, 0x5A], long_only(hub));
    match exchange(hub, tr, &ping, PING_TIMEOUT) {
        Outcome::Answer(p) => match decode_ping(&p) {
            Some((maj, min, echo)) => {
                tr.note(&format!(
                    "device {idx}: ONLINE, HID++ {maj}.{min}, ping byte echoed 0x{echo:02x}"
                ));
            }
            None => {
                tr.note(&format!("device {idx}: answered with a short frame"));
                return false;
            }
        },
        Outcome::Error { v1: true, code: 1 } => {
            tr.note(&format!(
                "device {idx}: ONLINE? answered the HID++ 2.0 ping with a HID++ 1.0 invalid-sub-id error: a HID++ 1.0 device"
            ));
            return true;
        }
        other => {
            tr.note(&format!(
                "device {idx}: OFFLINE or empty slot: {}",
                describe_outcome(&other)
            ));
            return false;
        }
    }

    // Feature list through FeatureSet.
    let Some((fs_index, _, _)) = get_feature(hub, tr, idx, F_FEATURE_SET) else {
        tr.note(&format!(
            "device {idx}: no FeatureSet, feature list not read"
        ));
        return true;
    };
    let count = match exchange(
        hub,
        tr,
        &feature_request(hub, idx, fs_index, 0, &[]),
        TIMEOUT,
    ) {
        Outcome::Answer(p) => p.first().copied().unwrap_or(0),
        other => {
            tr.note(&format!(
                "FeatureSet.getCount: {}",
                describe_outcome(&other)
            ));
            return true;
        }
    };
    tr.note(&format!(
        "FeatureSet.getCount: {count} features besides Root"
    ));
    let mut list: Vec<(u8, u16, u8, u8)> = vec![(0, 0x0000, 0, 0)];
    for i in 1..=count {
        match exchange(
            hub,
            tr,
            &feature_request(hub, idx, fs_index, 1, &[i]),
            TIMEOUT,
        ) {
            Outcome::Answer(p) => {
                if let Some((id, t, v)) = decode_feature_id(&p) {
                    list.push((i, id, t, v));
                }
            }
            other => {
                tr.note(&format!(
                    "FeatureSet.getFeatureID({i}): {}",
                    describe_outcome(&other)
                ));
                break;
            }
        }
    }
    tr.note(&format!(
        "device {idx} feature list ({} entries):",
        list.len()
    ));
    for (i, id, t, v) in &list {
        tr.note(&format!(
            "  index {i:>2}  0x{id:04x} {:<14} type 0x{t:02x} ({}) version {v}",
            feature_name(*id),
            feature_type_flags(*t)
        ));
    }

    // Name.
    if let Some((dn, _, _)) = get_feature(hub, tr, idx, F_DEVICE_NAME) {
        let len = match exchange(hub, tr, &feature_request(hub, idx, dn, 0, &[]), TIMEOUT) {
            Outcome::Answer(p) => p.first().copied().unwrap_or(0) as usize,
            other => {
                tr.note(&format!(
                    "DeviceName.getCount: {}",
                    describe_outcome(&other)
                ));
                0
            }
        };
        let mut name = Vec::new();
        while name.len() < len {
            match exchange(
                hub,
                tr,
                &feature_request(hub, idx, dn, 1, &[name.len() as u8]),
                TIMEOUT,
            ) {
                Outcome::Answer(p) => {
                    let take = (len - name.len()).min(p.len());
                    if take == 0 {
                        break;
                    }
                    name.extend_from_slice(&p[..take]);
                }
                other => {
                    tr.note(&format!(
                        "DeviceName.getDeviceName: {}",
                        describe_outcome(&other)
                    ));
                    break;
                }
            }
        }
        tr.note(&format!(
            "device {idx} name: \"{}\" (length {len})",
            decode_text(&name)
        ));
    }

    // Battery features: the root's answer is cross-checked with the list.
    for id in BATTERY_FEATURES {
        let root = get_feature(hub, tr, idx, id);
        let listed = list.iter().find(|e| e.1 == id).map(|e| e.0);
        if root.map(|r| r.0) != listed {
            tr.note(&format!(
                "MISMATCH root.getFeature and FeatureSet for 0x{id:04x}: {root:?} vs {listed:?}"
            ));
        }
        let Some((fi, _, version)) = root else {
            continue;
        };
        known.battery.insert((idx, fi), id);
        match id {
            F_BATTERY_STATUS => {
                read_battery(hub, tr, idx, fi, 0, id, version, decode_battery_status)
            }
            F_BATTERY_VOLTAGE => {
                read_battery(hub, tr, idx, fi, 0, id, version, decode_battery_voltage)
            }
            F_UNIFIED_BATTERY => {
                match exchange(hub, tr, &feature_request(hub, idx, fi, 0, &[]), TIMEOUT) {
                    Outcome::Answer(p) => tr.note(&format!(
                        "0x1004 function 0 (capabilities): {}",
                        describe_unified_capabilities(&p)
                    )),
                    other => tr.note(&format!(
                        "0x1004 function 0 (capabilities): {}",
                        describe_outcome(&other)
                    )),
                }
                read_battery(hub, tr, idx, fi, 1, id, version, decode_unified_status);
            }
            F_ADC_MEASUREMENT => read_battery(hub, tr, idx, fi, 0, id, version, decode_adc),
            _ => {}
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn read_battery(
    hub: &mut Hub,
    tr: &mut Trace,
    idx: u8,
    fi: u8,
    func: u8,
    id: u16,
    version: u8,
    decode: fn(&[u8]) -> Option<Battery>,
) {
    match exchange(hub, tr, &feature_request(hub, idx, fi, func, &[]), TIMEOUT) {
        Outcome::Answer(p) => match decode(&p) {
            Some(b) => tr.note(&format!(
                "battery 0x{id:04x} (feature version {version}) function {func}: {} [raw {}]",
                format_battery(&b),
                hex(&p[..p.len().min(6)])
            )),
            None => tr.note(&format!("battery 0x{id:04x}: answer too short to decode")),
        },
        other => tr.note(&format!(
            "battery 0x{id:04x} function {func}: {}",
            describe_outcome(&other)
        )),
    }
}

fn listen(hub: &mut Hub, tr: &mut Trace, secs: u64, known: &Known) {
    tr.note(&format!(
        "listening {secs} s, every input report on every vendor collection is printed"
    ));
    let lookup = |d: u8, f: u8| known.battery.get(&(d, f)).copied();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut n = 0u32;
    while Instant::now() < deadline {
        let left = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(500));
        if let Some((pi, frame)) = hub.poll(left) {
            n += 1;
            tr.recv(&frame);
            let label = hub.ports[pi].label.clone();
            tr.note(&format!("{} [{}]", describe_event(&frame, &lookup), label));
        }
        if hub.ports.iter().all(|p| p.dead) {
            tr.note("all collections died (device removed?)");
            break;
        }
    }
    tr.note(&format!("listen ended, {n} input report(s) seen"));
}

fn win32_name(code: u32) -> &'static str {
    match code {
        5 => "access denied",
        32 => "sharing violation",
        2 => "not found",
        _ => "other",
    }
}

fn print_enumeration(entries: &[Entry]) {
    println!("HID interfaces with VID 046D: {}", entries.len());
    for e in entries {
        let open = match &e.open_rw {
            Ok(()) => "open read+write shared: OK".to_string(),
            Err(c) => format!(
                "open read+write shared: FAILED, Win32 error {c} ({})",
                win32_name(*c)
            ),
        };
        match &e.caps {
            Some(c) => println!(
                "  VID {:04X} PID {:04X} {:<11} usage page 0x{:04X} usage 0x{:04X} in {:>2} out {:>2} feature {:>2} | {} | \"{}\"",
                e.vid,
                e.pid,
                e.label(),
                c.usage_page,
                c.usage,
                c.input_len,
                c.output_len,
                c.feature_len,
                open,
                e.product
            ),
            None => println!(
                "  VID {:04X} PID {:04X} {:<11} caps not readable | {}",
                e.vid,
                e.pid,
                e.label(),
                open
            ),
        }
    }
}

struct Opts {
    enumerate_only: bool,
    pid: Option<u16>,
    listen: Option<u64>,
    no_probe: bool,
    trace: Option<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts {
        enumerate_only: false,
        pid: None,
        listen: None,
        no_probe: false,
        trace: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--enumerate" => o.enumerate_only = true,
            "--no-probe" => o.no_probe = true,
            "--pid" => {
                let v = args.next().ok_or("--pid needs a hex value")?;
                o.pid = Some(u16::from_str_radix(v.trim_start_matches("0x"), 16).map_err(|e| e.to_string())?);
            }
            "--listen" => {
                let v: u64 = args.next().ok_or("--listen needs seconds")?.parse().map_err(|_| "bad seconds")?;
                if v > 120 {
                    return Err("--listen is capped at 120 seconds".into());
                }
                o.listen = Some(v);
            }
            "--trace" => o.trace = Some(args.next().ok_or("--trace needs a file")?),
            "--help" | "-h" => return Err("usage: juicebar-probe [--enumerate] [--pid <hex>] [--listen <s>] [--no-probe] [--trace <file>]".into()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(o)
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let entries = hid::enumerate(hid::LOGITECH_VID);
    print_enumeration(&entries);
    if opts.enumerate_only {
        return;
    }

    // HID++ collections (usage page 0xFF00 on receivers, 0xFF43 on direct devices) that opened and carry reports of 7 bytes or more.
    let mut pids: Vec<u16> = entries
        .iter()
        .filter(|e| {
            e.open_rw.is_ok()
                && matches!(&e.caps, Some(c) if is_hidpp_page(c.usage_page) && c.input_len >= 7)
                && opts.pid.is_none_or(|p| p == e.pid)
        })
        .map(|e| e.pid)
        .collect();
    pids.dedup();
    if pids.is_empty() {
        println!("no HID++ collection opened for the requested device");
        return;
    }

    let file = opts
        .trace
        .as_ref()
        .map(|p| BufWriter::new(File::create(p).expect("cannot create the trace file")));
    let mut tr = Trace {
        file,
        t0: Instant::now(),
    };
    tr.note("juicebar-probe trace: '<ms since start> > sent | < received | # note'; frames are full HID++ reports");
    for pid in pids {
        let ports: Vec<Port> = entries
            .iter()
            .filter(|e| {
                e.pid == pid
                    && e.open_rw.is_ok()
                    && matches!(&e.caps, Some(c) if is_hidpp_page(c.usage_page) && c.input_len >= 7)
            })
            .filter_map(|e| Port::open(e).ok())
            .collect();
        tr.note(&format!(
            "receiver VID 046D PID {pid:04X}: {} vendor collection(s) opened: {}",
            ports.len(),
            ports
                .iter()
                .map(|p| format!(
                    "{} in {} out {}",
                    p.label, p.caps.input_len, p.caps.output_len
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
        let mut hub = Hub { ports };
        let known = if opts.no_probe {
            Known::default()
        } else {
            probe_receiver(&mut hub, &mut tr)
        };
        if let Some(s) = opts.listen {
            listen(&mut hub, &mut tr, s, &known);
        }
    }
}
