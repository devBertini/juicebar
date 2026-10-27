// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Every visible text, in English (the source) and Brazilian Portuguese, from `spec/strings.tsv`.
//! The language is a tag the shell passes in: `"pt-BR"` selects Portuguese, anything else English.

use std::collections::HashMap;
use std::sync::OnceLock;

const TABLE: &str = include_str!("../../../spec/strings.tsv");

/// The two languages JuiceBar ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    PtBr,
}

impl Lang {
    /// `"pt-BR"` (any case, `-` or `_`) is Brazilian Portuguese; every other tag is English.
    pub fn from_tag(tag: &str) -> Lang {
        let t = tag.trim().replace('_', "-");
        if t.eq_ignore_ascii_case("pt-BR") {
            Lang::PtBr
        } else {
            Lang::En
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtBr => "pt-BR",
        }
    }
}

macro_rules! ids {
    ($($name:ident = $text:expr),* $(,)?) => {
        /// The id of every text (the first column of `spec/strings.tsv`).
        pub mod id {
            $(pub const $name: &str = $text;)*
        }
        /// Every id, to prove the table and the code agree.
        pub const ALL: &[&str] = &[$($text),*];
    };
}

ids! {
    HOURS_MIN = "hours_min",
    HOURS_ONE_HOUR = "hours_one_hour",
    HOURS_HALF = "hours_half",
    HOURS_WHOLE = "hours_whole",
    DAYS_ONE = "days_one",
    DAYS_MANY = "days_many",
    OF_USE = "of_use",
    LABEL_INITIAL = "label_initial",
    ETA_TODAY = "eta_today",
    ETA_TOMORROW = "eta_tomorrow",
    ETA_WEEKDAY = "eta_weekday",
    WD_0 = "wd_0",
    WD_1 = "wd_1",
    WD_2 = "wd_2",
    WD_3 = "wd_3",
    WD_4 = "wd_4",
    WD_5 = "wd_5",
    WD_6 = "wd_6",
    FULL_IN_MIN = "full_in_min",
    FULL_IN_H = "full_in_h",
    FULL_IN_HMIN = "full_in_hmin",
    FULL_AT = "full_at",
    USAGE_MIN = "usage_min",
    USAGE_H = "usage_h",
    USAGE_HMIN = "usage_hmin",
    USAGE_TODAY = "usage_today",
    HEALTH_LEARNING = "health_learning",
    HEALTH_PERCENT = "health_percent",
    HEALTH_LINE = "health_line",
    SMART_WARNING = "smart_warning",
    LOW_WARNING = "low_warning",
    NOTICE_FULL_TITLE = "notice_full_title",
    NOTICE_FULL_BODY = "notice_full_body",
    CONN_RECEIVER = "conn_receiver",
    CONN_DONGLE = "conn_dongle",
    CONN_BLUETOOTH = "conn_bluetooth",
    CONN_USB = "conn_usb",
    STATUS_CHARGING = "status_charging",
    STATUS_FULL = "status_full",
    STATUS_NO_READING = "status_no_reading",
    STATUS_OFF = "status_off",
    STATUS_OFF_AT = "status_off_at",
    COARSE_FULL = "coarse_full",
    COARSE_GOOD = "coarse_good",
    COARSE_LOW = "coarse_low",
    COARSE_CRITICAL = "coarse_critical",
    SECTION_OFF = "section_off",
    PANEL_TITLE = "panel_title",
    PANEL_ON_COUNT = "panel_on_count",
    RGB_SAVER = "rgb_saver",
    TOOLTIP_LINE = "tooltip_line",
    TOOLTIP_LINE_STATUS = "tooltip_line_status",
    TOOLTIP_OFF = "tooltip_off",
    TOOLTIP_EMPTY = "tooltip_empty",
    TOOLTIP_PLAIN = "tooltip_plain",
    MENU_OPEN = "menu_open",
    MENU_HOTKEY_UNAVAILABLE = "menu_hotkey_unavailable",
    MENU_QUIT = "menu_quit",
    EMPTY_TITLE = "empty_title",
    EMPTY_HINT = "empty_hint",
    CHART_LAST_7_DAYS = "chart_last_7_days",
    CHART_FORECAST = "chart_forecast",
    STAT_TODAY = "stat_today",
    STAT_OF_USE = "stat_of_use",
    STAT_HEALTH = "stat_health",
    RGB_SAVER_INFO = "rgb_saver_info",
    LIGHTING_REAPPLIED = "lighting_reapplied",
    CHART_USAGE_BY_DAY = "chart_usage_by_day",
    WELCOME_TITLE = "welcome_title",
    WELCOME_BODY = "welcome_body",
    WELCOME_TRAY_TITLE = "welcome_tray_title",
    WELCOME_TRAY_STEPS = "welcome_tray_steps",
    WELCOME_CREDIT = "welcome_credit",
    WELCOME_START = "welcome_start",
    MENU_SETTINGS = "menu_settings",
    SEC_APPEARANCE = "sec_appearance",
    SET_THEME = "set_theme",
    THEME_AUTO = "theme_auto",
    THEME_LIGHT = "theme_light",
    THEME_DARK = "theme_dark",
    SET_ICON_ANIM = "set_icon_anim",
    SET_ICON_ANIM_DETAIL = "set_icon_anim_detail",
    SET_TRANSPARENCY_HINT = "set_transparency_hint",
    SET_OPEN = "set_open",
    SEC_ICON = "sec_icon",
    SET_GREEN = "set_green",
    SET_RED = "set_red",
    SET_ADJUSTED_GREEN = "set_adjusted_green",
    SET_ADJUSTED_RED = "set_adjusted_red",
    SEC_ALERTS = "sec_alerts",
    SET_LOW = "set_low",
    SET_SMART = "set_smart",
    SET_SMART_DETAIL = "set_smart_detail",
    SET_FULL = "set_full",
    SET_FULL_DETAIL = "set_full_detail",
    SET_DND = "set_dnd",
    SET_DND_DETAIL = "set_dnd_detail",
    SEC_GENERAL = "sec_general",
    SET_AUTOSTART = "set_autostart",
    SET_LANGUAGE = "set_language",
    LANG_AUTO = "lang_auto",
    LANG_PT = "lang_pt",
    LANG_EN = "lang_en",
    SET_HOTKEY = "set_hotkey",
    SET_HOTKEY_DETAIL = "set_hotkey_detail",
    SEC_ABOUT = "sec_about",
    ABOUT_VERSION = "about_version",
    ABOUT_AUTHOR = "about_author",
    ABOUT_GITHUB = "about_github",
    ABOUT_LICENSE = "about_license",
    ABOUT_LICENSE_DETAIL = "about_license_detail",
    ABOUT_PRIVACY = "about_privacy"
}

type Table = HashMap<&'static str, (&'static str, &'static str)>;

fn table() -> &'static Table {
    static CELL: OnceLock<Table> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut map = HashMap::new();
        let mut header_seen = false;
        for line in TABLE.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if !header_seen {
                header_seen = true;
                continue;
            }
            let mut cells = line.split('\t');
            if let (Some(id), Some(en), Some(pt)) = (cells.next(), cells.next(), cells.next()) {
                map.insert(id, (en, pt));
            }
        }
        map
    })
}

/// The template of an id in a language; an empty string for an id the table lacks (the tests prove none does).
pub fn get(lang: Lang, id: &str) -> &'static str {
    match table().get(id) {
        Some((en, pt)) => match lang {
            Lang::En => en,
            Lang::PtBr => pt,
        },
        None => "",
    }
}

/// The text of an id with `{0}`, `{1}`, ... replaced by `args`.
pub fn fmt(lang: Lang, id: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = get(lang, id);
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let bytes = tail.as_bytes();
        match (bytes.get(1), bytes.get(2)) {
            (Some(d @ b'0'..=b'9'), Some(b'}')) => {
                if let Some(arg) = args.get(usize::from(d - b'0')) {
                    out.push_str(arg);
                }
                rest = &tail[3..];
            }
            _ => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `fmt` with integer arguments, the common case.
pub fn fmt_n(lang: Lang, id: &str, args: &[i64]) -> String {
    let texts: Vec<String> = args.iter().map(|n| n.to_string()).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    fmt(lang, id, &refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags() {
        assert_eq!(Lang::from_tag("pt-BR"), Lang::PtBr);
        assert_eq!(Lang::from_tag("pt_br"), Lang::PtBr);
        assert_eq!(Lang::from_tag("en-US"), Lang::En);
        assert_eq!(Lang::from_tag("fr"), Lang::En);
        assert_eq!(Lang::from_tag(""), Lang::En);
    }

    #[test]
    fn table_and_ids_agree() {
        let mut in_table: Vec<&str> = table().keys().copied().collect();
        in_table.sort_unstable();
        let mut in_code: Vec<&str> = ALL.to_vec();
        in_code.sort_unstable();
        assert_eq!(in_table, in_code);
        let rows = TABLE
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .count()
            - 1;
        assert_eq!(ALL.len(), rows, "duplicate id in the table");
    }

    #[test]
    fn every_text_exists_in_both_languages_with_the_same_placeholders() {
        fn slots(s: &str) -> Vec<char> {
            let mut v: Vec<char> = s
                .match_indices('{')
                .filter_map(|(i, _)| s[i + 1..].chars().next())
                .collect();
            v.sort_unstable();
            v
        }
        for id in ALL {
            let en = get(Lang::En, id);
            let pt = get(Lang::PtBr, id);
            assert!(!en.is_empty() && !pt.is_empty(), "{id} is empty");
            assert_eq!(slots(en), slots(pt), "{id}: placeholders differ");
            assert!(
                !en.contains('\u{2014}') && !pt.contains('\u{2014}'),
                "{id}: em dash"
            );
        }
    }

    #[test]
    fn formats_in_both_languages() {
        assert_eq!(
            fmt(Lang::PtBr, id::ETA_TODAY, &["17:40"]),
            "acaba hoje ~17:40"
        );
        assert_eq!(
            fmt(Lang::En, id::ETA_TODAY, &["17:40"]),
            "runs out today ~17:40"
        );
        assert_eq!(
            fmt(Lang::En, id::FULL_IN_HMIN, &["1", "10"]),
            "full in ~1 h 10 min"
        );
        assert_eq!(
            fmt(Lang::PtBr, id::FULL_IN_HMIN, &["1", "10"]),
            "cheio em ~1 h 10 min"
        );
        assert_eq!(fmt_n(Lang::PtBr, id::HOURS_HALF, &[2]), "~2,5 h");
        assert_eq!(fmt_n(Lang::En, id::HOURS_HALF, &[2]), "~2.5 h");
        assert_eq!(fmt(Lang::En, id::OF_USE, &["~3 h"]), "~3 h of use");
        assert_eq!(
            fmt(Lang::PtBr, id::LOW_WARNING, &["G703", "7"]),
            "G703 com 7%"
        );
        assert_eq!(
            fmt(Lang::En, id::SMART_WARNING, &["15"]),
            "Won't last the day: charge before 15:00"
        );
    }

    #[test]
    fn unknown_id_gives_an_empty_text() {
        assert_eq!(fmt(Lang::En, "no_such_id", &["x"]), "");
        assert_eq!(get(Lang::PtBr, "no_such_id"), "");
    }
}
