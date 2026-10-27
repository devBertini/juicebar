// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
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
}

impl Args {
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
    fn bad_numbers_and_unknown_flags_are_ignored() {
        let a = parse("--seconds abc --what --lang en");
        assert_eq!(a.seconds, None);
        assert_eq!(a.lang.as_deref(), Some("en"));
        assert!(!a.demo);
    }
}
