// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! What the UI shows (spec/design/DESIGN.md): the bar icon's state, the tooltip and one `DeviceView` per device with
//! every text already formatted. The core rebuilds it after any event that changed what is shown; the shell redraws
//! only when it differs from the last one (`PartialEq`).

use crate::estimator::{
    self, DeviceHistory, Reading, clock_string, eta, eta_text, forecast_label, forecast_text,
    full_in_text, health_text, low_warning, round_half_up, smart_warning, usage_today_text,
};
use crate::hidpp::catalog;
use crate::strings::{self, Lang, id};
use crate::types::{Charging, Coarse, Connection, DeviceInfo, DeviceKey, Kind, Now, Quality};

/// The Windows tooltip limit, in UTF-16 code units.
pub const TOOLTIP_LIMIT: usize = 127;
/// The chart shows this many days.
pub const CHART_DAYS: i64 = 7;

/// The colour of the juice in the bar icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Juice {
    /// Every online device with a reading is at or above the high limit.
    Green,
    /// Grape juice: nothing to report (also with no device at all).
    #[default]
    Purple,
    /// Some online device is below the low limit, or a smart warning is active.
    Red,
}

/// How the single bar icon looks: the juice colour, and whether any device charges (the mark then lets a few
/// bubbles rise now and then).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IconLook {
    pub juice: Juice,
    pub charging: bool,
}

/// The battery levels that colour the juice (settings of the app).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Below this percent a device turns the juice red.
    pub low: u8,
    /// At or above this percent on every device turns the juice green.
    pub high: u8,
}

impl Limits {
    pub const DEFAULT_LOW: u8 = 20;
    pub const DEFAULT_HIGH: u8 = 50;

    /// The limits as set, made consistent: both at most 100 and the high one never under the low one.
    pub fn new(low: u8, high: u8) -> Limits {
        let low = low.min(100);
        Limits {
            low,
            high: high.clamp(low, 100),
        }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::new(Limits::DEFAULT_LOW, Limits::DEFAULT_HIGH)
    }
}

/// The longest name the user can give a device, in characters.
pub const NAME_MAX: usize = 24;

/// The user's names and order for the devices (the Settings page). A device without an entry keeps the catalog's
/// short name and goes after the ordered ones, devices that are on first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    /// The name the user gave a device, by key.
    pub names: Vec<(DeviceKey, String)>,
    /// The devices the user ordered, first to last.
    pub order: Vec<DeviceKey>,
}

impl Arrangement {
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.order.is_empty()
    }

    /// The name the user gave the device, if any.
    pub fn name_of(&self, key: &DeviceKey) -> Option<&str> {
        self.names
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, n)| n.as_str())
    }

    /// Gives a device a name, or takes the name away when `name` is empty. The name is cleaned first (`clean_name`).
    pub fn set_name(&mut self, key: &DeviceKey, name: &str) {
        let name = clean_name(name);
        self.names.retain(|(k, _)| k != key);
        if !name.is_empty() {
            self.names.push((key.clone(), name));
        }
    }

    /// Where the user put the device; `None` when it was never ordered.
    pub fn rank(&self, key: &DeviceKey) -> Option<usize> {
        self.order.iter().position(|k| k == key)
    }

    /// The sort key of a device named `name` (the name shown): the ordered ones by their place, then the rest with
    /// the devices that are on first and by name.
    pub fn sort_key(
        &self,
        key: &DeviceKey,
        on: bool,
        name: &str,
    ) -> (usize, bool, String, DeviceKey) {
        (
            self.rank(key).unwrap_or(usize::MAX),
            !on,
            name.to_lowercase(),
            key.clone(),
        )
    }

    /// Moves a device one place up (`delta < 0`) or down in the list `current`, which is the list as shown now
    /// (every device, in the order the user sees it). The whole list becomes the user's order, so the move sticks
    /// even for devices that were never ordered. Returns false when the device is not in the list or already at
    /// that end.
    pub fn move_in(&mut self, current: &[DeviceKey], key: &DeviceKey, delta: i32) -> bool {
        let Some(at) = current.iter().position(|k| k == key) else {
            return false;
        };
        let to = at as i64 + i64::from(delta.signum());
        if to < 0 || to >= current.len() as i64 {
            return false;
        }
        let mut order: Vec<DeviceKey> = current.to_vec();
        order.swap(at, to as usize);
        self.order = order;
        true
    }
}

/// A device name as stored: trimmed, no control characters, at most `NAME_MAX` characters.
pub fn clean_name(raw: &str) -> String {
    let kept: String = raw.chars().filter(|c| !c.is_control()).collect();
    kept.trim()
        .chars()
        .take(NAME_MAX)
        .collect::<String>()
        .trim_end()
        .to_string()
}

impl IconLook {
    /// The look of the shown `devices`. `smart_alert` says that a smart warning is active on an online device.
    /// Offline devices and devices without a reading count for nothing but the charging flag (which needs an
    /// online device).
    pub fn of(devices: &[DeviceView], smart_alert: bool, limits: Limits) -> IconLook {
        let read: Vec<u8> = devices
            .iter()
            .filter(|d| d.on)
            .filter_map(|d| d.percent)
            .collect();
        let juice = if smart_alert || read.iter().any(|&p| p < limits.low) {
            Juice::Red
        } else if !read.is_empty() && read.iter().all(|&p| p >= limits.high) {
            Juice::Green
        } else {
            Juice::Purple
        };
        IconLook {
            juice,
            charging: devices.iter().any(|d| d.on && d.charging),
        }
    }
}

/// A point of the level chart: epoch seconds and percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartPoint {
    pub at: i64,
    pub percent: f64,
}

/// The dashed forecast segment, from the last reading to 0 %.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Forecast {
    pub from: ChartPoint,
    pub to: ChartPoint,
}

/// A panel row and its expanded part.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceView {
    pub key: DeviceKey,
    /// The short name from the catalog (`G703`), else the name the device reports.
    pub name: String,
    pub kind: Kind,
    /// "Receptor", "Dongle", "Bluetooth", "Cabo USB".
    pub connection: String,
    /// Whether the device is on (online) now.
    pub on: bool,
    /// The last known level, rounded; `None` for a coarse-only device or before any reading.
    pub percent: Option<u8>,
    pub charging: bool,
    /// The row's one line: "~3 h de uso", "cheio em ~1 h 10 min", "desligado · 71% às 14:02".
    pub status: String,
    /// "estimativa inicial" while the forecast is still learning.
    pub label: Option<String>,
    /// "acaba hoje ~17:40" (expanded row).
    pub eta: Option<String>,
    /// The smart warning or the low-battery text, if one holds now.
    pub warning: Option<String>,
    /// The last 7 days of levels, at most one point per hour.
    pub chart: Vec<ChartPoint>,
    pub forecast: Option<Forecast>,
    /// "Hoje: 4 h 10 min de uso".
    pub usage_today: String,
    /// Seconds of use on each of the last 7 local days, oldest first; the last one is today.
    pub usage_week: [i64; 7],
    /// "Saúde da bateria: 92%" or "... aprendendo".
    pub health: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub icon: IconLook,
    /// One line per device, at most 127 UTF-16 units in total.
    pub tooltip: String,
    /// Devices that are on first, then off devices.
    pub devices: Vec<DeviceView>,
    /// How many devices are on ("3 ligados").
    pub on_count: usize,
}

/// One device as the registry holds it.
#[derive(Clone, Copy)]
pub struct DeviceInput<'a> {
    pub info: &'a DeviceInfo,
    pub history: &'a DeviceHistory,
    /// The stored readings (oldest first), for the chart.
    pub readings: &'a [Reading],
}

impl Snapshot {
    /// Builds the snapshot at `now` in the language of `lang_tag` (`"pt-BR"`, anything else English) with the
    /// default limits.
    pub fn build(inputs: &[DeviceInput<'_>], now: &Now, lang_tag: &str) -> Snapshot {
        Snapshot::build_with(inputs, now, lang_tag, Limits::default())
    }

    /// `build` with the icon's limits as the user set them.
    pub fn build_with(
        inputs: &[DeviceInput<'_>],
        now: &Now,
        lang_tag: &str,
        limits: Limits,
    ) -> Snapshot {
        let lang = Lang::from_tag(lang_tag);
        let wall = now.wall.floor() as i64;
        let mut smart_alert = false;
        let mut devices: Vec<DeviceView> = inputs
            .iter()
            .map(|d| {
                let (view, smart) = device_view(d, wall, lang);
                smart_alert |= smart;
                view
            })
            .collect();
        devices.sort_by(|a, b| {
            (!a.on, a.name.to_lowercase(), &a.key).cmp(&(!b.on, b.name.to_lowercase(), &b.key))
        });
        let icon = IconLook::of(&devices, smart_alert, limits);
        let on_count = devices.iter().filter(|d| d.on).count();
        let tooltip = tooltip(&devices, lang);
        Snapshot {
            icon,
            tooltip,
            devices,
            on_count,
        }
    }
}

impl Snapshot {
    /// The snapshot with the user's names and order applied: the panel, the tooltip and the notices built from it
    /// follow. The icon does not depend on either.
    pub fn arranged(mut self, a: &Arrangement, lang: Lang) -> Snapshot {
        if a.is_empty() {
            return self;
        }
        for d in &mut self.devices {
            if let Some(name) = a.name_of(&d.key) {
                d.name = name.to_string();
            }
        }
        self.devices
            .sort_by_cached_key(|d| a.sort_key(&d.key, d.on, &d.name));
        self.tooltip = tooltip(&self.devices, lang);
        self
    }
}

fn coarse_text(c: Coarse, lang: Lang) -> &'static str {
    strings::get(
        lang,
        match c {
            Coarse::Full => id::COARSE_FULL,
            Coarse::Good => id::COARSE_GOOD,
            Coarse::Low => id::COARSE_LOW,
            Coarse::Critical => id::COARSE_CRITICAL,
        },
    )
}

/// "Receptor", "Dongle", "Bluetooth", "Cabo USB".
pub fn connection_text(c: Connection, lang: Lang) -> String {
    strings::get(
        lang,
        match c {
            Connection::Receiver => id::CONN_RECEIVER,
            Connection::Dongle => id::CONN_DONGLE,
            Connection::Bluetooth => id::CONN_BLUETOOTH,
            Connection::Usb => id::CONN_USB,
        },
    )
    .to_string()
}

/// The row of one device, and whether its smart warning is active (the icon turns red on it).
fn device_view(d: &DeviceInput<'_>, now: i64, lang: Lang) -> (DeviceView, bool) {
    let h = d.history;
    let on = h.online();
    let level = h.level();
    let percent = level.map(|l| round_half_up(l).clamp(0, 100) as u8);
    let last = h.last;
    let charging = on && last.is_some_and(|l| l.charging == Charging::Charging);

    let mut label = None;
    let mut eta_line = None;
    let status;
    if !on {
        status = match (last, level) {
            (Some(l), Some(_)) => {
                let clock = clock_string(l.at + h.utc_offset);
                strings::fmt(
                    lang,
                    id::STATUS_OFF_AT,
                    &[&percent.unwrap_or(0).to_string(), &clock],
                )
            }
            _ => strings::get(lang, id::STATUS_OFF).to_string(),
        };
    } else if let Some(l) = last {
        match l.charging {
            Charging::Charging | Charging::Full => {
                let full = l.charging == Charging::Full || level.is_some_and(|p| p >= 100.0);
                status = if full {
                    strings::get(lang, id::STATUS_FULL).to_string()
                } else {
                    full_in_text(h, lang)
                        .unwrap_or_else(|| strings::get(lang, id::STATUS_CHARGING).to_string())
                };
            }
            _ => match level {
                Some(_) => {
                    status = forecast_text(h, lang).unwrap_or_default();
                    label = forecast_label(h, lang);
                    eta_line = eta_text(h, now, lang);
                }
                None => {
                    status = match l.coarse {
                        Some(c) if l.quality == Quality::Coarse || l.percent.is_none() => {
                            coarse_text(c, lang).to_string()
                        }
                        _ => strings::get(lang, id::STATUS_NO_READING).to_string(),
                    };
                }
            },
        }
    } else {
        status = strings::get(lang, id::STATUS_NO_READING).to_string();
    }

    let smart = if on {
        smart_warning(h, now, lang)
    } else {
        None
    };
    let smart_active = smart.is_some();
    let warning = if on {
        low_warning(h, lang).or(smart)
    } else {
        None
    };

    let forecast = match (eta(h, now), last, level) {
        (Some(t), Some(l), Some(p)) if on => Some(Forecast {
            from: ChartPoint {
                at: l.at,
                percent: p,
            },
            to: ChartPoint {
                at: round_half_up(t),
                percent: 0.0,
            },
        }),
        _ => None,
    };

    let view = DeviceView {
        key: d.info.key.clone(),
        name: display_name(d.info),
        kind: d.info.kind,
        connection: connection_text(d.info.connection, lang),
        on,
        percent,
        charging,
        status,
        label,
        eta: eta_line,
        warning,
        chart: chart_points(d.readings, now),
        forecast,
        usage_today: usage_today_text(h, now, lang),
        usage_week: usage_week(h, now),
        health: strings::fmt(lang, id::HEALTH_LINE, &[&health_text(h, lang)]),
    };
    (view, smart_active)
}

/// The name the panel, the tooltip and the notices show: the catalog's short name for the device's key, else the
/// name the device reports.
pub fn display_name(info: &DeviceInfo) -> String {
    catalog::short_name(&info.key.0).map_or_else(|| info.name.clone(), str::to_string)
}

/// Seconds of use on each of the last 7 local days, oldest first; days with no online time are 0.
pub fn usage_week(h: &DeviceHistory, now: i64) -> [i64; 7] {
    let today = (now + h.utc_offset).div_euclid(estimator::DAY);
    let by_day = estimator::usage_by_day(h, now);
    std::array::from_fn(|i| {
        let day = today - (6 - i as i64);
        by_day
            .iter()
            .find(|&&(d, _)| d == day)
            .map_or(0, |&(_, secs)| secs)
    })
}

/// The measurable readings of the last 7 days, one point per hour (the last reading of the hour).
pub fn chart_points(readings: &[Reading], now: i64) -> Vec<ChartPoint> {
    let from = now - CHART_DAYS * estimator::DAY;
    let mut out: Vec<ChartPoint> = Vec::new();
    let mut bucket = i64::MIN;
    for r in readings {
        let Some(percent) = r.percent else { continue };
        if r.quality == Quality::Coarse || r.at < from || r.at > now {
            continue;
        }
        let point = ChartPoint { at: r.at, percent };
        let b = r.at.div_euclid(estimator::HOUR);
        match out.last_mut() {
            Some(prev) if b == bucket => *prev = point,
            _ => out.push(point),
        }
        bucket = b;
    }
    out
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn line_full(d: &DeviceView, lang: Lang) -> String {
    match d.percent {
        _ if !d.on => strings::fmt(lang, id::TOOLTIP_OFF, &[&d.name]),
        Some(p) => strings::fmt(
            lang,
            id::TOOLTIP_LINE_STATUS,
            &[&d.name, &p.to_string(), &d.status],
        ),
        None => strings::fmt(lang, id::TOOLTIP_PLAIN, &[&d.name, &d.status]),
    }
}

fn line_short(d: &DeviceView, lang: Lang) -> String {
    match d.percent {
        _ if !d.on => strings::fmt(lang, id::TOOLTIP_OFF, &[&d.name]),
        Some(p) => strings::fmt(lang, id::TOOLTIP_LINE, &[&d.name, &p.to_string()]),
        None => d.name.clone(),
    }
}

/// One line per device; if the whole does not fit 127 UTF-16 units, the short lines (name and percent); if that
/// still does not fit, whole lines up to the limit and a final "…".
pub fn tooltip(devices: &[DeviceView], lang: Lang) -> String {
    if devices.is_empty() {
        return fit(strings::get(lang, id::TOOLTIP_EMPTY).to_string());
    }
    let full = devices
        .iter()
        .map(|d| line_full(d, lang))
        .collect::<Vec<_>>();
    let joined = full.join("\n");
    if utf16_len(&joined) <= TOOLTIP_LIMIT {
        return joined;
    }
    let short = devices
        .iter()
        .map(|d| line_short(d, lang))
        .collect::<Vec<_>>();
    let joined = short.join("\n");
    if utf16_len(&joined) <= TOOLTIP_LIMIT {
        return joined;
    }
    // keep whole lines while they fit, leaving room for "\n…"
    let mut out = String::new();
    for line in &short {
        let extra = if out.is_empty() { 0 } else { 1 };
        if utf16_len(&out) + extra + utf16_len(line) + 2 > TOOLTIP_LIMIT {
            break;
        }
        if extra == 1 {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.is_empty() {
        return fit(short.join("\n"));
    }
    out.push_str("\n…");
    out
}

/// Cuts a text to the limit on a character boundary and ends it with "…".
fn fit(text: String) -> String {
    if utf16_len(&text) <= TOOLTIP_LIMIT {
        return text;
    }
    let mut out = String::new();
    for c in text.chars() {
        if utf16_len(&out) + c.len_utf16() + 1 > TOOLTIP_LIMIT {
            break;
        }
        out.push(c);
    }
    let trimmed = out.trim_end().to_string();
    format!("{trimmed}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::estimator::{NoticeKind, Presence, Reading};

    const OFF: i64 = -10_800;
    const T0: i64 = 1_793_016_000; // 2026-10-26 09:00 local, a Monday

    fn info(name: &str, key: &str, kind: Kind, connection: Connection) -> DeviceInfo {
        DeviceInfo {
            key: DeviceKey(key.to_string()),
            provider: "logitech-hidpp",
            name: name.to_string(),
            kind,
            connection,
            brand: "Logitech".to_string(),
        }
    }

    fn now_at(t: i64) -> Now {
        Now {
            wall: t as f64,
            mono: 0.0,
            utc_offset: OFF as i32,
        }
    }

    fn history(online_at: Option<i64>, readings: &[(i64, f64, Charging)]) -> DeviceHistory {
        let mut h = DeviceHistory::new("dev", 20.0, 2.5, OFF);
        h.lang = Lang::PtBr;
        if let Some(t) = online_at {
            h.add_presence(Presence {
                at: t,
                online: true,
            });
        }
        for &(at, p, c) in readings {
            h.add_reading(Reading::new(at, Some(p), c, Quality::Exact));
        }
        h
    }

    fn view_of(
        name: &str,
        key: &str,
        h: &DeviceHistory,
        readings: &[Reading],
        now: i64,
        tag: &str,
    ) -> DeviceView {
        let i = info(name, key, Kind::Mouse, Connection::Receiver);
        Snapshot::build(
            &[DeviceInput {
                info: &i,
                history: h,
                readings,
            }],
            &now_at(now),
            tag,
        )
        .devices
        .remove(0)
    }

    #[test]
    fn on_devices_come_first_then_off_by_name() {
        let on = history(Some(T0), &[(T0, 80.0, Charging::Discharging)]);
        let mut off = history(Some(T0), &[(T0, 71.0, Charging::Discharging)]);
        off.add_presence(Presence {
            at: T0 + 100,
            online: false,
        });
        let ia = info("Zed mouse", "z", Kind::Mouse, Connection::Receiver);
        let ib = info("alpha pad", "a", Kind::Gamepad, Connection::Bluetooth);
        let ic = info("Beta keys", "b", Kind::Keyboard, Connection::Usb);
        let inputs = [
            DeviceInput {
                info: &ib,
                history: &off,
                readings: &[],
            },
            DeviceInput {
                info: &ia,
                history: &on,
                readings: &[],
            },
            DeviceInput {
                info: &ic,
                history: &off,
                readings: &[],
            },
        ];
        let s = Snapshot::build(&inputs, &now_at(T0 + 200), "pt-BR");
        let names: Vec<&str> = s.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Zed mouse", "alpha pad", "Beta keys"]);
        assert_eq!(s.on_count, 1);
        assert!(s.devices[0].on && !s.devices[1].on);
        assert_eq!(s.devices[1].connection, "Bluetooth");
        assert_eq!(s.devices[2].connection, "Cabo USB");
    }

    #[test]
    fn row_texts_for_on_charging_and_off_devices() {
        let h = history(Some(T0), &[(T0, 80.0, Charging::Discharging)]);
        let v = view_of("G703", "k", &h, &[], T0 + 60, "pt-BR");
        assert_eq!(v.percent, Some(80));
        assert_eq!(v.status, "~16 h de uso");
        assert_eq!(v.label.as_deref(), Some("estimativa inicial"));
        assert!(v.eta.as_deref().unwrap().starts_with("acaba "));
        assert_eq!(v.usage_today, "Hoje: 1 min de uso");
        assert_eq!(v.health, "Saúde da bateria: aprendendo");
        assert!(v.forecast.is_some());
        let e = view_of("G703", "k", &h, &[], T0 + 60, "en-US");
        assert_eq!(e.status, "~16 h of use");
        assert_eq!(e.label.as_deref(), Some("initial estimate"));
        assert_eq!(e.health, "Battery health: learning");

        let c = history(Some(T0), &[(T0, 40.0, Charging::Charging)]);
        let v = view_of("G703", "k", &c, &[], T0 + 60, "pt-BR");
        assert!(v.charging);
        assert_eq!(v.status, "cheio em ~1 h 40 min");
        assert!(v.forecast.is_none());

        let mut o = history(
            Some(T0),
            &[(T0 + 3600 * 5 + 120, 71.0, Charging::Discharging)],
        );
        o.add_presence(Presence {
            at: T0 + 3600 * 5 + 240,
            online: false,
        });
        let v = view_of("G703", "k", &o, &[], T0 + 7 * 3600, "pt-BR");
        assert!(!v.on);
        assert_eq!(v.status, "desligado · 71% às 14:02");
        assert_eq!(
            view_of("G703", "k", &o, &[], T0 + 7 * 3600, "en").status,
            "off · 71% at 14:02"
        );
        assert!(v.warning.is_none() && v.eta.is_none() && v.forecast.is_none());
    }

    #[test]
    fn coarse_and_unread_devices() {
        let mut h = DeviceHistory::new("dev", 20.0, 2.5, OFF);
        h.add_presence(Presence {
            at: T0,
            online: true,
        });
        let v = view_of("Headset", "k", &h, &[], T0 + 5, "pt-BR");
        assert_eq!(v.status, "sem leitura");
        assert_eq!(v.percent, None);
        h.add_reading(Reading {
            coarse: Some(Coarse::Good),
            ..Reading::new(T0 + 10, None, Charging::Discharging, Quality::Coarse)
        });
        let v = view_of("Headset", "k", &h, &[], T0 + 20, "pt-BR");
        assert_eq!(v.status, "bateria boa");
        assert_eq!(v.percent, None);
        assert!(v.forecast.is_none() && v.eta.is_none());
    }

    #[test]
    fn icon_follows_levels_warnings_and_charging_in_a_real_snapshot() {
        let normal = history(Some(T0), &[(T0, 80.0, Charging::Discharging)]);
        let charging = history(Some(T0), &[(T0, 40.0, Charging::Charging)]);
        let low = history(
            Some(T0),
            &[
                (T0, 100.0, Charging::Discharging),
                (T0 + 3600, 6.0, Charging::Discharging),
            ],
        );
        let (a, b, c) = (
            info("a", "a", Kind::Mouse, Connection::Receiver),
            info("b", "b", Kind::Mouse, Connection::Receiver),
            info("c", "c", Kind::Mouse, Connection::Receiver),
        );
        let build = |hs: &[(&DeviceInfo, &DeviceHistory)]| {
            let inputs: Vec<DeviceInput<'_>> = hs
                .iter()
                .map(|&(i, h)| DeviceInput {
                    info: i,
                    history: h,
                    readings: &[],
                })
                .collect();
            Snapshot::build(&inputs, &now_at(T0 + 3700), "pt-BR")
        };
        let look = |juice, charging| IconLook { juice, charging };
        assert_eq!(build(&[(&a, &normal)]).icon, look(Juice::Green, false));
        assert_eq!(
            build(&[(&a, &normal), (&b, &charging)]).icon,
            look(Juice::Purple, true),
            "40 % is between the limits"
        );
        let s = build(&[(&a, &normal), (&b, &charging), (&c, &low)]);
        assert_eq!(s.icon, look(Juice::Red, true));
        let w = s.devices.iter().find(|d| d.name == "c").unwrap();
        assert_eq!(w.warning.as_deref(), Some("dev com 6%"));
        assert_eq!(build(&[]).icon, look(Juice::Purple, false));
    }

    #[test]
    fn limits_come_in_through_build_with() {
        let h = history(Some(T0), &[(T0, 70.0, Charging::Discharging)]);
        let i = info("a", "a", Kind::Mouse, Connection::Receiver);
        let input = [DeviceInput {
            info: &i,
            history: &h,
            readings: &[],
        }];
        let at = |low, high| {
            Snapshot::build_with(&input, &now_at(T0 + 60), "en", Limits::new(low, high)).icon
        };
        assert_eq!(at(20, 50).juice, Juice::Green);
        assert_eq!(
            at(20, 80).juice,
            Juice::Purple,
            "70 % is under a high limit of 80"
        );
        assert_eq!(
            at(75, 90).juice,
            Juice::Red,
            "70 % is under a low limit of 75"
        );
        let plain = Snapshot::build(&input, &now_at(T0 + 60), "en");
        assert_eq!(plain.icon, at(20, 50), "build uses the default limits");
    }

    #[test]
    fn an_active_smart_warning_turns_a_real_snapshot_red_at_a_middling_level() {
        // 35 % at 09:00 will not last the day (the estimator's own warning).
        let h = history(Some(T0), &[(T0, 35.0, Charging::Discharging)]);
        let i = info("a", "a", Kind::Mouse, Connection::Receiver);
        let input = [DeviceInput {
            info: &i,
            history: &h,
            readings: &[],
        }];
        let s = Snapshot::build(&input, &now_at(T0 + 60), "en");
        assert!(s.devices[0].warning.is_some());
        assert_eq!(s.icon.juice, Juice::Red);
        assert!(
            s.devices[0].percent.unwrap() >= 20,
            "not under the low limit"
        );
    }

    #[test]
    fn the_smart_warning_switch_removes_the_warning_and_the_red() {
        let mut h = history(Some(T0), &[(T0, 35.0, Charging::Discharging)]);
        let i = info("a", "a", Kind::Mouse, Connection::Receiver);
        let build = |h: &DeviceHistory| {
            let input = [DeviceInput {
                info: &i,
                history: h,
                readings: &[],
            }];
            Snapshot::build(&input, &now_at(T0 + 60), "en")
        };
        assert_eq!(build(&h).icon.juice, Juice::Red, "on by default");
        h.set_alerts(10.0, false);
        let s = build(&h);
        assert!(s.devices[0].warning.is_none());
        assert_eq!(s.icon.juice, Juice::Purple, "35 % is between the limits");
        h.set_alerts(10.0, true);
        assert_eq!(build(&h).icon.juice, Juice::Red);
    }

    #[test]
    fn the_low_level_setting_moves_the_low_battery_warning() {
        // 12 % while discharging: silent under the default 10 %, a warning once the level is 15 %.
        let mut h = history(
            Some(T0),
            &[
                (T0, 100.0, Charging::Discharging),
                (T0 + 60, 12.0, Charging::Discharging),
            ],
        );
        h.set_alerts(10.0, false);
        let i = info("a", "a", Kind::Mouse, Connection::Receiver);
        let warn = |h: &DeviceHistory| {
            let input = [DeviceInput {
                info: &i,
                history: h,
                readings: &[],
            }];
            Snapshot::build(&input, &now_at(T0 + 120), "en").devices[0]
                .warning
                .clone()
        };
        assert_eq!(warn(&h), None);
        h.set_alerts(15.0, false);
        assert_eq!(warn(&h).as_deref(), Some("dev at 12%"));
        // The setting is kept inside its bounds.
        h.set_alerts(99.0, true);
        assert_eq!(h.low_percent, 30.0);
        h.set_alerts(1.0, true);
        assert_eq!(h.low_percent, 5.0);
    }

    #[test]
    fn a_new_low_level_makes_the_notice_fire_at_that_level() {
        let mut h = DeviceHistory::new("dev", 20.0, 2.5, OFF);
        h.set_alerts(20.0, false);
        h.add_presence(Presence {
            at: T0,
            online: true,
        });
        h.add_reading(Reading::new(
            T0,
            Some(40.0),
            Charging::Discharging,
            Quality::Exact,
        ));
        let n = h.add_reading(Reading::new(
            T0 + 60,
            Some(19.0),
            Charging::Discharging,
            Quality::Exact,
        ));
        assert!(
            n.iter().any(|n| n.kind == NoticeKind::Low),
            "{n:?} at 19 % with a 20 % limit"
        );
    }

    fn row(on: bool, pct: Option<u8>, charging: bool) -> DeviceView {
        DeviceView {
            charging,
            ..view("d", on, pct, "")
        }
    }

    fn juice_of(rows: &[DeviceView]) -> Juice {
        IconLook::of(rows, false, Limits::default()).juice
    }

    #[test]
    fn red_when_any_online_device_is_under_the_low_limit() {
        assert_eq!(juice_of(&[row(true, Some(19), false)]), Juice::Red);
        assert_eq!(
            juice_of(&[row(true, Some(90), false), row(true, Some(5), false)]),
            Juice::Red,
            "one low device is enough"
        );
        // Equal to the limit is not under it.
        assert_eq!(juice_of(&[row(true, Some(20), false)]), Juice::Purple);
        assert_eq!(juice_of(&[row(true, Some(0), false)]), Juice::Red);
    }

    #[test]
    fn red_on_a_smart_warning_even_with_a_healthy_level() {
        let rows = [row(true, Some(80), false)];
        assert_eq!(
            IconLook::of(&rows, true, Limits::default()).juice,
            Juice::Red
        );
        assert_eq!(
            IconLook::of(&rows, false, Limits::default()).juice,
            Juice::Green
        );
    }

    #[test]
    fn green_only_when_every_online_device_with_a_reading_is_at_or_above_high() {
        assert_eq!(
            juice_of(&[row(true, Some(50), false)]),
            Juice::Green,
            "equal counts"
        );
        assert_eq!(juice_of(&[row(true, Some(49), false)]), Juice::Purple);
        assert_eq!(
            juice_of(&[row(true, Some(100), false), row(true, Some(49), false)]),
            Juice::Purple,
            "all of them, not any"
        );
        assert_eq!(
            juice_of(&[row(true, Some(100), false), row(true, Some(50), false)]),
            Juice::Green
        );
    }

    #[test]
    fn no_device_and_no_reading_is_purple() {
        assert_eq!(juice_of(&[]), Juice::Purple);
        assert_eq!(juice_of(&[row(true, None, false)]), Juice::Purple);
        // A device without a reading does not stop the others from deciding.
        assert_eq!(
            juice_of(&[row(true, None, false), row(true, Some(80), false)]),
            Juice::Green
        );
    }

    #[test]
    fn offline_devices_are_ignored() {
        assert_eq!(juice_of(&[row(false, Some(3), false)]), Juice::Purple);
        assert_eq!(
            juice_of(&[row(false, Some(3), false), row(true, Some(90), false)]),
            Juice::Green,
            "an off device at 3 % does not turn it red nor stop green"
        );
        assert_eq!(
            juice_of(&[row(false, Some(100), false), row(true, Some(30), false)]),
            Juice::Purple
        );
    }

    #[test]
    fn charging_needs_an_online_charging_device() {
        let charging = |rows: &[DeviceView]| IconLook::of(rows, false, Limits::default()).charging;
        assert!(charging(&[row(true, Some(40), true)]));
        assert!(!charging(&[row(false, Some(40), true)]));
        assert!(!charging(&[row(true, Some(40), false)]));
        assert!(!charging(&[]));
        // Charging does not change the colour.
        let look = IconLook::of(&[row(true, Some(10), true)], false, Limits::default());
        assert_eq!(
            look,
            IconLook {
                juice: Juice::Red,
                charging: true
            }
        );
    }

    #[test]
    fn limits_stay_consistent() {
        assert_eq!(Limits::default(), Limits { low: 20, high: 50 });
        assert_eq!(Limits::new(60, 40), Limits { low: 60, high: 60 });
        assert_eq!(
            Limits::new(120, 200),
            Limits {
                low: 100,
                high: 100
            }
        );
        let tight = Limits::new(30, 30);
        let rows = [row(true, Some(30), false)];
        assert_eq!(IconLook::of(&rows, false, tight).juice, Juice::Green);
    }

    #[test]
    fn equal_inputs_give_equal_snapshots() {
        let h = history(
            Some(T0),
            &[
                (T0, 80.0, Charging::Discharging),
                (T0 + 3600, 76.0, Charging::Discharging),
            ],
        );
        let i = info("G703", "k", Kind::Mouse, Connection::Receiver);
        let readings = [Reading::new(
            T0,
            Some(80.0),
            Charging::Discharging,
            Quality::Exact,
        )];
        let input = [DeviceInput {
            info: &i,
            history: &h,
            readings: &readings,
        }];
        let a = Snapshot::build(&input, &now_at(T0 + 4000), "pt-BR");
        let b = Snapshot::build(&input, &now_at(T0 + 4000), "pt-BR");
        assert_eq!(a, b);
        let c = Snapshot::build(&input, &now_at(T0 + 4000), "en");
        assert_ne!(a, c);
    }

    #[test]
    fn chart_is_seven_days_one_point_per_hour() {
        let now = T0 + 10 * estimator::DAY;
        let ex = |at: i64, p: f64| Reading::new(at, Some(p), Charging::Discharging, Quality::Exact);
        let rs = vec![
            ex(now - 8 * estimator::DAY, 99.0),
            ex(now - 3 * 3600 - 1200, 90.0),
            ex(now - 3 * 3600 - 600, 89.0),
            ex(now - 600, 80.0),
            Reading::new(now - 300, None, Charging::Unknown, Quality::Coarse),
        ];
        let pts = chart_points(&rs, now);
        assert_eq!(pts.len(), 2, "{pts:?}");
        assert_eq!(pts[0].percent, 89.0);
        assert_eq!(
            pts[1],
            ChartPoint {
                at: now - 600,
                percent: 80.0
            }
        );
    }

    fn view(name: &str, on: bool, pct: Option<u8>, status: &str) -> DeviceView {
        DeviceView {
            key: DeviceKey(name.to_string()),
            name: name.to_string(),
            kind: Kind::Mouse,
            connection: String::new(),
            on,
            percent: pct,
            charging: false,
            status: status.to_string(),
            label: None,
            eta: None,
            warning: None,
            chart: Vec::new(),
            forecast: None,
            usage_today: String::new(),
            usage_week: [0; 7],
            health: String::new(),
        }
    }

    #[test]
    fn tooltip_is_one_line_per_device_and_within_the_limit() {
        let ds = vec![
            view("G703", true, Some(76), "~16 h de uso"),
            view("G915 X", true, Some(40), "cheio em ~1 h 10 min"),
            view("G733", false, Some(71), "desligado"),
        ];
        let t = tooltip(&ds, Lang::PtBr);
        assert_eq!(
            t,
            "G703 76% · ~16 h de uso\nG915 X 40% · cheio em ~1 h 10 min\nG733 · desligado"
        );
        assert!(utf16_len(&t) <= TOOLTIP_LIMIT);
        assert_eq!(tooltip(&[], Lang::En), "JuiceBar: no devices");
    }

    #[test]
    fn tooltip_shortens_then_truncates_cleanly() {
        let long: Vec<DeviceView> = (0..4)
            .map(|i| {
                view(
                    &format!("Wireless gaming mouse number {i}"),
                    true,
                    Some(50),
                    "cheio em ~1 h 10 min",
                )
            })
            .collect();
        let t = tooltip(&long, Lang::PtBr);
        assert!(utf16_len(&t) <= TOOLTIP_LIMIT, "{} units", utf16_len(&t));
        assert!(!t.contains("cheio"), "short lines expected: {t}");
        assert_eq!(t.lines().count(), 4);

        let many: Vec<DeviceView> = (0..12)
            .map(|i| view(&format!("Dispositivo {i}"), true, Some(50), "x"))
            .collect();
        let t = tooltip(&many, Lang::PtBr);
        assert!(utf16_len(&t) <= TOOLTIP_LIMIT);
        assert!(t.ends_with("\n…"));
        assert!(t.lines().count() > 3);
        assert!(
            t.lines().rev().nth(1).unwrap().ends_with("50%"),
            "last kept line is whole: {t}"
        );

        // astral characters count two units and are never split
        let name = "\u{1F3AE}".repeat(100);
        let t = tooltip(&[view(&name, true, Some(5), "x")], Lang::En);
        assert!(utf16_len(&t) <= TOOLTIP_LIMIT);
        assert!(t.ends_with('…'));
        assert!(t.chars().all(|c| c == '\u{1F3AE}' || c == '…'));
    }

    #[test]
    fn the_panel_shows_the_catalogs_short_name() {
        let mut i = info(
            "G703 LIGHTSPEED Wireless Gaming Mouse",
            "logitech-hidpp:4086",
            Kind::Mouse,
            Connection::Receiver,
        );
        assert_eq!(display_name(&i), "G703");
        let h = history(Some(T0), &[(T0, 80.0, Charging::Discharging)]);
        let s = Snapshot::build(
            &[DeviceInput {
                info: &i,
                history: &h,
                readings: &[],
            }],
            &now_at(T0 + 60),
            "pt-BR",
        );
        assert_eq!(s.devices[0].name, "G703");
        assert!(s.tooltip.starts_with("G703"), "{}", s.tooltip);
        assert!(!s.tooltip.contains("Wireless"));
        // A key the catalog does not know keeps the name the device reports.
        i.key = DeviceKey("os-bluetooth:ab".into());
        assert_eq!(display_name(&i), "G703 LIGHTSPEED Wireless Gaming Mouse");
    }

    #[test]
    fn usage_week_has_seven_local_days_oldest_first() {
        let h = history(Some(T0), &[(T0, 80.0, Charging::Discharging)]);
        // Monday 09:00 to 12:00: three hours today, nothing before.
        assert_eq!(usage_week(&h, T0 + 3 * 3600), [0, 0, 0, 0, 0, 0, 3 * 3600]);
        // Tuesday 11:00: Monday from 09:00 to midnight (15 h) and Tuesday so far (11 h).
        assert_eq!(
            usage_week(&h, T0 + 26 * 3600),
            [0, 0, 0, 0, 0, 15 * 3600, 11 * 3600]
        );
        // The view carries it.
        let v = view_of("n", "n", &h, &[], T0 + 3 * 3600, "pt-BR");
        assert_eq!(v.usage_week[6], 3 * 3600);
        // A device that was never online has seven empty days.
        let never = history(None, &[]);
        assert_eq!(usage_week(&never, T0), [0; 7]);
    }

    fn snapshot_of(devices: Vec<DeviceView>) -> Snapshot {
        Snapshot {
            icon: IconLook::of(&devices, false, Limits::default()),
            tooltip: tooltip(&devices, Lang::PtBr),
            on_count: devices.iter().filter(|d| d.on).count(),
            devices,
        }
    }

    fn key(n: &str) -> DeviceKey {
        DeviceKey(n.to_string())
    }

    fn names_of(s: &Snapshot) -> Vec<&str> {
        s.devices.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn no_arrangement_leaves_the_snapshot_as_it_is() {
        let s = snapshot_of(vec![
            view("G915 X", true, Some(40), "a"),
            view("G703", false, Some(76), "b"),
        ]);
        assert_eq!(s.clone().arranged(&Arrangement::default(), Lang::PtBr), s);
    }

    #[test]
    fn a_name_the_user_gave_reaches_the_panel_and_the_tooltip() {
        let s = snapshot_of(vec![
            view("G703", true, Some(76), "~16 h de uso"),
            view("G915 X", true, Some(40), "a"),
        ]);
        let mut a = Arrangement::default();
        a.set_name(&key("G703"), "  Mouse da mesa ");
        let s = s.arranged(&a, Lang::PtBr);
        assert_eq!(
            names_of(&s),
            ["G915 X", "Mouse da mesa"],
            "ordered by the shown name"
        );
        assert!(s.tooltip.contains("Mouse da mesa 76%"), "{}", s.tooltip);
        assert!(!s.tooltip.contains("G703"));
        // An empty name takes the user's name away.
        a.set_name(&key("G703"), "   ");
        assert_eq!(a.name_of(&key("G703")), None);
    }

    #[test]
    fn names_are_cleaned_and_cut_to_the_limit() {
        assert_eq!(clean_name("  abc\t\n "), "abc");
        assert_eq!(clean_name(&"x".repeat(40)).chars().count(), NAME_MAX);
        assert_eq!(clean_name(&format!("{} y", "a".repeat(23))), "a".repeat(23));
        assert_eq!(clean_name("\u{7}\r"), "");
        let accented = "é".repeat(30);
        assert_eq!(
            clean_name(&accented).chars().count(),
            NAME_MAX,
            "characters, not bytes"
        );
    }

    #[test]
    fn the_ordered_devices_come_first_then_the_rest_with_those_on_first() {
        let s = snapshot_of(vec![
            view("A", false, Some(10), "a"),
            view("B", true, Some(20), "b"),
            view("C", true, Some(30), "c"),
            view("D", false, Some(40), "d"),
        ]);
        let a = Arrangement {
            names: Vec::new(),
            order: vec![key("D"), key("B")],
        };
        let s = s.arranged(&a, Lang::PtBr);
        assert_eq!(names_of(&s), ["D", "B", "C", "A"]);
        assert!(s.tooltip.starts_with('D'), "{}", s.tooltip);
        assert_eq!(s.on_count, 2, "the order does not change what is on");
    }

    #[test]
    fn moving_a_device_swaps_it_with_its_neighbour_and_keeps_the_whole_list() {
        let list = [key("A"), key("B"), key("C")];
        let mut a = Arrangement::default();
        assert!(a.move_in(&list, &key("C"), -1));
        assert_eq!(a.order, [key("A"), key("C"), key("B")]);
        assert!(a.move_in(&[key("A"), key("C"), key("B")], &key("A"), 1));
        assert_eq!(a.order, [key("C"), key("A"), key("B")]);
        // The ends and a stranger do not move.
        let mut b = Arrangement::default();
        assert!(!b.move_in(&list, &key("A"), -1));
        assert!(!b.move_in(&list, &key("C"), 1));
        assert!(!b.move_in(&list, &key("Z"), 1));
        assert!(b.order.is_empty());
        assert_eq!(a.rank(&key("A")), Some(1));
        assert_eq!(a.rank(&key("Z")), None);
    }
}
