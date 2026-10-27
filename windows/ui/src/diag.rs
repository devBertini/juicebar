// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! What the process costs: private bytes, working set and the loaded modules. Used by the memory
//! test and `examples/mem_probe.rs`.

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::ProcessStatus::{
    K32EmptyWorkingSet, K32EnumProcessModules, K32GetModuleBaseNameW, K32GetProcessMemoryInfo,
    PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;

/// Private bytes (committed memory only this process uses) and working set, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub private: usize,
    pub working_set: usize,
}

/// This process's memory counters (zeros if the system refuses).
pub fn memory() -> Memory {
    let mut c = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: `c` is a PROCESS_MEMORY_COUNTERS_EX whose size is passed; its layout starts with
    // PROCESS_MEMORY_COUNTERS, as the API expects.
    let _ = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            c.cb,
        )
    };
    Memory {
        private: c.PrivateUsage,
        working_set: c.WorkingSetSize,
    }
}

/// Base names of the modules loaded in this process ("d2d1.dll", ...).
pub fn loaded_modules() -> Vec<String> {
    let mut mods = vec![HMODULE::default(); 1024];
    let mut needed = 0u32;
    // SAFETY: `mods` holds 1024 handles and the byte size passed matches it.
    let ok = unsafe {
        K32EnumProcessModules(
            GetCurrentProcess(),
            mods.as_mut_ptr(),
            (mods.len() * std::mem::size_of::<HMODULE>()) as u32,
            &mut needed,
        )
    };
    if !ok.as_bool() {
        return Vec::new();
    }
    let n = (needed as usize / std::mem::size_of::<HMODULE>()).min(mods.len());
    mods[..n]
        .iter()
        .map(|m| {
            let mut buf = [0u16; 260];
            // SAFETY: `buf` is a valid buffer of the length passed.
            let len = unsafe { K32GetModuleBaseNameW(GetCurrentProcess(), Some(*m), &mut buf) };
            String::from_utf16_lossy(&buf[..len as usize])
        })
        .collect()
}

/// True when a module whose name starts with `prefix` (any case) is loaded.
pub fn is_loaded(prefix: &str) -> bool {
    let p = prefix.to_ascii_lowercase();
    loaded_modules()
        .iter()
        .any(|m| m.to_ascii_lowercase().starts_with(&p))
}

/// Hands this process's working set back to the system (pages come back on use).
pub fn trim_working_set() {
    // SAFETY: trims this process's own working set; failure changes nothing.
    let _ = unsafe { K32EmptyWorkingSet(GetCurrentProcess()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_modules_are_read() {
        let m = memory();
        assert!(m.private > 0 && m.working_set > 0, "{m:?}");
        assert!(is_loaded("kernel32"));
        assert!(!is_loaded("no-such-module"));
    }
}
