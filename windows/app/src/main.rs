// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! JuiceBar for Windows: one tray icon, a flyout above it, notices, the Ctrl+Alt+B hotkey.
//! W5a is the shell, fed by a demo source; the devices are wired in W5b.
#![windows_subsystem = "windows"]

mod app;
mod args;
mod menu;
mod notify;
mod runkey;
mod single;
mod source;
mod tray;
mod view;

fn main() {
    let args = args::Args::parse(std::env::args().skip(1));
    let user = single::current_user();
    let _instance = match single::acquire(&user) {
        single::Acquire::First(i) => i,
        single::Acquire::Second => {
            single::notify_first(&user);
            return;
        }
    };
    if let Err(e) = app::run(args, &user) {
        // A windowed app has no console; the failure is visible as a missing icon, so say why in the debugger.
        let msg = windows::core::HSTRING::from(format!("JuiceBar: {e}\n"));
        // SAFETY: a debug-output string that lives through the call.
        unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(&msg) };
    }
}
