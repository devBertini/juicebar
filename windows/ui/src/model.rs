// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The view model of the flyout and the tray mark: plain data, no Win32.
//!
//! The app integration maps the core's snapshot onto these types; this crate only draws them.
//! `UiState::sample()` is the Windows scene of the design mockup (`docs/design/options/1-native`).

use crate::icon::Juice;
use crate::settings::{SettingsRow, SettingsSection, UiSettings};

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
    /// The level; `None` while the device gives no reading: the row shows "–", an empty capsule and no red.
    pub percent: Option<u8>,
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
    /// Hours of use on each of the last 7 local days, oldest first; the last one is today.
    pub week_hours: [f32; 7],
    /// Battery health in percent; `None` shows "aprendendo".
    pub health: Option<u8>,
    pub charging: bool,
    pub expanded: bool,
}

/// The flyout's fixed labels, in the user's language. `Default` is pt-BR; the app fills them
/// from the core's string table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiLabels {
    /// Over the chart: "Últimos 7 dias".
    pub last_7_days: String,
    /// The chart's key for the dashed line: "previsão".
    pub forecast: String,
    /// Stat label: "Hoje".
    pub today: String,
    /// After today's usage time: "de uso" ("4 h 10 min de uso").
    pub of_use: String,
    /// Stat label: "Saúde da bateria".
    pub battery_health: String,
    /// The health value while it is still being learned: "aprendendo".
    pub learning: String,
    /// Separator over the devices that are off: "Desligados".
    pub off_devices: String,
    /// The footer switch: "Economia de Energia".
    pub rgb_economy: String,
    /// What the (i) next to the switch says: it turns the devices' LEDs off to save battery.
    pub rgb_info: String,
    /// Over the week's bars: "Horas de uso por dia".
    pub usage_by_day: String,
}

impl Default for UiLabels {
    fn default() -> UiLabels {
        UiLabels {
            last_7_days: "Últimos 7 dias".into(),
            forecast: "previsão".into(),
            today: "Hoje".into(),
            of_use: "de uso".into(),
            battery_health: "Saúde da bateria".into(),
            learning: "aprendendo".into(),
            off_devices: "Desligados".into(),
            rgb_economy: "Economia de Energia".into(),
            rgb_info:
                "Desliga as luzes (LEDs) dos dispositivos compatíveis para a bateria durar mais"
                    .into(),
            usage_by_day: "Horas de uso por dia".into(),
        }
    }
}

/// The first-open page that replaces the device list (`UiState::welcome`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiWelcome {
    pub title: String,
    /// What JuiceBar does.
    pub body: String,
    /// "Deixe o ícone ao lado do relógio".
    pub tray_title: String,
    /// The Settings path, and the other way: drag it from the arrow.
    pub tray_steps: String,
    /// "Idealizado por Claudio Bertini".
    pub credit: String,
    /// The button: "Começar".
    pub start: String,
}

impl Default for UiWelcome {
    fn default() -> UiWelcome {
        UiWelcome {
            title: "Boas-vindas ao JuiceBar".into(),
            body: "O JuiceBar mostra a bateria do mouse, do teclado, do headset e dos controles em um só lugar e avisa antes que alguma acabe.".into(),
            tray_title: "Deixe o ícone ao lado do relógio".into(),
            tray_steps: "Abra Configurações > Personalização > Barra de tarefas > Outros ícones da bandeja do sistema e ative o JuiceBar. Ou arraste-o da seta ^ para a barra de tarefas.".into(),
            credit: "Idealizado por Claudio Bertini".into(),
            start: "Começar".into(),
        }
    }
}

/// Everything the flyout draws.
#[derive(Clone, Debug, PartialEq)]
pub struct UiState {
    pub theme: Theme,
    pub accent: Rgb,
    pub devices: Vec<UiDevice>,
    pub rgb_economy: bool,
    /// The (i) next to the switch was clicked: its text stays open until the next click (it also shows while
    /// the pointer is over the (i)).
    pub info_open: bool,
    pub hotkey_text: String,
    /// The small line under the footer while G HUB puts the lighting back; `None` hides it.
    pub ghub_notice: Option<String>,
    pub alert: IconState,
    /// The juice color of the tray mark: the glow at the top of the panel takes it.
    pub juice: Juice,
    /// At or below this percent a device is drawn as low (red).
    pub low_threshold: u8,
    /// The seven weekday labels under the chart, oldest first.
    pub day_labels: [String; 7],
    /// The weekday under each of the 7 usage bars, oldest first (the last is today).
    pub usage_labels: [String; 7],
    /// Shown in place of the device list while there are no devices: the first line is the
    /// title, a second line (after a line break) the hint. Unused when `devices` is not empty.
    pub empty_text: String,
    /// The fixed labels.
    pub labels: UiLabels,
    /// The first-open page; while `Some`, it is drawn instead of the list and the footer.
    pub welcome: Option<UiWelcome>,
    /// The Settings page; while `Some` (and no welcome), it is drawn instead of the list and the footer.
    pub settings: Option<UiSettings>,
}

impl UiState {
    /// True for a device that is on and at or below the low threshold.
    pub fn is_low(&self, d: &UiDevice) -> bool {
        d.on && d.percent.is_some_and(|p| p <= self.low_threshold)
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
    /// Xbox Controller 64 %, G733 off.
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
                    percent: Some(23),
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
                    week_hours: [3.2, 5.1, 4.6, 6.0, 1.4, 0.8, 4.2],
                    health: Some(94),
                    charging: false,
                    expanded: true,
                },
                UiDevice {
                    name: "G915 X".into(),
                    glyph: Glyph::Keyboard,
                    connection: "LIGHTSPEED".into(),
                    on: true,
                    percent: Some(46),
                    capsule_fraction: 0.46,
                    status_line: "cheio em ~1 h 10 min".into(),
                    warning: None,
                    chart: Vec::new(),
                    forecast_from: None,
                    forecast_empty_hours: 0.0,
                    usage_today: "6 h 05 min".into(),
                    week_hours: [5.0, 6.4, 7.2, 6.8, 2.0, 1.2, 6.1],
                    health: Some(98),
                    charging: true,
                    expanded: false,
                },
                UiDevice {
                    name: "G733".into(),
                    glyph: Glyph::Headset,
                    connection: String::new(),
                    on: false,
                    percent: Some(71),
                    capsule_fraction: 0.71,
                    status_line: "desligado · 71% às 14:02".into(),
                    warning: None,
                    chart: Vec::new(),
                    forecast_from: None,
                    forecast_empty_hours: 0.0,
                    usage_today: "1 h 30 min".into(),
                    week_hours: [0.5, 1.8, 2.2, 1.0, 0.0, 0.0, 1.5],
                    health: Some(89),
                    charging: false,
                    expanded: false,
                },
                UiDevice {
                    name: "Xbox Controller".into(),
                    glyph: Glyph::Gamepad,
                    connection: "Bluetooth".into(),
                    on: true,
                    percent: Some(64),
                    capsule_fraction: 0.64,
                    status_line: "~18 h de uso".into(),
                    warning: None,
                    chart: Vec::new(),
                    forecast_from: None,
                    forecast_empty_hours: 0.0,
                    usage_today: "1 h 15 min".into(),
                    week_hours: [0.0, 0.0, 2.5, 0.0, 3.1, 4.0, 1.3],
                    health: None,
                    charging: false,
                    expanded: false,
                },
            ],
            rgb_economy: false,
            info_open: false,
            hotkey_text: "Ctrl+Alt+B".into(),
            ghub_notice: None,
            alert: IconState::Warning,
            juice: Juice::Red,
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
            usage_labels: ["ter", "qua", "qui", "sex", "sáb", "dom", "seg"].map(String::from),
            empty_text: String::new(),
            labels: UiLabels::default(),
            welcome: None,
            settings: None,
        }
    }

    /// The sample with every card closed.
    pub fn sample_collapsed(theme: Theme) -> UiState {
        let mut s = UiState::sample(theme);
        for d in &mut s.devices {
            d.expanded = false;
        }
        s
    }

    /// The sample where the Xbox controller gives no reading: "–", an empty capsule, nothing red.
    pub fn sample_no_reading(theme: Theme) -> UiState {
        let mut s = UiState::sample_collapsed(theme);
        s.juice = Juice::Purple;
        s.alert = IconState::Normal;
        let d = &mut s.devices[3];
        d.percent = None;
        d.capsule_fraction = 0.0;
        d.status_line = "sem leitura".into();
        s.devices[0].percent = Some(58);
        s.devices[0].capsule_fraction = 0.58;
        s.devices[0].warning = None;
        s
    }

    /// The Settings page with every kind of row, as the app fills it (Brazilian Portuguese).
    pub fn sample_settings(theme: Theme) -> UiState {
        let mut s = UiState::sample(theme);
        s.juice = Juice::Purple;
        s.settings = Some(sample_settings_page());
        s
    }

    /// The first-open page.
    pub fn sample_welcome(theme: Theme) -> UiState {
        let mut s = UiState::sample(theme);
        s.juice = Juice::Purple;
        s.welcome = Some(UiWelcome::default());
        s
    }
}

/// The Settings page of the drawings and tests: the app's sections and rows, with their texts in Portuguese.
pub fn sample_settings_page() -> UiSettings {
    UiSettings::new(
        "Configurações",
        vec![
            SettingsSection::new(
                "Aparência",
                vec![
                    SettingsRow::choice(1, "Tema", &["Automático", "Claro", "Escuro"], 0),
                    SettingsRow::toggle(2, "Animação do ícone", true)
                        .with_detail("O suco se mexe de vez em quando"),
                    SettingsRow::link(
                        3,
                        "Ative os efeitos de transparência do Windows para o fundo translúcido",
                        "Abrir",
                    ),
                ],
            ),
            SettingsSection::new(
                "Ícone",
                vec![
                    SettingsRow::stepper(4, "Verde quando todos acima de", 50, (25, 95, 5), "50%"),
                    SettingsRow::stepper(
                        5,
                        "Vermelho quando algum abaixo de",
                        20,
                        (5, 50, 5),
                        "20%",
                    ),
                ],
            ),
            SettingsSection::new(
                "Alertas",
                vec![
                    SettingsRow::stepper(6, "Avisar com bateria abaixo de", 10, (5, 30, 5), "10%"),
                    SettingsRow::toggle(7, "Aviso inteligente", true)
                        .with_detail("Quando um dispositivo não vai durar o dia"),
                    SettingsRow::toggle(8, "Carga completa", true)
                        .with_detail("Avisar quando um dispositivo terminar de carregar"),
                    SettingsRow::toggle(9, "Não perturbe", false)
                        .with_detail("Sem notificações; o ícone continua mudando"),
                ],
            ),
            SettingsSection::new(
                "Dispositivos",
                vec![
                    SettingsRow::device(17, "G703", Glyph::Mouse, true, (false, true))
                        .with_detail("Receptor"),
                    SettingsRow::device(18, "G915 X", Glyph::Keyboard, true, (true, true))
                        .with_detail("Receptor"),
                    SettingsRow::device(19, "G733", Glyph::Headset, false, (true, true))
                        .with_detail("Dongle · oculto"),
                    SettingsRow::device(20, "Xbox Controller", Glyph::Gamepad, true, (true, false))
                        .with_detail("Bluetooth"),
                    SettingsRow::note(
                        21,
                        "Chave: mostrar no painel. Lápis: renomear. Setas: mudar a ordem.",
                    ),
                ],
            ),
            SettingsSection::new(
                "Geral",
                vec![
                    SettingsRow::toggle(10, "Iniciar com o Windows", true),
                    SettingsRow::choice(11, "Idioma", &["Automático", "Português", "English"], 0),
                    SettingsRow::link(12, "Atalho: Ctrl+Alt+B", "Alterar")
                        .with_detail("Abre o JuiceBar de qualquer lugar"),
                ],
            ),
            SettingsSection::new(
                "Sobre",
                vec![
                    SettingsRow::identity(
                        13,
                        "JuiceBar",
                        "Versão 0.1.0",
                        "Idealizado por Claudio Bertini",
                        "devBertini",
                    ),
                    SettingsRow::link(14, "Código no GitHub", ""),
                    SettingsRow::link(15, "Licença GPL-3.0", "")
                        .with_detail("Software livre: use, estude, compartilhe e altere"),
                    SettingsRow::note(16, "Sem rede, sem telemetria: tudo fica neste computador"),
                ],
            ),
        ],
    )
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
        assert_eq!(s.devices[0].percent, Some(23));
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
        d.percent = Some(20);
        assert!(s.is_low(&d));
        d.on = false;
        assert!(!s.is_low(&d));
    }

    #[test]
    fn a_device_without_a_reading_is_never_low() {
        let s = UiState::sample_no_reading(Theme::Light);
        assert!(s.devices[3].on && s.devices[3].percent.is_none());
        assert!(!s.is_low(&s.devices[3]));
        assert_eq!(s.devices[3].capsule_fraction, 0.0);
    }

    #[test]
    fn sample_variants_differ_where_they_should() {
        assert!(
            UiState::sample_collapsed(Theme::Dark)
                .devices
                .iter()
                .all(|d| !d.expanded)
        );
        let w = UiState::sample_welcome(Theme::Dark);
        assert!(w.welcome.is_some() && UiState::sample(Theme::Dark).welcome.is_none());
        assert!(
            w.devices.len() == 4,
            "the list stays for the page behind it"
        );
        let p = UiState::sample_settings(Theme::Light);
        assert!(p.settings.is_some() && p.welcome.is_none());
    }

    #[test]
    fn the_sample_settings_page_has_unique_ids_and_a_focusable_control_in_each_section() {
        let page = sample_settings_page();
        let mut ids: Vec<u16> = page.rows().map(|r| r.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for s in &page.sections {
            assert!(s.rows.iter().any(|r| r.focusable()), "{}", s.title);
        }
    }

    #[test]
    fn week_hours_are_non_negative_and_end_today() {
        for d in &UiState::sample(Theme::Light).devices {
            assert!(d.week_hours.iter().all(|h| (0.0..=24.0).contains(h)));
        }
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
