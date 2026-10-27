// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Live proof of the flyout's backdrop: puts a vivid striped window (a stand-in for a colorful
//! wallpaper) under the flyout, opens the flyout over it, and saves a screen capture so the
//! blurred stripes (real acrylic) or their absence (a flat gray) can be seen and measured.
//!
//! `cargo run -p juicebar-ui --example acrylic_probe -- <png> [--theme light|dark] [--no-stripes]`
//! `[--hit N]` clicks row N (a device index) before the capture; `--welcome` shows the first-run page.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use juicebar_ui::flyout::{Flyout, Handlers};
use juicebar_ui::gfx::{Pixels, write_png};
use juicebar_ui::layout::PxRect;
use juicebar_ui::model::{Theme, UiState};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::Sleep;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const CLASS: PCWSTR = w!("JuiceBarProbeStripes");

/// Paints vertical bands of saturated colors.
unsafe extern "system" fn stripes_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_PAINT {
        let mut ps = PAINTSTRUCT::default();
        // SAFETY: Begin/EndPaint pair on this window, brushes deleted after use.
        unsafe {
            let dc = BeginPaint(h, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(h, &mut rc);
            let colors = [
                0x2b6cff_u32,
                0xff3b8a,
                0xffc233,
                0x28d17c,
                0x8f4bff,
                0x19c3e6,
            ];
            let band = 70;
            let mut x = rc.left;
            let mut i = 0;
            while x < rc.right {
                let c = colors[i % colors.len()];
                let bgr = ((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff);
                let b = CreateSolidBrush(COLORREF(bgr));
                let r = RECT {
                    left: x,
                    top: rc.top,
                    right: (x + band).min(rc.right),
                    bottom: rc.bottom,
                };
                FillRect(dc, &r, b);
                let _ = DeleteObject(b.into());
                x += band;
                i += 1;
            }
            let _ = EndPaint(h, &ps);
        }
        return LRESULT(0);
    }
    // SAFETY: forwarding the message unchanged.
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

fn stripes(r: PxRect) -> HWND {
    // SAFETY: a plain popup window of our own class, shown without activation.
    unsafe {
        let inst = GetModuleHandleW(None).expect("module");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(stripes_proc),
            hInstance: inst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let h = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS,
            w!("stripes"),
            WS_POPUP,
            r.left - 60,
            r.top - 60,
            r.right - r.left + 120,
            r.bottom - r.top + 120,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .expect("stripes window");
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(h);
        h
    }
}

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

fn capture(r: PxRect, path: &Path) -> windows::core::Result<Pixels> {
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
    let px = Pixels {
        width: w,
        height: h,
        data,
    };
    write_png(path, &px)?;
    Ok(px)
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let out = PathBuf::from(args.first().expect("png path"));
    let theme = match value("--theme").as_deref() {
        Some("light") => Theme::Light,
        _ => Theme::Dark,
    };
    let mut state = UiState::sample(theme);
    state.accent = juicebar_ui::system::accent_for(theme, juicebar_ui::system::accent_color());
    juicebar_ui::flyout::ensure_dpi_awareness();
    let anchor = PxRect {
        left: 1500,
        top: 1390,
        right: 1524,
        bottom: 1440,
    };
    let backdrop = (!args.iter().any(|a| a == "--no-stripes")).then(|| {
        stripes(PxRect {
            left: 1100,
            top: 760,
            right: 1900,
            bottom: 1400,
        })
    });
    pump(300);
    let flyout = Flyout::show(
        state,
        anchor,
        Handlers {
            on_hit: Box::new(|_| {}),
            on_close: Box::new(|| {}),
        },
    )?;
    flyout.set_hide_on_deactivate(false);
    pump(900);
    // SAFETY: plain query.
    let fg = unsafe { GetForegroundWindow() };
    println!("flyout rect {:?}", flyout.rect());
    println!("flyout is foreground window: {}", fg == flyout.hwnd());
    let px = capture(flyout.rect().expect("rect"), &out)?;
    // Samples through the panel's header band: variation means the backdrop shows through.
    let y = 24 + 8;
    let row: Vec<(u8, u8, u8)> = (24..px.width - 24)
        .step_by(20)
        .map(|x| {
            let i = ((y * px.width + x) * 4) as usize;
            (px.data[i + 2], px.data[i + 1], px.data[i])
        })
        .collect();
    println!("header band samples (r,g,b): {row:?}");
    drop(flyout);
    if let Some(b) = backdrop {
        // SAFETY: our own window.
        unsafe {
            let _ = DestroyWindow(b);
        }
    }
    Ok(())
}
