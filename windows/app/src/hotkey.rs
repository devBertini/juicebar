// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The global shortcut that opens the panel: its value, its text ("Ctrl+Alt+B"), what a typed key combination may
//! become, and the change of the registered one. Pure: the registration itself sits behind `Registrar`, so a fake
//! proves the "already in use" path without touching the system's hot keys.

use juicebar_ui::settings::Chord;

/// A shortcut: the modifier bits (`Chord::ALT`, `CONTROL`, `SHIFT`, `WIN`: Windows' `MOD_*` values) and a virtual key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: u8,
    pub vk: u16,
}

/// The first function key's virtual-key code; F1 to F24 follow.
const VK_F1: u16 = 0x70;
const VK_F24: u16 = 0x87;

impl Hotkey {
    /// Ctrl+Alt+B.
    pub const DEFAULT: Hotkey = Hotkey {
        mods: Chord::CONTROL | Chord::ALT,
        vk: b'B' as u16,
    };

    /// The shortcut a typed combination makes, or `None` when it cannot be one: it must hold Ctrl, Alt or Win, and
    /// its key must be a letter, a digit or an F key. Alt+F4 stays the system's.
    pub fn from_chord(c: Chord) -> Option<Hotkey> {
        let hk = Hotkey {
            mods: c.mods & (Chord::ALT | Chord::CONTROL | Chord::SHIFT | Chord::WIN),
            vk: c.vk,
        };
        hk.is_valid().then_some(hk)
    }

    /// True when the shortcut holds at least two of Ctrl, Alt, Shift and Win with a letter, a digit or an F key.
    /// One modifier would take a common combination (Ctrl+C, Alt+F4, Win+L) from every other program.
    pub fn is_valid(self) -> bool {
        let modifiers =
            (self.mods & (Chord::ALT | Chord::CONTROL | Chord::SHIFT | Chord::WIN)).count_ones();
        let key = matches!(self.vk, 0x30..=0x39 | 0x41..=0x5A | VK_F1..=VK_F24);
        modifiers >= 2 && key
    }

    /// "Ctrl+Alt+B": Ctrl, Alt, Shift, Win, then the key.
    pub fn text(self) -> String {
        let mut out = String::new();
        for (bit, name) in [
            (Chord::CONTROL, "Ctrl"),
            (Chord::ALT, "Alt"),
            (Chord::SHIFT, "Shift"),
            (Chord::WIN, "Win"),
        ] {
            if self.mods & bit != 0 {
                out.push_str(name);
                out.push('+');
            }
        }
        match self.vk {
            0x30..=0x39 | 0x41..=0x5A => out.push(char::from(self.vk as u8)),
            VK_F1..=VK_F24 => out.push_str(&format!("F{}", self.vk - VK_F1 + 1)),
            _ => out.push_str(&format!("0x{:02X}", self.vk)),
        }
        out
    }

    /// The shortcut a text names ("ctrl + alt + b" too); `None` for anything that is not a valid shortcut.
    pub fn parse(text: &str) -> Option<Hotkey> {
        let mut mods = 0;
        let mut vk = None;
        for part in text.split('+').map(str::trim) {
            let lower = part.to_ascii_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => mods |= Chord::CONTROL,
                "alt" => mods |= Chord::ALT,
                "shift" => mods |= Chord::SHIFT,
                "win" | "windows" => mods |= Chord::WIN,
                _ if vk.is_some() => return None,
                _ => {
                    let mut chars = lower.chars();
                    vk = match (chars.next(), chars.next()) {
                        (Some(c), None) if c.is_ascii_alphanumeric() => {
                            Some(c.to_ascii_uppercase() as u16)
                        }
                        (Some('f'), Some(d)) if d.is_ascii_digit() => {
                            let n: u16 = lower[1..].parse().ok()?;
                            (1..=24).contains(&n).then_some(VK_F1 + n - 1)
                        }
                        _ => return None,
                    };
                }
            }
        }
        let hk = Hotkey { mods, vk: vk? };
        hk.is_valid().then_some(hk)
    }
}

/// Where a shortcut is registered with the system.
pub trait Registrar {
    /// Registers the shortcut; false when another program holds it.
    fn register(&mut self, hotkey: Hotkey) -> bool;
    /// Lets go of the registered shortcut.
    fn unregister(&mut self);
}

/// What a change of the shortcut came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switched {
    /// The new shortcut is registered (and is the one in use).
    pub changed: bool,
    /// Some shortcut is registered now (the wanted one, or the old one put back).
    pub registered: bool,
}

/// Moves the registration from `current` to `wanted`. When another program holds `wanted`, `current` is put
/// back and `changed` is false. Asking for the shortcut already registered changes nothing.
pub fn switch(
    registrar: &mut dyn Registrar,
    current: Hotkey,
    registered: bool,
    wanted: Hotkey,
) -> Switched {
    if wanted == current && registered {
        return Switched {
            changed: true,
            registered: true,
        };
    }
    // Windows keeps a second registration with the same id beside the first, so the old one goes first.
    registrar.unregister();
    if registrar.register(wanted) {
        return Switched {
            changed: true,
            registered: true,
        };
    }
    Switched {
        changed: false,
        registered: registrar.register(current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: u16 = b'B' as u16;

    fn chord(mods: u8, vk: u16) -> Chord {
        Chord { mods, vk }
    }

    #[test]
    fn the_default_is_ctrl_alt_b_and_round_trips_through_its_text() {
        assert_eq!(Hotkey::DEFAULT.text(), "Ctrl+Alt+B");
        assert_eq!(Hotkey::parse("Ctrl+Alt+B"), Some(Hotkey::DEFAULT));
        assert!(Hotkey::DEFAULT.is_valid());
    }

    #[test]
    fn every_valid_shortcut_formats_and_parses_back() {
        for mods in 0u8..16 {
            for vk in (0x30u16..=0x39).chain(0x41..=0x5A).chain(0x70..=0x87) {
                let hk = Hotkey { mods, vk };
                if !hk.is_valid() {
                    continue;
                }
                assert_eq!(Hotkey::parse(&hk.text()), Some(hk), "{}", hk.text());
            }
        }
    }

    #[test]
    fn text_orders_the_modifiers_and_names_digits_and_function_keys() {
        let all = Chord::CONTROL | Chord::ALT | Chord::SHIFT | Chord::WIN;
        assert_eq!(Hotkey { mods: all, vk: B }.text(), "Ctrl+Alt+Shift+Win+B");
        assert_eq!(
            Hotkey {
                mods: Chord::WIN,
                vk: b'7' as u16
            }
            .text(),
            "Win+7"
        );
        assert_eq!(
            Hotkey {
                mods: Chord::ALT,
                vk: 0x7B
            }
            .text(),
            "Alt+F12"
        );
        assert_eq!(
            Hotkey {
                mods: Chord::CONTROL,
                vk: 0x87
            }
            .text(),
            "Ctrl+F24"
        );
    }

    #[test]
    fn a_typed_combination_needs_two_modifiers_and_a_letter_digit_or_f_key() {
        let ok = |m, v| Hotkey::from_chord(chord(m, v));
        let c = u16::from(b'C');
        let j = u16::from(b'J');
        // Accepted: two or more modifiers.
        assert_eq!(ok(Chord::CONTROL | Chord::ALT, B), Some(Hotkey::DEFAULT));
        assert!(
            ok(Chord::CONTROL | Chord::SHIFT, j).is_some(),
            "Ctrl+Shift+J"
        );
        assert!(ok(Chord::WIN | Chord::ALT, B).is_some(), "Win+Alt+B");
        assert!(ok(Chord::WIN | Chord::SHIFT, b'1' as u16).is_some());
        assert!(
            ok(Chord::ALT | Chord::SHIFT, 0x7A).is_some(),
            "Alt+Shift+F11"
        );
        // Refused: one modifier would take a common key from every program.
        assert_eq!(ok(Chord::CONTROL, c), None, "Ctrl+C");
        assert_eq!(ok(Chord::CONTROL, u16::from(b'V')), None, "Ctrl+V");
        assert_eq!(ok(Chord::ALT, B), None, "Alt+B");
        assert_eq!(ok(Chord::WIN, b'Q' as u16), None);
        assert_eq!(ok(Chord::ALT, 0x7A), None, "Alt+F11");
        assert_eq!(ok(Chord::SHIFT, B), None);
        assert_eq!(ok(0, B), None);
        // Not a letter, digit or F key: Tab, Enter, Space, the arrows, Escape.
        for vk in [0x09, 0x0D, 0x20, 0x25, 0x1B, 0xBA, 0x6B] {
            assert_eq!(ok(Chord::CONTROL | Chord::ALT, vk), None, "{vk:#x}");
        }
        // Past F24 is not an F key.
        assert_eq!(ok(Chord::CONTROL | Chord::ALT, 0x88), None);
        // Alt+F4 and Win+L stay the system's (one modifier each, refused either way).
        assert_eq!(ok(Chord::ALT, 0x73), None);
        assert_eq!(ok(Chord::WIN, u16::from(b'L')), None);
        assert_eq!(Hotkey::parse("Alt+F4"), None);
        assert_eq!(Hotkey::parse("Win+L"), None);
        // Unknown bits are dropped and do not count as a modifier.
        assert_eq!(ok(Chord::CONTROL | 0x80, B), None);
        assert!(ok(Chord::CONTROL | Chord::ALT | 0x80, B).is_some());
    }

    #[test]
    fn parsing_is_forgiving_about_case_and_spaces_and_strict_about_the_rest() {
        let want = Some(Hotkey {
            mods: Chord::CONTROL | Chord::WIN,
            vk: 0x7A,
        });
        assert_eq!(Hotkey::parse(" ctrl + WIN + f11 "), want);
        assert_eq!(Hotkey::parse("Control+Windows+F11"), want);
        for bad in [
            "",
            "B",
            "Ctrl",
            "Ctrl+",
            "Ctrl+B+C",
            "Ctrl+Alt+F25",
            "Ctrl+Alt+F0",
            "Ctrl+Alt+Tab",
            "Shift+B",
            "Ctrl+C",
            "Alt+B",
            "Win+B",
            "Alt+F4",
            "Ctrl+Alt+Ç",
            "nonsense",
        ] {
            assert_eq!(Hotkey::parse(bad), None, "{bad:?}");
        }
    }

    /// A registrar that knows which shortcuts other programs hold, and what it registered.
    struct Fake {
        taken: Vec<Hotkey>,
        active: Option<Hotkey>,
        calls: Vec<String>,
    }

    impl Fake {
        fn new(taken: &[Hotkey]) -> Fake {
            Fake {
                taken: taken.to_vec(),
                active: None,
                calls: Vec::new(),
            }
        }
    }

    impl Registrar for Fake {
        fn register(&mut self, hotkey: Hotkey) -> bool {
            self.calls.push(format!("register {}", hotkey.text()));
            if self.active.is_some() || self.taken.contains(&hotkey) {
                return false;
            }
            self.active = Some(hotkey);
            true
        }

        fn unregister(&mut self) {
            self.calls.push("unregister".into());
            self.active = None;
        }
    }

    const OTHER: Hotkey = Hotkey {
        mods: Chord::CONTROL | Chord::ALT,
        vk: b'J' as u16,
    };

    #[test]
    fn a_free_shortcut_replaces_the_old_one() {
        let mut r = Fake::new(&[]);
        assert!(r.register(Hotkey::DEFAULT));
        let s = switch(&mut r, Hotkey::DEFAULT, true, OTHER);
        assert_eq!(
            s,
            Switched {
                changed: true,
                registered: true
            }
        );
        assert_eq!(r.active, Some(OTHER));
    }

    #[test]
    fn a_shortcut_another_program_holds_keeps_the_old_one() {
        let mut r = Fake::new(&[OTHER]);
        assert!(r.register(Hotkey::DEFAULT));
        let s = switch(&mut r, Hotkey::DEFAULT, true, OTHER);
        assert_eq!(
            s,
            Switched {
                changed: false,
                registered: true
            }
        );
        assert_eq!(r.active, Some(Hotkey::DEFAULT), "the old one is back");
        assert_eq!(
            r.calls,
            [
                "register Ctrl+Alt+B",
                "unregister",
                "register Ctrl+Alt+J",
                "register Ctrl+Alt+B"
            ]
        );
    }

    #[test]
    fn asking_for_the_shortcut_in_use_touches_nothing() {
        let mut r = Fake::new(&[]);
        assert!(r.register(Hotkey::DEFAULT));
        r.calls.clear();
        let s = switch(&mut r, Hotkey::DEFAULT, true, Hotkey::DEFAULT);
        assert!(s.changed && s.registered);
        assert!(r.calls.is_empty());
    }

    #[test]
    fn a_shortcut_that_never_registered_can_still_be_replaced() {
        // The old one was taken at start (not registered); the new one is free.
        let mut r = Fake::new(&[Hotkey::DEFAULT]);
        let s = switch(&mut r, Hotkey::DEFAULT, false, OTHER);
        assert!(s.changed && s.registered);
        assert_eq!(r.active, Some(OTHER));
        // Both taken: nothing registered, and the answer says so.
        let mut r = Fake::new(&[Hotkey::DEFAULT, OTHER]);
        let s = switch(&mut r, Hotkey::DEFAULT, false, OTHER);
        assert_eq!(
            s,
            Switched {
                changed: false,
                registered: false
            }
        );
    }
}
