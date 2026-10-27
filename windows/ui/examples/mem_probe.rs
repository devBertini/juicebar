// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Measures what the flyout costs the process: private bytes, working set and the graphics
//! modules loaded, at idle (tray marks drawn), with the flyout open and after it is destroyed.
//!
//! `cargo run --release -p juicebar-ui --example mem_probe -- [--cycles N] [--hold-ms MS]
//! [--no-marks] [--hold-gfx] [--all-modules] [--settings]`
//!
//! Each phase prints `phase private=<MiB> ws=<MiB> modules=<n> gfx=[...]`, where `gfx` lists the
//! loaded graphics modules (Direct3D, DXGI, DirectComposition, Direct2D, DirectWrite, WIC, GPU
//! drivers, overlay hooks), or every module with `--all-modules`. `--settings` opens the Settings page instead of
//! the device list. "trimmed" lines follow
//! `EmptyWorkingSet`, which the app calls after closing the flyout. `--hold-gfx` keeps one
//! Direct2D factory (and so `d3d11.dll`) loaded for the whole run, which shows what an activation
//! with Direct3D loaded costs.

use std::time::{Duration, Instant};

use juicebar_ui::diag;
use juicebar_ui::flyout::{Flyout, Handlers};
use juicebar_ui::gfx::{Color, Gfx, render_offscreen};
use juicebar_ui::icon::{Frame, render_mark};
use juicebar_ui::layout::PxRect;
use juicebar_ui::model::{IconState, Theme, UiState};
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

const GFX: &[&str] = &[
    "d3d",
    "dxgi",
    "dcomp",
    "d2d1",
    "dwrite",
    "windowscodecs",
    "nvwgf",
    "nvldumd",
    "nvgpucomp",
    "igd",
    "igc",
    "rtss",
];

fn mib(b: usize) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn report(phase: &str, all: bool) {
    let m = diag::memory();
    let mods = diag::loaded_modules();
    let shown: Vec<&String> = mods
        .iter()
        .filter(|m| {
            let l = m.to_ascii_lowercase();
            all || GFX.iter().any(|g| l.contains(g))
        })
        .collect();
    println!(
        "{phase:<20} private={:7.2} ws={:7.2} modules={:3} gfx={shown:?}",
        mib(m.private),
        mib(m.working_set),
        mods.len()
    );
}

/// Dispatches this thread's messages for `ms` milliseconds.
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    while Instant::now() < end {
        // SAFETY: the standard non-blocking message loop on this thread.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            Sleep(10);
        }
    }
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let cycles = value("--cycles").unwrap_or(3);
    let hold = value("--hold-ms").unwrap_or(1500);
    let all = flag("--all-modules");
    juicebar_ui::flyout::ensure_dpi_awareness();
    report("start", all);
    if !flag("--no-marks") {
        for st in [IconState::Normal, IconState::Warning, IconState::Charging] {
            let icon = render_mark(st.into(), &Frame::STILL, true, 24)?;
            // SAFETY: the icon was just made and is not used elsewhere.
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
    diag::trim_working_set();
    report("idle", all);
    let _held = if flag("--hold-gfx") {
        let g = Gfx::new()?;
        render_offscreen(&g, 8, 8, 1.0, |c| {
            c.clear(Color::hex(0));
            Ok(())
        })?;
        Some(g)
    } else {
        None
    };
    let anchor = PxRect {
        left: 1000,
        top: 1050,
        right: 1024,
        bottom: 1080,
    };
    for i in 1..=cycles {
        let flyout = Flyout::show(
            if flag("--settings") {
                UiState::sample_settings(Theme::Dark)
            } else {
                UiState::sample(Theme::Dark)
            },
            anchor,
            Handlers {
                on_hit: Box::new(|_| {}),
                on_close: Box::new(|| {}),
            },
        )?;
        flyout.set_hide_on_deactivate(false);
        pump(hold);
        report(&format!("open #{i}"), all);
        drop(flyout);
        pump(500);
        report(&format!("closed #{i}"), all);
        diag::trim_working_set();
        report(&format!("closed+trimmed #{i}"), all);
    }
    Ok(())
}
