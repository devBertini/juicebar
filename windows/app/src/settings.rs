// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The user's settings, kept in `%LOCALAPPDATA%\JuiceBar\settings.txt` as simple `key=value` lines:
//!
//! ```text
//! rgb_economy=1
//! welcome_seen=1
//! theme=auto
//! icon_animation=1
//! green_above=50
//! red_below=20
//! warn_below=10
//! smart_warning=1
//! full_notice=1
//! do_not_disturb=0
//! language=auto
//! hotkey=Ctrl+Alt+B
//! device.logitech-hidpp:0AB5.shown=0
//! device.logitech-hidpp:40B5.name=Teclado da mesa
//! device.logitech-hidpp:40B5.order=0
//! light=logitech-hidpp:40B5 brightness:13
//! ```
//!
//! * `theme` is `auto` (follows Windows' app mode), `light` or `dark`: the flyout's colors only.
//! * `icon_animation=0` stops every run of the tray mark's motion.
//! * `green_above` (25 to 95) and `red_below` (5 to 50) are the two limits of the tray mark's juice; the red one
//!   always stays at least 5 under the green one.
//! * `warn_below` (5 to 30) is the level under which a discharging device is told to charge.
//! * `smart_warning` and `full_notice` switch those two notices (and the smart warning's red) on or off;
//!   `do_not_disturb=1` shows no balloon at all, the icon still changes.
//! * `language` is `auto` (Windows' display language), `pt-BR` or `en`.
//! * `hotkey` is the global shortcut that opens the panel (`Ctrl+Alt+B` by default; it must hold Ctrl, Alt or Win).
//! * `device.<key>.shown=0` hides a device from the panel, the tooltip and the icon's colour (the key is the
//!   device's registry key, which holds no serial number or address); `.name` is the name the user gave it (at most
//!   24 characters; none means the catalog's short name); `.order` is its place in the user's order (0 first).
//!   A device with no line keeps the defaults: shown, the catalog's name, after the ordered ones. The older
//!   `hidden=<key>` line is still read.
//! * `welcome_seen` is set once the first-open page has been shown, so it never comes back.
//! * `light` is the level the RGB economy took a device's lighting down from, kept until it is restored, so a run
//!   that ended with the lighting down still knows what to give back (the core's `SavedLight` text).
//!
//! Start with Windows is not here: its source of truth is the Run key (`runkey`).
//! Unknown lines and comments are ignored, so a newer file still reads in an older build.

use juicebar_core::snapshot::{Arrangement, Limits};
use juicebar_core::strings::Lang;
use juicebar_core::types::DeviceKey;
use juicebar_ui::model::Theme;

use crate::hotkey::Hotkey;

/// The flyout's theme as the user chose it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    Auto,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::Auto, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn tag(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "auto",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn from_tag(tag: &str) -> Option<ThemeChoice> {
        Self::ALL.into_iter().find(|t| t.tag() == tag)
    }

    /// The theme the flyout takes: the chosen one, or Windows' app mode for `Auto`.
    pub fn resolve(self, system: Theme) -> Theme {
        match self {
            ThemeChoice::Auto => system,
            ThemeChoice::Light => Theme::Light,
            ThemeChoice::Dark => Theme::Dark,
        }
    }
}

/// The language as the user chose it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LangChoice {
    #[default]
    Auto,
    Pt,
    En,
}

impl LangChoice {
    pub const ALL: [LangChoice; 3] = [LangChoice::Auto, LangChoice::Pt, LangChoice::En];

    pub fn tag(self) -> &'static str {
        match self {
            LangChoice::Auto => "auto",
            LangChoice::Pt => "pt-BR",
            LangChoice::En => "en",
        }
    }

    pub fn from_tag(tag: &str) -> Option<LangChoice> {
        Self::ALL.into_iter().find(|l| l.tag() == tag)
    }

    /// The language the texts take: the chosen one, or `detected` (Windows' display language, or the command
    /// line's) for `Auto`.
    pub fn resolve(self, detected: Lang) -> Lang {
        match self {
            LangChoice::Auto => detected,
            LangChoice::Pt => Lang::PtBr,
            LangChoice::En => Lang::En,
        }
    }
}

/// The bounds of the numbers the user sets, in percent.
pub const GREEN_MIN: i32 = 25;
pub const GREEN_MAX: i32 = 95;
pub const RED_MIN: i32 = 5;
pub const RED_MAX: i32 = 50;
pub const WARN_MIN: i32 = 5;
pub const WARN_MAX: i32 = 30;
/// The red limit stays at least this far under the green one.
pub const LIMIT_GAP: i32 = 5;

/// Which limit a change of the other one moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adjusted {
    Green,
    Red,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The power saver switch.
    pub rgb_economy: bool,
    /// The first-open welcome page has been shown.
    pub welcome_seen: bool,
    pub theme: ThemeChoice,
    /// The tray mark's motion (it also needs the system's animation effects on).
    pub icon_animation: bool,
    /// Green when every device with a reading is at or above this percent.
    pub green_above: u8,
    /// Red when any device is below this percent.
    pub red_below: u8,
    /// The low-battery notice fires under this percent.
    pub warn_below: u8,
    pub smart_warning: bool,
    pub full_notice: bool,
    /// No balloon at all while on.
    pub do_not_disturb: bool,
    pub language: LangChoice,
    /// The global shortcut that opens the panel.
    pub hotkey: Hotkey,
    /// Devices the user hid from the flyout and the icon (the registry's removed list).
    pub hidden: Vec<DeviceKey>,
    /// The names the user gave the devices and the order he put them in.
    pub arrangement: Arrangement,
    /// The lighting level kept for each device the economy turned down (device key, `SavedLight` text).
    pub lights: Vec<(DeviceKey, String)>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            rgb_economy: false,
            welcome_seen: false,
            theme: ThemeChoice::Auto,
            icon_animation: true,
            green_above: Limits::DEFAULT_HIGH,
            red_below: Limits::DEFAULT_LOW,
            warn_below: 10,
            smart_warning: true,
            full_notice: true,
            do_not_disturb: false,
            language: LangChoice::Auto,
            hotkey: Hotkey::DEFAULT,
            hidden: Vec::new(),
            arrangement: Arrangement::default(),
            lights: Vec::new(),
        }
    }
}

fn number(v: &str, min: i32, max: i32) -> Option<u8> {
    let n: i32 = v.parse().ok()?;
    u8::try_from(n.clamp(min, max)).ok()
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        // (order, place in the file, key): sorted at the end, so a repeated or missing number is harmless.
        let mut ranked: Vec<(u32, usize, DeviceKey)> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if let Some(rest) = name.strip_prefix("device.") {
                s.device_line(rest, value, &mut ranked);
                continue;
            }
            match (name, value) {
                ("rgb_economy", v) => s.rgb_economy = v == "1",
                ("welcome_seen", v) => s.welcome_seen = v == "1",
                ("theme", v) => s.theme = ThemeChoice::from_tag(v).unwrap_or_default(),
                ("icon_animation", v) => s.icon_animation = v != "0",
                ("green_above", v) => {
                    s.green_above = number(v, GREEN_MIN, GREEN_MAX).unwrap_or(s.green_above)
                }
                ("red_below", v) => {
                    s.red_below = number(v, RED_MIN, RED_MAX).unwrap_or(s.red_below)
                }
                ("warn_below", v) => {
                    s.warn_below = number(v, WARN_MIN, WARN_MAX).unwrap_or(s.warn_below)
                }
                ("smart_warning", v) => s.smart_warning = v != "0",
                ("full_notice", v) => s.full_notice = v != "0",
                ("do_not_disturb", v) => s.do_not_disturb = v == "1",
                ("language", v) => s.language = LangChoice::from_tag(v).unwrap_or_default(),
                ("hotkey", v) => s.hotkey = Hotkey::parse(v).unwrap_or(Hotkey::DEFAULT),
                ("hidden", v) if !v.is_empty() => {
                    let key = DeviceKey(v.to_string());
                    if !s.hidden.contains(&key) {
                        s.hidden.push(key);
                    }
                }
                ("light", v) => {
                    if let Some((key, text)) = v.split_once(' ') {
                        let key = DeviceKey(key.trim().to_string());
                        let text = text.trim().to_string();
                        if !key.0.is_empty()
                            && !text.is_empty()
                            && !s.lights.iter().any(|(k, _)| *k == key)
                        {
                            s.lights.push((key, text));
                        }
                    }
                }
                _ => {}
            }
        }
        ranked.sort();
        for (_, _, key) in ranked {
            if !s.arrangement.order.contains(&key) {
                s.arrangement.order.push(key);
            }
        }
        s.keep_limits_apart();
        s
    }

    /// One `device.<key>.<field>=value` line (`rest` is what follows `device.`).
    fn device_line(&mut self, rest: &str, value: &str, ranked: &mut Vec<(u32, usize, DeviceKey)>) {
        // The key may hold dots of its own: the field is what follows the last one.
        let Some((key, field)) = rest.rsplit_once('.') else {
            return;
        };
        if key.is_empty() {
            return;
        }
        let key = DeviceKey(key.to_string());
        match field {
            "shown" if value == "0" => {
                if !self.hidden.contains(&key) {
                    self.hidden.push(key);
                }
            }
            "shown" if value == "1" => self.hidden.retain(|k| *k != key),
            "name" => self.arrangement.set_name(&key, value),
            "order" => {
                if let Ok(n) = value.parse::<u32>() {
                    ranked.push((n, ranked.len(), key));
                }
            }
            _ => {}
        }
    }

    pub fn encode(&self) -> String {
        let flag = |b: bool| u8::from(b);
        let mut out = format!(
            "rgb_economy={}\nwelcome_seen={}\ntheme={}\nicon_animation={}\ngreen_above={}\nred_below={}\nwarn_below={}\nsmart_warning={}\nfull_notice={}\ndo_not_disturb={}\nlanguage={}\nhotkey={}\n",
            flag(self.rgb_economy),
            flag(self.welcome_seen),
            self.theme.tag(),
            flag(self.icon_animation),
            self.green_above,
            self.red_below,
            self.warn_below,
            flag(self.smart_warning),
            flag(self.full_notice),
            flag(self.do_not_disturb),
            self.language.tag(),
            self.hotkey.text(),
        );
        // Every device the user touched, the ordered ones first.
        let mut keys: Vec<&DeviceKey> = self.arrangement.order.iter().collect();
        let others = self
            .arrangement
            .names
            .iter()
            .map(|(k, _)| k)
            .chain(&self.hidden);
        for key in others {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for key in keys {
            if self.hidden.contains(key) {
                out.push_str(&format!("device.{}.shown=0\n", key.0));
            }
            if let Some(name) = self.arrangement.name_of(key) {
                out.push_str(&format!("device.{}.name={}\n", key.0, name));
            }
            if let Some(rank) = self.arrangement.rank(key) {
                out.push_str(&format!("device.{}.order={}\n", key.0, rank));
            }
        }
        for (key, text) in &self.lights {
            out.push_str(&format!("light={} {}\n", key.0, text));
        }
        out
    }

    /// The tray mark's two limits, as the core takes them.
    pub fn limits(&self) -> Limits {
        Limits::new(self.red_below, self.green_above)
    }

    /// Makes the red limit at least `LIMIT_GAP` under the green one (the green one wins in a file that says
    /// otherwise). Returns true when it moved the red one.
    fn keep_limits_apart(&mut self) -> bool {
        let top = i32::from(self.green_above) - LIMIT_GAP;
        if i32::from(self.red_below) > top {
            self.red_below = u8::try_from(top.max(RED_MIN)).unwrap_or(self.red_below);
            return true;
        }
        false
    }

    /// Sets the green limit (kept inside its bounds). When the red one would no longer be under it, the red one
    /// moves down; the answer says so.
    pub fn set_green(&mut self, value: i32) -> Option<Adjusted> {
        self.green_above =
            u8::try_from(value.clamp(GREEN_MIN, GREEN_MAX)).unwrap_or(self.green_above);
        self.keep_limits_apart().then_some(Adjusted::Red)
    }

    /// Sets the red limit (kept inside its bounds). When the green one would no longer be over it, the green one
    /// moves up; the answer says so.
    pub fn set_red(&mut self, value: i32) -> Option<Adjusted> {
        self.red_below = u8::try_from(value.clamp(RED_MIN, RED_MAX)).unwrap_or(self.red_below);
        let need = i32::from(self.red_below) + LIMIT_GAP;
        if i32::from(self.green_above) < need {
            self.green_above = u8::try_from(need.min(GREEN_MAX)).unwrap_or(self.green_above);
            return Some(Adjusted::Green);
        }
        None
    }

    /// Sets the low-battery level (kept inside its bounds).
    pub fn set_warn(&mut self, value: i32) {
        self.warn_below = u8::try_from(value.clamp(WARN_MIN, WARN_MAX)).unwrap_or(self.warn_below);
    }

    /// Keep the level of a device, or replace the one kept.
    pub fn keep_light(&mut self, key: &DeviceKey, text: String) {
        match self.lights.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = text,
            None => self.lights.push((key.clone(), text)),
        }
    }

    pub fn drop_light(&mut self, key: &DeviceKey) {
        self.lights.retain(|(k, _)| k != key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_ui::settings::Chord;

    #[test]
    fn a_missing_or_empty_file_is_the_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        let d = Settings::default();
        assert!(!d.rgb_economy);
        assert!(!d.welcome_seen, "a first run has not seen the welcome page");
        assert_eq!(d.theme, ThemeChoice::Auto);
        assert!(d.icon_animation && d.smart_warning && d.full_notice && !d.do_not_disturb);
        assert_eq!((d.green_above, d.red_below, d.warn_below), (50, 20, 10));
        assert_eq!(d.language, LangChoice::Auto);
        assert_eq!(d.limits(), Limits::default());
    }

    #[test]
    fn the_welcome_flag_is_kept_and_an_old_file_without_it_has_not_seen_it() {
        assert!(Settings::parse("welcome_seen=1\n").welcome_seen);
        assert!(!Settings::parse("welcome_seen=0\n").welcome_seen);
        assert!(!Settings::parse("rgb_economy=1\n").welcome_seen);
        let s = Settings {
            welcome_seen: true,
            ..Settings::default()
        };
        assert!(s.encode().starts_with("rgb_economy=0\nwelcome_seen=1\n"));
        assert!(Settings::parse(&s.encode()).welcome_seen);
    }

    #[test]
    fn an_old_file_keeps_its_values_and_gets_the_new_defaults() {
        let s = Settings::parse("rgb_economy=1\nwelcome_seen=1\nhidden=a:b\n");
        assert!(s.rgb_economy && s.welcome_seen);
        assert_eq!(s.hidden, vec![DeviceKey("a:b".into())]);
        assert_eq!(s.theme, ThemeChoice::Auto);
        assert!(s.icon_animation && s.smart_warning && s.full_notice);
        assert_eq!((s.green_above, s.red_below, s.warn_below), (50, 20, 10));
    }

    #[test]
    fn every_key_round_trips() {
        let s = Settings {
            rgb_economy: true,
            welcome_seen: true,
            theme: ThemeChoice::Dark,
            icon_animation: false,
            green_above: 70,
            red_below: 35,
            warn_below: 25,
            smart_warning: false,
            full_notice: false,
            do_not_disturb: true,
            language: LangChoice::Pt,
            hotkey: Hotkey {
                mods: Chord::WIN | Chord::ALT,
                vk: 0x7A,
            },
            hidden: vec![
                DeviceKey("logitech-hidpp:0AB5".into()),
                DeviceKey("os-bluetooth:ab:cd".into()),
            ],
            arrangement: Arrangement {
                names: vec![(DeviceKey("logitech-hidpp:40B5".into()), "Teclado".into())],
                order: vec![
                    DeviceKey("logitech-hidpp:40B5".into()),
                    DeviceKey("logitech-hidpp:0AB5".into()),
                ],
            },
            lights: vec![(
                DeviceKey("logitech-hidpp:40B5".into()),
                "brightness:13".into(),
            )],
        };
        let text = s.encode();
        assert_eq!(
            text,
            "rgb_economy=1\nwelcome_seen=1\ntheme=dark\nicon_animation=0\ngreen_above=70\nred_below=35\nwarn_below=25\nsmart_warning=0\nfull_notice=0\ndo_not_disturb=1\nlanguage=pt-BR\nhotkey=Alt+Win+F11\ndevice.logitech-hidpp:40B5.name=Teclado\ndevice.logitech-hidpp:40B5.order=0\ndevice.logitech-hidpp:0AB5.shown=0\ndevice.logitech-hidpp:0AB5.order=1\ndevice.os-bluetooth:ab:cd.shown=0\nlight=logitech-hidpp:40B5 brightness:13\n"
        );
        assert_eq!(Settings::parse(&text), s);
        // Each choice of the two enumerations survives a round trip on its own.
        for t in ThemeChoice::ALL {
            let s = Settings {
                theme: t,
                ..Settings::default()
            };
            assert_eq!(Settings::parse(&s.encode()).theme, t);
        }
        for l in LangChoice::ALL {
            let s = Settings {
                language: l,
                ..Settings::default()
            };
            assert_eq!(Settings::parse(&s.encode()).language, l);
        }
    }

    #[test]
    fn unknown_lines_comments_and_repeats_are_ignored() {
        let s = Settings::parse(
            "# note\nfuture=1\nrgb_economy = 1\nhidden=a:b\nhidden=a:b\nhidden=\nnonsense\r\n",
        );
        assert!(s.rgb_economy);
        assert_eq!(s.hidden, vec![DeviceKey("a:b".into())]);
        assert!(!Settings::parse("rgb_economy=0").rgb_economy);
    }

    #[test]
    fn numbers_out_of_range_or_unreadable_stay_inside_the_bounds() {
        let s = Settings::parse("green_above=200\nred_below=-4\nwarn_below=99\n");
        assert_eq!((s.green_above, s.red_below, s.warn_below), (95, 5, 30));
        let s = Settings::parse("green_above=3\nred_below=abc\nwarn_below=\n");
        assert_eq!(
            (s.green_above, s.red_below, s.warn_below),
            (25, 20, 10),
            "an unreadable number keeps the default"
        );
        // A file that puts red over green: the green limit wins, red goes under it.
        let s = Settings::parse("green_above=30\nred_below=40\n");
        assert_eq!((s.green_above, s.red_below), (30, 25));
        let s = Settings::parse("green_above=25\nred_below=50\n");
        assert_eq!((s.green_above, s.red_below), (25, 20));
    }

    #[test]
    fn unknown_choices_fall_back_to_automatic() {
        let s =
            Settings::parse("theme=purple\nlanguage=fr\nicon_animation=\nsmart_warning=maybe\n");
        assert_eq!(s.theme, ThemeChoice::Auto);
        assert_eq!(s.language, LangChoice::Auto);
        assert!(s.icon_animation && s.smart_warning, "only 0 turns them off");
    }

    #[test]
    fn the_red_limit_never_reaches_the_green_one_and_the_answer_says_which_moved() {
        let mut s = Settings::default();
        assert_eq!(s.set_red(30), None, "room under green 50");
        // Lowering green to the minimum drags red down with it.
        assert_eq!(s.set_green(25), Some(Adjusted::Red));
        assert_eq!((s.green_above, s.red_below), (25, 20));
        // Raising red past the room under green pushes green up.
        assert_eq!(s.set_red(30), Some(Adjusted::Green));
        assert_eq!((s.green_above, s.red_below), (35, 30));
        // No move of the other one when there is room.
        assert_eq!(s.set_green(80), None);
        assert_eq!(s.set_red(50), None);
        assert_eq!((s.green_above, s.red_below), (80, 50));
        // The bounds hold whatever is asked.
        s.set_green(500);
        assert_eq!(s.green_above, 95);
        s.set_green(-5);
        assert_eq!((s.green_above, s.red_below), (25, 20));
        s.set_red(-9);
        assert_eq!(s.red_below, 5);
        for g in (25..=95).step_by(5) {
            for r in (5..=50).step_by(5) {
                let mut s = Settings::default();
                s.set_green(g);
                s.set_red(r);
                assert!(s.red_below + 5 <= s.green_above, "g{g} r{r}: {s:?}");
                let l = s.limits();
                assert!(l.low < l.high);
            }
        }
    }

    #[test]
    fn the_limits_reach_the_core_as_set() {
        let s = Settings {
            green_above: 80,
            red_below: 30,
            ..Settings::default()
        };
        assert_eq!(s.limits(), Limits { low: 30, high: 80 });
        let mut w = Settings::default();
        w.set_warn(15);
        assert_eq!(w.warn_below, 15);
        w.set_warn(1);
        assert_eq!(w.warn_below, 5);
        w.set_warn(99);
        assert_eq!(w.warn_below, 30);
    }

    #[test]
    fn the_theme_follows_windows_only_on_automatic() {
        assert_eq!(ThemeChoice::Auto.resolve(Theme::Dark), Theme::Dark);
        assert_eq!(ThemeChoice::Auto.resolve(Theme::Light), Theme::Light);
        assert_eq!(ThemeChoice::Light.resolve(Theme::Dark), Theme::Light);
        assert_eq!(ThemeChoice::Dark.resolve(Theme::Light), Theme::Dark);
    }

    #[test]
    fn the_language_follows_the_system_only_on_automatic() {
        assert_eq!(LangChoice::Auto.resolve(Lang::PtBr), Lang::PtBr);
        assert_eq!(LangChoice::Auto.resolve(Lang::En), Lang::En);
        assert_eq!(LangChoice::Pt.resolve(Lang::En), Lang::PtBr);
        assert_eq!(LangChoice::En.resolve(Lang::PtBr), Lang::En);
    }

    #[test]
    fn kept_lights_are_replaced_dropped_and_read_back() {
        let key = DeviceKey("logitech-hidpp:40B5".into());
        let mut s = Settings::default();
        s.keep_light(&key, "brightness:13".into());
        s.keep_light(&key, "brightness:50".into());
        assert_eq!(s.lights.len(), 1);
        assert_eq!(Settings::parse(&s.encode()).lights, s.lights);
        s.drop_light(&key);
        assert!(s.lights.is_empty());
        assert!(
            Settings::parse(
                "light=
light=k
light= x
"
            )
            .lights
            .is_empty()
        );
    }

    fn key(k: &str) -> DeviceKey {
        DeviceKey(k.to_string())
    }

    #[test]
    fn the_shortcut_is_kept_and_an_unreadable_one_falls_back_to_the_default() {
        assert_eq!(Settings::default().hotkey, Hotkey::DEFAULT);
        assert_eq!(
            Settings::parse("hotkey=Ctrl+Shift+F5\n").hotkey.text(),
            "Ctrl+Shift+F5"
        );
        for bad in [
            "hotkey=\n",
            "hotkey=B\n",
            "hotkey=Alt+F4\n",
            "hotkey=Ctrl+Ç\n",
        ] {
            assert_eq!(Settings::parse(bad).hotkey, Hotkey::DEFAULT, "{bad:?}");
        }
        assert_eq!(Settings::parse("rgb_economy=1\n").hotkey, Hotkey::DEFAULT);
    }

    #[test]
    fn device_lines_read_names_order_and_visibility_and_write_back_the_same() {
        let text = "device.logitech-hidpp:40B5.name=Teclado da mesa\n\
                    device.logitech-hidpp:40B5.order=1\n\
                    device.logitech-hidpp:4086.order=0\n\
                    device.logitech-hidpp:0AB5.shown=0\n\
                    device.os-bluetooth:ab:cd.name=Fones\n";
        let s = Settings::parse(text);
        assert_eq!(s.hidden, vec![key("logitech-hidpp:0AB5")]);
        assert_eq!(
            s.arrangement.order,
            vec![key("logitech-hidpp:4086"), key("logitech-hidpp:40B5")]
        );
        assert_eq!(
            s.arrangement.name_of(&key("logitech-hidpp:40B5")),
            Some("Teclado da mesa")
        );
        assert_eq!(
            s.arrangement.name_of(&key("os-bluetooth:ab:cd")),
            Some("Fones")
        );
        let again = Settings::parse(&s.encode());
        assert_eq!(again, s);
        assert!(s.encode().contains("device.logitech-hidpp:4086.order=0\n"));
        assert!(s.encode().contains("device.logitech-hidpp:40B5.order=1\n"));
    }

    #[test]
    fn a_name_can_hold_equals_signs_and_is_cut_to_24_characters() {
        let s = Settings::parse(
            "device.k:1.name=  a=b  \ndevice.k:2.name=0123456789012345678901234567890\n",
        );
        assert_eq!(s.arrangement.name_of(&key("k:1")), Some("a=b"));
        assert_eq!(
            s.arrangement.name_of(&key("k:2")),
            Some("012345678901234567890123")
        );
        assert_eq!(Settings::parse(&s.encode()), s);
    }

    #[test]
    fn device_lines_that_make_no_sense_are_ignored() {
        let s = Settings::parse(
            "device.nodot=1\ndevice..name=x\ndevice.k:1.color=red\ndevice.k:1.order=abc\ndevice.k:1.order=-1\ndevice.k:1.shown=maybe\ndevice.k:1.name=\n",
        );
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn a_key_with_a_dot_keeps_it() {
        let s = Settings::parse("device.game-controllers:054C:0CE6.v2.name=Pad\n");
        assert_eq!(
            s.arrangement.name_of(&key("game-controllers:054C:0CE6.v2")),
            Some("Pad")
        );
    }

    #[test]
    fn showing_a_device_again_and_the_old_hidden_line_agree() {
        let s = Settings::parse("hidden=a:b\nhidden=c:d\ndevice.a:b.shown=1\n");
        assert_eq!(s.hidden, vec![key("c:d")], "the later line wins");
        // The old line still reads, and the new file names it the new way.
        let enc = Settings::parse("hidden=a:b\n").encode();
        assert!(enc.contains("device.a:b.shown=0\n") && !enc.contains("hidden="));
    }

    #[test]
    fn repeated_order_numbers_keep_file_order_and_never_repeat_a_device() {
        let s = Settings::parse(
            "device.a:1.order=3\ndevice.b:1.order=3\ndevice.a:1.order=0\ndevice.c:1.order=9\n",
        );
        assert_eq!(
            s.arrangement.order,
            vec![key("a:1"), key("b:1"), key("c:1")]
        );
    }
}
