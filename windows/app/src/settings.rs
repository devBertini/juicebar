// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The user's settings, kept in `%LOCALAPPDATA%\JuiceBar\settings.txt` as simple `key=value` lines:
//!
//! ```text
//! rgb_economy=1
//! hidden=logitech-hidpp:0AB5
//! light=logitech-hidpp:40B5 brightness:13
//! ```
//!
//! `light` is the level the RGB economy took a device's lighting down from, kept until it is restored, so a run
//! that ended with the lighting down still knows what to give back (the core's `SavedLight` text).
//!
//! Unknown lines and comments are ignored, so a newer file still reads in an older build.

use juicebar_core::types::DeviceKey;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The power saver switch.
    pub rgb_economy: bool,
    /// Devices the user hid from the flyout and the icon (the registry's removed list).
    pub hidden: Vec<DeviceKey>,
    /// The lighting level kept for each device the economy turned down (device key, `SavedLight` text).
    pub lights: Vec<(DeviceKey, String)>,
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
        s
    }

    pub fn encode(&self) -> String {
        let mut out = format!("rgb_economy={}\n", u8::from(self.rgb_economy));
        for key in &self.hidden {
            out.push_str("hidden=");
            out.push_str(&key.0);
            out.push('\n');
        }
        for (key, text) in &self.lights {
            out.push_str(&format!("light={} {}\n", key.0, text));
        }
        out
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
            lights: vec![(
                DeviceKey("logitech-hidpp:40B5".into()),
                "brightness:13".into(),
            )],
        };
        let text = s.encode();
        assert_eq!(
            text,
            "rgb_economy=1\nhidden=logitech-hidpp:0AB5\nhidden=os-bluetooth:ab:cd\nlight=logitech-hidpp:40B5 brightness:13\n"
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
}
