// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The `logitech-hidpp` provider as a sans-I/O state machine (spec/PROTOCOL.md, spec/PROVIDERS.md).
//!
//! The shell feeds `handle` the transport events (a collection arrived or went, a report came in) and calls `tick`
//! when a `Wake` it was given falls due. Both return actions: `Send` a report, `Emit` a provider event, `Wake` at a
//! monotonic moment. Nothing here reads a clock, opens a device or sleeps.
//!
//! One physical thing is a group: a receiver (its short and long collections), a dongle (long only) or a cabled
//! device. A group has one request in flight at a time and a queue behind it. Groups are told apart by USB product
//! id, so two receivers of the same model at once are not supported.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::hidpp::catalog::{self, Match, Row};
use crate::hidpp::{
    self, BatteryData, ConnectionNotice, F_DEVICE_NAME, F_UNIFIED_BATTERY, Reply, Request,
};
use crate::lighting::{self, LightStep, Lighting, LightingEvent, SavedLight};
use crate::registry::logitech_key;
use crate::types::{
    Action, BatteryReading, Capabilities, Connection, DeviceInfo, DeviceKey, Endpoint, Kind, Now,
    Presence, ProviderEvent, Quality, TransportEvent,
};

mod light;

pub const PROVIDER_ID: &str = "logitech-hidpp";
const VENDOR_ID: u16 = 0x046D;

/// The first request to a device that may be asleep waits this long (a sleeping keyboard answered after 1.47 s).
const FIRST_TIMEOUT: f64 = 4.0;
const TIMEOUT: f64 = 1.0;
/// A dongle whose headset is off is checked (a battery read) at most this often.
const DONGLE_PING_INTERVAL: f64 = 60.0;
/// The slow battery refresh of an online device that does not push its battery (also `min_refresh_seconds`).
const REFRESH_INTERVAL: f64 = 600.0;
/// After a link-established notification a pushed battery event within this time spares the read.
const PUSH_GRACE: f64 = 2.0;
const SLOTS: u8 = 6;
const NAME_CHUNK: usize = 16;
const NAME_MAX: usize = 64;
const EARLY_EVENTS_MAX: usize = 8;

/// What this provider declares about itself.
pub fn capabilities() -> Capabilities {
    Capabilities {
        pushes_presence: true,
        pushes_battery: true,
        min_refresh_seconds: Some(REFRESH_INTERVAL as u32),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    ReadNotify,
    WriteNotify([u8; 3]),
    Pair(u8),
    Ping,
    Feature(u16),
    NameCount,
    NameChunk(u8),
    Battery,
    /// The RGB economy's own requests (`lighting`), made only on the app's explicit order.
    Light(LightStep),
}

impl Step {
    fn receiver_level(self) -> bool {
        matches!(
            self,
            Step::ReadNotify | Step::WriteNotify(_) | Step::Pair(_)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Job {
    /// The device index the request goes to (0xFF for the receiver's own registers and for a direct device).
    dev: u8,
    step: Step,
}

struct InFlight {
    job: Job,
    req: Request,
    deadline: f64,
}

enum JobResult {
    Answer(Vec<u8>),
    Error,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Unknown,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cause {
    /// The receiver said the link is gone.
    LinkLost,
    /// The device or receiver answered with an error to the ping.
    Refused,
    /// A request timed out.
    Timeout,
    /// The PC went to sleep: nothing can be reached, and nothing is retried until it wakes.
    Suspended,
}

struct EarlyEvent {
    feature_index: u8,
    payload: Vec<u8>,
    wall: f64,
    mono: f64,
}

struct Dev {
    index: u8,
    wpid: Option<u16>,
    row: Option<&'static Row>,
    kind: Kind,
    connection: Connection,
    key: DeviceKey,
    link: Link,
    online: bool,
    /// Has answered since it last went off: later requests use the short timeout.
    warm: bool,
    /// A ping or the discovery chain is queued or running.
    probing: bool,
    /// The battery search is over (found or not).
    discovered: bool,
    battery: Option<(u16, u8)>,
    name_index: Option<u8>,
    name_len: usize,
    name_buf: Vec<u8>,
    established_at: Option<f64>,
    last_push: Option<f64>,
    last_read: Option<f64>,
    /// A 0x1004 device has pushed a battery event: it needs no slow refresh.
    pushes: bool,
    battery_due: Option<f64>,
    refresh_at: Option<f64>,
    ping_at: Option<f64>,
    early: Vec<EarlyEvent>,
    /// The RGB economy's state for this path to the device.
    light: Lighting,
}

impl Dev {
    fn new(
        index: u8,
        wpid: Option<u16>,
        row: Option<&'static Row>,
        kind: Kind,
        connection: Connection,
    ) -> Dev {
        let key = match (row, wpid) {
            (Some(r), _) => logitech_key(r.matcher, r.id),
            (None, Some(w)) => logitech_key(Match::Wpid, w),
            (None, None) => logitech_key(Match::Wpid, 0),
        };
        Dev {
            index,
            wpid,
            row,
            kind,
            connection,
            key,
            link: Link::Unknown,
            online: false,
            warm: false,
            probing: false,
            discovered: false,
            battery: None,
            name_index: None,
            name_len: 0,
            name_buf: Vec::new(),
            established_at: None,
            last_push: None,
            last_read: None,
            pushes: false,
            battery_due: None,
            refresh_at: None,
            ping_at: None,
            early: Vec::new(),
            light: Lighting::default(),
        }
    }

    fn slot(index: u8, wpid: u16, kind_nibble: u8) -> Dev {
        let row = catalog::find(Match::Wpid, wpid);
        let kind = row.map_or_else(|| kind_from_nibble(kind_nibble), |r| r.kind);
        Dev::new(index, Some(wpid), row, kind, Connection::Receiver)
    }

    fn direct(row: &'static Row) -> Dev {
        let connection = if row.matcher == Match::Usb {
            Connection::Usb
        } else {
            Connection::Dongle
        };
        Dev::new(hidpp::DIRECT, None, Some(row), row.kind, connection)
    }
}

fn kind_from_nibble(n: u8) -> Kind {
    match n {
        1 => Kind::Keyboard,
        2 => Kind::Mouse,
        _ => Kind::Other,
    }
}

struct Group {
    pid: u16,
    matcher: Match,
    row: &'static Row,
    short: Option<u32>,
    long: Option<u32>,
    started: bool,
    devs: Vec<Dev>,
    queue: VecDeque<Job>,
    inflight: Option<InFlight>,
}

impl Group {
    fn ready(&self) -> bool {
        match self.matcher {
            Match::Receiver => self.short.is_some() && self.long.is_some(),
            _ => self.long.is_some(),
        }
    }

    fn has_endpoint(&self, id: u32) -> bool {
        self.short == Some(id) || self.long == Some(id)
    }

    fn dev_pos(&self, index: u8) -> Option<usize> {
        self.devs.iter().position(|d| d.index == index)
    }

    fn queued(&self, dev: u8, pred: impl Fn(Step) -> bool) -> bool {
        self.queue.iter().any(|j| j.dev == dev && pred(j.step))
            || self
                .inflight
                .as_ref()
                .is_some_and(|f| f.job.dev == dev && pred(f.job.step))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyState {
    Gone,
    Offline,
    Online,
}

struct Announced {
    state: KeyState,
    info: DeviceInfo,
}

/// The provider.
#[derive(Default)]
pub struct Logitech {
    groups: Vec<Group>,
    names: HashMap<DeviceKey, String>,
    announced: HashMap<DeviceKey, Announced>,
    last_wake: Option<f64>,
    /// Devices the economy is wanted on, and whether it is wanted on every device (also the ones not seen yet).
    wanted: HashSet<DeviceKey>,
    all_wanted: bool,
    /// The level each device had before the economy (kept by the app between runs).
    saved: HashMap<DeviceKey, SavedLight>,
    light_events: Vec<LightingEvent>,
}

impl Logitech {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one transport event.
    pub fn handle(&mut self, event: TransportEvent, now: Now) -> Vec<Action> {
        let mut out = Vec::new();
        match event {
            TransportEvent::Arrived(ep) => self.on_arrived(&ep, now, &mut out),
            TransportEvent::Removed(id) => self.on_removed(id, now, &mut out),
            TransportEvent::Report(id, frame) => self.on_report(id, &frame, now, &mut out),
        }
        self.finish(&mut out);
        out
    }

    /// Call when a `Wake` falls due: timeouts, the dongle ping, the slow refresh, the grace after a reconnection.
    pub fn tick(&mut self, now: Now) -> Vec<Action> {
        let mut out = Vec::new();
        self.last_wake = None;
        for gi in 0..self.groups.len() {
            self.expire_inflight(gi, now, &mut out);
            let mut di = 0;
            while di < self.groups[gi].devs.len() {
                self.run_timers(gi, di, now);
                self.light_timers(gi, di, now);
                di += 1;
            }
            self.pump(gi, now, &mut out);
        }
        self.finish(&mut out);
        out
    }

    /// The PC is going to sleep: every device is offline from now (no request can reach it), nothing queued
    /// or in flight survives, and no timer asks for anything until `resume`.
    pub fn suspend(&mut self, now: Now) -> Vec<Action> {
        let mut out = Vec::new();
        for gi in 0..self.groups.len() {
            for di in 0..self.groups[gi].devs.len() {
                self.dev_offline(gi, di, Cause::Suspended, now, &mut out);
            }
            let g = &mut self.groups[gi];
            g.queue.clear();
            g.inflight = None;
        }
        self.finish(&mut out);
        out
    }

    /// The PC woke: every device is asked once whether it is on (a ping, or the battery read an offline dongle
    /// is checked by); the ones that answer give their battery as on a reconnection.
    pub fn resume(&mut self, now: Now) -> Vec<Action> {
        let mut out = Vec::new();
        for gi in 0..self.groups.len() {
            if !(self.groups[gi].started && self.groups[gi].ready()) {
                continue;
            }
            for di in 0..self.groups[gi].devs.len() {
                self.groups[gi].devs[di].ping_at = Some(now.mono);
                self.run_timers(gi, di, now);
            }
            self.pump(gi, now, &mut out);
        }
        self.finish(&mut out);
        out
    }

    /// One battery read of a device, for the registry's `refresh(key)`: only for an online device, and not within
    /// `min_refresh_seconds` of its last reading.
    pub fn refresh(&mut self, key: &DeviceKey, now: Now) -> Vec<Action> {
        let mut out = Vec::new();
        for gi in 0..self.groups.len() {
            for di in 0..self.groups[gi].devs.len() {
                let d = &self.groups[gi].devs[di];
                let spaced = d.last_read.is_none_or(|t| now.mono - t >= REFRESH_INTERVAL);
                if &d.key == key && d.online && d.discovered && d.battery.is_some() && spaced {
                    self.enqueue_battery(gi, di);
                }
            }
            self.pump(gi, now, &mut out);
        }
        self.finish(&mut out);
        out
    }

    // ------------------------------------------------------------ transport events

    fn on_arrived(&mut self, ep: &Endpoint, now: Now, out: &mut Vec<Action>) {
        if ep.vid != VENDOR_ID || ep.bluetooth || !hidpp::is_hidpp_page(ep.usage_page) {
            return;
        }
        if self.groups.iter().any(|g| g.has_endpoint(ep.id)) {
            return;
        }
        let found = [Match::Receiver, Match::Dongle, Match::Usb]
            .into_iter()
            .find_map(|m| catalog::find(m, ep.pid).map(|r| (m, r)));
        let Some((matcher, row)) = found else {
            return;
        };
        let gi = match self.groups.iter().position(|g| g.pid == ep.pid) {
            Some(gi) => gi,
            None => {
                self.groups.push(Group {
                    pid: ep.pid,
                    matcher,
                    row,
                    short: None,
                    long: None,
                    started: false,
                    devs: Vec::new(),
                    queue: VecDeque::new(),
                    inflight: None,
                });
                self.groups.len() - 1
            }
        };
        let g = &mut self.groups[gi];
        // The 32-byte very long collection is never used.
        let slot = match ep.output_len {
            7 => &mut g.short,
            20 => &mut g.long,
            _ => return,
        };
        if slot.is_some() {
            return;
        }
        *slot = Some(ep.id);
        self.try_start(gi, now, out);
    }

    fn try_start(&mut self, gi: usize, now: Now, out: &mut Vec<Action>) {
        let g = &mut self.groups[gi];
        if g.started || !g.ready() {
            return;
        }
        g.started = true;
        match g.matcher {
            Match::Receiver => {
                g.queue.push_back(Job {
                    dev: hidpp::DIRECT,
                    step: Step::ReadNotify,
                });
                for n in 1..=SLOTS {
                    g.queue.push_back(Job {
                        dev: hidpp::DIRECT,
                        step: Step::Pair(n),
                    });
                }
            }
            _ => {
                let mut dev = Dev::direct(g.row);
                dev.probing = true;
                g.devs.push(dev);
                g.queue.push_back(Job {
                    dev: hidpp::DIRECT,
                    step: Step::Ping,
                });
            }
        }
        self.pump(gi, now, out);
    }

    fn on_removed(&mut self, id: u32, now: Now, out: &mut Vec<Action>) {
        let Some(gi) = self.groups.iter().position(|g| g.has_endpoint(id)) else {
            return;
        };
        let g = &mut self.groups[gi];
        if g.short == Some(id) {
            g.short = None;
        }
        if g.long == Some(id) {
            g.long = None;
        }
        let keep_running = g.started && g.ready();
        if keep_running {
            return;
        }
        // The group cannot talk any more: forget what it knew, keep the endpoints still present.
        let keys: Vec<DeviceKey> = g.devs.iter().map(|d| d.key.clone()).collect();
        g.devs.clear();
        g.queue.clear();
        g.inflight = None;
        g.started = false;
        if g.short.is_none() && g.long.is_none() {
            self.groups.remove(gi);
        }
        for key in keys {
            self.sync_key(&key, now, out);
        }
    }

    fn on_report(&mut self, endpoint: u32, frame: &[u8], now: Now, out: &mut Vec<Action>) {
        let Some(gi) = self.groups.iter().position(|g| g.has_endpoint(endpoint)) else {
            return;
        };
        if frame.len() < hidpp::SHORT_LEN
            || (frame[0] != hidpp::SHORT_ID && frame[0] != hidpp::LONG_ID)
        {
            return;
        }
        if let Some(inflight) = &self.groups[gi].inflight {
            let result = match hidpp::classify(&inflight.req, frame) {
                Reply::Answer(p) => Some(JobResult::Answer(p)),
                Reply::Error { .. } => Some(JobResult::Error),
                Reply::Ignore => None,
            };
            if let Some(result) = result {
                let job = inflight.job;
                self.groups[gi].inflight = None;
                self.complete(gi, job, result, now, out);
                self.pump(gi, now, out);
                return;
            }
        }
        if self.groups[gi].matcher == Match::Receiver
            && let Some(notice) = hidpp::parse_connection(frame)
        {
            self.on_connection(gi, notice, now, out);
            self.pump(gi, now, out);
            return;
        }
        self.on_event(gi, frame, now, out);
        self.pump(gi, now, out);
    }

    // ------------------------------------------------------------ presence

    fn on_connection(&mut self, gi: usize, n: ConnectionNotice, now: Now, out: &mut Vec<Action>) {
        // A different wireless product id in the slot means another device took it.
        let di = match self.groups[gi].dev_pos(n.index) {
            Some(di) if self.groups[gi].devs[di].wpid == Some(n.wpid) => di,
            Some(di) => {
                let old = self.groups[gi].devs[di].key.clone();
                self.groups[gi].queue.retain(|j| j.dev != n.index);
                self.groups[gi].devs[di] = Dev::slot(n.index, n.wpid, n.kind);
                self.sync_key(&old, now, out);
                di
            }
            None => {
                self.groups[gi]
                    .devs
                    .push(Dev::slot(n.index, n.wpid, n.kind));
                self.groups[gi].devs.len() - 1
            }
        };
        if n.link_lost {
            self.dev_offline(gi, di, Cause::LinkLost, now, out);
        } else {
            self.established(gi, di, now, out);
        }
    }

    /// The receiver says the link came up: the device is on. Discover it if needed; the battery is read after the
    /// grace period unless the device pushes it first.
    fn established(&mut self, gi: usize, di: usize, now: Now, out: &mut Vec<Action>) {
        let g = &mut self.groups[gi];
        let index = g.devs[di].index;
        g.queue
            .retain(|j| !(j.dev == index && j.step == Step::Ping));
        let d = &mut g.devs[di];
        let was_online = d.online;
        d.link = Link::Up;
        d.online = true;
        d.warm = false;
        d.ping_at = None;
        d.established_at = Some(now.mono);
        d.battery_due = Some(now.mono + PUSH_GRACE);
        if !d.discovered && !d.probing {
            d.probing = true;
            g.queue.push_back(Job {
                dev: index,
                step: Step::Feature(hidpp::BATTERY_FEATURES[0]),
            });
        }
        if !was_online {
            self.light_on_online(gi, di, now);
        }
        let key = self.groups[gi].devs[di].key.clone();
        self.sync_key(&key, now, out);
    }

    fn dev_offline(&mut self, gi: usize, di: usize, cause: Cause, now: Now, out: &mut Vec<Action>) {
        let g = &mut self.groups[gi];
        let direct = g.matcher != Match::Receiver;
        let d = &mut g.devs[di];
        let index = d.index;
        d.online = false;
        d.warm = false;
        d.probing = false;
        d.battery_due = None;
        d.refresh_at = None;
        d.early.clear();
        d.light.connection_lost();
        d.link = match cause {
            Cause::LinkLost | Cause::Refused => Link::Down,
            Cause::Timeout | Cause::Suspended => Link::Unknown,
        };
        // A dongle is pinged every 60 s while its headset is off. A receiver's device comes back by its own
        // notification; only a timeout, which that notification would not explain, gets a late ping.
        d.ping_at = if cause == Cause::Suspended {
            None
        } else if direct {
            Some(now.mono + DONGLE_PING_INTERVAL)
        } else if cause == Cause::Timeout {
            Some(now.mono + REFRESH_INTERVAL)
        } else {
            None
        };
        let key = d.key.clone();
        g.queue.retain(|j| j.dev != index);
        if g.inflight.as_ref().is_some_and(|f| f.job.dev == index) {
            g.inflight = None;
        }
        self.sync_key(&key, now, out);
    }

    fn mark_online(&mut self, gi: usize, di: usize, now: Now, out: &mut Vec<Action>) {
        let d = &mut self.groups[gi].devs[di];
        let was_online = d.online;
        d.link = Link::Up;
        d.online = true;
        d.warm = true;
        d.ping_at = None;
        let key = d.key.clone();
        if !was_online {
            self.light_on_online(gi, di, now);
        }
        self.sync_key(&key, now, out);
    }

    // ------------------------------------------------------------ the request queue

    fn pump(&mut self, gi: usize, now: Now, out: &mut Vec<Action>) {
        while self.groups[gi].inflight.is_none() {
            let Some(job) = self.groups[gi].queue.pop_front() else {
                break;
            };
            let Some((req, timeout)) = self.build(gi, job) else {
                self.drop_light_job(gi, job);
                continue;
            };
            // The allow-list is the last gate: nothing outside spec/PROTOCOL.md reaches a wire. The economy's
            // requests have their own list (`lighting::is_allowed`), which the general one never admits.
            let allowed = match job.step {
                Step::Light(_) => self.light_allowed(gi, job.dev, &req),
                _ => hidpp::is_allowed(&req, &self.known_indexes(gi, job.dev)),
            };
            if !allowed {
                debug_assert!(false, "request outside the allow-list: {req:?}");
                self.drop_light_job(gi, job);
                continue;
            }
            let g = &mut self.groups[gi];
            let endpoint = if req.is_long() { g.long } else { g.short };
            let Some(endpoint) = endpoint else {
                self.drop_light_job(gi, job);
                continue;
            };
            out.push(Action::Send(endpoint, req.encode()));
            g.inflight = Some(InFlight {
                job,
                req,
                deadline: now.mono + timeout,
            });
        }
    }

    /// The feature indexes (battery, DeviceName) the device itself reported: the only ones the general allow-list
    /// admits besides the root.
    fn known_indexes(&self, gi: usize, dev: u8) -> Vec<u8> {
        let g = &self.groups[gi];
        g.dev_pos(dev)
            .map(|di| &g.devs[di])
            .into_iter()
            .flat_map(|d| {
                d.battery
                    .map(|(_, index)| index)
                    .into_iter()
                    .chain(d.name_index)
            })
            .collect()
    }

    /// A lighting job that cannot be sent ends its chain, so the next order can start another.
    fn drop_light_job(&mut self, gi: usize, job: Job) {
        if matches!(job.step, Step::Light(_))
            && let Some(di) = self.groups[gi].dev_pos(job.dev)
        {
            self.groups[gi].devs[di].light.chain = None;
        }
    }

    /// The request for a job and its timeout, or `None` when the job no longer makes sense.
    fn build(&self, gi: usize, job: Job) -> Option<(Request, f64)> {
        let g = &self.groups[gi];
        if job.step.receiver_level() {
            let req = match job.step {
                Step::ReadNotify => {
                    Request::register_read(hidpp::DIRECT, hidpp::REG_NOTIFICATIONS, [0; 3])
                }
                Step::WriteNotify(flags) => {
                    Request::register_write(hidpp::DIRECT, hidpp::REG_NOTIFICATIONS, flags)
                }
                Step::Pair(n) => Request::register_read_long(
                    hidpp::DIRECT,
                    hidpp::REG_PAIRING,
                    [0x20 + n - 1, 0, 0],
                ),
                _ => return None,
            };
            return Some((req, TIMEOUT));
        }
        let d = &g.devs[g.dev_pos(job.dev)?];
        // A dongle's presence is its battery read: its requests go out while the device is offline.
        if !d.online && job.step != Step::Ping && g.matcher != Match::Dongle {
            return None;
        }
        let long = g.long.is_some();
        let req = match job.step {
            // A ping goes as a short report when there is a short collection.
            Step::Ping => {
                Request::feature(job.dev, 0, 1, &[0, 0, hidpp::PING_BYTE], g.short.is_none())
            }
            Step::Feature(id) => Request::feature(job.dev, 0, 0, &id.to_be_bytes(), long),
            Step::NameCount => Request::feature(job.dev, d.name_index?, 0, &[], long),
            Step::NameChunk(offset) => Request::feature(job.dev, d.name_index?, 1, &[offset], long),
            Step::Battery => {
                let (feature, index) = d.battery?;
                let function = if feature == F_UNIFIED_BATTERY { 1 } else { 0 };
                Request::feature(job.dev, index, function, &[], long)
            }
            Step::Light(step) => d.light.request(job.dev, step)?,
            _ => return None,
        };
        // A keyboard falls asleep within seconds (a request after 2 s of quiet was answered after 1.5 s): the
        // economy's requests, which the owner asked for, always get the long wait.
        let timeout = if d.warm && !matches!(job.step, Step::Light(_)) {
            TIMEOUT
        } else {
            FIRST_TIMEOUT
        };
        Some((req, timeout))
    }

    fn push_front(&mut self, gi: usize, job: Job) {
        self.groups[gi].queue.push_front(job);
    }

    fn enqueue_battery(&mut self, gi: usize, di: usize) {
        let g = &mut self.groups[gi];
        let index = g.devs[di].index;
        if !g.queued(index, |s| s == Step::Battery) {
            g.queue.push_back(Job {
                dev: index,
                step: Step::Battery,
            });
        }
    }

    fn expire_inflight(&mut self, gi: usize, now: Now, out: &mut Vec<Action>) {
        let expired = self.groups[gi]
            .inflight
            .as_ref()
            .is_some_and(|f| now.mono >= f.deadline);
        if expired && let Some(f) = self.groups[gi].inflight.take() {
            self.complete(gi, f.job, JobResult::Timeout, now, out);
        }
    }

    /// The timers of one device that have fallen due.
    fn run_timers(&mut self, gi: usize, di: usize, now: Now) {
        let g = &mut self.groups[gi];
        let d = &mut g.devs[di];
        let index = d.index;
        if !d.online && d.ping_at.is_some_and(|t| now.mono >= t) {
            d.ping_at = None;
            // An offline dongle whose battery feature is known is checked by a battery read, not a ping.
            if g.matcher == Match::Dongle && d.discovered && d.battery.is_some() {
                self.enqueue_battery(gi, di);
                return;
            }
            d.probing = true;
            if !g.queued(index, |s| s == Step::Ping) {
                g.queue.push_back(Job {
                    dev: index,
                    step: Step::Ping,
                });
            }
            return;
        }
        if d.online && d.refresh_at.is_some_and(|t| now.mono >= t) {
            d.refresh_at = None;
            if d.discovered && d.battery.is_some() {
                self.enqueue_battery(gi, di);
            } else if !d.discovered && !d.probing {
                d.probing = true;
                g.queue.push_back(Job {
                    dev: index,
                    step: Step::Feature(hidpp::BATTERY_FEATURES[0]),
                });
            }
        }
        self.check_due(gi, di, now);
    }

    /// The battery read that follows a reconnection: due once the grace has passed, skipped when the device pushed.
    fn check_due(&mut self, gi: usize, di: usize, now: Now) {
        let d = &mut self.groups[gi].devs[di];
        let due = d.battery_due.is_some_and(|t| now.mono >= t);
        if !(due && d.online && d.discovered && !d.probing) {
            return;
        }
        d.battery_due = None;
        let pushed = matches!((d.last_push, d.established_at), (Some(p), Some(e)) if p >= e);
        if !pushed && d.battery.is_some() {
            self.enqueue_battery(gi, di);
        }
    }

    // ------------------------------------------------------------ answers

    fn complete(
        &mut self,
        gi: usize,
        job: Job,
        result: JobResult,
        now: Now,
        out: &mut Vec<Action>,
    ) {
        if job.step.receiver_level() {
            self.complete_receiver(gi, job, result, now, out);
            return;
        }
        let Some(di) = self.groups[gi].dev_pos(job.dev) else {
            return;
        };
        if let Step::Light(step) = job.step {
            self.complete_light(gi, di, step, result, now, out);
            return;
        }
        if matches!(result, JobResult::Timeout) {
            self.dev_offline(gi, di, Cause::Timeout, now, out);
            return;
        }
        // A dongle answers the ping itself: for it an error answer means the headset is off or out of range.
        if self.groups[gi].matcher == Match::Dongle
            && job.step != Step::Ping
            && matches!(result, JobResult::Error)
        {
            self.dev_offline(gi, di, Cause::Refused, now, out);
            return;
        }
        // Any answer, an error included, shows the device is awake.
        if job.step != Step::Ping {
            self.groups[gi].devs[di].warm = true;
        }
        let payload = match result {
            JobResult::Answer(p) => Some(p),
            _ => None,
        };
        match job.step {
            Step::Ping => match payload {
                Some(p) if hidpp::decode_ping(&p).is_some() => {
                    // A dongle's ping proves only the dongle: its first battery answer brings the device online.
                    if self.groups[gi].matcher != Match::Dongle {
                        self.mark_online(gi, di, now, out);
                    }
                    self.after_ping(gi, di, now);
                }
                _ => self.dev_offline(gi, di, Cause::Refused, now, out),
            },
            Step::Feature(id) => {
                let found = payload
                    .as_deref()
                    .and_then(hidpp::decode_get_feature)
                    .filter(|f| f.0 != 0)
                    .map(|f| f.0);
                self.after_feature(gi, di, id, found, now, out);
            }
            Step::NameCount => {
                let len = payload
                    .as_deref()
                    .and_then(hidpp::decode_name_count)
                    .unwrap_or(0)
                    .min(NAME_MAX);
                if len == 0 {
                    self.finish_chain(gi, di, now, out);
                } else {
                    let d = &mut self.groups[gi].devs[di];
                    d.name_len = len;
                    d.name_buf.clear();
                    self.push_front(
                        gi,
                        Job {
                            dev: job.dev,
                            step: Step::NameChunk(0),
                        },
                    );
                }
            }
            Step::NameChunk(offset) => self.after_name_chunk(gi, di, offset, payload, now, out),
            Step::Battery => {
                let d = &self.groups[gi].devs[di];
                let data = d
                    .battery
                    .zip(payload)
                    .and_then(|((feature, _), p)| hidpp::decode_battery(feature, &p));
                match data {
                    Some(data) => self.record_reading(gi, di, data, now.wall, now.mono, false, out),
                    None => self.rearm_refresh(gi, di, now.mono),
                }
            }
            _ => {}
        }
    }

    fn complete_receiver(
        &mut self,
        gi: usize,
        job: Job,
        result: JobResult,
        now: Now,
        out: &mut Vec<Action>,
    ) {
        let JobResult::Answer(p) = result else {
            return;
        };
        match job.step {
            Step::ReadNotify => {
                if let Some(flags) = hidpp::decode_notification_flags(&p)
                    && !hidpp::notifications_enabled(flags)
                {
                    self.push_front(
                        gi,
                        Job {
                            dev: hidpp::DIRECT,
                            step: Step::WriteNotify(hidpp::notification_flags_enabled(flags)),
                        },
                    );
                }
            }
            Step::Pair(slot) => {
                if let Some((wpid, kind)) = hidpp::decode_pairing(&p) {
                    self.on_paired(gi, slot, wpid, kind, now, out);
                }
            }
            _ => {}
        }
    }

    /// A pairing slot holds a device: ping it unless a notification already told us its state.
    fn on_paired(
        &mut self,
        gi: usize,
        slot: u8,
        wpid: u16,
        kind: u8,
        now: Now,
        out: &mut Vec<Action>,
    ) {
        let g = &mut self.groups[gi];
        let di = match g.dev_pos(slot) {
            Some(di) if g.devs[di].wpid == Some(wpid) => di,
            Some(di) => {
                let old = g.devs[di].key.clone();
                g.queue.retain(|j| j.dev != slot);
                g.devs[di] = Dev::slot(slot, wpid, kind);
                self.sync_key(&old, now, out);
                di
            }
            None => {
                g.devs.push(Dev::slot(slot, wpid, kind));
                g.devs.len() - 1
            }
        };
        let g = &mut self.groups[gi];
        let d = &mut g.devs[di];
        if d.link == Link::Unknown && !d.probing {
            d.probing = true;
            g.queue.push_back(Job {
                dev: slot,
                step: Step::Ping,
            });
        }
    }

    fn after_ping(&mut self, gi: usize, di: usize, now: Now) {
        let d = &mut self.groups[gi].devs[di];
        let index = d.index;
        if d.discovered {
            // Features are cached from an earlier connection: only the battery is left.
            d.probing = false;
            if d.battery_due.is_some() {
                self.check_due(gi, di, now);
            } else if d.battery.is_some() {
                self.push_front(
                    gi,
                    Job {
                        dev: index,
                        step: Step::Battery,
                    },
                );
            }
        } else {
            d.probing = true;
            self.push_front(
                gi,
                Job {
                    dev: index,
                    step: Step::Feature(hidpp::BATTERY_FEATURES[0]),
                },
            );
        }
    }

    /// The answer to a root getFeature: `found` is the feature index, `None` when the device lacks it.
    fn after_feature(
        &mut self,
        gi: usize,
        di: usize,
        id: u16,
        found: Option<u8>,
        now: Now,
        out: &mut Vec<Action>,
    ) {
        let index = self.groups[gi].devs[di].index;
        if id == F_DEVICE_NAME {
            match found {
                Some(fi) => {
                    self.groups[gi].devs[di].name_index = Some(fi);
                    self.push_front(
                        gi,
                        Job {
                            dev: index,
                            step: Step::NameCount,
                        },
                    );
                }
                None => self.finish_chain(gi, di, now, out),
            }
            return;
        }
        if let Some(fi) = found {
            self.groups[gi].devs[di].battery = Some((id, fi));
        } else if let Some(pos) = hidpp::BATTERY_FEATURES.iter().position(|f| *f == id)
            && let Some(next) = hidpp::BATTERY_FEATURES.get(pos + 1)
        {
            self.push_front(
                gi,
                Job {
                    dev: index,
                    step: Step::Feature(*next),
                },
            );
            return;
        }
        // The battery search is over: found, or no battery feature at all.
        self.groups[gi].devs[di].discovered = true;
        self.replay_early(gi, di, out);
        let key = self.groups[gi].devs[di].key.clone();
        if self.names.contains_key(&key) {
            self.finish_chain(gi, di, now, out);
        } else {
            self.push_front(
                gi,
                Job {
                    dev: index,
                    step: Step::Feature(F_DEVICE_NAME),
                },
            );
        }
    }

    fn after_name_chunk(
        &mut self,
        gi: usize,
        di: usize,
        offset: u8,
        payload: Option<Vec<u8>>,
        now: Now,
        out: &mut Vec<Action>,
    ) {
        let d = &mut self.groups[gi].devs[di];
        let want = d
            .name_len
            .saturating_sub(usize::from(offset))
            .min(NAME_CHUNK);
        let got = payload.as_deref().unwrap_or(&[]);
        let take = want.min(got.len());
        d.name_buf.extend_from_slice(&got[..take]);
        let next = usize::from(offset) + take;
        if take > 0 && next < d.name_len {
            let index = d.index;
            self.push_front(
                gi,
                Job {
                    dev: index,
                    step: Step::NameChunk(next as u8),
                },
            );
            return;
        }
        let end = d
            .name_buf
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(d.name_buf.len());
        let name = String::from_utf8_lossy(&d.name_buf[..end])
            .trim()
            .to_string();
        let key = d.key.clone();
        if !name.is_empty() {
            self.names.insert(key, name);
        }
        self.finish_chain(gi, di, now, out);
    }

    /// Discovery is done: announce the device with its name, then read the battery now, or at the end of the grace
    /// after a reconnection.
    fn finish_chain(&mut self, gi: usize, di: usize, now: Now, out: &mut Vec<Action>) {
        let d = &mut self.groups[gi].devs[di];
        d.probing = false;
        let key = d.key.clone();
        let reconnecting = d.battery_due.is_some();
        let has_battery = d.battery.is_some();
        let index = d.index;
        if self.groups[gi].matcher == Match::Dongle && !self.groups[gi].devs[di].online {
            // Announced with its first battery answer, or when it is found offline.
            if !has_battery {
                self.mark_online(gi, di, now, out);
            }
        } else {
            self.sync_key(&key, now, out);
        }
        if !has_battery {
            return;
        }
        if reconnecting {
            self.check_due(gi, di, now);
        } else {
            self.push_front(
                gi,
                Job {
                    dev: index,
                    step: Step::Battery,
                },
            );
        }
    }

    // ------------------------------------------------------------ battery

    /// An unsolicited report of a device: a battery event of its battery feature.
    fn on_event(&mut self, gi: usize, frame: &[u8], now: Now, out: &mut Vec<Action>) {
        let (index, feature_index, b3) = (frame[1], frame[2], frame[3]);
        // Events carry software ID 0 and are event 0; everything else answers someone's request.
        if b3 != 0 || feature_index == 0 || feature_index >= 0x80 {
            return;
        }
        let Some(di) = self.groups[gi].dev_pos(index) else {
            return;
        };
        let d = &mut self.groups[gi].devs[di];
        match d.battery {
            Some((feature, fi)) if fi == feature_index => {
                if let Some(data) = hidpp::decode_battery(feature, &frame[4..]) {
                    self.record_reading(gi, di, data, now.wall, now.mono, true, out);
                }
            }
            None if !d.discovered && d.early.len() < EARLY_EVENTS_MAX => d.early.push(EarlyEvent {
                feature_index,
                payload: frame[4..].to_vec(),
                wall: now.wall,
                mono: now.mono,
            }),
            _ => {}
        }
    }

    /// Events that came before the battery feature was known are read now.
    fn replay_early(&mut self, gi: usize, di: usize, out: &mut Vec<Action>) {
        let d = &mut self.groups[gi].devs[di];
        let early = std::mem::take(&mut d.early);
        let Some((feature, fi)) = d.battery else {
            return;
        };
        for e in early.into_iter().filter(|e| e.feature_index == fi) {
            if let Some(data) = hidpp::decode_battery(feature, &e.payload) {
                self.record_reading(gi, di, data, e.wall, e.mono, true, out);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_reading(
        &mut self,
        gi: usize,
        di: usize,
        data: BatteryData,
        wall: f64,
        mono: f64,
        pushed: bool,
        out: &mut Vec<Action>,
    ) {
        if self.groups[gi].matcher == Match::Dongle && !self.groups[gi].devs[di].online {
            let at = Now {
                wall,
                mono,
                utc_offset: 0,
            };
            self.mark_online(gi, di, at, out);
        }
        let d = &mut self.groups[gi].devs[di];
        let reading = to_reading(&data, wall, d.row);
        d.last_read = Some(mono);
        if pushed {
            d.last_push = Some(mono);
            if data.feature == F_UNIFIED_BATTERY {
                d.pushes = true;
            }
        }
        let key = d.key.clone();
        self.rearm_refresh(gi, di, mono);
        let at = Now {
            wall,
            mono,
            utc_offset: 0,
        };
        // A reading never goes out before its device is announced.
        self.sync_key(&key, at, out);
        out.push(Action::Emit(ProviderEvent::Battery(key, reading)));
    }

    /// The slow refresh restarts after every reading, and does not run for a device that pushes its battery.
    fn rearm_refresh(&mut self, gi: usize, di: usize, mono: f64) {
        let d = &mut self.groups[gi].devs[di];
        let pusher = d.pushes && matches!(d.battery, Some((F_UNIFIED_BATTERY, _)));
        d.refresh_at = if d.online && !pusher {
            Some(mono + REFRESH_INTERVAL)
        } else {
            None
        };
    }

    // ------------------------------------------------------------ announcing

    fn key_state(&self, key: &DeviceKey) -> KeyState {
        let mut state = KeyState::Gone;
        for d in self.groups.iter().flat_map(|g| g.devs.iter()) {
            if &d.key == key {
                if d.online {
                    return KeyState::Online;
                }
                state = KeyState::Offline;
            }
        }
        state
    }

    fn info_of(&self, key: &DeviceKey) -> Option<DeviceInfo> {
        let all: Vec<&Dev> = self
            .groups
            .iter()
            .flat_map(|g| g.devs.iter())
            .filter(|d| &d.key == key)
            .collect();
        let d = all.iter().find(|d| d.online).or_else(|| all.first())?;
        let name = self
            .names
            .get(key)
            .cloned()
            .or_else(|| d.row.map(|r| r.model.to_string()))
            .unwrap_or_else(|| "Logitech device".to_string());
        Some(DeviceInfo {
            key: key.clone(),
            provider: PROVIDER_ID,
            name,
            kind: d.kind,
            connection: d.connection,
            brand: "Logitech".to_string(),
        })
    }

    /// Emit what changed for one identity across every path to it (a receiver slot and a cable are one device).
    fn sync_key(&mut self, key: &DeviceKey, now: Now, out: &mut Vec<Action>) {
        let state = self.key_state(key);
        let was = self.announced.get(key).map(|a| a.state);
        if state == KeyState::Gone {
            if matches!(was, Some(s) if s != KeyState::Gone) {
                out.push(Action::Emit(ProviderEvent::Disappeared(key.clone())));
                if let Some(a) = self.announced.get_mut(key) {
                    a.state = KeyState::Gone;
                }
            }
            return;
        }
        let Some(info) = self.info_of(key) else {
            return;
        };
        let visible = matches!(was, Some(s) if s != KeyState::Gone);
        let info_changed = self.announced.get(key).is_none_or(|a| a.info != info);
        if !visible || info_changed {
            out.push(Action::Emit(ProviderEvent::Appeared(info.clone())));
        }
        if !visible || was != Some(state) {
            out.push(Action::Emit(ProviderEvent::Presence(
                key.clone(),
                Presence {
                    online: state == KeyState::Online,
                    at: now.wall,
                },
            )));
        }
        self.announced
            .insert(key.clone(), Announced { state, info });
    }

    // ------------------------------------------------------------ time

    /// The earliest moment the core needs `tick` for.
    fn next_deadline(&self) -> Option<f64> {
        let mut next: Option<f64> = None;
        let mut take = |t: f64| next = Some(next.map_or(t, |n| n.min(t)));
        for g in &self.groups {
            if let Some(f) = &g.inflight {
                take(f.deadline);
            }
            for d in &g.devs {
                if let Some(t) = d.ping_at
                    && !d.online
                {
                    take(t);
                }
                if let Some(t) = d.refresh_at
                    && d.online
                {
                    take(t);
                }
                if let Some(t) = d.battery_due
                    && d.online
                    && d.discovered
                    && !d.probing
                {
                    take(t);
                }
                if d.online
                    && let Some(t) = d.light.next_due()
                {
                    take(t);
                }
            }
        }
        next
    }

    /// Ask the shell to call `tick` at the earliest deadline, once per change.
    fn finish(&mut self, out: &mut Vec<Action>) {
        match self.next_deadline() {
            Some(t) if self.last_wake != Some(t) => {
                out.push(Action::Wake(t));
                self.last_wake = Some(t);
            }
            Some(_) => {}
            None => self.last_wake = None,
        }
    }
}

/// A reading from what the device gave: its own percentage, or one derived through the model's curve.
fn to_reading(data: &BatteryData, wall: f64, row: Option<&Row>) -> BatteryReading {
    let (percent, quality) = match (data.percent, data.millivolts) {
        (Some(p), _) => (Some(f64::from(p)), Quality::Exact),
        (None, Some(mv)) => {
            match row
                .and_then(|r| r.curve)
                .and_then(|c| catalog::curve_percent(c, mv))
            {
                Some(p) => (Some(p), Quality::Derived),
                None => (None, Quality::Coarse),
            }
        }
        (None, None) => (None, Quality::Coarse),
    };
    BatteryReading {
        at: wall,
        percent,
        coarse: data.coarse,
        millivolts: data.millivolts,
        charging: data.charging,
        quality,
    }
}
