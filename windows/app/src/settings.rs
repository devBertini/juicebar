// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The user's settings, kept in `%LOCALAPPDATA%\JuiceBar\settings.txt` as simple `key=value` lines:
//!
//! ```text
//! rgb_economy=1
//! hidden=logitech-hidpp:0AB5
//! ```
//!
//! Unknown lines and comments are ignored, so a newer file still reads in an older build.

use juicebar_core::types::DeviceKey;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The "RGB saver" switch.
    pub rgb_economy: bool,
    /// Devices the user hid from the flyout and the icon (the registry's removed list).
    pub hidden: Vec<DeviceKey>,
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            match (name.trim(), value.trim()) {
                ("rgb_economy", v) => s.rgb_economy = v == "1",
                ("hidden", v) if !v.is_empty() => {
                    let key = DeviceKey(v.to_string());
                    if !s.hidden.contains(&key) {
                        s.hidden.push(key);
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn encode(&self) -> String {
        let mut out = format!("rgb_economy={}\n", u8::from(self.rgb_economy));
        for key in &self.hidden {
            out.push_str("hidden=");
            out.push_str(&key.0);
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_empty_file_is_the_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert!(!Settings::default().rgb_economy);
    }

    #[test]
    fn the_settings_round_trip() {
        let s = Settings {
            rgb_economy: true,
            hidden: vec![
                DeviceKey("logitech-hidpp:0AB5".into()),
                DeviceKey("os-bluetooth:ab:cd".into()),
            ],
        };
        let text = s.encode();
        assert_eq!(
            text,
            "rgb_economy=1\nhidden=logitech-hidpp:0AB5\nhidden=os-bluetooth:ab:cd\n"
        );
        assert_eq!(Settings::parse(&text), s);
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
}
