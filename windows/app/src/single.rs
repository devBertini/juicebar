// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! One running JuiceBar per user session: a named mutex, and a registered window message a second
//! launch sends to the first so that it opens its flyout.

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
    RegisterWindowMessageW,
};
use windows::core::{HSTRING, PCWSTR, w};

/// The class of the hidden main window, which the second launch looks for.
pub const WINDOW_CLASS: PCWSTR = w!("JuiceBar.Main");

/// The signed-in user's name, or a fixed word when Windows does not say.
pub fn current_user() -> String {
    std::env::var("USERNAME")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "user".to_string())
}

/// Keeps letters, digits, `-` and `_`; everything else (backslash included) becomes `_`.
fn sanitize(user: &str) -> String {
    user.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `Local\JuiceBar.<user>`: `Local\` is per logon session, so two users never meet.
pub fn mutex_name(user: &str) -> String {
    format!("Local\\JuiceBar.{}", sanitize(user))
}

/// The registered message that asks the running instance to open its flyout.
pub fn open_message_name(user: &str) -> String {
    format!("JuiceBar.OpenFlyout.{}", sanitize(user))
}

/// Registers (or looks up) the open-flyout message.
pub fn open_message(user: &str) -> u32 {
    let name = HSTRING::from(open_message_name(user));
    // SAFETY: registering a message by name; the string outlives the call.
    unsafe { RegisterWindowMessageW(&name) }
}

/// The first instance owns the mutex until this guard is dropped.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed once here.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub enum Acquire {
    First(Instance),
    Second,
}

pub fn acquire(user: &str) -> Acquire {
    let name = HSTRING::from(mutex_name(user));
    // SAFETY: a named mutex with default security; the name outlives the call.
    let created = unsafe { CreateMutexW(None, false, &name) };
    // SAFETY: read right after the call, on the same thread.
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    match created {
        Ok(h) if !already => Acquire::First(Instance(h)),
        Ok(h) => {
            // SAFETY: the handle is ours and unused.
            unsafe {
                let _ = CloseHandle(h);
            }
            Acquire::Second
        }
        // No mutex: run anyway rather than refuse to start.
        Err(_) => Acquire::First(Instance(HANDLE::default())),
    }
}

/// The second launch: ask the first to open its flyout (waits up to two seconds for its window,
/// in case it is still starting).
pub fn notify_first(user: &str) {
    let msg = open_message(user);
    for _ in 0..20 {
        // SAFETY: a plain window lookup by class name.
        if let Ok(hwnd) = unsafe { FindWindowW(WINDOW_CLASS, None) } {
            let mut pid = 0u32;
            // SAFETY: `pid` is a valid out pointer; the permission lets the first instance take the
            // foreground, which this fresh launch has.
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_per_user_and_safe() {
        assert_eq!(mutex_name("Bertini"), "Local\\JuiceBar.Bertini");
        assert_eq!(
            mutex_name("DOMAIN\\ana maria"),
            "Local\\JuiceBar.DOMAIN_ana_maria"
        );
        assert_eq!(open_message_name("Bertini"), "JuiceBar.OpenFlyout.Bertini");
        assert!(!open_message_name("a\\b").contains('\\'));
    }

    #[test]
    fn the_open_message_is_a_registered_one() {
        assert!(open_message("JuiceBarUnitTest") >= 0xC000);
        assert_eq!(
            open_message("JuiceBarUnitTest"),
            open_message("JuiceBarUnitTest")
        );
    }

    #[test]
    fn a_second_acquire_of_the_same_name_is_refused() {
        let user = "JuiceBarUnitTest";
        let first = acquire(user);
        assert!(matches!(first, Acquire::First(_)));
        assert!(matches!(acquire(user), Acquire::Second));
        drop(first);
        assert!(matches!(acquire(user), Acquire::First(_)));
    }
}
