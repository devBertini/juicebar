// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The tray's context menu: the Win32 popup, with its texts from the core's table (`spec/strings.tsv`).
//! It holds only Abrir, Configurações and Sair; everything else lives on the Settings page.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MF_SEPARATOR, MF_STRING, PostMessageW,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCSTR, w};

use juicebar_core::strings::{self, Lang, id};

/// What the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    /// Open the panel on the Settings page.
    Settings,
    Quit,
}

const ID_OPEN: usize = 1;
const ID_SETTINGS: usize = 2;
const ID_QUIT: usize = 3;

impl Command {
    fn from_id(id: usize) -> Option<Command> {
        match id {
            ID_OPEN => Some(Command::Open),
            ID_SETTINGS => Some(Command::Settings),
            ID_QUIT => Some(Command::Quit),
            _ => None,
        }
    }
}

/// The visible texts of the menu and of the empty flyout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuText {
    pub open: &'static str,
    pub settings: &'static str,
    pub quit: &'static str,
    /// The flyout's text while no device is known yet: a title line and a hint line.
    pub empty: String,
}

pub fn texts(lang: Lang) -> MenuText {
    MenuText {
        open: strings::get(lang, id::MENU_OPEN),
        settings: strings::get(lang, id::MENU_SETTINGS),
        quit: strings::get(lang, id::MENU_QUIT),
        empty: format!(
            "{}\n{}",
            strings::get(lang, id::EMPTY_TITLE),
            strings::get(lang, id::EMPTY_HINT)
        ),
    }
}

/// Makes the popup menu dark or light to match the user's app mode. Win32 menus have no documented
/// switch for it; this is `SetPreferredAppMode` (`ForceDark` or `Default`) and `FlushMenuThemes`,
/// uxtheme's exports by ordinal (135 and 136, present since Windows 10 1903). When they are missing
/// the menu stays light.
pub fn style_menus(dark: bool) {
    type SetMode = unsafe extern "system" fn(i32) -> i32;
    type Flush = unsafe extern "system" fn();
    // SAFETY: loads a system DLL and calls two of its exports with the signatures they have; a
    // missing export is skipped.
    unsafe {
        let Ok(lib) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        if let Some(f) = GetProcAddress(lib, PCSTR(135 as *const u8)) {
            let set: SetMode = std::mem::transmute::<_, SetMode>(f);
            set(if dark { 2 } else { 0 });
        }
        if let Some(f) = GetProcAddress(lib, PCSTR(136 as *const u8)) {
            let flush: Flush = std::mem::transmute::<_, Flush>(f);
            flush();
        }
    }
}

/// Shows the menu at `at` (screen pixels) and returns the command picked, if any.
pub fn show(owner: HWND, at: POINT, lang: Lang) -> Option<Command> {
    let t = texts(lang);
    // SAFETY: creating a popup menu that is destroyed before returning.
    let menu: HMENU = unsafe { CreatePopupMenu() }.ok()?;
    let append = |id: usize, text: &str| {
        let s = HSTRING::from(text);
        // SAFETY: `menu` is valid; the text is copied by the call.
        unsafe {
            let _ = AppendMenuW(menu, MF_STRING, id, &s);
        }
    };
    append(ID_OPEN, t.open);
    append(ID_SETTINGS, t.settings);
    // SAFETY: a separator takes no text.
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    }
    append(ID_QUIT, t.quit);
    // SAFETY: the owner window is ours; it must be foreground so the menu closes on an outside click.
    let picked = unsafe {
        let _ = SetForegroundWindow(owner);
        let r = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_NONOTIFY,
            at.x,
            at.y,
            None,
            owner,
            None,
        );
        // The documented fix for a popup that does not close when the user clicks away.
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        r.0 as usize
    };
    Command::from_id(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_holds_only_open_settings_and_quit_in_both_languages() {
        let pt = texts(Lang::PtBr);
        assert_eq!(
            (pt.open, pt.settings, pt.quit),
            ("Abrir", "Configurações", "Sair")
        );
        let en = texts(Lang::En);
        assert_eq!(
            (en.open, en.settings, en.quit),
            ("Open", "Settings", "Quit")
        );
        assert_eq!(
            pt.empty,
            "Nenhum dispositivo ainda\nLigue um mouse, teclado ou fone compatível"
        );
        assert_eq!(
            en.empty,
            "No devices yet\nTurn on a compatible mouse, keyboard or headset"
        );
    }

    #[test]
    fn no_text_is_empty_or_uses_an_em_dash() {
        for lang in [Lang::PtBr, Lang::En] {
            let t = texts(lang);
            for s in [t.open, t.settings, t.quit, t.empty.as_str()] {
                assert!(!s.is_empty() && !s.contains('\u{2014}'), "{s:?}");
            }
        }
    }

    #[test]
    fn ids_map_to_the_three_commands_and_nothing_else() {
        assert_eq!(Command::from_id(ID_OPEN), Some(Command::Open));
        assert_eq!(Command::from_id(ID_SETTINGS), Some(Command::Settings));
        assert_eq!(Command::from_id(ID_QUIT), Some(Command::Quit));
        assert_eq!(Command::from_id(0), None);
        assert_eq!(Command::from_id(4), None);
        assert_eq!(Command::from_id(100), None, "no device submenu any more");
    }
}
