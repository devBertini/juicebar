// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The Dispositivos section's text entry: renaming a device. The row turns into a text field with the current name
//! selected; typing replaces it, Backspace erases, Enter saves, Escape cancels. Pure, so the keys are unit tested.

use juicebar_core::snapshot::NAME_MAX;
use juicebar_core::types::DeviceKey;
use juicebar_ui::settings::EditKey;

use crate::settings::Settings;

/// Shows (`true`) or hides a device in the user's settings; the live source is told separately.
pub fn set_shown(settings: &mut Settings, key: &DeviceKey, shown: bool) {
    if shown {
        settings.hidden.retain(|k| k != key);
    } else if !settings.hidden.contains(key) {
        settings.hidden.push(key.clone());
    }
}

/// A rename in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub key: DeviceKey,
    /// The text as typed so far.
    pub text: String,
    /// The whole text is selected: the next key replaces it.
    pub selected: bool,
    /// The catalog's short name: what an empty name goes back to.
    pub default_name: String,
}

/// What a key did to a rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edited {
    /// Still typing.
    Editing,
    /// Enter: the name to keep (empty = back to the catalog's short name).
    Save(String),
    /// Escape.
    Cancel,
}

impl Rename {
    /// Starts renaming `key`, whose name now is `current`, with that name selected.
    pub fn new(key: DeviceKey, current: &str, default_name: &str) -> Rename {
        Rename {
            key,
            text: current.to_string(),
            selected: true,
            default_name: default_name.to_string(),
        }
    }

    pub fn key(&mut self, k: EditKey) -> Edited {
        match k {
            EditKey::Char(c) => {
                if c.is_control() {
                    return Edited::Editing;
                }
                if self.selected {
                    self.text.clear();
                    self.selected = false;
                }
                if self.text.chars().count() < NAME_MAX {
                    self.text.push(c);
                }
                Edited::Editing
            }
            EditKey::Backspace => {
                if self.selected {
                    self.text.clear();
                    self.selected = false;
                } else {
                    self.text.pop();
                }
                Edited::Editing
            }
            EditKey::Enter => {
                let name = juicebar_core::snapshot::clean_name(&self.text);
                // The catalog's own name is no name at all: the device keeps following the catalog.
                if name == self.default_name {
                    Edited::Save(String::new())
                } else {
                    Edited::Save(name)
                }
            }
            EditKey::Escape => Edited::Cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename() -> Rename {
        Rename::new(DeviceKey("logitech-hidpp:4086".into()), "G703", "G703")
    }

    fn type_all(r: &mut Rename, s: &str) {
        for c in s.chars() {
            assert_eq!(r.key(EditKey::Char(c)), Edited::Editing);
        }
    }

    #[test]
    fn the_current_name_starts_selected_and_the_first_key_replaces_it() {
        let mut r = rename();
        assert!(r.selected && r.text == "G703");
        type_all(&mut r, "Mouse");
        assert_eq!(r.text, "Mouse");
        assert!(!r.selected);
        assert_eq!(r.key(EditKey::Enter), Edited::Save("Mouse".into()));
    }

    #[test]
    fn backspace_clears_a_selection_then_erases_one_character_at_a_time() {
        let mut r = rename();
        r.key(EditKey::Backspace);
        assert_eq!(r.text, "");
        assert!(!r.selected);
        type_all(&mut r, "abc");
        r.key(EditKey::Backspace);
        assert_eq!(r.text, "ab");
        r.key(EditKey::Backspace);
        r.key(EditKey::Backspace);
        r.key(EditKey::Backspace);
        assert_eq!(r.text, "", "nothing left to erase is not an error");
    }

    #[test]
    fn the_name_stops_at_24_characters() {
        let mut r = rename();
        type_all(&mut r, &"x".repeat(30));
        assert_eq!(r.text.chars().count(), NAME_MAX);
        // Accents count as one character each.
        let mut r = rename();
        type_all(&mut r, &"é".repeat(30));
        assert_eq!(r.text.chars().count(), NAME_MAX);
        // Erasing makes room again.
        r.key(EditKey::Backspace);
        r.key(EditKey::Char('z'));
        assert!(r.text.ends_with('z'));
    }

    #[test]
    fn control_characters_never_enter_the_text() {
        let mut r = rename();
        r.key(EditKey::Char('\u{7}'));
        assert!(
            r.selected && r.text == "G703",
            "a control key changes nothing"
        );
        r.key(EditKey::Char('a'));
        r.key(EditKey::Char('\n'));
        r.key(EditKey::Char('\t'));
        assert_eq!(r.text, "a");
    }

    #[test]
    fn an_empty_name_goes_back_to_the_catalogs() {
        let mut r = rename();
        r.key(EditKey::Backspace);
        assert_eq!(r.key(EditKey::Enter), Edited::Save(String::new()));
        // Only spaces is empty too.
        let mut r = rename();
        type_all(&mut r, "   ");
        assert_eq!(r.key(EditKey::Enter), Edited::Save(String::new()));
        // Typing the catalog's own name back is the same.
        let mut r = rename();
        type_all(&mut r, "G703");
        assert_eq!(r.key(EditKey::Enter), Edited::Save(String::new()));
    }

    #[test]
    fn enter_on_the_untouched_name_keeps_following_the_catalog_and_a_custom_one_stays() {
        let mut r = rename();
        assert_eq!(r.key(EditKey::Enter), Edited::Save(String::new()));
        let mut r = Rename::new(DeviceKey("k".into()), "Mouse da mesa", "G703");
        assert_eq!(r.key(EditKey::Enter), Edited::Save("Mouse da mesa".into()));
    }

    #[test]
    fn spaces_around_a_name_are_trimmed_and_escape_cancels() {
        let mut r = rename();
        type_all(&mut r, "  Mouse  ");
        assert_eq!(r.key(EditKey::Enter), Edited::Save("Mouse".into()));
        assert_eq!(r.key(EditKey::Escape), Edited::Cancel);
    }
}
