// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The Settings page's content: which sections and rows the flyout shows for the user's settings, and what a
//! click or key on a row asks the app to do. Pure: no Win32, so it is unit tested.
//!
//! To add a section, build its rows with new ids in `build` and map their hits in `action`; the flyout's drawing,
//! layout, keyboard and scrolling need nothing. The Dispositivos section has one row per device, with ids from
//! `row::DEVICE_BASE` up that the app hands out (a device keeps its id for the whole run).

use juicebar_core::strings::{self, Lang, id};
use juicebar_ui::model::Glyph;
use juicebar_ui::settings::{
    PART_DOWN, PART_RENAME, PART_ROW, PART_SHOW, PART_UP, RowId, RowKind, SettingsRow,
    SettingsSection, UiSettings, segment_of, stepped,
};

use crate::devices::Rename;
use crate::hotkey::Hotkey;
use crate::live::DeviceChoice;
use crate::settings::{
    Adjusted, GREEN_MAX, GREEN_MIN, LangChoice, RED_MAX, RED_MIN, Settings, ThemeChoice, WARN_MAX,
    WARN_MIN,
};
use crate::view::glyph_for;

/// The id of every row (the flyout hands them back in a hit).
pub mod row {
    use juicebar_ui::settings::RowId;

    pub const THEME: RowId = 1;
    pub const ANIMATION: RowId = 2;
    pub const TRANSPARENCY: RowId = 3;
    pub const GREEN: RowId = 4;
    pub const RED: RowId = 5;
    pub const WARN: RowId = 6;
    pub const SMART: RowId = 7;
    pub const FULL: RowId = 8;
    pub const DND: RowId = 9;
    pub const AUTOSTART: RowId = 10;
    pub const LANGUAGE: RowId = 11;
    pub const HOTKEY: RowId = 12;
    pub const ABOUT: RowId = 13;
    pub const GITHUB: RowId = 14;
    pub const LICENSE: RowId = 15;
    pub const PRIVACY: RowId = 16;
    pub const HOTKEY_RESET: RowId = 17;
    pub const DEVICES_NOTE: RowId = 18;
    pub const CALIBRATION: RowId = 19;
    /// The first id of the device rows.
    pub const DEVICE_BASE: RowId = 100;
}

pub const GITHUB_URL: &str = "https://github.com/devBertini/juicebar";
pub const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
pub const TRANSPARENCY_URL: &str = "ms-settings:personalization-colors";

/// Where the shortcut's row stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HotkeyState {
    #[default]
    Idle,
    /// The page waits for the next key combination.
    Capturing,
    /// The page still waits: the last combination cannot be a shortcut (too few modifiers) and the row says why.
    Refused,
    /// The last combination was taken by another program.
    InUse,
}

impl HotkeyState {
    /// The page is waiting for a key combination.
    pub fn waits(self) -> bool {
        matches!(self, HotkeyState::Capturing | HotkeyState::Refused)
    }
}

/// The "Start with Windows" row. In a test or development launch it shows the stored state, dimmed, with a line
/// saying why it cannot be changed.
fn autostart_row(i: &PageInput<'_>, lang: Lang, label: &str) -> SettingsRow {
    let row = SettingsRow::toggle(row::AUTOSTART, label, i.autostart);
    if !i.autostart_locked {
        return row;
    }
    row.with_enabled(false)
        .with_detail(strings::get(lang, id::AUTOSTART_TEST_MODE))
}

/// Everything the page depends on.
pub struct PageInput<'a> {
    pub settings: &'a Settings,
    pub lang: Lang,
    /// "Start with Windows": the Run key's state.
    pub autostart: bool,
    /// A test or development launch: the row shows the state and never writes the Run value.
    pub autostart_locked: bool,
    /// Windows' "Transparency effects" are on.
    pub transparency: bool,
    pub hotkey: Hotkey,
    pub hotkey_ok: bool,
    pub hotkey_state: HotkeyState,
    /// Every known device with its row id, in the order the user sees them.
    pub devices: &'a [(RowId, DeviceChoice)],
    /// The rename in progress, if any.
    pub rename: Option<&'a Rename>,
    /// The limit that the last change of the other one moved; its row says so.
    pub adjusted: Option<Adjusted>,
    /// `dd/mm` of the day the battery calibration ends, while it runs.
    pub calibration_ends: Option<String>,
}

/// What a row asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Theme(ThemeChoice),
    IconAnimation(bool),
    Green(i32),
    Red(i32),
    Warn(i32),
    Smart(bool),
    Full(bool),
    DoNotDisturb(bool),
    Autostart(bool),
    Language(LangChoice),
    /// A control of a device row (the id is the row's).
    Device(RowId, DeviceOp),
    /// The shortcut's "Alterar": wait for the next key combination (again: stop waiting).
    HotkeyCapture,
    /// Back to Ctrl+Alt+B.
    HotkeyReset,
    /// Open a web page or a Windows settings page.
    Open(&'static str),
}

/// What a device row's control asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceOp {
    /// Show (true) or hide the device in the panel, the tooltip and the icon's color.
    Show(bool),
    /// Turn the row into a text field.
    Rename,
    Up,
    Down,
}

fn percent(v: u8) -> String {
    format!("{v}%")
}

/// The page for the user's settings, in the app's language.
pub fn build(i: &PageInput<'_>) -> UiSettings {
    let lang = i.lang;
    let t = |key| strings::get(lang, key);
    let s = i.settings;

    let mut appearance = vec![
        SettingsRow::choice(
            row::THEME,
            t(id::SET_THEME),
            &[t(id::THEME_AUTO), t(id::THEME_LIGHT), t(id::THEME_DARK)],
            ThemeChoice::ALL
                .iter()
                .position(|c| *c == s.theme)
                .unwrap_or(0),
        ),
        SettingsRow::toggle(row::ANIMATION, t(id::SET_ICON_ANIM), s.icon_animation)
            .with_detail(t(id::SET_ICON_ANIM_DETAIL)),
    ];
    if !i.transparency {
        appearance.push(SettingsRow::link(
            row::TRANSPARENCY,
            t(id::SET_TRANSPARENCY_HINT),
            t(id::SET_OPEN),
        ));
    }

    let mut green = SettingsRow::stepper(
        row::GREEN,
        t(id::SET_GREEN),
        i32::from(s.green_above),
        (GREEN_MIN, GREEN_MAX, 5),
        &percent(s.green_above),
    );
    let mut red = SettingsRow::stepper(
        row::RED,
        t(id::SET_RED),
        i32::from(s.red_below),
        (RED_MIN, RED_MAX, 5),
        &percent(s.red_below),
    );
    match i.adjusted {
        Some(Adjusted::Green) => green = green.with_detail(t(id::SET_ADJUSTED_GREEN)),
        Some(Adjusted::Red) => red = red.with_detail(t(id::SET_ADJUSTED_RED)),
        None => {}
    }

    let hotkey_text = i.hotkey.text();
    let (hotkey_action, hotkey_detail) = match i.hotkey_state {
        HotkeyState::Capturing => (
            t(id::SET_WAITING).to_string(),
            t(id::SET_HOTKEY_CAPTURE).to_string(),
        ),
        HotkeyState::Refused => (
            t(id::SET_WAITING).to_string(),
            t(id::SET_HOTKEY_REFUSED).to_string(),
        ),
        HotkeyState::InUse => (
            t(id::SET_CHANGE).to_string(),
            t(id::SET_HOTKEY_IN_USE).to_string(),
        ),
        HotkeyState::Idle if i.hotkey_ok => (
            t(id::SET_CHANGE).to_string(),
            t(id::SET_HOTKEY_DETAIL).to_string(),
        ),
        HotkeyState::Idle => (
            t(id::SET_CHANGE).to_string(),
            strings::fmt(lang, id::MENU_HOTKEY_UNAVAILABLE, &[&hotkey_text]),
        ),
    };
    let mut general = vec![
        autostart_row(i, lang, t(id::SET_AUTOSTART)),
        SettingsRow::choice(
            row::LANGUAGE,
            t(id::SET_LANGUAGE),
            &[t(id::LANG_AUTO), t(id::LANG_PT), t(id::LANG_EN)],
            LangChoice::ALL
                .iter()
                .position(|c| *c == s.language)
                .unwrap_or(0),
        ),
        SettingsRow::link(
            row::HOTKEY,
            &strings::fmt(lang, id::SET_HOTKEY_NOW, &[&hotkey_text]),
            &hotkey_action,
        )
        .with_detail(&hotkey_detail),
    ];
    if i.hotkey != Hotkey::DEFAULT {
        general.push(SettingsRow::link(
            row::HOTKEY_RESET,
            &strings::fmt(lang, id::SET_HOTKEY_RESET, &[&Hotkey::DEFAULT.text()]),
            "",
        ));
    }

    let mut about = vec![
        SettingsRow::identity(
            row::ABOUT,
            strings::get(lang, id::PANEL_TITLE),
            &strings::fmt(lang, id::ABOUT_VERSION, &[env!("CARGO_PKG_VERSION")]),
            t(id::WELCOME_CREDIT),
            t(id::ABOUT_AUTHOR),
        ),
        SettingsRow::link(row::GITHUB, t(id::ABOUT_GITHUB), ""),
        SettingsRow::link(row::LICENSE, t(id::ABOUT_LICENSE), "")
            .with_detail(t(id::ABOUT_LICENSE_DETAIL)),
        SettingsRow::note(row::PRIVACY, t(id::ABOUT_PRIVACY)),
    ];
    if let Some(date) = &i.calibration_ends {
        about.push(SettingsRow::note(
            row::CALIBRATION,
            &strings::fmt(lang, id::ABOUT_CALIBRATING, &[date]),
        ));
    }

    let mut page = UiSettings::new(
        t(id::MENU_SETTINGS),
        vec![
            SettingsSection::new(t(id::SEC_APPEARANCE), appearance),
            SettingsSection::new(t(id::SEC_ICON), vec![green, red]),
            SettingsSection::new(
                t(id::SEC_ALERTS),
                vec![
                    SettingsRow::stepper(
                        row::WARN,
                        t(id::SET_LOW),
                        i32::from(s.warn_below),
                        (WARN_MIN, WARN_MAX, 5),
                        &percent(s.warn_below),
                    ),
                    SettingsRow::toggle(row::SMART, t(id::SET_SMART), s.smart_warning)
                        .with_detail(t(id::SET_SMART_DETAIL)),
                    SettingsRow::toggle(row::FULL, t(id::SET_FULL), s.full_notice)
                        .with_detail(t(id::SET_FULL_DETAIL)),
                    SettingsRow::toggle(row::DND, t(id::SET_DND), s.do_not_disturb)
                        .with_detail(t(id::SET_DND_DETAIL)),
                ],
            ),
            SettingsSection::new(t(id::SEC_DEVICES), device_rows(i)),
            SettingsSection::new(t(id::SEC_GENERAL), general),
            SettingsSection::new(t(id::SEC_ABOUT), about),
        ],
    );
    page.capturing = i.hotkey_state.waits();
    if page.capturing {
        page.focus = Some(row::HOTKEY);
    }
    page
}

/// The rows of the Dispositivos section: one per device (a text field while it is renamed), then the help line.
fn device_rows(i: &PageInput<'_>) -> Vec<SettingsRow> {
    let lang = i.lang;
    let t = |key| strings::get(lang, key);
    let last = i.devices.len().saturating_sub(1);
    let mut rows: Vec<SettingsRow> = Vec::with_capacity(i.devices.len() + 1);
    for (n, (rid, d)) in i.devices.iter().enumerate() {
        if let Some(r) = i.rename.filter(|r| r.key == d.key) {
            rows.push(
                SettingsRow::text(
                    *rid,
                    &strings::fmt(lang, id::SET_DEV_RENAME, &[&d.default_name]),
                    &r.text,
                    true,
                )
                .with_selected(r.selected)
                .with_placeholder(&d.default_name)
                .with_detail(&strings::fmt(
                    lang,
                    id::SET_DEV_RENAME_HINT,
                    &[&d.default_name],
                )),
            );
            continue;
        }
        let detail = match (d.connection.is_empty(), d.shown) {
            (_, true) => d.connection.clone(),
            (true, false) => t(id::SET_DEV_HIDDEN).to_string(),
            (false, false) => format!("{} \u{b7} {}", d.connection, t(id::SET_DEV_HIDDEN)),
        };
        let mut row = SettingsRow::device(
            *rid,
            &d.name,
            d.kind.map_or(Glyph::Wireless, glyph_for),
            d.shown,
            (n > 0, n < last),
        );
        if !detail.is_empty() {
            row = row.with_detail(&detail);
        }
        rows.push(row);
    }
    if i.devices.is_empty() {
        rows.push(SettingsRow::note(row::DEVICES_NOTE, t(id::SET_DEV_NONE)));
    } else {
        rows.push(SettingsRow::note(row::DEVICES_NOTE, t(id::SET_DEV_HELP)));
    }
    rows
}

/// What the hit `(id, part)` on `page` asks for; `None` when it asks nothing (a part the row does not use, a
/// stepper at its bound, a row that only shows).
pub fn action(page: &UiSettings, id: RowId, part: u8) -> Option<Change> {
    let kind = &page.row(id)?.kind;
    match (id, kind) {
        (row::THEME, k) => ThemeChoice::ALL
            .get(segment_of(k, part)?)
            .copied()
            .map(Change::Theme),
        (row::LANGUAGE, k) => LangChoice::ALL
            .get(segment_of(k, part)?)
            .copied()
            .map(Change::Language),
        (row::GREEN, k) => stepped(k, part).map(Change::Green),
        (row::RED, k) => stepped(k, part).map(Change::Red),
        (row::WARN, k) => stepped(k, part).map(Change::Warn),
        (row::ANIMATION, RowKind::Toggle { on }) if part == PART_ROW => {
            Some(Change::IconAnimation(!on))
        }
        (row::SMART, RowKind::Toggle { on }) if part == PART_ROW => Some(Change::Smart(!on)),
        (row::FULL, RowKind::Toggle { on }) if part == PART_ROW => Some(Change::Full(!on)),
        (row::DND, RowKind::Toggle { on }) if part == PART_ROW => Some(Change::DoNotDisturb(!on)),
        (row::AUTOSTART, RowKind::Toggle { on }) if part == PART_ROW => {
            Some(Change::Autostart(!on))
        }
        (row::HOTKEY, RowKind::Link { .. }) if part == PART_ROW => Some(Change::HotkeyCapture),
        (row::HOTKEY_RESET, _) if part == PART_ROW => Some(Change::HotkeyReset),
        (
            rid,
            RowKind::Device {
                shown,
                can_up,
                can_down,
                ..
            },
        ) => match part {
            PART_SHOW => Some(Change::Device(rid, DeviceOp::Show(!shown))),
            PART_RENAME => Some(Change::Device(rid, DeviceOp::Rename)),
            PART_UP if *can_up => Some(Change::Device(rid, DeviceOp::Up)),
            PART_DOWN if *can_down => Some(Change::Device(rid, DeviceOp::Down)),
            _ => None,
        },
        (row::TRANSPARENCY, _) if part == PART_ROW => Some(Change::Open(TRANSPARENCY_URL)),
        (row::GITHUB, _) if part == PART_ROW => Some(Change::Open(GITHUB_URL)),
        (row::LICENSE, _) if part == PART_ROW => Some(Change::Open(LICENSE_URL)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_core::types::{DeviceKey, Kind};
    use juicebar_ui::settings::{NavKey, PART_MINUS, PART_PLUS, PART_SEGMENT};

    fn page_for(
        s: &Settings,
        lang: Lang,
        transparency: bool,
        adjusted: Option<Adjusted>,
    ) -> UiSettings {
        build(&PageInput {
            settings: s,
            lang,
            autostart: true,
            autostart_locked: false,
            transparency,
            hotkey: Hotkey::DEFAULT,
            hotkey_ok: true,
            hotkey_state: HotkeyState::Idle,
            devices: &[],
            rename: None,
            adjusted,
            calibration_ends: None,
        })
    }

    fn choice(key: &str, name: &str, kind: Kind, shown: bool) -> DeviceChoice {
        DeviceChoice {
            key: DeviceKey(key.into()),
            name: name.into(),
            default_name: name.into(),
            kind: Some(kind),
            connection: "Receptor".into(),
            shown,
            on: true,
        }
    }

    /// Three devices with the ids the app would hand out; the last one is hidden.
    fn three() -> Vec<(RowId, DeviceChoice)> {
        vec![
            (
                row::DEVICE_BASE,
                choice("k:mouse", "G703", Kind::Mouse, true),
            ),
            (
                row::DEVICE_BASE + 1,
                choice("k:keys", "G915 X", Kind::Keyboard, true),
            ),
            (
                row::DEVICE_BASE + 2,
                choice("k:head", "G733", Kind::Headset, false),
            ),
        ]
    }

    fn page_with(
        devices: &[(RowId, DeviceChoice)],
        rename: Option<&Rename>,
        hotkey: Hotkey,
        state: HotkeyState,
        lang: Lang,
    ) -> UiSettings {
        build(&PageInput {
            settings: &Settings::default(),
            lang,
            autostart: false,
            autostart_locked: false,
            transparency: true,
            hotkey,
            hotkey_ok: true,
            hotkey_state: state,
            devices,
            rename,
            adjusted: None,
            calibration_ends: None,
        })
    }

    fn page() -> UiSettings {
        page_for(&Settings::default(), Lang::PtBr, true, None)
    }

    #[test]
    fn the_sections_come_in_the_owners_order_in_both_languages() {
        let titles = |lang| -> Vec<String> {
            page_for(&Settings::default(), lang, true, None)
                .sections
                .iter()
                .map(|s| s.title.clone())
                .collect()
        };
        assert_eq!(
            titles(Lang::PtBr),
            [
                "Aparência",
                "Ícone",
                "Alertas",
                "Dispositivos",
                "Geral",
                "Sobre"
            ]
        );
        assert_eq!(
            titles(Lang::En),
            [
                "Appearance",
                "Icon",
                "Alerts",
                "Devices",
                "General",
                "About"
            ]
        );
        assert_eq!(page().title, "Configurações");
    }

    #[test]
    fn every_row_has_a_unique_id_and_text_in_both_languages() {
        for lang in [Lang::PtBr, Lang::En] {
            let p = page_for(&Settings::default(), lang, false, Some(Adjusted::Red));
            let mut ids: Vec<RowId> = p.rows().map(|r| r.id).collect();
            let n = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n, "unique ids");
            for r in p.rows() {
                assert!(
                    !r.label.is_empty() && !r.label.contains('\u{2014}'),
                    "{}",
                    r.label
                );
            }
        }
    }

    #[test]
    fn the_page_shows_the_settings_as_they_are() {
        let s = Settings {
            theme: ThemeChoice::Dark,
            icon_animation: false,
            green_above: 70,
            red_below: 35,
            warn_below: 25,
            smart_warning: false,
            full_notice: true,
            do_not_disturb: true,
            language: LangChoice::En,
            ..Settings::default()
        };
        let p = page_for(&s, Lang::En, true, None);
        let kind = |id| p.row(id).unwrap().kind.clone();
        assert_eq!(
            kind(row::THEME),
            RowKind::Choice {
                options: vec!["Auto".into(), "Light".into(), "Dark".into()],
                selected: 2
            }
        );
        assert_eq!(kind(row::ANIMATION), RowKind::Toggle { on: false });
        assert!(
            matches!(kind(row::GREEN), RowKind::Stepper { value: 70, min: 25, max: 95, step: 5, ref text } if text == "70%")
        );
        assert!(
            matches!(kind(row::RED), RowKind::Stepper { value: 35, min: 5, max: 50, step: 5, ref text } if text == "35%")
        );
        assert!(
            matches!(kind(row::WARN), RowKind::Stepper { value: 25, min: 5, max: 30, step: 5, ref text } if text == "25%")
        );
        assert_eq!(kind(row::SMART), RowKind::Toggle { on: false });
        assert_eq!(kind(row::FULL), RowKind::Toggle { on: true });
        assert_eq!(kind(row::DND), RowKind::Toggle { on: true });
        assert_eq!(kind(row::AUTOSTART), RowKind::Toggle { on: true });
        assert!(matches!(
            kind(row::LANGUAGE),
            RowKind::Choice { selected: 2, .. }
        ));
        assert_eq!(
            kind(row::HOTKEY),
            RowKind::Link {
                action: "Change".into()
            }
        );
        assert_eq!(p.row(row::HOTKEY).unwrap().label, "Shortcut: Ctrl+Alt+B");
    }

    #[test]
    fn the_transparency_hint_shows_only_while_windows_transparency_is_off() {
        assert!(
            page_for(&Settings::default(), Lang::PtBr, true, None)
                .row(row::TRANSPARENCY)
                .is_none()
        );
        let p = page_for(&Settings::default(), Lang::PtBr, false, None);
        let r = p.row(row::TRANSPARENCY).unwrap();
        assert_eq!(
            r.label,
            "Ative os efeitos de transparência do Windows para o fundo translúcido"
        );
        assert_eq!(
            r.kind,
            RowKind::Link {
                action: "Abrir".into()
            }
        );
        assert_eq!(
            action(&p, row::TRANSPARENCY, PART_ROW),
            Some(Change::Open("ms-settings:personalization-colors"))
        );
    }

    #[test]
    fn the_about_block_carries_the_credit_the_version_and_the_links() {
        let p = page();
        let RowKind::Identity {
            name,
            version,
            credit,
            author,
        } = &p.row(row::ABOUT).unwrap().kind
        else {
            panic!("an identity row");
        };
        assert_eq!(name, "JuiceBar");
        assert_eq!(version, &format!("Versão {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(credit, "Idealizado por Claudio Bertini");
        assert_eq!(author, "devBertini");
        assert_eq!(p.row(row::GITHUB).unwrap().label, "Código no GitHub");
        assert_eq!(p.row(row::LICENSE).unwrap().label, "Licença GPL-3.0");
        assert_eq!(
            p.row(row::PRIVACY).unwrap().label,
            "Nada sai deste computador: sem internet, sem telemetria"
        );
        assert_eq!(
            action(&p, row::GITHUB, PART_ROW),
            Some(Change::Open("https://github.com/devBertini/juicebar"))
        );
        assert_eq!(
            action(&p, row::LICENSE, PART_ROW),
            Some(Change::Open("https://www.gnu.org/licenses/gpl-3.0.html"))
        );
        let en = page_for(&Settings::default(), Lang::En, true, None);
        assert!(
            matches!(&en.row(row::ABOUT).unwrap().kind, RowKind::Identity { credit, .. } if credit == "Created by Claudio Bertini")
        );
    }

    #[test]
    fn the_about_section_says_until_when_the_batteries_are_calibrated_and_only_then() {
        let with = |lang, ends: Option<&str>| {
            build(&PageInput {
                settings: &Settings::default(),
                lang,
                autostart: true,
                autostart_locked: false,
                transparency: true,
                hotkey: Hotkey::DEFAULT,
                hotkey_ok: true,
                hotkey_state: HotkeyState::Idle,
                devices: &[],
                rename: None,
                adjusted: None,
                calibration_ends: ends.map(str::to_string),
            })
        };
        assert!(with(Lang::PtBr, None).row(row::CALIBRATION).is_none());
        assert_eq!(
            with(Lang::PtBr, Some("09/11"))
                .row(row::CALIBRATION)
                .unwrap()
                .label,
            "Calibrando as baterias até 09/11"
        );
        assert_eq!(
            with(Lang::En, Some("09/11"))
                .row(row::CALIBRATION)
                .unwrap()
                .label,
            "Calibrating the batteries until 09/11"
        );
        let p = with(Lang::PtBr, Some("09/11"));
        assert_eq!(action(&p, row::CALIBRATION, PART_ROW), None);
    }

    #[test]
    fn a_segment_click_picks_the_theme_and_the_language() {
        let p = page();
        assert_eq!(
            action(&p, row::THEME, PART_SEGMENT),
            Some(Change::Theme(ThemeChoice::Auto))
        );
        assert_eq!(
            action(&p, row::THEME, PART_SEGMENT + 1),
            Some(Change::Theme(ThemeChoice::Light))
        );
        assert_eq!(
            action(&p, row::THEME, PART_SEGMENT + 2),
            Some(Change::Theme(ThemeChoice::Dark))
        );
        assert_eq!(action(&p, row::THEME, PART_SEGMENT + 3), None);
        assert_eq!(
            action(&p, row::LANGUAGE, PART_SEGMENT + 1),
            Some(Change::Language(LangChoice::Pt))
        );
        assert_eq!(
            action(&p, row::LANGUAGE, PART_SEGMENT + 2),
            Some(Change::Language(LangChoice::En))
        );
    }

    #[test]
    fn a_toggle_click_asks_for_the_opposite() {
        let mut s = Settings::default();
        let p = page_for(&s, Lang::PtBr, true, None);
        assert_eq!(
            action(&p, row::ANIMATION, PART_ROW),
            Some(Change::IconAnimation(false))
        );
        assert_eq!(action(&p, row::SMART, PART_ROW), Some(Change::Smart(false)));
        assert_eq!(action(&p, row::FULL, PART_ROW), Some(Change::Full(false)));
        assert_eq!(
            action(&p, row::DND, PART_ROW),
            Some(Change::DoNotDisturb(true))
        );
        assert_eq!(
            action(&p, row::AUTOSTART, PART_ROW),
            Some(Change::Autostart(false))
        );
        s.do_not_disturb = true;
        let p = page_for(&s, Lang::PtBr, true, None);
        assert_eq!(
            action(&p, row::DND, PART_ROW),
            Some(Change::DoNotDisturb(false))
        );
        // A toggle has no other part.
        assert_eq!(action(&p, row::DND, PART_PLUS), None);
    }

    #[test]
    fn the_steppers_ask_for_the_next_value_and_stop_at_their_bounds() {
        let p = page();
        assert_eq!(action(&p, row::GREEN, PART_PLUS), Some(Change::Green(55)));
        assert_eq!(action(&p, row::GREEN, PART_MINUS), Some(Change::Green(45)));
        assert_eq!(action(&p, row::RED, PART_PLUS), Some(Change::Red(25)));
        assert_eq!(action(&p, row::WARN, PART_MINUS), Some(Change::Warn(5)));
        assert_eq!(action(&p, row::WARN, PART_ROW), None);
        let low = Settings {
            warn_below: 5,
            ..Settings::default()
        };
        assert_eq!(
            action(
                &page_for(&low, Lang::PtBr, true, None),
                row::WARN,
                PART_MINUS
            ),
            None
        );
        let high = Settings {
            warn_below: 30,
            ..Settings::default()
        };
        assert_eq!(
            action(
                &page_for(&high, Lang::PtBr, true, None),
                row::WARN,
                PART_PLUS
            ),
            None
        );
    }

    #[test]
    fn the_row_that_was_adjusted_says_so() {
        let detail = |p: &UiSettings, id| p.row(id).unwrap().detail.clone();
        let p = page_for(&Settings::default(), Lang::PtBr, true, Some(Adjusted::Red));
        assert_eq!(
            detail(&p, row::RED).as_deref(),
            Some("Ajustado para ficar abaixo do limite verde")
        );
        assert_eq!(detail(&p, row::GREEN), None);
        let p = page_for(&Settings::default(), Lang::En, true, Some(Adjusted::Green));
        assert_eq!(
            detail(&p, row::GREEN).as_deref(),
            Some("Adjusted to stay above the red limit")
        );
        assert_eq!(detail(&p, row::RED), None);
    }

    #[test]
    fn a_shortcut_that_could_not_register_says_so_in_its_row() {
        let p = build(&PageInput {
            settings: &Settings::default(),
            lang: Lang::PtBr,
            autostart: false,
            autostart_locked: false,
            transparency: true,
            hotkey: Hotkey::DEFAULT,
            hotkey_ok: false,
            hotkey_state: HotkeyState::Idle,
            devices: &[],
            rename: None,
            adjusted: None,
            calibration_ends: None,
        });
        assert_eq!(
            p.row(row::HOTKEY).unwrap().detail.as_deref(),
            Some("Atalho Ctrl+Alt+B indisponível")
        );
        assert_eq!(
            action(&p, row::HOTKEY, PART_ROW),
            Some(Change::HotkeyCapture),
            "and it can be changed"
        );
    }

    #[test]
    fn the_shortcut_row_shows_the_combination_and_waits_for_keys_on_a_click() {
        let hk = Hotkey::parse("Win+Alt+F11").unwrap();
        let p = page_with(&[], None, hk, HotkeyState::Idle, Lang::PtBr);
        let r = p.row(row::HOTKEY).unwrap();
        assert_eq!(r.label, "Atalho: Alt+Win+F11");
        assert_eq!(
            r.kind,
            RowKind::Link {
                action: "Alterar".into()
            }
        );
        assert_eq!(
            r.detail.as_deref(),
            Some("Abre o JuiceBar de qualquer lugar")
        );
        assert!(!p.capturing && p.focus.is_none());
        assert_eq!(
            action(&p, row::HOTKEY, PART_ROW),
            Some(Change::HotkeyCapture)
        );
    }

    #[test]
    fn while_it_waits_the_page_says_how_to_finish_and_takes_the_keys() {
        for (lang, waiting, how) in [
            (
                Lang::PtBr,
                "Aguardando...",
                "Segure dois entre Ctrl, Alt, Shift ou Win e aperte uma letra, número ou tecla F. Esc cancela.",
            ),
            (
                Lang::En,
                "Waiting...",
                "Hold two of Ctrl, Alt, Shift or Win and press a letter, digit or F key. Esc cancels.",
            ),
        ] {
            let p = page_with(&[], None, Hotkey::DEFAULT, HotkeyState::Capturing, lang);
            assert!(p.capturing, "the flyout sends the combination");
            assert_eq!(p.focus, Some(row::HOTKEY), "the row shows it waits");
            let r = p.row(row::HOTKEY).unwrap();
            assert_eq!(
                r.kind,
                RowKind::Link {
                    action: waiting.into()
                }
            );
            assert_eq!(r.detail.as_deref(), Some(how));
        }
    }

    #[test]
    fn a_refused_combination_says_why_and_the_wait_goes_on() {
        for (lang, why) in [
            (
                Lang::PtBr,
                "Use pelo menos dois modificadores, como Ctrl+Alt",
            ),
            (Lang::En, "Use at least two modifiers, like Ctrl+Alt"),
        ] {
            let p = page_with(&[], None, Hotkey::DEFAULT, HotkeyState::Refused, lang);
            assert!(p.capturing, "still waiting for keys");
            assert_eq!(p.focus, Some(row::HOTKEY));
            assert_eq!(p.row(row::HOTKEY).unwrap().detail.as_deref(), Some(why));
        }
        assert!(HotkeyState::Capturing.waits() && HotkeyState::Refused.waits());
        assert!(!HotkeyState::Idle.waits() && !HotkeyState::InUse.waits());
    }

    #[test]
    fn a_taken_shortcut_says_so_in_the_owners_words() {
        let p = page_with(&[], None, Hotkey::DEFAULT, HotkeyState::InUse, Lang::PtBr);
        assert_eq!(
            p.row(row::HOTKEY).unwrap().detail.as_deref(),
            Some("Esse atalho já está em uso")
        );
        assert!(!p.capturing);
        let en = page_with(&[], None, Hotkey::DEFAULT, HotkeyState::InUse, Lang::En);
        assert_eq!(
            en.row(row::HOTKEY).unwrap().detail.as_deref(),
            Some("That shortcut is already in use")
        );
    }

    #[test]
    fn restoring_the_default_shows_only_for_another_shortcut() {
        let idle = |hk| page_with(&[], None, hk, HotkeyState::Idle, Lang::PtBr);
        assert!(idle(Hotkey::DEFAULT).row(row::HOTKEY_RESET).is_none());
        let p = idle(Hotkey::parse("Ctrl+Alt+J").unwrap());
        let r = p.row(row::HOTKEY_RESET).unwrap();
        assert_eq!(r.label, "Restaurar padrão (Ctrl+Alt+B)");
        assert_eq!(
            action(&p, row::HOTKEY_RESET, PART_ROW),
            Some(Change::HotkeyReset)
        );
        // Both are in the Geral card, the reset right under the shortcut.
        let general = p.sections.iter().find(|s| s.title == "Geral").unwrap();
        let ids: Vec<RowId> = general.rows.iter().map(|r| r.id).collect();
        assert_eq!(ids[ids.len() - 2..], [row::HOTKEY, row::HOTKEY_RESET]);
    }

    #[test]
    fn the_devices_section_lists_every_device_with_glyph_name_and_connection() {
        let p = page_with(
            &three(),
            None,
            Hotkey::DEFAULT,
            HotkeyState::Idle,
            Lang::PtBr,
        );
        let sec = p
            .sections
            .iter()
            .find(|s| s.title == "Dispositivos")
            .unwrap();
        // Three devices and the help line.
        assert_eq!(sec.rows.len(), 4);
        let r = &sec.rows[0];
        assert_eq!(r.label, "G703");
        assert_eq!(r.detail.as_deref(), Some("Receptor"));
        assert_eq!(
            r.kind,
            RowKind::Device {
                glyph: Glyph::Mouse,
                shown: true,
                can_up: false,
                can_down: true
            }
        );
        assert!(matches!(
            sec.rows[1].kind,
            RowKind::Device {
                glyph: Glyph::Keyboard,
                can_up: true,
                can_down: true,
                ..
            }
        ));
        // The hidden one says so and has no way further down.
        assert_eq!(
            sec.rows[2].detail.as_deref(),
            Some("Receptor \u{b7} oculto")
        );
        assert_eq!(
            sec.rows[2].kind,
            RowKind::Device {
                glyph: Glyph::Headset,
                shown: false,
                can_up: true,
                can_down: false
            }
        );
        assert_eq!(
            sec.rows[3].label,
            "Chave: mostrar no painel. Lápis: renomear. Setas: mudar a ordem."
        );
        let en = page_with(&three(), None, Hotkey::DEFAULT, HotkeyState::Idle, Lang::En);
        assert_eq!(
            en.row(row::DEVICE_BASE + 2).unwrap().detail.as_deref(),
            Some("Receptor \u{b7} hidden")
        );
    }

    #[test]
    fn a_device_never_seen_this_run_shows_only_that_it_is_hidden() {
        let mut d = choice("k:old", "G502", Kind::Mouse, false);
        d.kind = None;
        d.connection = String::new();
        let list = vec![(row::DEVICE_BASE, d)];
        let p = page_with(&list, None, Hotkey::DEFAULT, HotkeyState::Idle, Lang::PtBr);
        let r = p.row(row::DEVICE_BASE).unwrap();
        assert_eq!(r.detail.as_deref(), Some("oculto"));
        assert!(matches!(
            r.kind,
            RowKind::Device {
                glyph: Glyph::Wireless,
                ..
            }
        ));
    }

    #[test]
    fn no_device_at_all_says_so() {
        let p = page_with(&[], None, Hotkey::DEFAULT, HotkeyState::Idle, Lang::PtBr);
        let sec = p
            .sections
            .iter()
            .find(|s| s.title == "Dispositivos")
            .unwrap();
        assert_eq!(sec.rows.len(), 1);
        assert_eq!(sec.rows[0].label, "Nenhum dispositivo visto ainda");
    }

    #[test]
    fn each_control_of_a_device_row_asks_for_its_own_change() {
        let p = page_with(
            &three(),
            None,
            Hotkey::DEFAULT,
            HotkeyState::Idle,
            Lang::PtBr,
        );
        let (mouse, keys, head) = (row::DEVICE_BASE, row::DEVICE_BASE + 1, row::DEVICE_BASE + 2);
        assert_eq!(
            action(&p, mouse, PART_SHOW),
            Some(Change::Device(mouse, DeviceOp::Show(false)))
        );
        assert_eq!(
            action(&p, head, PART_SHOW),
            Some(Change::Device(head, DeviceOp::Show(true))),
            "a hidden device is shown again"
        );
        assert_eq!(
            action(&p, keys, PART_RENAME),
            Some(Change::Device(keys, DeviceOp::Rename))
        );
        assert_eq!(
            action(&p, keys, PART_UP),
            Some(Change::Device(keys, DeviceOp::Up))
        );
        assert_eq!(
            action(&p, keys, PART_DOWN),
            Some(Change::Device(keys, DeviceOp::Down))
        );
        // The first cannot go up, the last cannot go down, a click on the rest of the row asks nothing.
        assert_eq!(action(&p, mouse, PART_UP), None);
        assert_eq!(action(&p, head, PART_DOWN), None);
        assert_eq!(action(&p, keys, PART_ROW), None);
    }

    #[test]
    fn the_keyboard_reaches_each_device_row_and_asks_the_same() {
        let mut p = page_with(
            &three(),
            None,
            Hotkey::DEFAULT,
            HotkeyState::Idle,
            Lang::PtBr,
        );
        let keys = row::DEVICE_BASE + 1;
        assert!(p.focusable().contains(&keys));
        p.focus = Some(keys);
        let ask = |p: &UiSettings, k| {
            let (id, part) = p.activate(k).unwrap();
            action(p, id, part)
        };
        assert_eq!(
            ask(&p, NavKey::Space),
            Some(Change::Device(keys, DeviceOp::Show(false)))
        );
        assert_eq!(
            ask(&p, NavKey::Enter),
            Some(Change::Device(keys, DeviceOp::Rename))
        );
        assert_eq!(
            ask(&p, NavKey::Left),
            Some(Change::Device(keys, DeviceOp::Up))
        );
        assert_eq!(
            ask(&p, NavKey::Right),
            Some(Change::Device(keys, DeviceOp::Down))
        );
        // At the ends the arrows do nothing.
        p.focus = Some(row::DEVICE_BASE);
        assert_eq!(p.activate(NavKey::Left), None);
        p.focus = Some(row::DEVICE_BASE + 2);
        assert_eq!(p.activate(NavKey::Right), None);
    }

    #[test]
    fn the_row_being_renamed_is_a_text_field_with_the_name_selected() {
        let mut r = Rename::new(DeviceKey("k:keys".into()), "G915 X", "G915 X");
        let p = page_with(
            &three(),
            Some(&r),
            Hotkey::DEFAULT,
            HotkeyState::Idle,
            Lang::PtBr,
        );
        let keys = row::DEVICE_BASE + 1;
        let row = p.row(keys).unwrap();
        assert_eq!(row.label, "Nome para G915 X");
        assert_eq!(
            row.detail.as_deref(),
            Some("Enter salva, Esc cancela. Vazio volta para G915 X.")
        );
        assert_eq!(
            row.kind,
            RowKind::Text {
                value: "G915 X".into(),
                placeholder: "G915 X".into(),
                editing: true,
                selected: true
            }
        );
        assert!(p.editing(), "the keys go to the field");
        // The other devices keep their controls, and a click in the field asks nothing.
        assert!(matches!(
            p.row(row::DEVICE_BASE).unwrap().kind,
            RowKind::Device { .. }
        ));
        assert_eq!(action(&p, keys, PART_ROW), None);
        // Once typed over, the selection is gone.
        r.key(juicebar_ui::settings::EditKey::Char('T'));
        let p = page_with(
            &three(),
            Some(&r),
            Hotkey::DEFAULT,
            HotkeyState::Idle,
            Lang::En,
        );
        assert_eq!(
            p.row(keys).unwrap().kind,
            RowKind::Text {
                value: "T".into(),
                placeholder: "G915 X".into(),
                editing: true,
                selected: false
            }
        );
        assert_eq!(p.row(keys).unwrap().label, "Name for G915 X");
    }

    #[test]
    fn device_row_ids_never_meet_the_fixed_ones() {
        let mut ids: Vec<RowId> = page_with(
            &three(),
            None,
            Hotkey::parse("Ctrl+Alt+J").unwrap(),
            HotkeyState::Idle,
            Lang::PtBr,
        )
        .rows()
        .map(|r| r.id)
        .collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
        const { assert!(row::PRIVACY < row::DEVICE_BASE && row::DEVICES_NOTE < row::DEVICE_BASE) };
    }

    #[test]
    fn rows_that_only_show_ask_nothing_and_the_keyboard_reaches_every_control() {
        let mut p = page();
        assert_eq!(action(&p, row::ABOUT, PART_ROW), None);
        assert_eq!(action(&p, row::PRIVACY, PART_ROW), None);
        assert_eq!(action(&p, 999, PART_ROW), None);
        // Tab visits Tema, animation, the three steppers, three toggles, autostart, language, the shortcut, then the
        // two links.
        let mut order = Vec::new();
        for _ in 0..p.focusable().len() {
            order.push(p.move_focus(true).unwrap());
        }
        assert_eq!(
            order,
            vec![
                row::THEME,
                row::ANIMATION,
                row::GREEN,
                row::RED,
                row::WARN,
                row::SMART,
                row::FULL,
                row::DND,
                row::AUTOSTART,
                row::LANGUAGE,
                row::HOTKEY,
                row::GITHUB,
                row::LICENSE
            ]
        );
        // The arrow keys on the focused stepper ask for a step.
        p.focus = Some(row::GREEN);
        let (id, part) = p.activate(NavKey::Right).unwrap();
        assert_eq!(action(&p, id, part), Some(Change::Green(55)));
        p.focus = Some(row::THEME);
        let (id, part) = p.activate(NavKey::Right).unwrap();
        assert_eq!(
            action(&p, id, part),
            Some(Change::Theme(ThemeChoice::Light))
        );
        p.focus = Some(row::GITHUB);
        let (id, part) = p.activate(NavKey::Enter).unwrap();
        assert_eq!(action(&p, id, part), Some(Change::Open(GITHUB_URL)));
    }

    #[test]
    fn a_test_launch_shows_the_autostart_row_read_only_with_the_reason() {
        let page = |locked: bool, lang: Lang| {
            build(&PageInput {
                settings: &Settings::default(),
                lang,
                autostart: true,
                autostart_locked: locked,
                transparency: true,
                hotkey: Hotkey::DEFAULT,
                hotkey_ok: true,
                hotkey_state: HotkeyState::Idle,
                devices: &[],
                rename: None,
                adjusted: None,
                calibration_ends: None,
            })
        };
        let plain = page(false, Lang::PtBr);
        let r = plain.row(row::AUTOSTART).unwrap();
        assert!(r.enabled);
        assert_eq!(r.detail, None);
        let locked = page(true, Lang::PtBr);
        let r = locked.row(row::AUTOSTART).unwrap();
        assert!(!r.enabled);
        assert!(matches!(r.kind, RowKind::Toggle { on: true }));
        assert_eq!(
            r.detail.as_deref(),
            Some("indisponível neste modo de teste")
        );
        let en = page(true, Lang::En);
        assert_eq!(
            en.row(row::AUTOSTART).unwrap().detail.as_deref(),
            Some("unavailable in this test mode")
        );
    }
}
