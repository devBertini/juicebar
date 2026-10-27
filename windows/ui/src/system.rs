// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! What the user's Windows settings say: app theme, taskbar theme, accent color.

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{PCWSTR, w};

use crate::model::{Rgb, Theme};

fn dword(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut len = 4u32;
    // SAFETY: `data` and `len` are valid out parameters for a 4-byte DWORD; the key strings are static.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut len),
        )
    };
    rc.is_ok().then_some(data)
}

const PERSONALIZE: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

/// The "choose your default app mode" setting (the flyout follows it).
pub fn app_theme() -> Theme {
    match dword(PERSONALIZE, w!("AppsUseLightTheme")) {
        Some(0) => Theme::Dark,
        _ => Theme::Light,
    }
}

/// True when the taskbar is dark ("choose your default Windows mode"), so the tray mark is white.
pub fn taskbar_is_dark() -> bool {
    dword(PERSONALIZE, w!("SystemUsesLightTheme")) == Some(0)
}

/// The user's accent color, or the Windows default blue.
pub fn accent_color() -> Rgb {
    match dword(w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor")) {
        // Stored as 0xAABBGGRR.
        Some(v) => Rgb(
            (v & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            ((v >> 16) & 0xff) as u8,
        ),
        None => Rgb(0x00, 0x67, 0xc0),
    }
}

/// The accent as Windows' own dark surfaces use it: a dark accent is lightened so the switch, the
/// chart and the glyph tiles keep their contrast on a dark panel.
pub fn accent_for(theme: Theme, accent: Rgb) -> Rgb {
    if theme == Theme::Light {
        return accent;
    }
    let lum = crate::gfx::Color::hex(
        ((accent.0 as u32) << 16) | ((accent.1 as u32) << 8) | accent.2 as u32,
    )
    .luminance();
    if lum >= 0.3 {
        return accent;
    }
    let mix = |c: u8| (c as f32 + (255.0 - c as f32) * 0.45).round() as u8;
    Rgb(mix(accent.0), mix(accent.1), mix(accent.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_reads_do_not_panic_and_give_valid_values() {
        let _ = app_theme();
        let _ = taskbar_is_dark();
        let _ = accent_color();
    }

    #[test]
    fn dark_theme_lightens_a_dark_accent_only() {
        let deep = Rgb(0x00, 0x67, 0xc0);
        assert_eq!(accent_for(Theme::Light, deep), deep);
        let lifted = accent_for(Theme::Dark, deep);
        assert!(lifted.0 > deep.0 && lifted.2 > deep.2);
        let light = Rgb(0x60, 0xcd, 0xff);
        assert_eq!(accent_for(Theme::Dark, light), light);
    }
}
