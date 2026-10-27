// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The live source: the sans-I/O loop of `spec/ARCHITECTURE.md` closed around the real devices.
//!
//! Transport events and the time go into the Logitech provider; its actions come back and are executed here
//! against a [`Host`] (the transport, the files, the one timer, the log): `Send` goes out, `Emit` reaches the
//! registry and the device's tracker, `Persist` is appended to the history file, `Wake` is kept as the one
//! pending moment. The module has no Win32 in it, so a fake host proves it.

use std::collections::BTreeMap;

use juicebar_core::estimator::{self, Notice as CoreNotice, NoticeKind, Priors, Reading};
use juicebar_core::lighting::{LightingEvent, SavedLight};
use juicebar_core::logitech::Logitech;
use juicebar_core::registry::{Registry, UserList};
use juicebar_core::snapshot::{DeviceInput, DeviceView, Limits, Snapshot};
use juicebar_core::store::Tracker;
use juicebar_core::strings::{self, Lang, id};
use juicebar_core::types::{
    Action, DeviceInfo, DeviceKey, Now, Presence, ProviderEvent, TransportEvent,
};

use crate::notify::Notice;

/// Everything outside the process that the loop touches.
pub trait Host {
    /// Queue one output report for an endpoint; false when it is refused.
    fn send(&mut self, endpoint: u32, bytes: &[u8]) -> bool;
    /// Append lines to a device's history file.
    fn append(&mut self, key: &DeviceKey, lines: &[String]);
    /// Replace a device's history file with its compacted text.
    fn replace(&mut self, key: &DeviceKey, text: &str);
    /// A history file that cannot be read: keep it aside so a fresh one starts.
    fn set_aside(&mut self, key: &DeviceKey);
    /// Arm the one-shot timer for this monotonic moment, replacing any earlier arming.
    fn set_wake(&mut self, at_mono: f64);
    /// Whether `log` is wanted (off by default); it keeps the formatting cost out of the normal run.
    fn logging(&self) -> bool {
        false
    }
    fn log(&mut self, _line: &str) {}
}

/// What the user decided about the icon's colors and the alerts (the Settings page); live-changeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// The two levels that color the tray mark's juice.
    pub limits: Limits,
    /// A discharging device is told to charge under this percent.
    pub low_percent: u8,
    /// The smart warning (its text, its notice and its red) is on.
    pub smart: bool,
    /// The "battery full" notice is on.
    pub full: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            limits: Limits::default(),
            low_percent: 10,
            smart: true,
            full: true,
        }
    }
}

impl Prefs {
    /// Hands the alert settings to one device's estimator.
    fn apply(&self, t: &mut Tracker) {
        t.history
            .set_alerts(f64::from(self.low_percent), self.smart);
    }
}

/// What the app knows before the first event.
#[derive(Clone, Debug)]
pub struct StartConfig {
    pub lang: Lang,
    pub prefs: Prefs,
    pub rgb_economy: bool,
    /// The devices the user hid in an earlier run.
    pub hidden: Vec<DeviceKey>,
    /// The lighting levels an earlier run kept (device key, `SavedLight` text); text that does not read is dropped.
    pub lights: Vec<(DeviceKey, String)>,
}

/// A change of the levels the app keeps in its settings for the RGB economy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LightChange {
    /// Keep this text for the device (replacing the one kept).
    Keep(DeviceKey, String),
    /// The kept level is spent.
    Drop(DeviceKey),
}

/// One entry of the "Dispositivos" menu.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // the Dispositivos section of the Settings page (part 2) lists them
pub struct DeviceChoice {
    pub key: DeviceKey,
    pub name: String,
    pub shown: bool,
}

pub struct Live<H: Host> {
    host: H,
    lang: Lang,
    prefs: Prefs,
    rgb_economy: bool,
    provider: Logitech,
    registry: Registry,
    trackers: BTreeMap<DeviceKey, Tracker>,
    /// The monotonic moment the host's timer is armed for.
    wake: Option<f64>,
    /// Decisions of the estimators waiting to become balloons.
    raw_notices: Vec<(DeviceKey, CoreNotice)>,
    /// Something shown may have changed since the app last took a snapshot.
    dirty: bool,
    closed: bool,
    /// Changes of the kept lighting levels the app has not taken yet.
    light_changes: Vec<LightChange>,
    /// A read-back found another program (G HUB) putting the lighting back after the economy turned it down.
    lighting_conflict: bool,
}

/// The catalog row behind a device key (`logitech-hidpp:40B5`): the key holds the canonical product id of the
/// model, which the catalog lists under one of its match kinds.
pub fn priors_for_key(key: &DeviceKey) -> Option<&'static Priors> {
    let (provider, rest) = key.0.split_once(':')?;
    if rest.contains(':') {
        return None;
    }
    let product = u16::from_str_radix(rest, 16).ok()?;
    ["wpid", "dongle", "usb", "receiver"]
        .into_iter()
        .find_map(|m| estimator::priors(provider, m, product))
}

fn wall_of(now: &Now) -> i64 {
    now.wall.floor() as i64
}

impl<H: Host> Live<H> {
    /// Restores every stored history (rewritten compacted), gives every known device an offline presence so the
    /// time the app was closed never counts as use, and hands the RGB economy flag to every tracker.
    pub fn start(
        host: H,
        histories: Vec<(DeviceKey, String)>,
        cfg: StartConfig,
        now: Now,
    ) -> Live<H> {
        let list = UserList {
            registered: Vec::new(),
            removed: cfg.hidden,
        };
        let mut live = Live {
            host,
            lang: cfg.lang,
            prefs: cfg.prefs,
            rgb_economy: cfg.rgb_economy,
            provider: Logitech::new(),
            registry: Registry::with_user_list(&list),
            trackers: BTreeMap::new(),
            wake: None,
            raw_notices: Vec::new(),
            dirty: true,
            closed: false,
            light_changes: Vec::new(),
            lighting_conflict: false,
        };
        live.note(now.mono, || "start".to_string());
        // The economy: what an earlier run kept, then the switch as the user left it (for every device, those
        // that appear later too).
        let kept: Vec<(DeviceKey, SavedLight)> = cfg
            .lights
            .iter()
            .filter_map(|(k, t)| Some((k.clone(), SavedLight::from_text(t)?)))
            .collect();
        live.provider.set_saved_lights(kept);
        if live.rgb_economy {
            let actions = live.provider.set_economy(true, now);
            live.execute(actions, now);
        }
        for (key, text) in histories {
            let priors = priors_for_key(&key);
            let name = priors.map_or_else(|| key.0.clone(), |p| p.model.to_string());
            match Tracker::restore(
                &text,
                &name,
                priors,
                wall_of(&now),
                i64::from(now.utc_offset),
                live.lang,
            ) {
                Ok(mut tracker) => {
                    live.prefs.apply(&mut tracker);
                    live.host.replace(&key, &tracker.record.encode_text());
                    live.trackers.insert(key, tracker);
                }
                Err(e) => {
                    live.note(now.mono, || {
                        format!("history {} unreadable: {}", key.0, e.reason)
                    });
                    live.host.set_aside(&key);
                }
            }
        }
        let keys: Vec<DeviceKey> = live.trackers.keys().cloned().collect();
        for key in &keys {
            live.presence_of(key, false, now.wall, &now);
        }
        for key in &keys {
            live.rgb_of(key, &now);
        }
        live.schedule_batches();
        live
    }

    #[cfg(test)]
    pub fn host(&self) -> &H {
        &self.host
    }

    #[cfg(test)]
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    // ------------------------------------------------------------ events in

    /// One transport event.
    pub fn on_event(&mut self, event: TransportEvent, now: Now) {
        if self.closed {
            return;
        }
        self.note(now.mono, || format!("event {}", describe_event(&event)));
        let actions = self.provider.handle(event, now);
        self.execute(actions, now);
    }

    /// The timer fired: the provider's timeouts and refreshes, and the history writes that have come due.
    /// A wake that nothing needs any more is normal; the provider asks again for what it still wants.
    pub fn on_tick(&mut self, now: Now) {
        self.wake = None;
        if self.closed {
            return;
        }
        self.note(now.mono, || "tick".to_string());
        let actions = self.provider.tick(now);
        self.execute(actions, now);
        let keys: Vec<DeviceKey> = self.trackers.keys().cloned().collect();
        for key in keys {
            let lines = self.trackers.get_mut(&key).and_then(|t| t.tick(now.mono));
            if let Some(lines) = lines {
                self.persist(&key, &lines, now.mono);
            }
        }
        self.schedule_batches();
    }

    /// The user flipped the power saver: every tracker learns it (a new anchor, another prior).
    pub fn set_rgb_economy(&mut self, on: bool, now: Now) {
        if self.rgb_economy == on {
            return;
        }
        self.rgb_economy = on;
        let keys: Vec<DeviceKey> = self.trackers.keys().cloned().collect();
        for key in &keys {
            self.rgb_of(key, &now);
        }
        if !on {
            self.lighting_conflict = false;
        }
        // The switch reaches the devices: lighting down on every online device that has it, back up on the way off.
        let actions = self.provider.set_economy(on, now);
        self.execute(actions, now);
        self.dirty = true;
        self.schedule_batches();
    }

    /// Shows or hides a device; true when its state changed.
    #[allow(dead_code)] // as `DeviceChoice`
    pub fn set_shown(&mut self, key: &DeviceKey, shown: bool) -> bool {
        let changed = !self.registry.set_registered(key, shown).is_empty();
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// Devices the user hid, seen or not, for `settings.txt`.
    #[allow(dead_code)] // as `DeviceChoice`
    pub fn hidden(&self) -> Vec<DeviceKey> {
        self.registry.user_list().removed
    }

    /// Every device seen this run, by name, for the "Dispositivos" menu.
    #[allow(dead_code)] // as `DeviceChoice`
    pub fn devices(&self) -> Vec<DeviceChoice> {
        let mut out: Vec<DeviceChoice> = self
            .registry
            .entries()
            .iter()
            .map(|e| DeviceChoice {
                key: e.info.key.clone(),
                name: e.info.name.clone(),
                shown: e.registered,
            })
            .collect();
        out.sort_by(|a, b| (a.name.to_lowercase(), &a.key).cmp(&(b.name.to_lowercase(), &b.key)));
        out
    }

    /// Every device off, every tracker flushed: the lines reach the files before the tray icon goes.
    pub fn quit(&mut self, now: Now) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.note(now.mono, || "quit".to_string());
        // A device the economy turned down gets its level back before the app goes; the level stays kept in case
        // the write does not arrive.
        let actions = self.provider.release_lights();
        self.execute(actions, now);
        let keys: Vec<DeviceKey> = self.trackers.keys().cloned().collect();
        for key in &keys {
            self.presence_of(key, false, now.wall, &now);
            let lines = self.trackers.get_mut(key).and_then(|t| t.flush(now.mono));
            if let Some(lines) = lines {
                self.persist(key, &lines, now.mono);
            }
        }
    }

    // ------------------------------------------------------------ what the app takes out

    /// The changes of the kept lighting levels since the last call, for `settings.txt`.
    pub fn take_light_changes(&mut self) -> Vec<LightChange> {
        std::mem::take(&mut self.light_changes)
    }

    /// True while the last read-back found G HUB (or the device) putting the lighting back.
    pub fn lighting_conflict(&self) -> bool {
        self.lighting_conflict
    }

    /// True once after anything shown may have changed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn snapshot(&self, now: &Now) -> Snapshot {
        let inputs: Vec<DeviceInput<'_>> = self
            .registry
            .registered()
            .filter_map(|e| {
                let t = self.trackers.get(&e.info.key)?;
                Some(DeviceInput {
                    info: &e.info,
                    history: &t.history,
                    readings: &t.record.readings,
                })
            })
            .collect();
        Snapshot::build_with(&inputs, now, self.lang.tag(), self.prefs.limits)
    }

    /// The user changed the icon's limits or the alerts: every tracker learns them and the next snapshot shows them.
    pub fn set_prefs(&mut self, prefs: Prefs) {
        if self.prefs == prefs {
            return;
        }
        self.prefs = prefs;
        for t in self.trackers.values_mut() {
            prefs.apply(t);
        }
        self.dirty = true;
    }

    /// The user changed the language: the texts of the snapshot and of the notices follow at once.
    pub fn set_lang(&mut self, lang: Lang) {
        if self.lang == lang {
            return;
        }
        self.lang = lang;
        for t in self.trackers.values_mut() {
            t.history.lang = lang;
        }
        self.dirty = true;
    }

    /// The balloons the estimators decided on since the last call, each once, for shown devices only.
    pub fn take_notices(&mut self, now: &Now) -> Vec<Notice> {
        if self.raw_notices.is_empty() {
            return Vec::new();
        }
        let raw = std::mem::take(&mut self.raw_notices);
        let snap = self.snapshot(now);
        let mut out = Vec::new();
        for (key, n) in &raw {
            // A device that is low is not also told it will not last the day: one balloon says it.
            let low_too = raw
                .iter()
                .any(|(k, o)| k == key && o.kind == NoticeKind::Low);
            if n.kind == NoticeKind::Smart && low_too {
                continue;
            }
            if (n.kind == NoticeKind::Full && !self.prefs.full)
                || (n.kind == NoticeKind::Smart && !self.prefs.smart)
            {
                continue;
            }
            let Some(view) = snap.devices.iter().find(|d| &d.key == key) else {
                continue;
            };
            let notice = notice_for(n, view, self.lang);
            self.note(now.mono, || {
                format!("notice {} | {}", notice.title, notice.body)
            });
            out.push(notice);
        }
        out
    }

    // ------------------------------------------------------------ the executor

    /// Carries out what the provider asked for.
    pub fn execute(&mut self, actions: Vec<Action>, now: Now) {
        for action in actions {
            self.note(now.mono, || format!("action {}", describe_action(&action)));
            match action {
                Action::Send(endpoint, bytes) => {
                    if !self.host.send(endpoint, &bytes) {
                        self.note(now.mono, || format!("send to {endpoint} refused"));
                    }
                }
                Action::Emit(event) => self.emit(event, &now),
                Action::Persist(key, lines) => self.persist(&key, &lines, now.mono),
                Action::Wake(at) => self.want_wake(at),
            }
        }
        self.collect_lighting(&now);
        self.schedule_batches();
    }

    /// What the lighting driver reported since: levels to keep, and whether G HUB is putting the lighting back.
    fn collect_lighting(&mut self, now: &Now) {
        for event in self.provider.take_lighting_events() {
            self.note(now.mono, || format!("lighting {event:?}"));
            match event {
                LightingEvent::Saved(key, saved) => {
                    self.light_changes
                        .push(LightChange::Keep(key, saved.to_text()));
                }
                LightingEvent::Cleared(key) => self.light_changes.push(LightChange::Drop(key)),
                LightingEvent::Reapplied(..) => self.set_conflict(true),
                LightingEvent::Off(_) | LightingEvent::Restored(_) => self.set_conflict(false),
                LightingEvent::Failed(..) => {}
            }
        }
    }

    fn set_conflict(&mut self, on: bool) {
        if self.lighting_conflict != on {
            self.lighting_conflict = on;
            self.dirty = true;
        }
    }

    fn emit(&mut self, event: ProviderEvent, now: &Now) {
        self.dirty = true;
        self.registry.apply(&event);
        match event {
            ProviderEvent::Appeared(info) => self.ensure_tracker(&info, now),
            ProviderEvent::Disappeared(key) => self.presence_of(&key, false, now.wall, now),
            ProviderEvent::Presence(key, p) => self.presence_of(&key, p.online, p.at, now),
            ProviderEvent::Battery(key, b) => {
                let Some(t) = self.trackers.get_mut(&key) else {
                    return;
                };
                t.history.utc_offset = i64::from(now.utc_offset);
                let (notices, lines) = t.on_reading(Reading::from_battery(&b), now.mono);
                self.after_tracker(&key, notices, lines, now.mono);
            }
        }
    }

    fn ensure_tracker(&mut self, info: &DeviceInfo, now: &Now) {
        if let Some(t) = self.trackers.get_mut(&info.key) {
            t.history.name = info.name.clone();
            return;
        }
        let mut tracker = Tracker::for_model(
            &info.name,
            priors_for_key(&info.key),
            i64::from(now.utc_offset),
            self.lang,
        );
        self.prefs.apply(&mut tracker);
        self.trackers.insert(info.key.clone(), tracker);
        self.rgb_of(&info.key, now);
    }

    /// A presence moment (wall time `at`) into a device's tracker.
    fn presence_of(&mut self, key: &DeviceKey, online: bool, at: f64, now: &Now) {
        let Some(t) = self.trackers.get_mut(key) else {
            return;
        };
        t.history.utc_offset = i64::from(now.utc_offset);
        let p = estimator::Presence::from(Presence { online, at });
        let (notices, lines) = t.on_presence(p, now.mono);
        self.after_tracker(key, notices, lines, now.mono);
    }

    /// The RGB economy flag into a device's tracker (it writes a line only when the flag changes).
    fn rgb_of(&mut self, key: &DeviceKey, now: &Now) {
        let on = self.rgb_economy;
        let Some(t) = self.trackers.get_mut(key) else {
            return;
        };
        if let Some(lines) = t.on_rgb(wall_of(now), on, now.mono) {
            self.persist(key, &lines, now.mono);
        }
    }

    fn after_tracker(
        &mut self,
        key: &DeviceKey,
        notices: Vec<CoreNotice>,
        lines: Option<Vec<String>>,
        mono: f64,
    ) {
        for n in notices {
            self.raw_notices.push((key.clone(), n));
        }
        if let Some(lines) = lines {
            self.persist(key, &lines, mono);
        }
    }

    fn persist(&mut self, key: &DeviceKey, lines: &[String], mono: f64) {
        self.note(mono, || {
            format!("persist {} {} line(s)", key.0, lines.len())
        });
        self.host.append(key, lines);
    }

    // ------------------------------------------------------------ time

    /// Keeps one pending moment, the earliest asked for. A later request leaves the timer alone (when it
    /// fires the provider asks again for what it still needs); an earlier one re-arms it.
    fn want_wake(&mut self, at: f64) {
        if self.wake.is_none_or(|w| at < w) {
            self.wake = Some(at);
            self.host.set_wake(at);
        }
    }

    /// The trackers' batched writes also need a moment to be released.
    fn schedule_batches(&mut self) {
        let due = self
            .trackers
            .values()
            .filter_map(Tracker::due)
            .min_by(f64::total_cmp);
        if let Some(at) = due {
            self.want_wake(at);
        }
    }

    // ------------------------------------------------------------ the log

    fn note(&mut self, mono: f64, line: impl FnOnce() -> String) {
        if self.host.logging() {
            let text = format!("{mono:9.3} {}", line());
            self.host.log(&text);
        }
    }
}

/// The balloon for an estimator decision: the title names the device, the body says why.
fn notice_for(n: &CoreNotice, view: &DeviceView, lang: Lang) -> Notice {
    let percent = view.percent.map_or(String::new(), |p| p.to_string());
    let with_level = || strings::fmt(lang, id::LOW_WARNING, &[&view.name, &percent]);
    match n.kind {
        NoticeKind::Full => Notice {
            title: strings::fmt(lang, id::NOTICE_FULL_TITLE, &[&view.name]),
            body: strings::get(lang, id::NOTICE_FULL_BODY).to_string(),
        },
        NoticeKind::Low => Notice {
            title: n.text.clone().unwrap_or_else(with_level),
            body: view.status.clone(),
        },
        NoticeKind::Smart => Notice {
            title: with_level(),
            body: n.text.clone().unwrap_or_else(|| view.status.clone()),
        },
    }
}

/// The bytes of a report as hex, trailing zeros dropped (reports are zero-padded to the endpoint's length).
fn hex(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |i| i + 1)
        .max(4.min(bytes.len()));
    bytes[..end]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe_event(e: &TransportEvent) -> String {
    match e {
        TransportEvent::Arrived(ep) => format!(
            "arrived id={} {:04X}:{:04X} page={:04X} usage={} in={} out={} {:?}",
            ep.id, ep.vid, ep.pid, ep.usage_page, ep.usage, ep.input_len, ep.output_len, ep.product
        ),
        TransportEvent::Removed(id) => format!("removed id={id}"),
        TransportEvent::Report(id, bytes) => format!("report id={id} {}", hex(bytes)),
    }
}

fn describe_action(a: &Action) -> String {
    match a {
        Action::Send(id, bytes) => format!("send id={id} {}", hex(bytes)),
        Action::Emit(ProviderEvent::Appeared(i)) => format!(
            "appeared {} {:?} {:?} {:?}",
            i.key.0, i.name, i.kind, i.connection
        ),
        Action::Emit(ProviderEvent::Disappeared(k)) => format!("disappeared {}", k.0),
        Action::Emit(ProviderEvent::Presence(k, p)) => {
            format!(
                "presence {} {}",
                k.0,
                if p.online { "online" } else { "offline" }
            )
        }
        Action::Emit(ProviderEvent::Battery(k, b)) => format!(
            "battery {} percent={} quality={:?} {:?} mv={}",
            k.0,
            b.percent.map_or("-".to_string(), |p| format!("{p:.1}")),
            b.quality,
            b.charging,
            b.millivolts.map_or("-".to_string(), |m| m.to_string())
        ),
        Action::Persist(k, lines) => format!("persist {} {} line(s)", k.0, lines.len()),
        Action::Wake(at) => format!("wake {at:.3}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_core::hidpp::catalog::Match;
    use juicebar_core::registry::logitech_key;
    use juicebar_core::store::DeviceRecord;
    use juicebar_core::types::{BatteryReading, Charging, Connection, Endpoint, Kind, Quality};

    #[derive(Default)]
    struct Fake {
        sent: Vec<(u32, Vec<u8>)>,
        appended: Vec<(DeviceKey, Vec<String>)>,
        replaced: Vec<(DeviceKey, String)>,
        aside: Vec<DeviceKey>,
        wakes: Vec<f64>,
        logs: Vec<String>,
        log_on: bool,
        refuse_send: bool,
    }

    impl Host for Fake {
        fn send(&mut self, endpoint: u32, bytes: &[u8]) -> bool {
            self.sent.push((endpoint, bytes.to_vec()));
            !self.refuse_send
        }
        fn append(&mut self, key: &DeviceKey, lines: &[String]) {
            self.appended.push((key.clone(), lines.to_vec()));
        }
        fn replace(&mut self, key: &DeviceKey, text: &str) {
            self.replaced.push((key.clone(), text.to_string()));
        }
        fn set_aside(&mut self, key: &DeviceKey) {
            self.aside.push(key.clone());
        }
        fn set_wake(&mut self, at_mono: f64) {
            self.wakes.push(at_mono);
        }
        fn logging(&self) -> bool {
            self.log_on
        }
        fn log(&mut self, line: &str) {
            self.logs.push(line.to_string());
        }
    }

    // 2026-10-26 14:30 in Sao Paulo.
    const T0: f64 = 1_793_035_800.0;

    fn at(secs: f64) -> Now {
        Now {
            wall: T0 + secs,
            mono: secs,
            utc_offset: -10800,
        }
    }

    fn cfg() -> StartConfig {
        StartConfig {
            lang: Lang::PtBr,
            prefs: Prefs::default(),
            rgb_economy: false,
            hidden: Vec::new(),
            lights: Vec::new(),
        }
    }

    fn g703() -> DeviceKey {
        logitech_key(Match::Wpid, 0x4086)
    }

    fn g915() -> DeviceKey {
        logitech_key(Match::Wpid, 0x40B5)
    }

    fn info(key: &DeviceKey, name: &str, kind: Kind) -> DeviceInfo {
        DeviceInfo {
            key: key.clone(),
            provider: "logitech-hidpp",
            name: name.into(),
            kind,
            connection: Connection::Receiver,
            brand: "Logitech".into(),
        }
    }

    fn appear(key: &DeviceKey, name: &str) -> Action {
        Action::Emit(ProviderEvent::Appeared(info(key, name, Kind::Mouse)))
    }

    fn online(key: &DeviceKey, online: bool, secs: f64) -> Action {
        Action::Emit(ProviderEvent::Presence(
            key.clone(),
            Presence {
                online,
                at: T0 + secs,
            },
        ))
    }

    fn battery(key: &DeviceKey, percent: f64, charging: Charging, secs: f64) -> Action {
        Action::Emit(ProviderEvent::Battery(
            key.clone(),
            BatteryReading {
                at: T0 + secs,
                percent: Some(percent),
                coarse: None,
                millivolts: None,
                charging,
                quality: Quality::Exact,
            },
        ))
    }

    fn live(histories: Vec<(DeviceKey, String)>) -> Live<Fake> {
        Live::start(Fake::default(), histories, cfg(), at(0.0))
    }

    fn lines_for<'a>(live: &'a Live<Fake>, key: &DeviceKey) -> Vec<&'a str> {
        live.host()
            .appended
            .iter()
            .filter(|(k, _)| k == key)
            .flat_map(|(_, l)| l.iter().map(String::as_str))
            .collect()
    }

    #[test]
    fn a_seen_device_registers_and_shows_with_its_battery() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 61.0, Charging::Discharging, 1.5),
            ],
            at(2.0),
        );
        let snap = l.snapshot(&at(2.0));
        assert_eq!(snap.devices.len(), 1);
        let d = &snap.devices[0];
        assert_eq!((d.name.as_str(), d.on, d.percent), ("G703", true, Some(61)));
        assert_eq!(snap.on_count, 1);
        assert!(l.take_dirty());
        assert!(!l.take_dirty());
        // The first presence is written at once, with the estimator state; the reading waits for the batch.
        let written = lines_for(&l, &g703());
        assert_eq!(
            written[0],
            format!("P {} 1", (T0 + 1.0) as i64),
            "{written:?}"
        );
        assert!(written.last().unwrap().starts_with("S "));
        assert!(!written.iter().any(|w| w.starts_with("R ")));
    }

    #[test]
    fn persist_actions_are_appended_to_that_devices_file() {
        let mut l = live(vec![]);
        l.execute(
            vec![Action::Persist(
                g915(),
                vec!["P 5 1".into(), "P 6 0".into()],
            )],
            at(1.0),
        );
        assert_eq!(
            l.host().appended,
            vec![(g915(), vec!["P 5 1".to_string(), "P 6 0".to_string()])]
        );
    }

    #[test]
    fn sends_go_to_the_transport_and_a_refusal_is_only_logged() {
        let mut l = live(vec![]);
        l.host_mut().log_on = true;
        l.host_mut().refuse_send = true;
        l.execute(vec![Action::Send(7, vec![0x10, 0xFF, 0x80, 0x00])], at(1.0));
        assert_eq!(l.host().sent, vec![(7, vec![0x10, 0xFF, 0x80, 0x00])]);
        assert!(
            l.host()
                .logs
                .iter()
                .any(|s| s.contains("send to 7 refused"))
        );
    }

    #[test]
    fn a_receiver_arriving_makes_the_provider_send_and_ask_for_a_wake() {
        let mut l = live(vec![]);
        let ep = |id, out| Endpoint {
            id,
            vid: 0x046D,
            pid: 0xC539,
            usage_page: 0xFF00,
            usage: if out == 7 { 1 } else { 2 },
            input_len: out,
            output_len: out,
            product: "USB Receiver".into(),
            bluetooth: false,
        };
        l.on_event(TransportEvent::Arrived(ep(1, 7)), at(1.0));
        l.on_event(TransportEvent::Arrived(ep(2, 20)), at(1.0));
        assert!(!l.host().sent.is_empty(), "the receiver is asked something");
        assert_eq!(l.host().wakes.len(), 1, "one timer for the request timeout");
        assert!(l.host().wakes[0] > 1.0);
    }

    #[test]
    fn a_wake_replaces_the_timer_and_never_stacks() {
        let mut l = live(vec![]);
        l.execute(vec![Action::Wake(200.0)], at(1.0));
        l.execute(vec![Action::Wake(150.0)], at(1.0));
        // A later moment than the one armed leaves the timer alone.
        l.execute(vec![Action::Wake(300.0), Action::Wake(150.0)], at(1.0));
        assert_eq!(l.host().wakes, vec![200.0, 150.0]);
        // The timer fires: the next request arms it again.
        l.on_tick(at(150.0));
        l.execute(vec![Action::Wake(300.0)], at(150.0));
        assert_eq!(l.host().wakes, vec![200.0, 150.0, 300.0]);
    }

    #[test]
    fn batched_lines_are_written_when_their_moment_comes() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 60.0, Charging::Discharging, 2.0),
            ],
            at(2.0),
        );
        assert!(!lines_for(&l, &g703()).iter().any(|w| w.starts_with("R ")));
        // The tracker asked for a wake 300 s after its first write.
        let due = *l.host().wakes.last().expect("a batch wake");
        assert!((due - 301.0).abs() < 1.5, "{due}");
        l.on_tick(at(due + 0.1));
        let written = lines_for(&l, &g703());
        assert!(written.iter().any(|w| w.starts_with("R ")), "{written:?}");
    }

    #[test]
    fn a_stored_history_is_restored_compacted_and_closed_offline() {
        let mut rec = DeviceRecord::new();
        rec.presence.push(estimator::Presence {
            at: (T0 - 7200.0) as i64,
            online: true,
        });
        rec.readings.push(Reading::new(
            (T0 - 3600.0) as i64,
            Some(42.0),
            Charging::Discharging,
            Quality::Exact,
        ));
        let text = rec.encode_text();
        let mut l = live(vec![(g703(), text)]);
        // The file is rewritten in its compacted form before anything else.
        assert_eq!(l.host().replaced.len(), 1);
        assert_eq!(l.host().replaced[0].0, g703());
        assert!(l.host().replaced[0].1.starts_with("juicebar-history 1\n"));
        // The closed time does not count as use: an offline moment is written for the start.
        let written = lines_for(&l, &g703());
        assert!(
            written.iter().any(|w| *w == format!("P {} 0", T0 as i64)),
            "{written:?}"
        );
        // Not shown until the device is seen again, then it is off with its last level.
        assert!(l.snapshot(&at(1.0)).devices.is_empty());
        l.execute(vec![appear(&g703(), "G703")], at(1.0));
        let snap = l.snapshot(&at(1.0));
        let d = &snap.devices[0];
        assert!(!d.on);
        assert_eq!(d.percent, Some(42));
        assert!(d.status.starts_with("desligado"), "{}", d.status);
    }

    #[test]
    fn an_unreadable_history_is_set_aside_and_the_device_starts_fresh() {
        let mut l = live(vec![(g703(), "juicebar-history 9\nP 1 1\n".to_string())]);
        assert_eq!(l.host().aside, vec![g703()]);
        assert!(l.host().replaced.is_empty());
        l.execute(vec![appear(&g703(), "G703")], at(1.0));
        assert_eq!(l.snapshot(&at(1.0)).devices.len(), 1);
    }

    #[test]
    fn quitting_closes_every_device_and_flushes_before_returning() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 55.0, Charging::Discharging, 2.0),
                appear(&g915(), "G915 X"),
                online(&g915(), true, 3.0),
            ],
            at(4.0),
        );
        l.quit(at(10.0));
        let w = lines_for(&l, &g703());
        let offline = format!("P {} 0", (T0 + 10.0) as i64);
        assert!(w.iter().any(|x| *x == offline), "{w:?}");
        assert!(
            w.iter().any(|x| x.starts_with("R ")),
            "the batched reading is flushed: {w:?}"
        );
        let w2 = lines_for(&l, &g915());
        assert!(w2.iter().any(|x| *x == offline), "{w2:?}");
        // Nothing is processed after the quit.
        let before = l.host().appended.len();
        l.execute(vec![online(&g703(), true, 11.0)], at(11.0));
        l.on_event(TransportEvent::Removed(1), at(12.0));
        l.on_tick(at(500.0));
        assert_eq!(l.host().appended.len(), before);
    }

    #[test]
    fn a_hidden_device_leaves_the_snapshot_and_comes_back() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                appear(&g915(), "G915 X"),
                online(&g915(), true, 1.0),
            ],
            at(2.0),
        );
        assert_eq!(l.snapshot(&at(2.0)).devices.len(), 2);
        assert!(l.set_shown(&g703(), false));
        assert!(!l.set_shown(&g703(), false));
        let snap = l.snapshot(&at(2.0));
        assert_eq!(snap.devices.len(), 1);
        assert_eq!(snap.devices[0].name, "G915 X");
        assert_eq!(l.hidden(), vec![g703()]);
        let menu = l.devices();
        assert_eq!(
            menu.iter()
                .map(|d| (d.name.as_str(), d.shown))
                .collect::<Vec<_>>(),
            vec![("G703", false), ("G915 X", true)]
        );
        assert!(l.set_shown(&g703(), true));
        assert_eq!(l.snapshot(&at(2.0)).devices.len(), 2);
        assert!(l.hidden().is_empty());
    }

    #[test]
    fn a_device_hidden_in_an_earlier_run_stays_hidden_when_seen() {
        let c = StartConfig {
            hidden: vec![g703()],
            lights: Vec::new(),
            ..cfg()
        };
        let mut l = Live::start(Fake::default(), vec![], c, at(0.0));
        l.execute(
            vec![appear(&g703(), "G703"), online(&g703(), true, 1.0)],
            at(2.0),
        );
        assert!(l.snapshot(&at(2.0)).devices.is_empty());
        assert_eq!(l.devices().len(), 1);
        assert_eq!(l.hidden(), vec![g703()]);
    }

    #[test]
    fn the_rgb_flag_reaches_every_tracker_and_is_written_once() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                appear(&g915(), "G915 X"),
                online(&g915(), true, 1.0),
            ],
            at(2.0),
        );
        l.set_rgb_economy(true, at(3.0));
        l.set_rgb_economy(true, at(4.0));
        // The lines ride the batch like any other; the timer was armed for it.
        l.on_tick(at(305.0));
        for key in [g703(), g915()] {
            let w = lines_for(&l, &key);
            let g: Vec<&&str> = w.iter().filter(|x| x.starts_with("G ")).collect();
            assert_eq!(g.len(), 1, "{w:?}");
            assert_eq!(*g[0], format!("G {} 1", (T0 + 3.0) as i64));
            assert!(l.trackers[&key].history.rgb);
        }
        // A device seen later starts with the flag already on.
        let g733 = logitech_key(Match::Dongle, 0x0AFE);
        l.execute(vec![appear(&g733, "G733")], at(5.0));
        assert!(l.trackers[&g733].history.rgb);
        assert!(
            lines_for(&l, &g733)
                .iter()
                .any(|x| x.starts_with("G ") && x.ends_with(" 1"))
        );
    }

    #[test]
    fn the_flag_set_at_start_is_given_to_restored_trackers() {
        let mut rec = DeviceRecord::new();
        rec.presence.push(estimator::Presence {
            at: 100,
            online: false,
        });
        let c = StartConfig {
            rgb_economy: true,
            ..cfg()
        };
        let l = Live::start(
            Fake::default(),
            vec![(g915(), rec.encode_text())],
            c,
            at(0.0),
        );
        assert!(l.trackers[&g915()].history.rgb);
    }

    #[test]
    fn a_low_battery_is_announced_once() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 30.0, Charging::Discharging, 2.0),
            ],
            at(3.0),
        );
        assert!(l.take_notices(&at(3.0)).is_empty());
        l.execute(
            vec![battery(&g703(), 8.0, Charging::Discharging, 600.0)],
            at(600.0),
        );
        let n = l.take_notices(&at(600.0));
        assert_eq!(n.len(), 1, "{n:?}");
        assert_eq!(n[0].title, "G703 com 8%");
        l.execute(
            vec![battery(&g703(), 7.0, Charging::Discharging, 1200.0)],
            at(1200.0),
        );
        assert!(l.take_notices(&at(1200.0)).is_empty());
    }

    #[test]
    fn a_full_charge_is_announced_once_and_not_for_a_hidden_device() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g915(), "G915 X"),
                online(&g915(), true, 1.0),
                battery(&g915(), 90.0, Charging::Charging, 2.0),
                battery(&g915(), 100.0, Charging::Full, 900.0),
            ],
            at(900.0),
        );
        let n = l.take_notices(&at(900.0));
        assert_eq!(n.len(), 1, "{n:?}");
        assert_eq!(n[0].title, "G915 X: bateria cheia");
        assert_eq!(n[0].body, "pode tirar do cabo");
        l.set_shown(&g915(), false);
        l.execute(
            vec![
                battery(&g915(), 50.0, Charging::Discharging, 1000.0),
                battery(&g915(), 100.0, Charging::Full, 2000.0),
            ],
            at(2000.0),
        );
        // The charge cycle restarted, but the device is hidden: no balloon.
        assert!(l.take_notices(&at(2000.0)).is_empty());
    }

    #[test]
    fn the_full_charge_notice_can_be_switched_off_and_on() {
        let mut l = live(vec![]);
        l.set_prefs(Prefs {
            full: false,
            ..Prefs::default()
        });
        l.execute(
            vec![
                appear(&g915(), "G915 X"),
                online(&g915(), true, 1.0),
                battery(&g915(), 90.0, Charging::Charging, 2.0),
                battery(&g915(), 100.0, Charging::Full, 900.0),
            ],
            at(900.0),
        );
        assert!(l.take_notices(&at(900.0)).is_empty(), "off: no balloon");
        l.set_prefs(Prefs::default());
        l.execute(
            vec![
                battery(&g915(), 50.0, Charging::Discharging, 1000.0),
                battery(&g915(), 90.0, Charging::Charging, 1500.0),
                battery(&g915(), 100.0, Charging::Full, 2000.0),
            ],
            at(2000.0),
        );
        assert_eq!(
            l.take_notices(&at(2000.0)).len(),
            1,
            "on again: the next one shows"
        );
    }

    #[test]
    fn the_low_level_setting_moves_the_low_battery_balloon() {
        let mut l = live(vec![]);
        l.set_prefs(Prefs {
            low_percent: 25,
            ..Prefs::default()
        });
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 50.0, Charging::Discharging, 2.0),
                battery(&g703(), 22.0, Charging::Discharging, 600.0),
            ],
            at(600.0),
        );
        let n = l.take_notices(&at(600.0));
        assert!(
            n.iter().any(|n| n.title == "G703 com 22%"),
            "{n:?}: 22 % is under the 25 % the user chose"
        );
        let mut d = live(vec![]);
        d.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 50.0, Charging::Discharging, 2.0),
                battery(&g703(), 22.0, Charging::Discharging, 600.0),
            ],
            at(600.0),
        );
        assert!(
            d.take_notices(&at(600.0))
                .iter()
                .all(|n| n.title != "G703 com 22%"),
            "the default 10 % stays quiet at 22 %"
        );
    }

    #[test]
    fn the_icon_limits_recolor_the_snapshot_at_once() {
        use juicebar_core::snapshot::Juice;
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 60.0, Charging::Discharging, 2.0),
            ],
            at(3.0),
        );
        assert_eq!(
            l.snapshot(&at(3.0)).icon.juice,
            Juice::Green,
            "60 is over 50"
        );
        l.take_dirty();
        l.set_prefs(Prefs {
            limits: Limits::new(20, 80),
            ..Prefs::default()
        });
        assert!(l.take_dirty(), "the app takes a new snapshot");
        assert_eq!(
            l.snapshot(&at(3.0)).icon.juice,
            Juice::Purple,
            "60 is under 80"
        );
        l.set_prefs(Prefs {
            limits: Limits::new(65, 90),
            ..Prefs::default()
        });
        assert_eq!(
            l.snapshot(&at(3.0)).icon.juice,
            Juice::Red,
            "60 is under 65"
        );
        l.take_dirty();
        l.set_prefs(Prefs {
            limits: Limits::new(65, 90),
            ..Prefs::default()
        });
        assert!(!l.take_dirty(), "the same settings change nothing");
    }

    #[test]
    fn the_smart_warning_switch_reaches_the_snapshot() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g703(), "G703"),
                online(&g703(), true, 1.0),
                battery(&g703(), 35.0, Charging::Discharging, 2.0),
            ],
            at(3.0),
        );
        let warned = l.snapshot(&at(3.0)).devices[0].warning.is_some();
        l.set_prefs(Prefs {
            smart: false,
            ..Prefs::default()
        });
        let s = l.snapshot(&at(3.0));
        assert!(s.devices[0].warning.is_none());
        assert_ne!(s.icon.juice, juicebar_core::snapshot::Juice::Red);
        l.set_prefs(Prefs::default());
        assert_eq!(l.snapshot(&at(3.0)).devices[0].warning.is_some(), warned);
    }

    #[test]
    fn a_new_language_changes_the_texts_of_the_snapshot_and_of_later_notices() {
        let mut l = live(vec![]);
        l.execute(
            vec![
                appear(&g915(), "G915 X"),
                online(&g915(), true, 1.0),
                battery(&g915(), 90.0, Charging::Charging, 2.0),
            ],
            at(3.0),
        );
        assert!(
            l.snapshot(&at(3.0)).devices[0]
                .connection
                .contains("Receptor")
        );
        l.take_dirty();
        l.set_lang(Lang::En);
        assert!(l.take_dirty());
        assert!(
            l.snapshot(&at(3.0)).devices[0]
                .connection
                .contains("Receiver")
        );
        l.execute(
            vec![battery(&g915(), 100.0, Charging::Full, 900.0)],
            at(900.0),
        );
        let n = l.take_notices(&at(900.0));
        assert_eq!(n[0].title, "G915 X: battery full");
        l.take_dirty();
        l.set_lang(Lang::En);
        assert!(!l.take_dirty(), "the same language changes nothing");
    }

    #[test]
    fn the_priors_come_from_the_catalog_by_the_models_key() {
        let p = priors_for_key(&g915()).expect("G915 X");
        assert_eq!(p.use_h, Some(800.0));
        assert!(priors_for_key(&logitech_key(Match::Dongle, 0x0AFE)).is_some());
        assert!(priors_for_key(&DeviceKey("os-bluetooth:ab:cd".into())).is_none());
        assert!(priors_for_key(&DeviceKey("logitech-hidpp:ZZZZ".into())).is_none());
        let l = {
            let mut l = live(vec![]);
            l.execute(vec![appear(&g915(), "G915 X")], at(1.0));
            l
        };
        assert_eq!(l.trackers[&g915()].history.rated_use_hours, 800.0);
    }

    #[test]
    fn the_log_is_off_unless_the_host_wants_it_and_has_a_line_per_event_and_action() {
        let mut quiet = live(vec![]);
        quiet.execute(vec![Action::Wake(5.0)], at(1.0));
        assert!(quiet.host().logs.is_empty());
        let mut l = live(vec![]);
        l.host_mut().log_on = true;
        l.on_event(TransportEvent::Removed(3), at(1.0));
        l.execute(vec![appear(&g703(), "G703"), Action::Wake(9.0)], at(2.0));
        let logs = &l.host().logs;
        assert!(logs[0].contains("event removed id=3"), "{logs:?}");
        assert!(
            logs.iter()
                .any(|s| s.contains("action appeared logitech-hidpp:4086"))
        );
        assert!(logs.iter().any(|s| s.contains("action wake 9.000")));
    }

    /// A G915 X on its cable (endpoints 1 short, 2 long), answering the way the measured one does, with the
    /// BrightnessControl feature at index 13.
    struct Keyboard {
        brightness: u16,
        writes: Vec<u16>,
    }

    impl Keyboard {
        /// The answer to a frame the host sent, as the long collection would deliver it.
        fn answer(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
            let (b2, b3) = (frame[2], frame[3]);
            let function = b3 >> 4;
            let mut a = vec![0x11, 0xFF, b2, b3];
            match (b2, function, b3 & 0x0F) {
                (0, 1, _) => a.extend([4, 2, 0x5A]),
                (0, 0, _) => {
                    let id = u16::from_be_bytes([frame[4], frame[5]]);
                    let index = match id {
                        0x1004 => 7,
                        0x0005 => 3,
                        0x8040 => 13,
                        _ => 0,
                    };
                    a.extend([index, 0, 0]);
                }
                (3, 0, _) => a.push(4),
                (3, 1, _) => a.extend(*b"G915"),
                (7, 1, _) => a.extend([49, 4, 0, 0]),
                (13, 0, 0x0C) => a.extend([0, 100, 5, 3, 0, 0]),
                (13, 1, 0x0C) => a.extend(self.brightness.to_be_bytes()),
                (13, 2, 0x0C) => {
                    self.brightness = u16::from_be_bytes([frame[4], frame[5]]);
                    self.writes.push(self.brightness);
                }
                _ => return None,
            }
            a.resize(20, 0);
            Some(a)
        }
    }

    /// Let the keyboard answer everything the host has sent, until it has nothing more to say.
    fn settle(l: &mut Live<Fake>, kb: &mut Keyboard, secs: &mut f64) {
        for _ in 0..200 {
            let sent = std::mem::take(&mut l.host_mut().sent);
            if sent.is_empty() {
                return;
            }
            for (_, frame) in sent {
                if let Some(answer) = kb.answer(&frame) {
                    *secs += 0.01;
                    l.on_event(TransportEvent::Report(2, answer), at(*secs));
                }
            }
        }
        panic!("the loop never settled");
    }

    fn keyboard_live(economy: bool, kept: Vec<(DeviceKey, String)>) -> (Live<Fake>, Keyboard, f64) {
        let mut cfg = cfg();
        cfg.rgb_economy = economy;
        cfg.lights = kept;
        let mut l = Live::start(Fake::default(), vec![], cfg, at(0.0));
        let mut kb = Keyboard {
            brightness: 13,
            writes: Vec::new(),
        };
        let mut secs = 1.0;
        for (id, usage, len) in [(1, 1, 7), (2, 2, 20)] {
            l.on_event(
                TransportEvent::Arrived(Endpoint {
                    id,
                    vid: 0x046D,
                    pid: 0xC356,
                    usage_page: 0xFF00,
                    usage,
                    input_len: len,
                    output_len: len,
                    product: String::new(),
                    bluetooth: false,
                }),
                at(secs),
            );
        }
        settle(&mut l, &mut kb, &mut secs);
        (l, kb, secs)
    }

    #[test]
    fn the_switch_turns_the_lighting_down_keeps_the_level_and_the_way_off_gives_it_back() {
        let (mut l, mut kb, mut secs) = keyboard_live(false, vec![]);
        assert!(kb.writes.is_empty(), "reading a battery never writes");
        l.set_rgb_economy(true, at(secs));
        settle(&mut l, &mut kb, &mut secs);
        assert_eq!(kb.writes, vec![0]);
        assert_eq!(
            l.take_light_changes(),
            vec![LightChange::Keep(g915(), "brightness:13".into())]
        );
        assert!(!l.lighting_conflict());
        l.set_rgb_economy(false, at(secs));
        settle(&mut l, &mut kb, &mut secs);
        assert_eq!(kb.writes, vec![0, 13]);
        assert_eq!(kb.brightness, 13);
        assert_eq!(l.take_light_changes(), vec![LightChange::Drop(g915())]);
    }

    #[test]
    fn g_hub_putting_the_lighting_back_sets_the_conflict_flag_and_the_switch_off_clears_it() {
        let (mut l, mut kb, mut secs) = keyboard_live(false, vec![]);
        l.set_rgb_economy(true, at(secs));
        settle(&mut l, &mut kb, &mut secs);
        l.take_light_changes();
        // G HUB reapplies its lighting; the 3 s read-back finds it.
        kb.brightness = 13;
        secs += 3.5;
        l.on_tick(at(secs));
        settle(&mut l, &mut kb, &mut secs);
        assert!(l.lighting_conflict());
        assert!(l.take_dirty());
        assert_eq!(kb.writes, vec![0], "no fight");
        l.set_rgb_economy(false, at(secs));
        settle(&mut l, &mut kb, &mut secs);
        assert!(!l.lighting_conflict());
        assert_eq!(
            kb.writes,
            vec![0],
            "the level is not ours: nothing is written back"
        );
    }

    #[test]
    fn a_run_that_starts_with_the_switch_on_and_a_kept_level_does_not_keep_the_dark_level() {
        // The last run died with the lighting down (level 13 kept, keyboard at 0); the switch is still on.
        let (mut l, mut kb, mut secs) = keyboard_live(true, vec![(g915(), "brightness:13".into())]);
        kb.brightness = 0;
        secs += 4.0;
        l.on_tick(at(secs));
        settle(&mut l, &mut kb, &mut secs);
        assert!(kb.writes.is_empty());
        assert!(l.take_light_changes().is_empty());
        l.set_rgb_economy(false, at(secs));
        settle(&mut l, &mut kb, &mut secs);
        assert_eq!(kb.brightness, 13);
    }

    #[test]
    fn quitting_with_the_economy_on_writes_the_level_back_before_the_app_goes() {
        let (mut l, mut kb, mut secs) = keyboard_live(false, vec![]);
        l.set_rgb_economy(true, at(secs));
        settle(&mut l, &mut kb, &mut secs);
        l.quit(at(secs + 1.0));
        settle(&mut l, &mut kb, &mut secs);
        assert_eq!(kb.writes, vec![0, 13]);
    }

    #[test]
    fn hex_drops_the_padding_but_keeps_a_short_frame_whole() {
        assert_eq!(
            hex(&[0x11, 0xFF, 0x00, 0x0D, 0x10, 0x00, 0x00]),
            "11 FF 00 0D 10"
        );
        assert_eq!(hex(&[0x10, 0x00, 0x00, 0x00]), "10 00 00 00");
        assert_eq!(hex(&[]), "");
    }
}
