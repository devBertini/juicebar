// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! JuiceBar for Windows: one tray icon, a flyout above it, notices, the Ctrl+Alt+B hotkey.
//! The shell is fed by the real devices through the core (`live`), or by a demo source.
#![windows_subsystem = "windows"]

mod app;
mod args;
mod controllers;
mod files;
mod live;
mod menu;
mod notify;
mod runkey;
mod settings;
mod single;
mod source;
mod tray;
mod view;
mod winhost;

fn main() {
    let args = args::Args::parse(std::env::args().skip(1));
    let user = single::current_user();
    // A run with its own data folder is a test: it gets its own single-instance name, so it never meets the owner's copy.
    let user = if args.data.is_some() {
        format!("{user}-data")
    } else {
        user
    };
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
