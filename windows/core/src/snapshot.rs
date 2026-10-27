// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! What the UI shows (spec/design/DESIGN.md): the bar icon's state, the tooltip and one `DeviceView` per device with
//! every text already formatted. The core rebuilds it after any event that changed what is shown; the shell redraws
//! only when it differs from the last one (`PartialEq`).

use crate::estimator::{
    self, DeviceHistory, Reading, clock_string, eta, eta_text, forecast_label, forecast_text,
    full_in_text, health_text, low_warning, round_half_up, smart_warning, usage_today_text,
};
use crate::strings::{self, Lang, id};
use crate::types::{Charging, Coarse, Connection, DeviceInfo, DeviceKey, Kind, Now, Quality};

/// The Windows tooltip limit, in UTF-16 code units.
pub const TOOLTIP_LIMIT: usize = 127;
/// The chart shows this many days.
pub const CHART_DAYS: i64 = 7;

/// The single bar icon: red while a warning is active, a small bolt while any device charges, otherwise plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    Normal,
    Warning,
    Charging,
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
    /// "Saúde da bateria: 92%" or "... aprendendo".
    pub health: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub icon: IconState,
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
    /// Builds the snapshot at `now` in the language of `lang_tag` (`"pt-BR"`, anything else English).
    pub fn build(inputs: &[DeviceInput<'_>], now: &Now, lang_tag: &str) -> Snapshot {
        let lang = Lang::from_tag(lang_tag);
        let wall = now.wall.floor() as i64;
        let mut devices: Vec<DeviceView> =
            inputs.iter().map(|d| device_view(d, wall, lang)).collect();
        devices.sort_by(|a, b| {
            (!a.on, a.name.to_lowercase(), &a.key).cmp(&(!b.on, b.name.to_lowercase(), &b.key))
        });
        let icon = if devices.iter().any(|d| d.warning.is_some()) {
            IconState::Warning
        } else if devices.iter().any(|d| d.on && d.charging) {
            IconState::Charging
        } else {
            IconState::Normal
        };
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

fn connection_text(c: Connection, lang: Lang) -> String {
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

fn device_view(d: &DeviceInput<'_>, now: i64, lang: Lang) -> DeviceView {
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

    let warning = if on {
        low_warning(h, lang).or_else(|| smart_warning(h, now, lang))
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

    DeviceView {
        key: d.info.key.clone(),
        name: d.info.name.clone(),
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
        health: strings::fmt(lang, id::HEALTH_LINE, &[&health_text(h, lang)]),
    }
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
    use crate::estimator::{Presence, Reading};

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
    fn icon_follows_warnings_then_charging() {
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
        assert_eq!(build(&[(&a, &normal)]).icon, IconState::Normal);
        assert_eq!(
            build(&[(&a, &normal), (&b, &charging)]).icon,
            IconState::Charging
        );
        let s = build(&[(&a, &normal), (&b, &charging), (&c, &low)]);
        assert_eq!(s.icon, IconState::Warning);
        let w = s.devices.iter().find(|d| d.name == "c").unwrap();
        assert_eq!(w.warning.as_deref(), Some("dev com 6%"));
        assert_eq!(build(&[]).icon, IconState::Normal);
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
}
