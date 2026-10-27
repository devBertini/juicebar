// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The tray's context menu: its texts in both languages and the Win32 popup.
//!
//! The core's string table (`spec/strings.tsv`) has no menu ids yet; until it does these live here.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MF_CHECKED, MF_DISABLED, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, SetForegroundWindow, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCSTR, w};

use juicebar_core::strings::Lang;

/// What the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    Autostart,
    Quit,
}

const ID_OPEN: usize = 1;
const ID_AUTOSTART: usize = 2;
const ID_HOTKEY: usize = 3;
const ID_QUIT: usize = 4;

impl Command {
    fn from_id(id: usize) -> Option<Command> {
        match id {
            ID_OPEN => Some(Command::Open),
            ID_AUTOSTART => Some(Command::Autostart),
            ID_QUIT => Some(Command::Quit),
            _ => None,
        }
    }
}

/// The visible texts of the menu and of the empty flyout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuText {
    pub open: &'static str,
    pub autostart: &'static str,
    pub hotkey_unavailable: String,
    pub quit: &'static str,
    /// The flyout's text while no device is known yet.
    pub empty: &'static str,
}

pub fn texts(lang: Lang, hotkey: &str) -> MenuText {
    match lang {
        Lang::PtBr => MenuText {
            open: "Abrir",
            autostart: "Iniciar com o Windows",
            hotkey_unavailable: format!("Atalho {hotkey} indisponível"),
            quit: "Sair",
            empty: "Nenhum dispositivo ainda
Ligue um mouse, teclado ou fone compatível",
        },
        Lang::En => MenuText {
            open: "Open",
            autostart: "Start with Windows",
            hotkey_unavailable: format!("Shortcut {hotkey} unavailable"),
            quit: "Quit",
            empty: "No devices yet
Turn on a compatible mouse, keyboard or headset",
        },
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
pub fn show(
    owner: HWND,
    at: POINT,
    lang: Lang,
    hotkey: &str,
    autostart: bool,
    hotkey_ok: bool,
) -> Option<Command> {
    let t = texts(lang, hotkey);
    // SAFETY: creating a popup menu that is destroyed before returning.
    let menu: HMENU = unsafe { CreatePopupMenu() }.ok()?;
    let append = |flags, id: usize, text: &str| {
        let s = HSTRING::from(text);
        // SAFETY: `menu` is valid; the text is copied by the call.
        unsafe {
            let _ = AppendMenuW(menu, flags, id, &s);
        }
    };
    append(MF_STRING, ID_OPEN, t.open);
    append(
        MF_STRING | if autostart { MF_CHECKED } else { MF_UNCHECKED },
        ID_AUTOSTART,
        t.autostart,
    );
    if !hotkey_ok {
        append(
            MF_STRING | MF_GRAYED | MF_DISABLED,
            ID_HOTKEY,
            &t.hotkey_unavailable,
        );
    }
    // SAFETY: a separator takes no text.
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    }
    append(MF_STRING, ID_QUIT, t.quit);
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
    fn menu_texts_exist_in_both_languages() {
        let pt = texts(Lang::PtBr, "Ctrl+Alt+B");
        assert_eq!(pt.open, "Abrir");
        assert_eq!(pt.autostart, "Iniciar com o Windows");
        assert_eq!(pt.quit, "Sair");
        assert_eq!(pt.hotkey_unavailable, "Atalho Ctrl+Alt+B indisponível");
        let en = texts(Lang::En, "Ctrl+Alt+B");
        assert_eq!(en.open, "Open");
        assert_eq!(en.autostart, "Start with Windows");
        assert_eq!(en.quit, "Quit");
        assert_eq!(en.hotkey_unavailable, "Shortcut Ctrl+Alt+B unavailable");
    }

    #[test]
    fn no_text_is_empty_or_uses_an_em_dash() {
        for lang in [Lang::PtBr, Lang::En] {
            let t = texts(lang, "Ctrl+Alt+B");
            for s in [
                t.open,
                t.autostart,
                t.quit,
                t.empty,
                t.hotkey_unavailable.as_str(),
            ] {
                assert!(!s.is_empty() && !s.contains('\u{2014}'), "{s:?}");
            }
        }
    }

    #[test]
    fn ids_map_to_commands_and_the_disabled_hotkey_item_to_none() {
        assert_eq!(Command::from_id(ID_OPEN), Some(Command::Open));
        assert_eq!(Command::from_id(ID_AUTOSTART), Some(Command::Autostart));
        assert_eq!(Command::from_id(ID_QUIT), Some(Command::Quit));
        assert_eq!(Command::from_id(ID_HOTKEY), None);
        assert_eq!(Command::from_id(0), None);
    }
}
