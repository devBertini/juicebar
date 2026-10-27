// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Hardware proof of the RGB economy (plan stage 19; spec/PROTOCOL.md, "Lighting").
//!
//! `cargo run -p juicebar --example lighting -- read <pid> [--trace <file>]`
//!   Sends only read requests to the lighting features of the device behind receiver or dongle `<pid>` (hex) and
//!   prints every answer. The requests are built here, outside the provider: this is how the lighting reads were
//!   measured before they were allowed in the core (`docs/research/probe-windows.md`, "Lighting").
//!
//! `cargo run -p juicebar --example lighting -- raw <pid> <feature hex> <function> [param hex bytes]`
//!   One request to a feature found by root getFeature; the caller passes getters only. The provider's own cycle is
//! `lighting_cycle`.

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use juicebar_core::hidpp::{self, Reply, Request};
use juicebar_core::types::Endpoint;
use juicebar_hid::{EndpointId, Transport, TransportKind};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(mode), Some(pid)) = (args.first(), args.get(1)) else {
        eprintln!("usage: lighting read <pid hex> [--trace file]");
        std::process::exit(2);
    };
    let pid = u16::from_str_radix(pid.trim_start_matches("0x"), 16).expect("pid in hex");
    let trace = flag(&args, "--trace");
    let mut out = Out::new(trace);
    match mode.as_str() {
        "read" => read(pid, &mut out),
        "raw" => raw(pid, &args[2..], &mut out),
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    }
    out.finish();
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// Console and trace file: `<ms since start> > sent`, `< received`, `# note`.
struct Out {
    start: Instant,
    file: Option<std::fs::File>,
}

impl Out {
    fn new(path: Option<String>) -> Out {
        let file = path.map(|p| std::fs::File::create(p).expect("trace file"));
        let mut out = Out {
            start: Instant::now(),
            file,
        };
        out.raw(
            "# juicebar lighting trace: '<ms since start> > sent | < received | # note'; frames are full HID++ reports",
        );
        out
    }

    fn raw(&mut self, line: &str) {
        println!("{line}");
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{line}");
        }
    }

    fn at(&mut self, tag: &str, text: &str) {
        let ms = self.start.elapsed().as_millis();
        self.raw(&format!("{ms:>7} {tag} {text}"));
    }

    fn note(&mut self, text: &str) {
        self.at("#", text);
    }

    fn finish(&mut self) {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
        }
    }
}

fn core_endpoint(e: &juicebar_hid::Endpoint) -> Endpoint {
    Endpoint {
        id: e.id.0,
        vid: e.vid,
        pid: e.pid,
        usage_page: e.usage_page,
        usage: e.usage,
        input_len: e.input_len,
        output_len: e.output_len,
        product: e.product.clone(),
        bluetooth: e.kind == TransportKind::Bluetooth,
    }
}

fn start_transport() -> Transport {
    let flag = Arc::new(AtomicBool::new(false));
    let f = flag.clone();
    Transport::start(Arc::new(move || f.store(true, Ordering::SeqCst))).expect("transport")
}

// ------------------------------------------------------------------------------------------------ read

struct Rig {
    transport: Transport,
    long: EndpointId,
    pid: u16,
}

impl Rig {
    /// One request and its answer (or the error / timeout), matched by the core's own rule.
    fn ask(&mut self, out: &mut Out, req: &Request, wait: Duration) -> Option<Reply> {
        let frame = req.encode();
        out.at(">", &hidpp::hex(&frame));
        self.transport.send(self.long, &frame).expect("send");
        let until = Instant::now() + wait;
        while Instant::now() < until {
            for ev in self.transport.drain() {
                if let juicebar_hid::TransportEvent::Report(id, bytes, _) = ev
                    && id == self.long
                {
                    match hidpp::classify(req, &bytes) {
                        Reply::Ignore => {}
                        reply => {
                            out.at("<", &hidpp::hex(&bytes));
                            return Some(reply);
                        }
                    }
                }
            }
            sleep(Duration::from_millis(2));
        }
        out.note("timeout");
        None
    }

    fn call(&mut self, out: &mut Out, dev: u8, fi: u8, f: u8, params: &[u8]) -> Option<Vec<u8>> {
        let req = Request::feature(dev, fi, f, params, true);
        match self.ask(out, &req, Duration::from_millis(1500))? {
            Reply::Answer(p) => Some(p),
            Reply::Error { code, .. } => {
                out.note(&format!("error answer, code {code:#04x}"));
                None
            }
            Reply::Ignore => None,
        }
    }

    fn feature(&mut self, out: &mut Out, dev: u8, id: u16) -> Option<u8> {
        let p = self.call(out, dev, 0, 0, &id.to_be_bytes())?;
        let (index, _, version) = hidpp::decode_get_feature(&p)?;
        out.note(&format!(
            "root.getFeature({id:#06x}): index {index}, version {version}"
        ));
        (index != 0).then_some(index)
    }
}

fn open(pid: u16) -> (Rig, Vec<Endpoint>) {
    let transport = start_transport();
    sleep(Duration::from_millis(600));
    let mut eps = Vec::new();
    for ev in transport.drain() {
        if let juicebar_hid::TransportEvent::Arrived(e) = ev
            && e.pid == pid
        {
            eps.push(e);
        }
    }
    let long = eps
        .iter()
        .find(|e| e.output_len == 20)
        .unwrap_or_else(|| panic!("no long HID++ collection for pid {pid:04x}"))
        .id;
    let core: Vec<Endpoint> = eps.iter().map(core_endpoint).collect();
    (
        Rig {
            transport,
            long,
            pid,
        },
        core,
    )
}

fn read(pid: u16, out: &mut Out) {
    let (mut rig, _) = open(pid);
    let dongle = matches!(rig.pid, 0x0AFE);
    // A receiver's device is slot 1 on both receivers measured; a dongle or a cable is index 0xFF.
    let dev = if dongle || rig.pid == 0xC356 { 0xFF } else { 1 };
    out.note(&format!("pid {:04x}, device index {dev:#04x}", rig.pid));
    // Wake the device: a ping.
    let ping = Request::feature(dev, 0, 1, &[0, 0, hidpp::PING_BYTE], true);
    rig.ask(out, &ping, Duration::from_secs(4));

    if let Some(fi) = rig.feature(out, dev, 0x8040) {
        out.note("0x8040 BrightnessControl: fn0 info, fn1 brightness, fn3 on/off state");
        rig.call(out, dev, fi, 0, &[]);
        rig.call(out, dev, fi, 1, &[]);
        rig.call(out, dev, fi, 3, &[]);
    }
    if let Some(fi) = rig.feature(out, dev, 0x8070) {
        out.note(
            "0x8070 ColorLedEffects: fn0 info, fn1 zone info, fn2 effect info, fn14 zone effect",
        );
        let info = rig.call(out, dev, fi, 0, &[]);
        let zones = info.as_ref().and_then(|p| p.first().copied()).unwrap_or(0);
        for z in 0..zones {
            let zi = rig.call(out, dev, fi, 1, &[z, 0xFF, 0]);
            let effects = zi.as_ref().and_then(|p| p.get(3).copied()).unwrap_or(0);
            for e in 0..effects {
                rig.call(out, dev, fi, 2, &[z, e, 0]);
            }
            rig.call(out, dev, fi, 14, &[z]);
        }
    }
    if let Some(fi) = rig.feature(out, dev, 0x8071) {
        out.note(
            "0x8071 RgbEffects: fn0 info, fn0 zone info, fn0 effect info, fn5 SW control (get)",
        );
        let info = rig.call(out, dev, fi, 0, &[0xFF, 0xFF, 0]);
        let zones = info.as_ref().and_then(|p| p.get(2).copied()).unwrap_or(0);
        for z in 0..zones {
            let zi = rig.call(out, dev, fi, 0, &[z, 0xFF, 0]);
            let effects = zi.as_ref().and_then(|p| p.get(4).copied()).unwrap_or(0);
            for e in 0..effects {
                rig.call(out, dev, fi, 0, &[z, e, 0]);
            }
        }
        rig.call(out, dev, fi, 5, &[0]);
    }
}

/// `raw <pid> <feature hex> <fn> [param hex bytes...]`: one request to a feature found by root.getFeature. For
/// reads that `read` does not cover; the caller chooses the function, so only getters are ever passed.
fn raw(pid: u16, rest: &[String], out: &mut Out) {
    let (mut rig, _) = open(pid);
    let dongle = matches!(rig.pid, 0x0AFE);
    let dev = if dongle || rig.pid == 0xC356 { 0xFF } else { 1 };
    let feature = u16::from_str_radix(&rest[0], 16).expect("feature hex");
    let function: u8 = rest[1].parse().expect("function number");
    let params: Vec<u8> = rest[2..]
        .iter()
        .take_while(|a| !a.starts_with("--"))
        .map(|p| u8::from_str_radix(p, 16).expect("param hex"))
        .collect();
    let ping = Request::feature(dev, 0, 1, &[0, 0, hidpp::PING_BYTE], true);
    rig.ask(out, &ping, Duration::from_secs(4));
    if let Some(fi) = rig.feature(out, dev, feature) {
        rig.call(out, dev, fi, function, &params);
    }
}
