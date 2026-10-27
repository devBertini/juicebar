// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Hardware test: needs the owner's receivers (046D:C539, 046D:C547) and optionally the G733
//! dongle (046D:0AFE) plugged in. `cargo test -p juicebar-hid -- --ignored`

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use juicebar_hid::{Endpoint, EndpointId, SendError, Transport, TransportEvent};

/// HID++ 2.0 root ping: report, device index, feature 0, function 1 | software ID 0x0D, params.
fn ping(report_id: u8, device_index: u8) -> [u8; 7] {
    [report_id, device_index, 0x00, 0x1D, 0x00, 0x00, 0xAA]
}

/// An answer to our ping: same device, feature, function and software ID, and the echoed byte.
fn is_ping_answer(bytes: &[u8], device_index: u8) -> bool {
    bytes.len() >= 7
        && (bytes[0] == 0x10 || bytes[0] == 0x11)
        && bytes[1] == device_index
        && bytes[2] == 0x00
        && bytes[3] == 0x1D
        && bytes[6] == 0xAA
}

struct Harness {
    transport: Transport,
    wake: Receiver<()>,
    endpoints: HashMap<EndpointId, Endpoint>,
    /// Reports not yet consumed by a wait, in arrival order.
    reports: Vec<(EndpointId, Vec<u8>)>,
}

impl Harness {
    /// Take what the transport has queued; the endpoints map follows arrivals and removals.
    fn pump(&mut self) {
        for e in self.transport.drain() {
            match e {
                TransportEvent::Arrived(ep) => {
                    self.endpoints.insert(ep.id, ep);
                }
                TransportEvent::Removed(id) => {
                    self.endpoints.remove(&id);
                }
                TransportEvent::Report(id, bytes, _at) => self.reports.push((id, bytes)),
            }
        }
    }

    /// Pump until `done` is true or `limit` passes; returns whether `done` held.
    fn wait_until(&mut self, limit: Duration, mut done: impl FnMut(&mut Harness) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            self.pump();
            if done(self) {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match self.wake.recv_timeout(left) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    fn with_pid(&self, pid: u16) -> Vec<Endpoint> {
        let mut v: Vec<Endpoint> = self
            .endpoints
            .values()
            .filter(|e| e.pid == pid)
            .cloned()
            .collect();
        v.sort_by_key(|e| e.id);
        v
    }
}

enum Outcome {
    /// The ping answer matched device, feature, function and software ID.
    Answered,
    /// The receiver answered a HID++ 1.0 error for the device index (device not linked).
    Offline,
    Silent,
}

fn is_offline_error(bytes: &[u8], device_index: u8) -> bool {
    bytes.len() >= 7
        && bytes[0] == 0x10
        && bytes[1] == device_index
        && bytes[2] == 0x8F
        && bytes[3] == 0x00
        && bytes[4] == 0x1D
}

/// Send the root ping to the endpoints of `pid` and wait up to 4 s for its answer.
fn ping_group(h: &mut Harness, pid: u16, device_index: u8, label: &str) -> Outcome {
    let group = h.with_pid(pid);
    // The short request goes to the short collection when there is one, else the long one
    // takes it padded; a receiver's answer comes on the long collection.
    let (target, request) = match group.iter().find(|e| e.output_len == 7) {
        Some(e) => (e.clone(), ping(0x10, device_index).to_vec()),
        None => {
            let e = group
                .iter()
                .find(|e| e.output_len == 20)
                .unwrap_or_else(|| panic!("{label}: no 7- or 20-byte output collection"))
                .clone();
            (e, ping(0x11, device_index).to_vec())
        }
    };
    h.pump();
    h.reports.clear();
    let sent = Instant::now();
    h.transport
        .send(target.id, &request)
        .unwrap_or_else(|e| panic!("{label}: send failed: {e}"));
    let ids: Vec<EndpointId> = group.iter().map(|e| e.id).collect();
    let mut found = None;
    h.wait_until(Duration::from_secs(4), |h| {
        found = h.reports.iter().find_map(|(id, b)| {
            (ids.contains(id)
                && (is_ping_answer(b, device_index) || is_offline_error(b, device_index)))
            .then(|| (*id, b.clone()))
        });
        found.is_some()
    });
    let Some((id, bytes)) = found else {
        return Outcome::Silent;
    };
    println!(
        "{label}: answer on {:?} after {} us: {:02x?}",
        id,
        sent.elapsed().as_micros(),
        &bytes[..7]
    );
    if is_ping_answer(&bytes, device_index) {
        Outcome::Answered
    } else {
        Outcome::Offline
    }
}

#[test]
#[ignore = "needs the owner's Logitech receivers and G HUB running"]
fn receivers_and_dongle_answer_a_root_ping() {
    let (tx, wake) = channel();
    let tx = Mutex::new(tx);
    let transport = Transport::start(Arc::new(move || {
        let _ = tx.lock().unwrap().send(());
    }))
    .expect("transport starts");
    let mut h = Harness {
        transport,
        wake,
        endpoints: HashMap::new(),
        reports: Vec::new(),
    };

    // Up to 2 s for the arrivals.
    h.wait_until(Duration::from_secs(2), |_| false);
    for e in h.endpoints.values() {
        println!(
            "arrived {:?} {:04X}:{:04X} page {:#06X} usage {:#06X} in {} out {} {:?} {:?}",
            e.id, e.vid, e.pid, e.usage_page, e.usage, e.input_len, e.output_len, e.product, e.kind
        );
    }
    assert!(
        h.endpoints.values().all(|e| e.vid == 0x046D),
        "only VID 046D"
    );
    assert!(
        h.endpoints
            .values()
            .all(|e| e.usage_page == 0xFF00 || e.usage_page == 0xFF43),
        "only the vendor collections"
    );

    // (pid, device index, label): receivers talk to slot 1; the G733 dongle is the device (0xFF).
    // A receiver whose device is switched off or on its cable answers a HID++ 1.0 error
    // (device offline); the same device on its cable is then pinged as its own USB endpoint
    // (G915 X: 046D:C356, device index 0xFF).
    let targets = [
        (0xC539u16, 0x01u8, "C539 receiver (G703)"),
        (0xC547, 0x01, "C547 receiver (G915 X)"),
        (0x0AFE, 0xFF, "0AFE dongle (G733)"),
    ];
    let mut answered = 0;
    for (pid, device_index, label) in targets {
        if h.with_pid(pid).is_empty() {
            assert_eq!(pid, 0x0AFE, "{label} must be present");
            println!("{label}: not present, skipped");
            continue;
        }
        match ping_group(&mut h, pid, device_index, label) {
            Outcome::Answered => answered += 1,
            Outcome::Offline if pid == 0xC547 && !h.with_pid(0xC356).is_empty() => {
                println!("{label}: device offline on the receiver, on its cable instead");
                assert!(
                    matches!(
                        ping_group(&mut h, 0xC356, 0xFF, "C356 G915 X cable"),
                        Outcome::Answered
                    ),
                    "C356 must answer"
                );
                answered += 1;
            }
            Outcome::Offline => panic!("{label}: device offline, no cable endpoint"),
            Outcome::Silent => panic!("{label}: no answer within 4 s"),
        }
    }
    assert!(answered >= 2, "both receivers' devices must answer");

    h.transport.stop();
    assert_eq!(
        h.transport.send(EndpointId(1), &[0x10]),
        Err(SendError::Stopped)
    );
    h.transport.stop(); // idempotent
}
