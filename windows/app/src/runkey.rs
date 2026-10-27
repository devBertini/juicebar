// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Start with Windows: the value `JuiceBar` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! holds the quoted path of the exe. The menu's check reads and writes it.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
/// The value name of the real app.
pub const VALUE: &str = "JuiceBar";

/// The command line Windows runs at sign-in: the path in quotes, so spaces survive.
pub fn quote_path(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// The stored command line, if the value exists.
pub fn read(value: &str) -> Option<String> {
    let name = HSTRING::from(value);
    let mut bytes = 0u32;
    // SAFETY: asks only for the size of the string value.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            &name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if rc != ERROR_SUCCESS || bytes < 2 {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    // SAFETY: `buf` holds `bytes` bytes, as the size query said.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut bytes),
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn is_enabled(value: &str) -> bool {
    read(value).is_some()
}

/// Writes (`on`) or removes (`!on`) the value.
pub fn set_enabled(value: &str, exe: &Path, on: bool) -> Result<()> {
    let name = HSTRING::from(value);
    if on {
        let data: Vec<u16> = quote_path(exe)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `data` is a NUL-terminated UTF-16 string and its byte length is passed.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                &name,
                REG_SZ.0,
                Some(data.as_ptr() as *const _),
                (data.len() * 2) as u32,
            )
            .ok()
        }
    } else {
        // SAFETY: deleting one value under a key of the current user.
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, &name) };
        // A value that is already gone is the wanted state.
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            rc.ok()
        }
    }
}

/// When the value exists but points elsewhere (the exe moved), rewrite it.
pub fn refresh(value: &str, exe: &Path) {
    if let Some(stored) = read(value)
        && stored != quote_path(exe)
    {
        let _ = set_enabled(value, exe, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_path_is_quoted_even_with_spaces() {
        let p = PathBuf::from(r"C:\Users\Ana Maria\Apps\JuiceBar.exe");
        assert_eq!(
            quote_path(&p),
            "\"C:\\Users\\Ana Maria\\Apps\\JuiceBar.exe\""
        );
    }

    #[test]
    fn writing_reading_and_deleting_a_test_value() {
        let exe = PathBuf::from(r"C:\Test Folder\JuiceBar.exe");
        let name = "JuiceBarTest";
        set_enabled(name, &exe, false).unwrap();
        assert!(!is_enabled(name));
        set_enabled(name, &exe, true).unwrap();
        assert!(is_enabled(name));
        assert_eq!(read(name).as_deref(), Some(quote_path(&exe).as_str()));
        let moved = PathBuf::from(r"D:\Moved\JuiceBar.exe");
        refresh(name, &moved);
        assert_eq!(read(name).as_deref(), Some(quote_path(&moved).as_str()));
        set_enabled(name, &exe, false).unwrap();
        assert!(!is_enabled(name));
        // Deleting what is not there is fine.
        set_enabled(name, &exe, false).unwrap();
    }
}
