// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The view model of the flyout and the tray mark: plain data, no Win32.
//!
//! The app integration maps the core's snapshot onto these types; this crate only draws them.
//! `UiState::sample()` is the Windows scene of the design mockup (`docs/design/options/1-native`).

/// Light or dark appearance of the flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

/// The user's accent color (Windows personalization), as 8-bit RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Our own 24 x 24 line drawings (`docs/design/harness/glyphs.js`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Mouse,
    Keyboard,
    Headset,
    Gamepad,
    Earbuds,
    Bolt,
    Bluetooth,
    Wireless,
    Warning,
    Gear,
    Leaf,
}

/// What the single tray mark shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconState {
    Normal,
    Warning,
    Charging,
}

/// One device row (and, when `expanded`, its detail card).
#[derive(Clone, Debug, PartialEq)]
pub struct UiDevice {
    pub name: String,
    pub glyph: Glyph,
    /// Connection label ("LIGHTSPEED", "Bluetooth"); empty hides it.
    pub connection: String,
    /// Devices that are on come first; the others sit under the "Desligados" separator.
    pub on: bool,
    pub percent: u8,
    /// Filled part of the capsule, 0.0 to 1.0.
    pub capsule_fraction: f32,
    pub status_line: String,
    /// The smart warning, shown in the expanded card.
    pub warning: Option<String>,
    /// Level history of the last 7 days: (hours relative to now, in -168..=0; percent).
    pub chart: Vec<(f32, f32)>,
    /// Where the dashed forecast starts: (hours, percent), normally "now".
    pub forecast_from: Option<(f32, f32)>,
    /// Hours (relative to now, may be positive) at which the forecast reaches 0 %.
    pub forecast_empty_hours: f32,
    pub usage_today: String,
    /// Battery health in percent; `None` shows "aprendendo".
    pub health: Option<u8>,
    pub charging: bool,
    pub expanded: bool,
}

/// Everything the flyout draws.
#[derive(Clone, Debug, PartialEq)]
pub struct UiState {
    pub theme: Theme,
    pub accent: Rgb,
    pub devices: Vec<UiDevice>,
    pub rgb_economy: bool,
    pub hotkey_text: String,
    pub alert: IconState,
    /// At or below this percent a device is drawn as low (red).
    pub low_threshold: u8,
    /// The seven weekday labels under the chart, oldest first.
    pub day_labels: [String; 7],
}

impl UiState {
    /// True for a device that is on and at or below the low threshold.
    pub fn is_low(&self, d: &UiDevice) -> bool {
        d.on && d.percent <= self.low_threshold
    }

    /// Indices of `devices` in display order: devices that are on first, then the off ones.
    pub fn display_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.devices.len())
            .filter(|&i| self.devices[i].on)
            .collect();
        order.extend((0..self.devices.len()).filter(|&i| !self.devices[i].on));
        order
    }

    /// The mockup's Windows scene: G703 23 % with warning and expanded chart, G915 X charging,
    /// Controle Xbox 64 %, G733 off.
    pub fn sample(theme: Theme) -> UiState {
        let accent = match theme {
            Theme::Light => Rgb(0x00, 0x67, 0xc0),
            Theme::Dark => Rgb(0x60, 0xcd, 0xff),
        };
        let day = |s: &str| s.to_string();
        UiState {
            theme,
            accent,
            devices: vec![
                UiDevice {
                    name: "G703".into(),
                    glyph: Glyph::Mouse,
                    connection: "LIGHTSPEED".into(),
                    on: true,
                    percent: 23,
                    capsule_fraction: 0.23,
                    status_line: "~3 h de uso".into(),
                    warning: Some("Não chega ao fim do dia: carregue antes das 18h".into()),
                    chart: sample_history(&[
                        (-168.0, 70.0),
                        (-120.0, 14.0),
                        (-117.0, 100.0),
                        (-52.0, 18.0),
                        (-49.0, 100.0),
                        (0.0, 23.0),
                    ]),
                    forecast_from: Some((0.0, 23.0)),
                    forecast_empty_hours: 3.1,
                    usage_today: "4 h 10 min".into(),
                    health: Some(94),
                    charging: false,
                    expanded: true,
                },
                UiDevice {
                    name: "G915 X".into(),
                    glyph: Glyph::Keyboard,
                    connection: "LIGHTSPEED".into(),
                    on: true,
                    percent: 46,
                    capsule_fraction: 0.46,
                    status_line: "cheio em ~1 h 10 min".into(),
                    warning: None,
                    chart: Vec::new(),
                    forecast_from: None,
                    forecast_empty_hours: 0.0,
                    usage_today: "6 h 05 min".into(),
                    health: Some(98),
                    charging: true,
                    expanded: false,
                },
                UiDevice {
                    name: "G733".into(),
                    glyph: Glyph::Headset,
                    connection: String::new(),
                    on: false,
                    percent: 71,
                    capsule_fraction: 0.71,
                    status_line: "desligado · 71% às 14:02".into(),
                    warning: None,
                    chart: Vec::new(),
                    forecast_from: None,
                    forecast_empty_hours: 0.0,
                    usage_today: "1 h 30 min".into(),
                    health: Some(89),
                    charging: false,
                    expanded: false,
                },
                UiDevice {
                    name: "Controle Xbox".into(),
                    glyph: Glyph::Gamepad,
                    connection: "Bluetooth".into(),
                    on: true,
                    percent: 64,
                    capsule_fraction: 0.64,
                    status_line: "~18 h de uso".into(),
                    warning: None,
                    chart: Vec::new(),
                    forecast_from: None,
                    forecast_empty_hours: 0.0,
                    usage_today: "1 h 15 min".into(),
                    health: None,
                    charging: false,
                    expanded: false,
                },
            ],
            rgb_economy: false,
            hotkey_text: "Ctrl+Alt+B".into(),
            alert: IconState::Warning,
            low_threshold: 20,
            day_labels: [
                day("seg"),
                day("ter"),
                day("qua"),
                day("qui"),
                day("sex"),
                day("sáb"),
                day("dom"),
            ],
        }
    }
}

/// Port of the mockup's `build()` in `docs/design/harness/sample-state.js`: one point per hour
/// between the anchors; a falling segment discharges fast in work hours (9 to 19) and slowly at
/// night, a rising one charges linearly. "Now" is 14:30.
fn sample_history(anchors: &[(f32, f32)]) -> Vec<(f32, f32)> {
    const NOW_HOD: f32 = 14.5;
    let weight = |h: f32| {
        let hod = (NOW_HOD + h).rem_euclid(24.0);
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
                let step = 0.25;
                let mut t = a.0;
                while t < b.0 {
                    let w = weight(t);
                    total += w;
                    if t < h {
                        part += w;
                    }
                    t += step;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_matches_the_mockup_scene() {
        let s = UiState::sample(Theme::Light);
        assert_eq!(s.devices.len(), 4);
        assert_eq!(s.devices[0].percent, 23);
        assert!(s.devices[0].warning.is_some() && s.devices[0].expanded);
        assert!(s.devices[1].charging);
        assert!(!s.devices[2].on);
        assert_eq!(s.devices[2].status_line, "desligado · 71% às 14:02");
    }

    #[test]
    fn display_order_puts_off_devices_last() {
        let s = UiState::sample(Theme::Light);
        assert_eq!(s.display_order(), vec![0, 1, 3, 2]);
    }

    #[test]
    fn low_needs_on_and_threshold() {
        let s = UiState::sample(Theme::Light);
        assert!(!s.is_low(&s.devices[0]), "23 is above 20");
        let mut d = s.devices[0].clone();
        d.percent = 20;
        assert!(s.is_low(&d));
        d.on = false;
        assert!(!s.is_low(&d));
    }

    #[test]
    fn sample_history_covers_seven_days_and_ends_at_now() {
        let h = &UiState::sample(Theme::Light).devices[0].chart;
        assert_eq!(h.first().unwrap().0, -168.0);
        assert_eq!(h.last().unwrap(), &(0.0, 23.0));
        assert!(h.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(h.iter().all(|p| (0.0..=100.0).contains(&p.1)));
    }
}
