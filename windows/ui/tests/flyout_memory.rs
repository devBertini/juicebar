// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening and closing the flyout leaves the process near its idle cost: no Direct3D, DXGI,
//! DirectComposition, Direct2D or WARP module stays loaded, and private memory comes back.
//! Its own test binary, so no other test holds Direct2D while it measures.

use std::time::{Duration, Instant};

use juicebar_ui::diag;
use juicebar_ui::flyout::{Flyout, Handlers};
use juicebar_ui::icon::{Frame, render_mark};
use juicebar_ui::layout::PxRect;
use juicebar_ui::model::{IconState, Theme, UiState};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

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
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

const GRAPHICS: &[&str] = &["d2d1", "d3d11", "d3d10warp", "dxgi", "dcomp"];

fn loaded_graphics() -> Vec<String> {
    diag::loaded_modules()
        .into_iter()
        .filter(|m| {
            let l = m.to_ascii_lowercase();
            GRAPHICS.iter().any(|g| l.starts_with(g))
        })
        .collect()
}

#[test]
fn closing_the_flyout_unloads_the_graphics_and_gives_the_memory_back() {
    juicebar_ui::flyout::ensure_dpi_awareness();
    // The app draws its tray marks first.
    for st in [IconState::Normal, IconState::Warning, IconState::Charging] {
        let icon = render_mark(st.into(), &Frame::STILL, true, 24).unwrap();
        // SAFETY: the icon was just made and is not used elsewhere.
        unsafe {
            let _ = DestroyIcon(icon);
        }
    }
    assert_eq!(loaded_graphics(), Vec::<String>::new(), "after the marks");
    let idle = diag::memory().private;
    let anchor = PxRect {
        left: 1000,
        top: 1050,
        right: 1024,
        bottom: 1080,
    };
    // Twice the device list (the second activation is the one that found Direct3D still loaded before), then the
    // Settings page, which is taller and draws more.
    for (round, settings) in [(1, false), (2, false), (3, true)] {
        let state = if settings {
            UiState::sample_settings(Theme::Dark)
        } else {
            UiState::sample(Theme::Dark)
        };
        let f = Flyout::show(
            state,
            anchor,
            Handlers {
                on_hit: Box::new(|_| {}),
                on_close: Box::new(|| {}),
            },
        )
        .unwrap();
        f.set_hide_on_deactivate(false);
        pump(300);
        let open = diag::memory().private;
        let drivers: Vec<String> = diag::loaded_modules()
            .into_iter()
            .filter(|m| {
                let l = m.to_ascii_lowercase();
                l.starts_with("nvwgf") || l.starts_with("nvldumd") || l.starts_with("igd")
            })
            .collect();
        assert_eq!(drivers, Vec::<String>::new(), "no GPU driver while open");
        assert!(diag::is_loaded("d2d1"), "Direct2D is loaded while open");
        drop(f);
        pump(300);
        let closed = diag::memory().private;
        assert_eq!(loaded_graphics(), Vec::<String>::new(), "round {round}");
        // The GPU path left 60 MiB and more; the CPU path about 2 to 6 MiB over idle.
        let mib = |b: usize| b as f64 / 1048576.0;
        assert!(
            closed < idle + 24 * 1024 * 1024,
            "round {round}: idle {:.1} MiB, open {:.1} MiB, closed {:.1} MiB",
            mib(idle),
            mib(open),
            mib(closed)
        );
    }
}
