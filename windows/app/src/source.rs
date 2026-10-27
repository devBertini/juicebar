// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Where the snapshot comes from: the real devices through the core (`Live`, the default), the demo
//! devices of `docs/design/harness/sample-state.js` (hand-ported), or "no devices yet".

use juicebar_core::snapshot::{
    ChartPoint, DeviceView, Forecast, IconLook, Limits, Snapshot, tooltip,
};
use juicebar_core::strings::{self, Lang, id};
use juicebar_core::types::{DeviceKey, Kind, Now};

use crate::live::Live;
use crate::winhost::WinHost;

/// Wall time and the local offset, both read by the shell and passed in (the core never reads a clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    /// Seconds since the Unix epoch, UTC.
    pub wall: i64,
    /// Local offset from UTC in seconds.
    pub utc_offset: i32,
}

impl Clock {
    pub fn now() -> Clock {
        use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let mut tz = TIME_ZONE_INFORMATION::default();
        // SAFETY: `tz` is a valid out structure.
        let id = unsafe { GetTimeZoneInformation(&mut tz) };
        // Bias is minutes to add to local time to get UTC; 2 is TIME_ZONE_ID_DAYLIGHT.
        let bias = tz.Bias
            + if id == 2 {
                tz.DaylightBias
            } else {
                tz.StandardBias
            };
        Clock {
            wall,
            utc_offset: -bias * 60,
        }
    }

    /// Hour of the local day, with minutes as a fraction.
    pub fn hour_of_day(&self) -> f32 {
        ((self.wall + self.utc_offset as i64).rem_euclid(86_400)) as f32 / 3600.0
    }
}

/// Which sample scene the demo shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoPhase {
    /// The mockup's scene: a warning on the mouse and the keyboard charging.
    Scene,
    /// Nothing to signal.
    Normal,
    /// The mouse will not last the day.
    Warning,
    /// The keyboard charges.
    Charging,
    /// The keyboard just reached 100 %.
    Full,
}

impl DemoPhase {
    /// The next scene of the `--demo-cycle` loop.
    pub fn next(self) -> DemoPhase {
        match self {
            DemoPhase::Scene | DemoPhase::Full => DemoPhase::Normal,
            DemoPhase::Normal => DemoPhase::Warning,
            DemoPhase::Warning => DemoPhase::Charging,
            DemoPhase::Charging => DemoPhase::Full,
        }
    }
}

/// The cycle's step in milliseconds.
pub const CYCLE_MS: u32 = 10_000;

/// What feeds the shell its snapshots.
pub enum Source {
    /// No device known yet (`--empty`, and the instant before the live source starts).
    Empty,
    Demo {
        phase: DemoPhase,
        cycle: bool,
    },
    /// The real devices, through the core.
    Live(Box<Live<WinHost>>),
}

impl Source {
    pub fn snapshot(&self, clock: &Clock, lang: Lang) -> Snapshot {
        match self {
            Source::Empty => empty_snapshot(lang),
            Source::Demo { phase, .. } => demo_snapshot(*phase, clock, lang),
            Source::Live(live) => live.snapshot(&Now {
                wall: clock.wall as f64,
                mono: 0.0,
                utc_offset: clock.utc_offset,
            }),
        }
    }

    pub fn live(&self) -> Option<&Live<WinHost>> {
        match self {
            Source::Live(live) => Some(live),
            _ => None,
        }
    }

    pub fn live_mut(&mut self) -> Option<&mut Live<WinHost>> {
        match self {
            Source::Live(live) => Some(live),
            _ => None,
        }
    }

    /// Milliseconds until the source wants `advance` called, if ever (the `wake_at` hook).
    pub fn next_wake_ms(&self) -> Option<u32> {
        match self {
            Source::Demo { cycle: true, .. } => Some(CYCLE_MS),
            _ => None,
        }
    }

    /// Moves to the next scene; true when the snapshot changed.
    pub fn advance(&mut self) -> bool {
        match self {
            Source::Demo { phase, cycle: true } => {
                *phase = phase.next();
                true
            }
            _ => false,
        }
    }
}

pub fn empty_snapshot(lang: Lang) -> Snapshot {
    Snapshot {
        icon: IconLook::default(),
        tooltip: tooltip(&[], lang),
        devices: Vec::new(),
        on_count: 0,
    }
}

// ---- the demo devices (docs/design/harness/sample-state.js) ----

/// Port of the harness' `build()`: one point per hour between the anchors (hours from now, percent);
/// a falling segment discharges fast in work hours (9 to 19) and slowly at night, a rising one
/// charges linearly.
fn history(anchors: &[(f32, f32)], now_hod: f32) -> Vec<(f32, f32)> {
    let weight = |h: f32| {
        let hod = (now_hod + h).rem_euclid(24.0);
        if (9.0..19.0).contains(&hod) {
            1.0
        } else {
            0.15
        }
    };
    let level = |h: f32| -> f32 {
        for pair in anchors.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if h >= a.0 && h <= b.0 {
                if b.1 >= a.1 {
                    return a.1 + (b.1 - a.1) * (h - a.0) / (b.0 - a.0);
                }
                let (mut total, mut part) = (0.0, 0.0);
                let mut t = a.0;
                while t < b.0 {
                    let w = weight(t);
                    total += w;
                    if t < h {
                        part += w;
                    }
                    t += 0.25;
                }
                return a.1 - (a.1 - b.1) * part / total;
            }
        }
        anchors[anchors.len() - 1].1
    };
    let mut hours: Vec<f32> = anchors.iter().map(|a| a.0).collect();
    let mut h = anchors[0].0.ceil();
    while h <= anchors[anchors.len() - 1].0 {
        hours.push(h);
        h += 1.0;
    }
    hours.sort_by(|a, b| a.total_cmp(b));
    hours.dedup();
    hours
        .into_iter()
        .map(|h| (h, (level(h) * 10.0).round() / 10.0))
        .collect()
}

struct Sample {
    key: &'static str,
    name: &'static str,
    kind: Kind,
    connection: &'static str,
    on: bool,
    percent: u8,
    anchors: &'static [(f32, f32)],
    usage: (&'static str, &'static str),
    /// Hours of use on each of the last 7 days, oldest first; the last is today.
    week: [f32; 7],
    health: Option<&'static str>,
}

const G703: Sample = Sample {
    key: "demo-g703",
    name: "G703",
    kind: Kind::Mouse,
    connection: id::CONN_RECEIVER,
    on: true,
    percent: 23,
    anchors: &[
        (-168.0, 70.0),
        (-120.0, 14.0),
        (-117.0, 100.0),
        (-52.0, 18.0),
        (-49.0, 100.0),
        (0.0, 23.0),
    ],
    usage: ("4", "10"),
    week: [3.2, 5.1, 4.6, 6.0, 1.4, 0.8, 4.2],
    health: Some("94"),
};
const G915: Sample = Sample {
    key: "demo-g915x",
    name: "G915 X",
    kind: Kind::Keyboard,
    connection: id::CONN_RECEIVER,
    on: true,
    percent: 46,
    anchors: &[
        (-168.0, 81.0),
        (-95.0, 22.0),
        (-92.0, 100.0),
        (-26.0, 33.0),
        (-1.0, 5.0),
        (0.0, 46.0),
    ],
    usage: ("6", "05"),
    week: [5.0, 6.4, 7.2, 6.8, 2.0, 1.2, 6.1],
    health: Some("98"),
};
const G733: Sample = Sample {
    key: "demo-g733",
    name: "G733",
    kind: Kind::Headset,
    connection: id::CONN_DONGLE,
    on: false,
    percent: 71,
    anchors: &[
        (-168.0, 60.0),
        (-120.0, 8.0),
        (-117.0, 100.0),
        (-50.0, 20.0),
        (-47.0, 100.0),
        (-0.5, 71.0),
    ],
    usage: ("1", "30"),
    week: [0.5, 1.8, 2.2, 1.0, 0.0, 0.0, 1.5],
    health: Some("89"),
};
const XBOX: Sample = Sample {
    key: "demo-xbox",
    name: "Xbox Controller",
    kind: Kind::Gamepad,
    connection: id::CONN_BLUETOOTH,
    on: true,
    percent: 64,
    anchors: &[(-168.0, 30.0), (-140.0, 100.0), (-30.0, 75.0), (0.0, 64.0)],
    usage: ("1", "15"),
    week: [0.0, 0.0, 2.5, 0.0, 3.1, 4.0, 1.3],
    health: None,
};

fn view(s: &Sample, clock: &Clock, lang: Lang) -> DeviceView {
    let at = |h: f32| clock.wall + (h * 3600.0).round() as i64;
    let hod = clock.hour_of_day();
    DeviceView {
        key: DeviceKey(s.key.to_string()),
        name: s.name.to_string(),
        kind: s.kind,
        connection: strings::get(lang, s.connection).to_string(),
        on: s.on,
        percent: Some(s.percent),
        charging: false,
        status: String::new(),
        label: None,
        eta: None,
        warning: None,
        chart: history(s.anchors, hod)
            .into_iter()
            .map(|(h, p)| ChartPoint {
                at: at(h),
                percent: f64::from(p),
            })
            .collect(),
        forecast: None,
        usage_today: strings::fmt(
            lang,
            id::USAGE_TODAY,
            &[&strings::fmt(lang, id::USAGE_HMIN, &[s.usage.0, s.usage.1])],
        ),
        usage_week: s.week.map(|h| (h * 3600.0) as i64),
        health: strings::fmt(
            lang,
            id::HEALTH_LINE,
            &[&match s.health {
                Some(p) => strings::fmt(lang, id::HEALTH_PERCENT, &[p]),
                None => strings::get(lang, id::HEALTH_LEARNING).to_string(),
            }],
        ),
    }
}

fn forecast(d: &mut DeviceView, clock: &Clock, hours: f64) {
    let from = ChartPoint {
        at: clock.wall,
        percent: f64::from(d.percent.unwrap_or(0)),
    };
    d.forecast = Some(Forecast {
        from,
        to: ChartPoint {
            at: clock.wall + (hours * 3600.0).round() as i64,
            percent: 0.0,
        },
    });
}

fn of_use(lang: Lang, hours: i64) -> String {
    strings::fmt(
        lang,
        id::OF_USE,
        &[&strings::fmt_n(lang, id::HOURS_WHOLE, &[hours])],
    )
}

/// The sample devices in a scene, built like the core builds them (same strings, same order).
pub fn demo_snapshot(phase: DemoPhase, clock: &Clock, lang: Lang) -> Snapshot {
    let warn = matches!(phase, DemoPhase::Scene | DemoPhase::Warning);
    let charge = matches!(phase, DemoPhase::Scene | DemoPhase::Charging);

    let mut mouse = view(&G703, clock, lang);
    mouse.status = of_use(lang, 3);
    mouse.eta = Some(strings::fmt(lang, id::ETA_TODAY, &["17:40"]));
    forecast(&mut mouse, clock, 3.1);
    if warn {
        mouse.warning = Some(strings::fmt(lang, id::SMART_WARNING, &["18"]));
    }

    let mut keys = view(&G915, clock, lang);
    if phase == DemoPhase::Full {
        keys.percent = Some(100);
        keys.status = strings::get(lang, id::STATUS_FULL).to_string();
    } else if charge {
        keys.charging = true;
        keys.status = strings::fmt(lang, id::FULL_IN_HMIN, &["1", "10"]);
    } else {
        keys.status = of_use(lang, 9);
        forecast(&mut keys, clock, 9.0);
    }

    let mut headset = view(&G733, clock, lang);
    headset.status = strings::fmt(lang, id::STATUS_OFF_AT, &["71", "14:02"]);

    let mut pad = view(&XBOX, clock, lang);
    pad.status = of_use(lang, 18);
    forecast(&mut pad, clock, 54.5);

    let mut devices = vec![mouse, keys, headset, pad];
    // The core's order: devices that are on first, then by name.
    devices.sort_by(|a, b| {
        (!a.on, a.name.to_lowercase(), &a.key).cmp(&(!b.on, b.name.to_lowercase(), &b.key))
    });
    // The demo's warnings are the scene's own (a smart warning on the mouse).
    let smart_alert = devices.iter().any(|d| d.on && d.warning.is_some());
    let icon = IconLook::of(&devices, smart_alert, Limits::default());
    Snapshot {
        icon,
        tooltip: tooltip(&devices, lang),
        on_count: devices.iter().filter(|d| d.on).count(),
        devices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_core::snapshot::Juice;

    const CLOCK: Clock = Clock {
        wall: 1_793_035_800,
        utc_offset: -10800,
    };

    #[test]
    fn every_scene_gives_the_icon_look_it_is_named_for() {
        let look = |p| demo_snapshot(p, &CLOCK, Lang::PtBr).icon;
        let is = |juice, charging| IconLook { juice, charging };
        assert_eq!(look(DemoPhase::Scene), is(Juice::Red, true));
        assert_eq!(look(DemoPhase::Normal), is(Juice::Purple, false));
        assert_eq!(look(DemoPhase::Warning), is(Juice::Red, false));
        assert_eq!(look(DemoPhase::Charging), is(Juice::Purple, true));
        assert_eq!(look(DemoPhase::Full), is(Juice::Purple, false));
    }

    #[test]
    fn the_scene_is_four_devices_three_on_in_the_cores_order() {
        let s = demo_snapshot(DemoPhase::Scene, &CLOCK, Lang::PtBr);
        let names: Vec<&str> = s.devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["G703", "G915 X", "Xbox Controller", "G733"]);
        assert_eq!(s.on_count, 3);
        assert!(s.tooltip.encode_utf16().count() <= 127, "{}", s.tooltip);
        assert!(s.tooltip.contains("G703 23%"));
    }

    #[test]
    fn the_cycle_visits_every_icon_state_and_comes_back() {
        let mut p = DemoPhase::Scene;
        let mut seen = vec![];
        for _ in 0..5 {
            p = p.next();
            seen.push(p);
        }
        assert_eq!(
            seen,
            [
                DemoPhase::Normal,
                DemoPhase::Warning,
                DemoPhase::Charging,
                DemoPhase::Full,
                DemoPhase::Normal
            ]
        );
    }

    #[test]
    fn only_the_cycling_demo_asks_for_a_wake() {
        assert_eq!(Source::Empty.next_wake_ms(), None);
        let plain = Source::Demo {
            phase: DemoPhase::Scene,
            cycle: false,
        };
        assert_eq!(plain.next_wake_ms(), None);
        let mut cycling = Source::Demo {
            phase: DemoPhase::Scene,
            cycle: true,
        };
        assert_eq!(cycling.next_wake_ms(), Some(CYCLE_MS));
        assert!(cycling.advance());
    }

    #[test]
    fn the_history_covers_the_week_and_ends_at_now() {
        let h = history(G703.anchors, 14.5);
        assert_eq!(h.first(), Some(&(-168.0, 70.0)));
        assert_eq!(h.last(), Some(&(0.0, 23.0)));
        assert!(h.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn the_empty_snapshot_has_the_core_tooltip_in_both_languages() {
        assert_eq!(
            empty_snapshot(Lang::PtBr).tooltip,
            "JuiceBar: nenhum dispositivo"
        );
        assert_eq!(empty_snapshot(Lang::En).tooltip, "JuiceBar: no devices");
    }

    #[test]
    fn hour_of_day_uses_the_local_offset() {
        assert!((CLOCK.hour_of_day() - 14.5).abs() < 1e-3);
    }
}
