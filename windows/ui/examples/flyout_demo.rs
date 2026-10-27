// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The flyout and the tray mark, for looking at.
//!
//! `cargo run -p juicebar-ui --example flyout_demo -- --render <folder>` writes the flyout (light and
//! dark, 100 % and 150 %) and the tray mark (three states, 16/24/32 px, both taskbar colors).
//! `--render-w7b <folder>` writes the panel's scenes (`w7b-<scene>-<theme>-<scale>.png`): every card closed, an
//! open card with the chart and the week bars, a device without a reading, the power saver (i) open with the
//! G HUB notice, and the welcome page.
//!
//! `--render-w7c <folder>` writes the Settings page (`w7c-<scene>-<theme>-<scale>.png`): the whole page in one tall
//! picture, the window as the user sees it (capped to a 720 DIP work area) at the top and scrolled to the end, and
//! the page with the keyboard focus on a stepper.
//!
//! `--render-w7c2 <folder>` writes the Dispositivos section with the Geral one under it (`w7c2-<scene>-<theme>-<scale>.png`):
//! the devices, a row being renamed (the name selected, and with a typed name), and the shortcut waiting for keys.
//!
//! Without `--render` it shows the live flyout above the bottom-right corner of the primary screen
//! until Esc, a click elsewhere, or `--seconds N`. `--capture <png>` also saves a screenshot of it
//! (taken from the screen, so the real backdrop shows) 1.5 s after it opens.

use std::path::{Path, PathBuf};

use juicebar_ui::flyout::{Flyout, Handlers};
use juicebar_ui::gfx::{Pixels, write_png};
use juicebar_ui::icon;
use juicebar_ui::layout::{FlyoutHit, PxRect};
use juicebar_ui::model::{IconState, Theme, UiState};
use juicebar_ui::render::{render_flyout_png, render_flyout_png_at};
use juicebar_ui::system;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, PostQuitMessage, SetTimer, TranslateMessage,
    WM_TIMER,
};

fn sample(theme: Theme) -> UiState {
    let mut s = UiState::sample(theme);
    s.accent = system::accent_for(theme, system::accent_color());
    s
}

fn render(dir: &Path) -> windows::core::Result<()> {
    std::fs::create_dir_all(dir).expect("create the render folder");
    for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        // The mockup's own accent per theme, so the PNGs compare with the mockup renders.
        let state = UiState::sample(theme);
        for (scale, tag) in [(1.0f32, "100"), (1.5, "150")] {
            let f = dir.join(format!("flyout-{name}-{tag}.png"));
            let (w, h) = render_flyout_png(&state, theme, scale, &f)?;
            println!("{} {w}x{h}", f.display());
        }
    }
    for (st, sname) in [
        (IconState::Normal, "normal"),
        (IconState::Warning, "warning"),
        (IconState::Charging, "charging"),
    ] {
        for dark in [true, false] {
            for px in [16u32, 24, 32] {
                let f = dir.join(format!(
                    "mark-{sname}-{px}-{}.png",
                    if dark { "dark" } else { "light" }
                ));
                icon::render_mark_png(st, dark, px, &f)?;
            }
        }
    }
    let sheet = dir.join("marks-sheet.png");
    icon::render_mark_sheet_png(&sheet)?;
    println!("{}", sheet.display());
    Ok(())
}

/// The panel's scenes for the review of the glass look, in both themes at 100 % and 150 %.
fn render_w7b(dir: &Path) -> windows::core::Result<()> {
    std::fs::create_dir_all(dir).expect("create the render folder");
    type Scene = (&'static str, fn(Theme) -> UiState);
    let scenes: [Scene; 5] = [
        ("collapsed", UiState::sample_collapsed),
        ("expanded", UiState::sample),
        ("noreading", UiState::sample_no_reading),
        ("footer", |t| {
            let mut s = UiState::sample_collapsed(t);
            s.rgb_economy = true;
            s.info_open = true;
            s.ghub_notice = Some("o G HUB está reaplicando a iluminação".into());
            s
        }),
        ("welcome", UiState::sample_welcome),
    ];
    for (name, make) in scenes {
        for (theme, tname) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
            for (scale, tag) in [(1.0f32, "100"), (1.5, "150")] {
                let f = dir.join(format!("w7b-{name}-{tname}-{tag}.png"));
                let (w, h) = render_flyout_png(&make(theme), theme, scale, &f)?;
                println!("{} {w}x{h}", f.display());
            }
        }
    }
    Ok(())
}

/// The Settings page in both themes at 100 % and 150 %: whole, as the window shows it, scrolled, and focused.
fn render_w7c(dir: &Path) -> windows::core::Result<()> {
    std::fs::create_dir_all(dir).expect("create the render folder");
    for (theme, tname) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        for (scale, tag) in [(1.0f32, "100"), (1.5, "150")] {
            let page = UiState::sample_settings(theme);
            let mut focused = page.clone();
            if let Some(ui) = focused.settings.as_mut() {
                ui.focus = Some(5);
            }
            let scenes: [(&str, &UiState, Option<f32>, f32); 4] = [
                ("whole", &page, Some(100_000.0), 0.0),
                ("window", &page, Some(720.0), 0.0),
                ("scrolled", &page, Some(720.0), 100_000.0),
                ("focus", &focused, Some(100_000.0), 0.0),
            ];
            for (name, state, max, scroll) in scenes {
                let f = dir.join(format!("w7c-{name}-{tname}-{tag}.png"));
                let (w, h) = render_flyout_png_at(state, theme, scale, &f, max, scroll)?;
                println!("{} {w}x{h}", f.display());
            }
        }
    }
    Ok(())
}

/// The Dispositivos and Geral sections of the sample Settings page in four states.
fn render_w7c2(dir: &Path) -> windows::core::Result<()> {
    use juicebar_ui::settings::SettingsRow;
    std::fs::create_dir_all(dir).expect("create the render folder");
    for (theme, tname) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        for (scale, tag) in [(1.0f32, "100"), (1.5, "150")] {
            let mut base = UiState::sample_settings(theme);
            if let Some(ui) = base.settings.as_mut() {
                ui.sections
                    .retain(|s| s.title == "Dispositivos" || s.title == "Geral");
            }
            let edit = |value: &str, selected: bool| {
                let mut st = base.clone();
                let ui = st.settings.as_mut().unwrap();
                let hint = "Enter salva, Esc cancela. Vazio volta para G915 X.";
                let row = SettingsRow::text(18, "Nome para G915 X", value, true)
                    .with_selected(selected)
                    .with_detail(hint);
                ui.sections[0].rows[1] = row;
                st
            };
            let mut capture = base.clone();
            if let Some(ui) = capture.settings.as_mut() {
                ui.capturing = true;
                ui.focus = Some(12);
                ui.sections[1].rows[2] = SettingsRow::link(
                    12,
                    "Atalho: Ctrl+Alt+B",
                    "Aguardando...",
                )
                .with_detail(
                    "Segure Ctrl, Alt ou Win e aperte uma letra, número ou tecla F. Esc cancela.",
                );
            }
            let mut focused = base.clone();
            if let Some(ui) = focused.settings.as_mut() {
                ui.focus = Some(18);
            }
            let scenes: [(&str, UiState); 6] = [
                ("devices", base.clone()),
                ("rename", edit("G915 X", true)),
                ("typing", edit("Teclado da mesa", false)),
                ("long", edit("WWWWWWWWWWWWWWWWWWWWWWWW", false)),
                ("capture", capture),
                ("focus", focused),
            ];
            for (name, state) in scenes {
                let f = dir.join(format!("w7c2-{name}-{tname}-{tag}.png"));
                let (w, h) = render_flyout_png_at(&state, theme, scale, &f, Some(100_000.0), 0.0)?;
                println!("{} {w}x{h}", f.display());
            }
        }
    }
    Ok(())
}

/// The primary monitor's tray corner: a small rectangle in the taskbar, bottom right.
fn tray_anchor() -> PxRect {
    // SAFETY: a point on the primary monitor; MONITOR_DEFAULTTOPRIMARY always returns a monitor.
    let mon = unsafe {
        MonitorFromPoint(
            windows::Win32::Foundation::POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        )
    };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `mi` has its size set and outlives the call.
    let _ = unsafe { GetMonitorInfoW(mon, &mut mi) };
    PxRect {
        left: mi.rcMonitor.right - 150,
        top: mi.rcWork.bottom,
        right: mi.rcMonitor.right - 126,
        bottom: mi.rcMonitor.bottom,
    }
}

fn capture(r: PxRect, path: &Path) -> windows::core::Result<()> {
    let (x, y) = (r.left - 24, r.top - 24);
    let (w, h) = (
        (r.right - r.left + 48) as u32,
        (r.bottom - r.top + 48) as u32,
    );
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: standard screen capture into a DIB section; every handle is released below.
    let data = unsafe {
        let sdc = GetDC(None);
        let mdc = CreateCompatibleDC(Some(sdc));
        let dib = CreateDIBSection(Some(sdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(mdc, dib.into());
        let blt = BitBlt(
            mdc,
            0,
            0,
            w as i32,
            h as i32,
            Some(sdc),
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        );
        let mut data = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        SelectObject(mdc, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        blt?;
        for p in data.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        data
    };
    write_png(
        path,
        &Pixels {
            width: w,
            height: h,
            data,
        },
    )
}

fn live(seconds: u32, shot: Option<PathBuf>) -> windows::core::Result<()> {
    let theme = system::app_theme();
    let state = sample(theme);
    let handlers = Handlers {
        on_hit: Box::new(|hit: FlyoutHit| println!("hit: {hit:?}")),
        // SAFETY: posts WM_QUIT to this thread's queue.
        on_close: Box::new(|| unsafe { PostQuitMessage(0) }),
    };
    let flyout = Flyout::show(state, tray_anchor(), handlers)?;
    if seconds > 0 {
        // A timed run stays open even if another window (this terminal) takes the focus.
        flyout.set_hide_on_deactivate(false);
    }
    println!("flyout open at {:?}", flyout.rect());
    // SAFETY: thread timers (no window) delivered to this loop as WM_TIMER; the system picks their ids.
    let (shot_timer, close_timer) = unsafe {
        (
            if shot.is_some() {
                SetTimer(None, 0, 1500, None)
            } else {
                0
            },
            if seconds > 0 {
                SetTimer(None, 0, seconds * 1000, None)
            } else {
                0
            },
        )
    };
    let mut msg = MSG::default();
    // SAFETY: the standard message loop; it ends when `on_close` posts WM_QUIT.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                if msg.wParam.0 == shot_timer && shot_timer != 0 {
                    if let (Some(p), Some(r)) = (shot.as_ref(), flyout.rect()) {
                        capture(r, p)?;
                        println!("captured {}", p.display());
                        let _ = KillTimer(None, shot_timer);
                    }
                } else if msg.wParam.0 == close_timer && close_timer != 0 {
                    flyout.hide();
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    println!("flyout closed");
    Ok(())
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    if let Some(dir) = value("--render") {
        return render(Path::new(dir));
    }
    if let Some(dir) = value("--render-w7c2") {
        return render_w7c2(Path::new(dir));
    }
    if let Some(dir) = value("--render-w7c") {
        return render_w7c(Path::new(dir));
    }
    if let Some(dir) = value("--render-w7b") {
        return render_w7b(Path::new(dir));
    }
    let seconds = value("--seconds").and_then(|s| s.parse().ok()).unwrap_or(0);
    live(seconds, value("--capture").map(PathBuf::from))
}
