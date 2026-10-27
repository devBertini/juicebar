// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Hardware proof of the RGB economy through the real provider (plan stage 19; spec/PROTOCOL.md, "Lighting").
//!
//! `cargo run -p juicebar --example lighting_cycle -- <pid hex> [--trace <file>] [--hold <seconds>] [--saved <n>]`
//!
//! Drives `juicebar_core::logitech::Logitech` over the real transport (it does not start `JuiceBar.exe`): waits for
//! the device behind receiver or dongle `<pid>`, orders `economy_on`, lets the provider read the lighting back after
//! 3 s and 30 s, holds for `--hold` seconds, orders `economy_off` and waits for the restore to be read back. The
//! trace holds only the lighting frames (software ID 0x0C) the provider sent and the answers to them: no battery
//! traffic, no DeviceInfo, no serial, no path. `--saved <n>` hands the provider a level kept by an earlier run (a
//! crash recovery), as the app does from its settings.

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use juicebar_core::hidpp;
use juicebar_core::lighting::{LIGHT_SWID, LightingEvent, SavedLight};
use juicebar_core::logitech::Logitech;
use juicebar_core::types::{Action, DeviceKey, Endpoint, Now, ProviderEvent, TransportEvent};
use juicebar_hid::{EndpointId, Transport, TransportKind};

struct Out {
    start: Instant,
    file: Option<std::fs::File>,
}

impl Out {
    fn line(&mut self, tag: &str, text: &str) {
        let ms = self.start.elapsed().as_millis();
        let line = format!("{ms:>7} {tag} {text}");
        println!("{line}");
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
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

fn convert(ev: juicebar_hid::TransportEvent) -> TransportEvent {
    match ev {
        juicebar_hid::TransportEvent::Arrived(e) => TransportEvent::Arrived(core_endpoint(&e)),
        juicebar_hid::TransportEvent::Removed(id) => TransportEvent::Removed(id.0),
        juicebar_hid::TransportEvent::Report(id, b, _) => TransportEvent::Report(id.0, b),
    }
}

/// A lighting frame, ours or the answer to ours (the software ID is in byte 3, or byte 4 of an error answer).
fn is_lighting_frame(f: &[u8]) -> bool {
    f.len() >= 5
        && ((f[2] != 0xFF && f[3] & 0x0F == LIGHT_SWID)
            || (f[2] == 0xFF && f[4] & 0x0F == LIGHT_SWID))
}

struct Run {
    transport: Transport,
    provider: Logitech,
    start: Instant,
    out: Out,
    wake: Option<f64>,
    online: Vec<DeviceKey>,
    events: Vec<LightingEvent>,
}

impl Run {
    fn now(&self) -> Now {
        Now {
            wall: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64()),
            mono: self.start.elapsed().as_secs_f64(),
            utc_offset: 0,
        }
    }

    fn act(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Send(ep, bytes) => {
                    if is_lighting_frame(&bytes) {
                        self.out.line(">", &hidpp::hex(&bytes));
                    }
                    let _ = self.transport.send(EndpointId(ep), &bytes);
                }
                Action::Emit(ProviderEvent::Presence(k, p)) if p.online => {
                    self.online.push(k);
                }
                Action::Wake(t) => self.wake = Some(t),
                _ => {}
            }
        }
        for e in self.provider.take_lighting_events() {
            self.out.line("#", &format!("event {e:?}"));
            self.events.push(e);
        }
    }

    /// Feed the provider what arrives, and its timer, for `secs`.
    fn pump(&mut self, secs: f64) {
        let until = Instant::now() + Duration::from_secs_f64(secs);
        while Instant::now() < until {
            for ev in self.transport.drain() {
                if let juicebar_hid::TransportEvent::Report(_, bytes, _) = &ev
                    && is_lighting_frame(bytes)
                {
                    self.out.line("<", &hidpp::hex(bytes));
                }
                let now = self.now();
                let actions = self.provider.handle(convert(ev), now);
                self.act(actions);
            }
            if let Some(t) = self.wake
                && self.start.elapsed().as_secs_f64() >= t
            {
                self.wake = None;
                let now = self.now();
                let actions = self.provider.tick(now);
                self.act(actions);
            }
            sleep(Duration::from_millis(2));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(pid) = args.first() else {
        eprintln!("usage: lighting_cycle <pid hex> [--trace file] [--hold seconds] [--saved n]");
        std::process::exit(2);
    };
    let pid = u16::from_str_radix(pid.trim_start_matches("0x"), 16).expect("pid in hex");
    let hold: f64 = flag(&args, "--hold")
        .and_then(|h| h.parse().ok())
        .unwrap_or(0.0);
    let file = flag(&args, "--trace").map(|p| std::fs::File::create(p).expect("trace file"));
    let mut out = Out {
        start: Instant::now(),
        file,
    };
    out.line(
        "#",
        "juicebar lighting trace: '<ms since start> > sent | < received | # note'; lighting frames only",
    );

    let woken = Arc::new(AtomicBool::new(false));
    let w = woken.clone();
    let transport =
        Transport::start(Arc::new(move || w.store(true, Ordering::SeqCst))).expect("transport");
    let mut run = Run {
        transport,
        provider: Logitech::new(),
        start: Instant::now(),
        out,
        wake: None,
        online: Vec::new(),
        events: Vec::new(),
    };
    if let Some(n) = flag(&args, "--saved").and_then(|n| n.parse::<u16>().ok()) {
        // Needs the key: the G915 X, as the catalog names it.
        let key = DeviceKey("logitech-hidpp:40B5".into());
        run.provider
            .set_saved_lights(vec![(key, SavedLight { brightness: n })]);
        run.out
            .line("#", &format!("a level of {n} kept by an earlier run"));
    }

    sleep(Duration::from_millis(500));
    for ev in run.transport.drain() {
        if let juicebar_hid::TransportEvent::Arrived(e) = &ev
            && e.pid != pid
        {
            continue;
        }
        if matches!(&ev, juicebar_hid::TransportEvent::Report(..)) {
            continue;
        }
        let now = run.now();
        let actions = run.provider.handle(convert(ev), now);
        run.act(actions);
    }
    let begun = Instant::now();
    while run.online.is_empty() {
        run.pump(0.05);
        if begun.elapsed() > Duration::from_secs(20) {
            run.out.line("#", "the device never came online");
            return;
        }
    }
    let key = run.online[0].clone();
    run.out.line("#", &format!("online: {}", key.0));
    run.pump(2.0);

    let capable = run.provider.lighting_keys();
    run.out.line("#", &format!("lighting keys: {capable:?}"));
    run.out.line("#", "economy_on");
    let now = run.now();
    let actions = run.provider.economy_on(&key, now);
    run.act(actions);
    run.pump(33.0 + hold);

    run.out.line("#", "economy_off");
    let now = run.now();
    let actions = run.provider.economy_off(&key, now);
    run.act(actions);
    run.pump(4.0);
    run.out.line("#", &format!("events: {:?}", run.events));
    run.transport.stop();
}
