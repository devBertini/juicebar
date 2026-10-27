// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! A test rig for the Logitech provider: a fake device answering from a probe trace (spec/fixtures), the shell's
//! one-shot timer, and a log of everything sent and emitted.

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};

use juicebar_core::hidpp::{self, Request};
use juicebar_core::logitech::Logitech;
use juicebar_core::types::{Action, Endpoint, Now, ProviderEvent, TransportEvent};

pub const G703_TRACE: &str = include_str!("../../../../spec/fixtures/windows-c539-g703.hidtrace");
pub const G703_LISTEN: &str =
    include_str!("../../../../spec/fixtures/windows-c539-g703-listen.hidtrace");
pub const G915X_TRACE: &str = include_str!("../../../../spec/fixtures/windows-c547-g915x.hidtrace");
pub const G915X_LISTEN: &str =
    include_str!("../../../../spec/fixtures/windows-c547-g915x-listen.hidtrace");
pub const G915X_CABLE: &str =
    include_str!("../../../../spec/fixtures/windows-c356-g915x-usb.hidtrace");
pub const G733_TRACE: &str = include_str!("../../../../spec/fixtures/windows-0afe-g733.hidtrace");

pub fn h(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// A short (7 byte) report from a hex prefix, zero padded.
pub fn short(prefix: &str) -> String {
    format!("{prefix:0<14}")
}

/// A long (20 byte) report from a hex prefix, zero padded.
pub fn long(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

/// The device's side of a trace: each frame the probe sent beside the answer it got.
pub struct Fake {
    map: HashMap<String, String>,
    pub short_ep: u32,
    pub long_ep: u32,
    /// A direct device that answers any index: the trace's index (1) stands for 0xFF.
    pub alias: Option<u8>,
    /// The receiver's register 0x00 (notification flags).
    pub reg0: [u8; 3],
    pub reg0_writes: Vec<[u8; 3]>,
    pub mute_all: bool,
    /// Device indexes whose requests go unanswered.
    pub mute: Vec<u8>,
    /// Device indexes whose ping gets an error answer.
    pub refuse: Vec<u8>,
    /// Answer only this many HID++ 2.0 requests, then stay silent.
    pub budget: Option<usize>,
    /// Frames the provider sent that the trace has no answer for.
    pub unknown: Vec<String>,
    /// The lighting of the device (BrightnessControl), answered by software ID 0x0C requests.
    pub light: Option<LightFake>,
}

/// A device's BrightnessControl feature, as the G915 X answered it (`spec/PROTOCOL.md`, "Lighting").
pub struct LightFake {
    /// Feature index of 0x8040 (14 behind the receiver, 13 on the cable).
    pub index: u8,
    pub brightness: u16,
    pub min: u16,
    pub max: u16,
    /// Every level the provider wrote.
    pub writes: Vec<u16>,
    /// Answer root getFeature(0x8040) with "not supported".
    pub no_feature: bool,
    /// Answer every lighting request with an error.
    pub refuse: bool,
    /// Stay silent to lighting requests.
    pub silent: bool,
    /// Every lighting request received, in order.
    pub requests: Vec<String>,
}

impl LightFake {
    pub fn g915x(index: u8, brightness: u16) -> LightFake {
        LightFake {
            index,
            brightness,
            min: 0,
            max: 100,
            writes: Vec::new(),
            no_feature: false,
            refuse: false,
            silent: false,
            requests: Vec::new(),
        }
    }

    fn answer(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        self.requests.push(hidpp::hex(frame));
        if self.silent {
            return None;
        }
        let (dev, b2, b3) = (frame[1], frame[2], frame[3]);
        let function = b3 >> 4;
        let mut a = vec![0x11, dev, b2, b3];
        if self.refuse {
            return Some(
                vec![0x11, dev, 0xFF, b2, b3, 0x05]
                    .into_iter()
                    .chain([0; 14])
                    .collect(),
            );
        }
        if b2 == 0 && function == 0 {
            let feature = u16::from_be_bytes([frame[4], frame[5]]);
            let index = if feature == 0x8040 && !self.no_feature {
                self.index
            } else {
                0
            };
            a.extend([index, 0, 0]);
        } else if b2 == self.index {
            match function {
                0 => {
                    a.extend(self.max.to_be_bytes());
                    a.extend([5, 3]);
                    a.extend(self.min.to_be_bytes());
                }
                1 => a.extend(self.brightness.to_be_bytes()),
                2 => {
                    self.brightness = u16::from_be_bytes([frame[4], frame[5]]);
                    self.writes.push(self.brightness);
                    a.extend(self.brightness.to_be_bytes());
                }
                _ => {
                    return Some(
                        vec![0x11, dev, 0xFF, b2, b3, 0x07]
                            .into_iter()
                            .chain([0; 14])
                            .collect(),
                    );
                }
            }
        } else {
            return Some(
                vec![0x11, dev, 0xFF, b2, b3, 0x06]
                    .into_iter()
                    .chain([0; 14])
                    .collect(),
            );
        }
        a.resize(20, 0);
        Some(a)
    }
}

impl Fake {
    pub fn from_trace(trace: &str, short_ep: u32, long_ep: u32) -> Fake {
        let mut map = HashMap::new();
        let mut pending: Option<String> = None;
        for line in trace.lines() {
            let mut it = line.split_whitespace();
            let (_, kind, rest) = (it.next(), it.next(), it.next());
            match (kind, rest) {
                (Some(">"), Some(hex)) => pending = Some(hex.to_string()),
                (Some("<"), Some(hex)) => {
                    if let Some(sent) = pending.take() {
                        map.entry(sent).or_insert_with(|| hex.to_string());
                    }
                }
                _ => {}
            }
        }
        Fake {
            map,
            short_ep,
            long_ep,
            alias: None,
            reg0: [0, 0x09, 0],
            reg0_writes: Vec::new(),
            mute_all: false,
            mute: Vec::new(),
            refuse: Vec::new(),
            budget: None,
            unknown: Vec::new(),
            light: None,
        }
    }

    /// Replace the answer to one request.
    pub fn script(&mut self, sent: &str, answer: &str) {
        self.map.insert(sent.to_string(), answer.to_string());
    }

    /// The endpoint and bytes of the answer, if the device answers.
    pub fn respond(&mut self, frame: &[u8]) -> Option<(u32, Vec<u8>)> {
        let index = frame[1];
        if self.mute_all || self.mute.contains(&index) {
            return None;
        }
        let (b2, b3) = (frame[2], frame[3]);
        if b2 < 0x80 && b3 & 0x0F == 0x0C {
            let a = self.light.as_mut()?.answer(frame)?;
            return Some((self.long_ep, a));
        }
        let answer: Vec<u8> = if b2 == 0x81 && b3 == 0x00 {
            let mut a = h("10ff810000000000");
            a.truncate(4);
            a.extend_from_slice(&self.reg0);
            a
        } else if b2 == 0x80 && b3 == 0x00 {
            self.reg0 = [frame[4], frame[5], frame[6]];
            self.reg0_writes.push(self.reg0);
            h(&short("10ff8000"))
        } else if b2 == 0 && b3 == 0x1D && self.refuse.contains(&index) {
            if index == 0xFF {
                h(&long("11ffff001d08"))
            } else {
                h(&format!("10{index:02x}8f001d0800"))
            }
        } else {
            let mut key = frame.to_vec();
            if let (Some(alias), 0xFF) = (self.alias, index) {
                key[1] = alias;
            }
            let hex = hidpp::hex(&key);
            if b2 < 0x80
                && let Some(budget) = &mut self.budget
            {
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
            }
            let Some(found) = self.map.get(&hex) else {
                self.unknown.push(hex);
                return None;
            };
            let mut a = h(found);
            if let (Some(alias), 0xFF) = (self.alias, index)
                && a[1] == alias
            {
                a[1] = 0xFF;
            }
            a
        };
        let ep = if answer.len() == hidpp::SHORT_LEN {
            self.short_ep
        } else {
            self.long_ep
        };
        Some((ep, answer))
    }
}

pub struct Sent {
    pub at: f64,
    pub ep: u32,
    pub frame: Vec<u8>,
}

pub struct Rig {
    pub core: Logitech,
    pub mono: f64,
    pub fakes: Vec<Fake>,
    binding: HashMap<u32, usize>,
    pub sent: Vec<Sent>,
    pub events: Vec<ProviderEvent>,
    pub wakes: Vec<f64>,
    ticks: usize,
}

pub fn wall_of(mono: f64) -> f64 {
    1_000_000.0 + mono
}

impl Rig {
    pub fn new() -> Rig {
        Rig {
            core: Logitech::new(),
            mono: 0.0,
            fakes: Vec::new(),
            binding: HashMap::new(),
            sent: Vec::new(),
            events: Vec::new(),
            wakes: Vec::new(),
            ticks: 0,
        }
    }

    pub fn now(&self) -> Now {
        Now {
            wall: wall_of(self.mono),
            mono: self.mono,
            utc_offset: -10800,
        }
    }

    /// Bind a fake device to the endpoint ids whose requests it answers.
    pub fn add_fake(&mut self, fake: Fake, endpoints: &[u32]) -> usize {
        self.fakes.push(fake);
        let idx = self.fakes.len() - 1;
        for ep in endpoints {
            self.binding.insert(*ep, idx);
        }
        idx
    }

    pub fn feed(&mut self, event: TransportEvent) {
        let actions = self.core.handle(event, self.now());
        self.process(actions);
    }

    /// Make an explicit call on the provider (the app's orders) and carry out what it asks for.
    pub fn call(&mut self, f: impl FnOnce(&mut Logitech, Now) -> Vec<Action>) {
        let now = self.now();
        let actions = f(&mut self.core, now);
        self.process(actions);
    }

    pub fn arrive(&mut self, ep: Endpoint) {
        self.feed(TransportEvent::Arrived(ep));
    }

    pub fn remove(&mut self, id: u32) {
        self.feed(TransportEvent::Removed(id));
    }

    pub fn report(&mut self, ep: u32, hex: &str) {
        self.feed(TransportEvent::Report(ep, h(hex)));
    }

    /// Move the clock to `to`, firing the timer every time it falls due on the way, like the shell does.
    pub fn advance(&mut self, to: f64) {
        for _ in 0..1000 {
            let next = self
                .wakes
                .iter()
                .copied()
                .fold(None, |n: Option<f64>, t| Some(n.map_or(t, |n| n.min(t))));
            match next {
                Some(t) if t <= to => {
                    self.wakes.retain(|w| *w > t);
                    self.mono = self.mono.max(t);
                    self.ticks += 1;
                    let actions = self.core.tick(self.now());
                    self.process(actions);
                }
                _ => {
                    self.mono = self.mono.max(to);
                    return;
                }
            }
        }
        panic!("the timer keeps firing: a busy loop");
    }

    fn process(&mut self, actions: Vec<Action>) {
        let mut work: VecDeque<Action> = VecDeque::new();
        self.check_batch(&actions);
        work.extend(actions);
        while let Some(action) = work.pop_front() {
            match action {
                Action::Send(ep, frame) => {
                    self.sent.push(Sent {
                        at: self.mono,
                        ep,
                        frame: frame.clone(),
                    });
                    let answer = self
                        .binding
                        .get(&ep)
                        .copied()
                        .and_then(|i| self.fakes[i].respond(&frame));
                    if let Some((rep, bytes)) = answer {
                        self.mono += 0.002;
                        let more = self
                            .core
                            .handle(TransportEvent::Report(rep, bytes), self.now());
                        self.check_batch(&more);
                        work.extend(more);
                    }
                }
                Action::Emit(e) => self.events.push(e),
                Action::Wake(t) => {
                    assert!(
                        t > self.mono,
                        "wake {t} is not in the future (now {})",
                        self.mono
                    );
                    if !self.wakes.contains(&t) {
                        self.wakes.push(t);
                    }
                }
                Action::Persist(..) => panic!("the provider persists nothing"),
            }
        }
    }

    /// One request in flight per group: a batch never holds two sends.
    fn check_batch(&self, actions: &[Action]) {
        let sends = actions
            .iter()
            .filter(|a| matches!(a, Action::Send(..)))
            .count();
        assert!(sends <= 1, "two requests at once: {actions:?}");
    }

    pub fn take_events(&mut self) -> Vec<ProviderEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn sent_hex(&self) -> Vec<String> {
        self.sent.iter().map(|s| hidpp::hex(&s.frame)).collect()
    }

    pub fn next_wake(&self) -> Option<f64> {
        self.wakes
            .iter()
            .copied()
            .fold(None, |n: Option<f64>, t| Some(n.map_or(t, |n| n.min(t))))
    }

    /// Every frame sent is a request of spec/PROTOCOL.md: it parses, passes the allow-list, goes to the collection
    /// of its length, and a HID++ 2.0 request to a feature other than Root names one of `features`
    /// (feature index, function).
    pub fn assert_protocol_only(&self, features: &[(u8, u8)], never: &[u32]) {
        for s in &self.sent {
            let req =
                Request::parse(&s.frame).unwrap_or_else(|| panic!("not a report: {:?}", s.frame));
            assert!(
                hidpp::is_allowed(&req),
                "outside the allow-list: {}",
                hidpp::hex(&s.frame)
            );
            assert!(
                !never.contains(&s.ep),
                "sent to a collection that is never written"
            );
            if req.b2 != 0 && req.b2 < 0x80 {
                let function = req.b3 >> 4;
                assert!(
                    features.contains(&(req.b2, function)),
                    "unexpected feature request {}",
                    hidpp::hex(&s.frame)
                );
            }
            if req.b2 == 0x83 {
                assert!(
                    (0x20..=0x25).contains(&req.params[0]),
                    "serial or name register"
                );
            }
            assert!(req.b3 & 0x0F == hidpp::SWID || req.b2 >= 0x80);
        }
        for f in &self.fakes {
            assert!(
                f.unknown.is_empty(),
                "frames the traces never saw: {:?}",
                f.unknown
            );
        }
    }
}

pub fn endpoint(id: u32, pid: u16, usage_page: u16, usage: u16, len: u16) -> Endpoint {
    Endpoint {
        id,
        vid: 0x046D,
        pid,
        usage_page,
        usage,
        input_len: len,
        output_len: len,
        product: String::new(),
        bluetooth: false,
    }
}

/// The three HID++ collections of a LIGHTSPEED receiver: short, long, very long (never written).
pub fn receiver_endpoints(pid: u16, first_id: u32) -> [Endpoint; 3] {
    [
        endpoint(first_id, pid, 0xFF00, 0x0001, 7),
        endpoint(first_id + 1, pid, 0xFF00, 0x0002, 20),
        endpoint(first_id + 2, pid, 0xFF00, 0x0004, 32),
    ]
}

/// A device on its cable: short and long collections.
pub fn usb_endpoints(pid: u16, first_id: u32) -> [Endpoint; 2] {
    [
        endpoint(first_id, pid, 0xFF00, 0x0001, 7),
        endpoint(first_id + 1, pid, 0xFF00, 0x0002, 20),
    ]
}

/// A receiver trace played through the provider: the receiver arrives with the device answering from its trace.
pub fn start_receiver(trace: &str, pid: u16, fake_setup: impl FnOnce(&mut Fake)) -> Rig {
    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(trace, 1, 2);
    fake_setup(&mut fake);
    rig.add_fake(fake, &[1, 2, 3]);
    for ep in receiver_endpoints(pid, 1) {
        rig.arrive(ep);
    }
    rig
}

/// The lines of a listen trace: (seconds, hex) of every received frame.
pub fn listen_frames(trace: &str) -> Vec<(f64, String)> {
    trace
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (ms, kind, hex) = (it.next()?, it.next()?, it.next()?);
            if kind != "<" {
                return None;
            }
            Some((ms.parse::<f64>().ok()? / 1000.0, hex.to_string()))
        })
        .collect()
}
