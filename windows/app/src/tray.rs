// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The notification-area icon: `Shell_NotifyIconW` with `NOTIFYICON_VERSION_4`, one icon with
//! `uID` 1. The mark is re-rendered only when its look, the taskbar theme or the icon size changes.
//! The animation (`anim`) puts its own pictures on the icon with `show_frame` and `show_rest`.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_LARGE_ICON, NIIF_USER, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFY_ICON_DATA_FLAGS, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, NOTIFYICONDATAW_0, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, FindWindowW, HICON, SM_CXICON, SM_CXSMICON,
};
use windows::core::{Result, w};

use juicebar_ui::icon::{Frame, IconLook, render_mark};
use juicebar_ui::layout::PxRect;

use crate::notify::Notice;

/// The one icon's id.
pub const ICON_ID: u32 = 1;

/// What decides how the mark looks; the icon is rebuilt only when this changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconKey {
    pub look: IconLook,
    pub dark_taskbar: bool,
    pub px: u32,
}

/// Copies `s` into a fixed UTF-16 buffer, cutting on a character boundary and leaving room for the
/// terminating NUL; the rest of the buffer is zeroed.
pub fn fill(dst: &mut [u16], s: &str) {
    dst.fill(0);
    let max = dst.len() - 1;
    let mut n = 0;
    for c in s.chars() {
        let len = c.len_utf16();
        if n + len > max {
            break;
        }
        c.encode_utf16(&mut dst[n..]);
        n += len;
    }
}

/// The DPI of the taskbar (the primary monitor's), which is what sizes the icon.
pub fn taskbar_dpi() -> u32 {
    // SAFETY: a plain lookup of the shell's window by class name, then a DPI query on it.
    unsafe {
        if let Ok(h) = FindWindowW(w!("Shell_TrayWnd"), None) {
            let d = GetDpiForWindow(h);
            if d != 0 {
                return d;
            }
        }
        GetDpiForSystem()
    }
}

/// Pixel size of a tray icon at `dpi`.
pub fn small_icon_px(dpi: u32) -> u32 {
    // SAFETY: a metrics query.
    (unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }).max(16) as u32
}

/// Pixel size of a notification's large icon at `dpi`.
pub fn large_icon_px(dpi: u32) -> u32 {
    // SAFETY: a metrics query.
    (unsafe { GetSystemMetricsForDpi(SM_CXICON, dpi) }).max(32) as u32
}

pub struct Tray {
    hwnd: HWND,
    callback: u32,
    added: bool,
    key: Option<IconKey>,
    icon: HICON,
    tip: String,
}

impl Tray {
    pub fn new(hwnd: HWND, callback: u32) -> Tray {
        Tray {
            hwnd,
            callback,
            added: false,
            key: None,
            icon: HICON::default(),
            tip: String::new(),
        }
    }

    /// Gives the tray the window it belongs to (the window is created after the tray).
    pub fn bind(&mut self, hwnd: HWND) {
        self.hwnd = hwnd;
    }

    fn data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            uFlags: flags,
            ..Default::default()
        }
    }

    /// Adds the icon (or adds it again after Explorer restarted).
    pub fn add(&mut self, key: IconKey, tip: &str) -> Result<()> {
        let new_icon = render_mark(key.look, &Frame::STILL, key.dark_taskbar, key.px)?;
        let mut nid = self.data(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        nid.uCallbackMessage = self.callback;
        nid.hIcon = new_icon;
        fill(&mut nid.szTip, tip);
        // SAFETY: `nid` is fully initialised with our window and icon.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) }.as_bool();
        if !added {
            // SAFETY: the icon is ours and was not handed over.
            unsafe {
                let _ = DestroyIcon(new_icon);
            }
            return Err(windows::core::Error::from_thread());
        }
        let mut ver = self.data(NOTIFY_ICON_DATA_FLAGS(0));
        ver.Anonymous = NOTIFYICONDATAW_0 {
            uVersion: NOTIFYICON_VERSION_4,
        };
        // SAFETY: as above; the version must be set after every NIM_ADD.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &ver);
        }
        self.swap_icon(new_icon);
        self.added = true;
        self.key = Some(key);
        self.tip = tip.to_string();
        Ok(())
    }

    fn swap_icon(&mut self, new_icon: HICON) {
        let old = std::mem::replace(&mut self.icon, new_icon);
        if !old.is_invalid() {
            // SAFETY: the shell has the new icon already; the old one is ours to free.
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
    }

    /// Brings the icon and tooltip up to date; renders the mark only if its key changed.
    pub fn update(&mut self, key: IconKey, tip: &str) -> Result<()> {
        if !self.added {
            return self.add(key, tip);
        }
        let icon_changed = self.key != Some(key);
        let tip_changed = self.tip != tip;
        if !icon_changed && !tip_changed {
            return Ok(());
        }
        let mut flags = NIF_TIP | NIF_SHOWTIP;
        let mut new_icon = None;
        if icon_changed {
            let icon = render_mark(key.look, &Frame::STILL, key.dark_taskbar, key.px)?;
            flags |= NIF_ICON;
            new_icon = Some(icon);
        }
        let mut nid = self.data(flags);
        if let Some(i) = new_icon {
            nid.hIcon = i;
        }
        fill(&mut nid.szTip, tip);
        // SAFETY: `nid` is initialised for our own icon.
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) }.as_bool();
        if !ok {
            if let Some(i) = new_icon {
                // SAFETY: not handed over, so still ours.
                unsafe {
                    let _ = DestroyIcon(i);
                }
            }
            // The icon is gone (Explorer restarted without telling us): add it afresh next time.
            self.added = false;
            return self.add(key, tip);
        }
        if let Some(i) = new_icon {
            self.swap_icon(i);
            self.key = Some(key);
        }
        self.tip = tip.to_string();
        Ok(())
    }

    /// Shows `icon`, one of the animation's pictures, on the icon. The animation keeps the picture
    /// alive; the icon the tray owns is not touched. False when the shell refused it.
    pub fn show_frame(&mut self, icon: HICON) -> bool {
        if !self.added {
            return false;
        }
        let mut nid = self.data(NIF_ICON);
        nid.hIcon = icon;
        // SAFETY: `nid` is initialised for our own icon; the shell takes its own copy of the picture.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) }.as_bool()
    }

    /// Puts the icon the tray owns (the mark at rest) back after an animation.
    pub fn show_rest(&mut self) -> bool {
        let rest = self.icon;
        !rest.is_invalid() && self.show_frame(rest)
    }

    /// Forgets that the icon is in the area (the `TaskbarCreated` message) and adds it again.
    pub fn readd(&mut self, key: IconKey, tip: &str) -> Result<()> {
        self.added = false;
        self.key = None;
        self.add(key, tip)
    }

    /// The icon's rectangle in screen pixels, if the shell can say.
    pub fn rect(&self) -> Option<PxRect> {
        if !self.added {
            return None;
        }
        let id = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        // SAFETY: `id` identifies our own icon.
        let r: RECT = unsafe { Shell_NotifyIconGetRect(&id) }.ok()?;
        (r.right > r.left && r.bottom > r.top).then_some(PxRect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        })
    }

    /// Shows a notice as a balloon (Windows 10 and 11 draw it as a toast), with the mark as its icon.
    pub fn balloon(
        &self,
        notice: &Notice,
        mark: IconLook,
        dark_toast: bool,
        px: u32,
    ) -> Result<()> {
        if !self.added {
            return Ok(());
        }
        let icon = render_mark(mark, &Frame::STILL, dark_toast, px)?;
        let mut nid = self.data(NIF_INFO);
        fill(&mut nid.szInfoTitle, &notice.title);
        fill(&mut nid.szInfo, &notice.body);
        nid.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        nid.hBalloonIcon = icon;
        // SAFETY: `nid` is initialised for our own icon; the shell copies the balloon icon.
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) }.as_bool();
        // SAFETY: the shell has its own copy, so ours can go.
        unsafe {
            let _ = DestroyIcon(icon);
        }
        if ok {
            Ok(())
        } else {
            Err(windows::core::Error::from_thread())
        }
    }

    /// Removes the icon from the area and frees the mark.
    pub fn remove(&mut self) {
        if self.added {
            let nid = self.data(NOTIFY_ICON_DATA_FLAGS(0));
            // SAFETY: deleting our own icon.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }
            self.added = false;
        }
        self.swap_icon(HICON::default());
        self.key = None;
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_cuts_at_the_limit_and_keeps_the_nul() {
        let mut buf = [0u16; 128];
        let long = "x".repeat(300);
        fill(&mut buf, &long);
        assert_eq!(buf[126], b'x' as u16);
        assert_eq!(buf[127], 0);
        let mut small = [9u16; 4];
        fill(&mut small, "ab");
        assert_eq!(small, [b'a' as u16, b'b' as u16, 0, 0]);
    }

    #[test]
    fn fill_never_splits_a_surrogate_pair() {
        let mut buf = [0u16; 4];
        // Three units fit; "ab" + one two-unit character would need four.
        fill(&mut buf, "ab\u{1F9C3}");
        assert_eq!(buf, [b'a' as u16, b'b' as u16, 0, 0]);
        fill(&mut buf, "a\u{1F9C3}");
        assert_eq!(String::from_utf16_lossy(&buf[..3]), "a\u{1F9C3}");
    }

    #[test]
    fn the_core_tooltip_limit_is_the_buffer_minus_one() {
        let limit = juicebar_core::snapshot::TOOLTIP_LIMIT;
        let mut nid = NOTIFYICONDATAW::default();
        assert_eq!(nid.szTip.len(), limit + 1);
        let tip = "é".repeat(200);
        fill(&mut nid.szTip, &tip);
        assert_eq!(nid.szTip.iter().filter(|&&c| c != 0).count(), limit);
    }

    #[test]
    fn icon_sizes_follow_the_dpi() {
        assert_eq!(small_icon_px(96), 16);
        assert_eq!(small_icon_px(144), 24);
        assert!(large_icon_px(96) >= 32);
    }
}
