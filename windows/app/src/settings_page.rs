// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The Settings page's content: which sections and rows the flyout shows for the user's settings, and what a
//! click or key on a row asks the app to do. Pure: no Win32, so it is unit tested.
//!
//! To add a section (the devices, for one), build its rows with new ids in `build` and map their hits in
//! `action`; the flyout's drawing, layout, keyboard and scrolling need nothing.

use juicebar_core::strings::{self, Lang, id};
use juicebar_ui::settings::{
    PART_ROW, RowId, RowKind, SettingsRow, SettingsSection, UiSettings, segment_of, stepped,
};

use crate::settings::{
    Adjusted, GREEN_MAX, GREEN_MIN, LangChoice, RED_MAX, RED_MIN, Settings, ThemeChoice, WARN_MAX,
    WARN_MIN,
};

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
}

pub const GITHUB_URL: &str = "https://github.com/devBertini/juicebar";
pub const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
pub const TRANSPARENCY_URL: &str = "ms-settings:personalization-colors";

/// Everything the page depends on.
pub struct PageInput<'a> {
    pub settings: &'a Settings,
    pub lang: Lang,
    /// "Start with Windows": the Run key's state.
    pub autostart: bool,
    /// Windows' "Transparency effects" are on.
    pub transparency: bool,
    pub hotkey: &'a str,
    pub hotkey_ok: bool,
    /// The limit that the last change of the other one moved; its row says so.
    pub adjusted: Option<Adjusted>,
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
    /// Open a web page or a Windows settings page.
    Open(&'static str),
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

    let hotkey_detail = if i.hotkey_ok {
        t(id::SET_HOTKEY_DETAIL).to_string()
    } else {
        strings::fmt(lang, id::MENU_HOTKEY_UNAVAILABLE, &[i.hotkey])
    };

    UiSettings::new(
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
            SettingsSection::new(
                t(id::SEC_GENERAL),
                vec![
                    SettingsRow::toggle(row::AUTOSTART, t(id::SET_AUTOSTART), i.autostart),
                    SettingsRow::choice(
                        row::LANGUAGE,
                        t(id::SET_LANGUAGE),
                        &[t(id::LANG_AUTO), t(id::LANG_PT), t(id::LANG_EN)],
                        LangChoice::ALL
                            .iter()
                            .position(|c| *c == s.language)
                            .unwrap_or(0),
                    ),
                    SettingsRow::value(row::HOTKEY, t(id::SET_HOTKEY), i.hotkey)
                        .with_detail(&hotkey_detail),
                ],
            ),
            SettingsSection::new(
                t(id::SEC_ABOUT),
                vec![
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
                ],
            ),
        ],
    )
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
        (row::TRANSPARENCY, _) if part == PART_ROW => Some(Change::Open(TRANSPARENCY_URL)),
        (row::GITHUB, _) if part == PART_ROW => Some(Change::Open(GITHUB_URL)),
        (row::LICENSE, _) if part == PART_ROW => Some(Change::Open(LICENSE_URL)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            transparency,
            hotkey: "Ctrl+Alt+B",
            hotkey_ok: true,
            adjusted,
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
            ["Aparência", "Ícone", "Alertas", "Geral", "Sobre"]
        );
        assert_eq!(
            titles(Lang::En),
            ["Appearance", "Icon", "Alerts", "General", "About"]
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
            RowKind::Value {
                text: "Ctrl+Alt+B".into()
            }
        );
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
            "Sem rede, sem telemetria: tudo fica neste computador"
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
            transparency: true,
            hotkey: "Ctrl+Alt+B",
            hotkey_ok: false,
            adjusted: None,
        });
        assert_eq!(
            p.row(row::HOTKEY).unwrap().detail.as_deref(),
            Some("Atalho Ctrl+Alt+B indisponível")
        );
        assert_eq!(
            action(&p, row::HOTKEY, PART_ROW),
            None,
            "part 2 makes it editable"
        );
    }

    #[test]
    fn rows_that_only_show_ask_nothing_and_the_keyboard_reaches_every_control() {
        let mut p = page();
        assert_eq!(action(&p, row::ABOUT, PART_ROW), None);
        assert_eq!(action(&p, row::PRIVACY, PART_ROW), None);
        assert_eq!(action(&p, 999, PART_ROW), None);
        // Tab visits Tema, animation, the three steppers, three toggles, autostart, language, then the two links.
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
}
