// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The command line. The flags exist for looking at the app and testing it; a normal launch has none.

use std::path::PathBuf;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Show the sample devices instead of the empty "no devices yet" state.
    pub demo: bool,
    /// With `demo`: step the icon through its states every 10 seconds.
    pub demo_cycle: bool,
    /// Quit by itself after this many seconds (tests and captures).
    pub seconds: Option<u32>,
    /// Force a language tag (`pt-BR`, `en`) instead of the user's.
    pub lang: Option<String>,
    /// Append the tray events to this file (tests).
    pub trace: Option<PathBuf>,
    /// Append one line per transport event and action to this file (off by default).
    pub log: Option<PathBuf>,
    /// Keep settings and histories in this folder instead of `%LOCALAPPDATA%\JuiceBar` (tests).
    pub data: Option<PathBuf>,
    /// Show the empty "no devices yet" state and read no device (captures).
    pub empty: bool,
    /// Play the tray mark's animation every this many seconds instead of its own pace (tests).
    pub anim_every: Option<u32>,
    /// Do not register the Ctrl+Alt+B hotkey (a test instance beside the owner's own).
    pub no_hotkey: bool,
    /// Open the flyout on the Settings page as soon as the icon is up (captures).
    pub open_settings: bool,
}

impl Args {
    /// A normal launch reads the real devices; the demo and the empty state do not.
    pub fn is_live(&self) -> bool {
        !self.demo && !self.empty
    }

    /// A launch with none of the test or development flags: the only one that may touch the
    /// owner's start-with-Windows value.
    pub fn is_plain(&self) -> bool {
        self.data.is_none()
            && !self.demo
            && !self.empty
            && !self.no_hotkey
            && self.seconds.is_none()
    }

    pub fn parse<I: IntoIterator<Item = String>>(items: I) -> Args {
        let mut a = Args::default();
        let mut it = items.into_iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--demo" => a.demo = true,
                "--demo-cycle" => {
                    a.demo = true;
                    a.demo_cycle = true;
                }
                "--seconds" => a.seconds = it.next().and_then(|v| v.parse().ok()),
                "--lang" => a.lang = it.next(),
                "--trace" => a.trace = it.next().map(PathBuf::from),
                "--log" => a.log = it.next().map(PathBuf::from),
                "--data" => a.data = it.next().map(PathBuf::from),
                "--empty" => a.empty = true,
                "--anim-every" => a.anim_every = it.next().and_then(|v| v.parse().ok()),
                "--no-hotkey" => a.no_hotkey = true,
                "--open-settings" => a.open_settings = true,
                _ => {}
            }
        }
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Args {
        Args::parse(s.split_whitespace().map(String::from))
    }

    #[test]
    fn no_flags_is_the_plain_launch() {
        assert_eq!(parse(""), Args::default());
    }

    #[test]
    fn demo_cycle_implies_demo_and_seconds_parses() {
        let a = parse("--demo-cycle --seconds 40");
        assert!(a.demo && a.demo_cycle);
        assert_eq!(a.seconds, Some(40));
    }

    #[test]
    fn log_data_and_empty_parse_and_only_the_plain_launch_is_live() {
        let a = parse("--log a.txt --data d --seconds 5");
        assert_eq!(a.log, Some(PathBuf::from("a.txt")));
        assert_eq!(a.data, Some(PathBuf::from("d")));
        assert!(a.is_live());
        assert!(!parse("--demo").is_live());
        assert!(!parse("--empty").is_live());
        assert!(parse("").log.is_none());
    }

    #[test]
    fn the_test_flags_for_the_animation_and_the_hotkey_parse() {
        let a = parse("--demo --anim-every 60 --no-hotkey");
        assert_eq!(a.anim_every, Some(60));
        assert!(a.no_hotkey);
        let plain = parse("--demo");
        assert_eq!(plain.anim_every, None);
        assert!(!plain.no_hotkey);
        assert_eq!(parse("--anim-every soon").anim_every, None);
    }

    #[test]
    fn only_a_launch_with_none_of_the_test_flags_is_plain() {
        assert!(parse("").is_plain());
        assert!(parse("--lang en --log a.txt --trace t.txt").is_plain());
        for flags in [
            "--data d",
            "--demo",
            "--demo-cycle",
            "--empty",
            "--no-hotkey",
            "--seconds 5",
        ] {
            assert!(!parse(flags).is_plain(), "{flags}");
        }
    }

    #[test]
    fn the_capture_flag_for_the_settings_page_parses() {
        assert!(parse("--demo --open-settings").open_settings);
        assert!(!parse("--demo").open_settings);
    }

    #[test]
    fn bad_numbers_and_unknown_flags_are_ignored() {
        let a = parse("--seconds abc --what --lang en");
        assert_eq!(a.seconds, None);
        assert_eq!(a.lang.as_deref(), Some("en"));
        assert!(!a.demo);
    }
}
