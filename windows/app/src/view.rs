// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Maps the core's `Snapshot` onto the flyout's `UiState`. Pure: no Win32, so it is unit tested.

use juicebar_core::estimator::LOW_PERCENT;
use juicebar_core::snapshot::{self, DeviceView, Snapshot};
use juicebar_core::strings::{self, Lang, id};
use juicebar_core::types::{DeviceKey, Kind};
use juicebar_ui::model::{Glyph, IconState, Rgb, Theme, UiDevice, UiLabels, UiState};

/// Everything besides the snapshot that the flyout depends on.
pub struct ViewInput<'a> {
    pub snapshot: &'a Snapshot,
    pub theme: Theme,
    pub accent: Rgb,
    pub lang: Lang,
    pub rgb_economy: bool,
    pub hotkey_text: &'a str,
    /// The device whose detail card is open.
    pub expanded: Option<&'a DeviceKey>,
    /// The text for an empty device list.
    pub empty_text: &'a str,
    /// Epoch seconds now.
    pub now: i64,
    /// Local offset from UTC in seconds.
    pub utc_offset: i32,
}

pub fn glyph_for(kind: Kind) -> Glyph {
    match kind {
        Kind::Mouse => Glyph::Mouse,
        Kind::Keyboard => Glyph::Keyboard,
        Kind::Headset => Glyph::Headset,
        Kind::Earbuds => Glyph::Earbuds,
        Kind::Gamepad => Glyph::Gamepad,
        Kind::Other => Glyph::Wireless,
    }
}

pub fn icon_state(s: snapshot::IconState) -> IconState {
    match s {
        snapshot::IconState::Normal => IconState::Normal,
        snapshot::IconState::Warning => IconState::Warning,
        snapshot::IconState::Charging => IconState::Charging,
    }
}

/// The part of `text` that stands where `{0}` stands in `template`; `None` when the text does not
/// fit the template. Used to take "4 h 10 min" out of the core's "Hoje: 4 h 10 min de uso".
pub fn extract_arg(template: &str, text: &str) -> Option<String> {
    let at = template.find("{0}")?;
    let (prefix, suffix) = (&template[..at], &template[at + 3..]);
    let rest = text.strip_prefix(prefix)?.strip_suffix(suffix)?;
    Some(rest.to_string())
}

/// Weekday index, Monday = 0, of an epoch second shifted to local time.
fn weekday(local_secs: i64) -> usize {
    // 1970-01-01 was a Thursday (3).
    ((local_secs.div_euclid(86_400) + 3).rem_euclid(7)) as usize
}

/// The seven labels under the chart, oldest first: the weekday at the middle of each 24 h block
/// of the 168 h the chart covers.
pub fn day_labels(now: i64, utc_offset: i32, lang: Lang) -> [String; 7] {
    const WD: [&str; 7] = [
        id::WD_0,
        id::WD_1,
        id::WD_2,
        id::WD_3,
        id::WD_4,
        id::WD_5,
        id::WD_6,
    ];
    std::array::from_fn(|i| {
        let center = now + utc_offset as i64 - (156 - 24 * i as i64) * 3600;
        strings::get(lang, WD[weekday(center)]).to_string()
    })
}

fn hours_from_now(at: i64, now: i64) -> f32 {
    (at - now) as f32 / 3600.0
}

fn ui_device(d: &DeviceView, v: &ViewInput<'_>) -> UiDevice {
    let expanded = v.expanded == Some(&d.key);
    let percent = d.percent.unwrap_or(0);
    let usage_today = extract_arg(strings::get(v.lang, id::USAGE_TODAY), &d.usage_today)
        .unwrap_or_else(|| d.usage_today.clone());
    // "Saúde da bateria: 92%" -> 92; "...: aprendendo" -> still learning.
    let health = extract_arg(strings::get(v.lang, id::HEALTH_LINE), &d.health).and_then(|t| {
        t.strip_suffix('%')
            .and_then(|n| n.trim().parse::<u8>().ok())
    });
    let mut status_line = d.status.clone();
    if expanded && let Some(label) = &d.label {
        status_line = format!("{status_line} · {label}");
    }
    let chart: Vec<(f32, f32)> = d
        .chart
        .iter()
        .map(|p| (hours_from_now(p.at, v.now), p.percent as f32))
        .filter(|(h, _)| (-168.0..=0.0).contains(h))
        .collect();
    let (forecast_from, forecast_empty_hours) = match &d.forecast {
        Some(f) => (
            Some((hours_from_now(f.from.at, v.now), f.from.percent as f32)),
            hours_from_now(f.to.at, v.now),
        ),
        None => (None, 0.0),
    };
    UiDevice {
        name: d.name.clone(),
        glyph: glyph_for(d.kind),
        connection: d.connection.clone(),
        on: d.on,
        percent,
        capsule_fraction: f32::from(percent) / 100.0,
        status_line,
        warning: d.warning.clone(),
        chart,
        forecast_from,
        forecast_empty_hours,
        usage_today,
        health,
        charging: d.charging,
        expanded,
    }
}

/// The flyout's state for a snapshot.
pub fn ui_state(v: &ViewInput<'_>) -> UiState {
    UiState {
        theme: v.theme,
        accent: v.accent,
        devices: v.snapshot.devices.iter().map(|d| ui_device(d, v)).collect(),
        rgb_economy: v.rgb_economy,
        hotkey_text: v.hotkey_text.to_string(),
        alert: icon_state(v.snapshot.icon),
        low_threshold: LOW_PERCENT as u8,
        day_labels: day_labels(v.now, v.utc_offset, v.lang),
        empty_text: v.empty_text.to_string(),
        labels: labels(v.lang),
    }
}

/// The flyout's static labels in the app's language.
pub fn labels(lang: Lang) -> UiLabels {
    let s = |key| strings::get(lang, key).to_string();
    UiLabels {
        last_7_days: s(strings::id::CHART_LAST_7_DAYS),
        forecast: s(strings::id::CHART_FORECAST),
        today: s(strings::id::STAT_TODAY),
        of_use: s(strings::id::STAT_OF_USE),
        battery_health: s(strings::id::STAT_HEALTH),
        learning: s(strings::id::HEALTH_LEARNING),
        off_devices: s(strings::id::SECTION_OFF),
        rgb_economy: s(strings::id::RGB_SAVER),
    }
}

/// The device a new snapshot opens by default: the first one with a warning.
pub fn default_expanded(s: &Snapshot) -> Option<DeviceKey> {
    s.devices
        .iter()
        .find(|d| d.warning.is_some())
        .map(|d| d.key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flyout_labels_follow_the_language() {
        let en = labels(Lang::En);
        assert_eq!(en.last_7_days, "Last 7 days");
        assert_eq!(en.off_devices, "Off");
        assert_eq!(en.rgb_economy, "RGB saver");
        let pt = labels(Lang::PtBr);
        assert_eq!(pt.last_7_days, "Últimos 7 dias");
        assert_eq!(pt.battery_health, "Saúde da bateria");
        assert_eq!(pt.of_use, "de uso");
    }
    use crate::source::{Clock, DemoPhase, demo_snapshot};

    fn input<'a>(s: &'a Snapshot, lang: Lang, exp: Option<&'a DeviceKey>) -> ViewInput<'a> {
        ViewInput {
            snapshot: s,
            theme: Theme::Dark,
            accent: Rgb(1, 2, 3),
            lang,
            rgb_economy: true,
            hotkey_text: "Ctrl+Alt+B",
            expanded: exp,
            empty_text: "vazio",
            now: DEMO_NOW,
            utc_offset: -10800,
        }
    }

    // Monday 26/10/2026 14:30 in Sao Paulo (17:30 UTC).
    const DEMO_NOW: i64 = 1_793_035_800;

    fn clock() -> Clock {
        Clock {
            wall: DEMO_NOW,
            utc_offset: -10800,
        }
    }

    #[test]
    fn extracts_the_value_out_of_a_template() {
        assert_eq!(
            extract_arg("Hoje: {0} de uso", "Hoje: 4 h 10 min de uso").as_deref(),
            Some("4 h 10 min")
        );
        assert_eq!(extract_arg("Hoje: {0} de uso", "outra coisa"), None);
        assert_eq!(extract_arg("sem slot", "sem slot"), None);
    }

    #[test]
    fn maps_the_demo_scene_row_by_row() {
        let snap = demo_snapshot(DemoPhase::Scene, &clock(), Lang::PtBr);
        let key = default_expanded(&snap);
        assert_eq!(
            key.as_ref().map(|k| k.0.as_str()),
            Some("demo-g703"),
            "the warned device opens by default"
        );
        let ui = ui_state(&input(&snap, Lang::PtBr, key.as_ref()));
        assert_eq!(ui.devices.len(), 4);
        assert_eq!(ui.alert, IconState::Warning);
        assert!(ui.rgb_economy && ui.theme == Theme::Dark);
        assert_eq!(ui.hotkey_text, "Ctrl+Alt+B");
        assert_eq!(ui.low_threshold, 10);
        let g703 = ui.devices.iter().find(|d| d.name == "G703").unwrap();
        assert_eq!(g703.name, "G703");
        assert_eq!(g703.glyph, Glyph::Mouse);
        assert_eq!(g703.percent, 23);
        assert!((g703.capsule_fraction - 0.23).abs() < 1e-6);
        assert!(g703.on && g703.expanded && !g703.charging);
        assert_eq!(g703.usage_today, "4 h 10 min");
        assert_eq!(g703.health, Some(94));
        assert!(g703.warning.as_deref().unwrap().contains("18"));
        assert_eq!(g703.chart.first().unwrap().0, -168.0);
        assert!(g703.chart.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(g703.chart.iter().all(|p| (0.0..=100.0).contains(&p.1)));
        assert_eq!(g703.forecast_from.unwrap().0, 0.0);
        assert!((g703.forecast_empty_hours - 3.1).abs() < 0.01);
        let g915 = ui.devices.iter().find(|d| d.name == "G915 X").unwrap();
        assert_eq!(g915.glyph, Glyph::Keyboard);
        assert!(g915.charging && !g915.expanded);
        let off = ui.devices.iter().find(|d| !d.on).unwrap();
        assert_eq!(off.name, "G733");
        assert_eq!(off.status_line, "desligado · 71% às 14:02");
        let xbox = ui.devices.iter().find(|d| d.name.contains("Xbox")).unwrap();
        assert_eq!(xbox.glyph, Glyph::Gamepad);
        assert_eq!(xbox.health, None, "learning health has no percent");
    }

    #[test]
    fn english_texts_are_extracted_too() {
        let snap = demo_snapshot(DemoPhase::Scene, &clock(), Lang::En);
        let ui = ui_state(&input(&snap, Lang::En, None));
        let g703 = ui.devices.iter().find(|d| d.name == "G703").unwrap();
        assert_eq!(g703.usage_today, "4 h 10 min");
        assert_eq!(g703.health, Some(94));
        assert!(!g703.expanded);
        assert_eq!(g703.status_line, "~3 h of use");
    }

    #[test]
    fn a_device_without_a_reading_shows_zero_and_an_empty_capsule() {
        let mut snap = demo_snapshot(DemoPhase::Normal, &clock(), Lang::PtBr);
        snap.devices[0].percent = None;
        let ui = ui_state(&input(&snap, Lang::PtBr, None));
        assert_eq!(ui.devices[0].percent, 0);
        assert_eq!(ui.devices[0].capsule_fraction, 0.0);
    }

    #[test]
    fn an_empty_snapshot_gives_an_empty_list_with_the_text() {
        let snap = crate::source::empty_snapshot(Lang::En);
        let ui = ui_state(&input(&snap, Lang::En, None));
        assert!(ui.devices.is_empty());
        assert_eq!(ui.empty_text, "vazio");
        assert_eq!(ui.alert, IconState::Normal);
    }

    #[test]
    fn day_labels_follow_the_weekday_of_each_block() {
        // Monday 14:30 now: the first block is centred on last Tuesday 02:30, the last on Monday 02:30.
        let l = day_labels(DEMO_NOW, -10800, Lang::PtBr);
        assert_eq!(
            l,
            ["ter", "qua", "qui", "sex", "sáb", "dom", "seg"].map(String::from)
        );
        let e = day_labels(DEMO_NOW, -10800, Lang::En);
        assert_eq!(e[0], "Tue");
        assert_eq!(e[4], "Sat");
    }

    #[test]
    fn weekday_of_known_dates() {
        assert_eq!(weekday(0), 3, "1970-01-01 is a Thursday");
        assert_eq!(weekday(DEMO_NOW - 3 * 3600), 0, "26/10/2026 is a Monday");
        assert_eq!(weekday(-1), 2, "one second before the epoch is a Wednesday");
    }

    #[test]
    fn every_kind_has_a_glyph() {
        for k in [
            Kind::Mouse,
            Kind::Keyboard,
            Kind::Headset,
            Kind::Earbuds,
            Kind::Gamepad,
            Kind::Other,
        ] {
            let _ = glyph_for(k);
        }
        assert_eq!(glyph_for(Kind::Other), Glyph::Wireless);
    }
}
