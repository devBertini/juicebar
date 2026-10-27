// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Drives a running `JuiceBar.exe` from outside, for proof by eye and by number.
//!
//! `cargo run -p juicebar --example drive -- capture <folder>`: starts `JuiceBar.exe --demo
//! --demo-cycle`, captures the tray, opens the flyout with a synthesized Ctrl+Alt+B, clicks the tray
//! icon to close and reopen it, launches a second instance, captures the cycle's icon states and
//! prints the app's trace.
//!
//! `cargo run -p juicebar --example drive -- measure`: starts `JuiceBar.exe --demo`, lets it idle
//! and prints its working set, private bytes and the CPU time used over 30 seconds.

use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

use juicebar_ui::gfx::{Pixels, write_png};
use juicebar_ui::layout::PxRect;
use windows::Win32::Foundation::{FILETIME, HANDLE, HWND, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::GetProcessTimes;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_MENU,
};
use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, GetWindowRect, SM_CXSCREEN, SM_CYSCREEN, SetCursorPos,
};
use windows::core::w;

fn exe_path() -> PathBuf {
    let me = std::env::current_exe().expect("own path");
    me.parent()
        .and_then(Path::parent)
        .expect("target folder")
        .join("JuiceBar.exe")
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    // SAFETY: well-formed INPUT structures and their size.
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn hotkey() {
    let b = VIRTUAL_KEY(b'B' as u16);
    send(&[
        key(VK_CONTROL, false),
        key(VK_MENU, false),
        key(b, false),
        key(b, true),
        key(VK_MENU, true),
        key(VK_CONTROL, true),
    ]);
}

fn escape() {
    send(&[key(VK_ESCAPE, false), key(VK_ESCAPE, true)]);
}

fn click_at(x: i32, y: i32, right: bool) {
    // SAFETY: moves the cursor; the shell wants to see it arrive before a click counts.
    unsafe {
        let _ = SetCursorPos(x - 4, y - 2);
        sleep(Duration::from_millis(120));
        let _ = SetCursorPos(x, y);
    }
    sleep(Duration::from_millis(400));
    let (down, up) = if right {
        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
    } else {
        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    };
    let m = |f: MOUSE_EVENT_FLAGS| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: f,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(&[m(down)]);
    sleep(Duration::from_millis(60));
    send(&[m(up)]);
}

fn screen() -> (i32, i32) {
    // SAFETY: metrics queries.
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

fn flyout_rect() -> Option<PxRect> {
    // SAFETY: a window lookup and a rectangle read.
    unsafe {
        let h = FindWindowW(w!("JuiceBarFlyout"), None).ok()?;
        let mut r = RECT::default();
        GetWindowRect(h, &mut r).ok()?;
        Some(PxRect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        })
    }
}

fn main_window() -> Option<HWND> {
    // SAFETY: a window lookup by class.
    unsafe { FindWindowW(w!("JuiceBar.Main"), None).ok() }
}

fn tray_rect() -> Option<PxRect> {
    let hwnd = main_window()?;
    let id = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    // SAFETY: identifies the icon of the running app.
    let r = unsafe { Shell_NotifyIconGetRect(&id) }.ok()?;
    Some(PxRect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    })
}

/// Copies a screen region into a PNG.
fn capture(l: i32, t: i32, w: i32, h: i32, path: &Path) {
    save(grab(l, t, w, h), path);
}

/// A screen region magnified with nearest-neighbour sampling, to judge the icon's pixels.
fn capture_zoom(l: i32, t: i32, w: i32, h: i32, k: u32, path: &Path) {
    let src = grab(l, t, w, h);
    let (nw, nh) = (src.width * k, src.height * k);
    let mut data = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let si = (((y / k) * src.width + x / k) * 4) as usize;
            let di = ((y * nw + x) * 4) as usize;
            data[di..di + 4].copy_from_slice(&src.data[si..si + 4]);
        }
    }
    save(
        Pixels {
            width: nw,
            height: nh,
            data,
        },
        path,
    );
}

fn save(p: Pixels, path: &Path) {
    write_png(path, &p).expect("write png");
    println!("saved {}", path.display());
}

fn grab(l: i32, t: i32, w: i32, h: i32) -> Pixels {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
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
        let dib = CreateDIBSection(Some(sdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            .expect("dib section");
        let old = SelectObject(mdc, dib.into());
        let _ = BitBlt(mdc, 0, 0, w, h, Some(sdc), l, t, SRCCOPY | CAPTUREBLT);
        let mut data = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        SelectObject(mdc, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        for p in data.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        data
    };
    Pixels {
        width: w as u32,
        height: h as u32,
        data,
    }
}

/// The bottom-right corner of the screen: the taskbar's end with the tray and the clock.
fn capture_tray(path: &Path) {
    // SAFETY: moves the cursor off the taskbar so no hover highlight is in the picture.
    unsafe {
        let _ = SetCursorPos(200, 200);
    }
    sleep(Duration::from_millis(400));
    let (sw, sh) = screen();
    capture(sw - 640, sh - 96, 640, 96, path);
    // The icon itself, 8x: it sits where the shell says (the overflow chevron when it is hidden).
    if let Some(r) = tray_rect() {
        let zoom = path.with_file_name(format!(
            "{}-zoom.png",
            path.file_stem().and_then(|s| s.to_str()).unwrap_or("tray")
        ));
        capture_zoom(r.left - 8, r.top, 48, r.bottom - r.top, 8, &zoom);
    }
}

fn capture_flyout(path: &Path) {
    let (sw, sh) = screen();
    match flyout_rect() {
        Some(r) => {
            let l = (r.left - 24).max(0);
            let t = (r.top - 24).max(0);
            capture(l, t, sw - l, sh - t, path);
        }
        None => println!("no flyout window to capture for {}", path.display()),
    }
}

fn spawn(args: &[&str]) -> Child {
    Command::new(exe_path())
        .args(args)
        .spawn()
        .expect("start JuiceBar.exe")
}

fn ft(f: FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

fn cpu_100ns(child: &Child) -> u64 {
    let h = HANDLE(child.as_raw_handle());
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: valid out pointers and a live process handle.
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).expect("process times") };
    ft(k) + ft(u)
}

fn memory(child: &Child) -> (usize, usize) {
    let h = HANDLE(child.as_raw_handle());
    let mut m = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: the counters structure is the documented extended one and its size is set.
    let ok = unsafe { K32GetProcessMemoryInfo(h, &mut m as *mut _ as *mut _, m.cb) };
    assert!(ok.as_bool(), "memory info");
    (m.WorkingSetSize, m.PrivateUsage)
}

fn mib(b: usize) -> f64 {
    b as f64 / 1_048_576.0
}

fn measure() {
    let mut child = spawn(&["--demo", "--seconds", "60"]);
    sleep(Duration::from_secs(3));
    let (ws0, pv0) = memory(&child);
    let cpu0 = cpu_100ns(&child);
    println!(
        "t=3s idle: working set {:.1} MiB, private {:.1} MiB",
        mib(ws0),
        mib(pv0)
    );
    sleep(Duration::from_secs(30));
    let cpu1 = cpu_100ns(&child);
    let (ws1, pv1) = memory(&child);
    println!(
        "t=33s idle: working set {:.1} MiB, private {:.1} MiB",
        mib(ws1),
        mib(pv1)
    );
    println!(
        "cpu time over those 30 s: {:.1} ms",
        (cpu1 - cpu0) as f64 / 10_000.0
    );
    hotkey();
    sleep(Duration::from_millis(2500));
    let (ws2, pv2) = memory(&child);
    println!(
        "flyout open: working set {:.1} MiB, private {:.1} MiB",
        mib(ws2),
        mib(pv2)
    );
    hotkey();
    sleep(Duration::from_millis(1500));
    let (ws3, pv3) = memory(&child);
    println!(
        "flyout closed again: working set {:.1} MiB, private {:.1} MiB",
        mib(ws3),
        mib(pv3)
    );
    let _ = child.kill();
    let _ = child.wait();
}

fn capture_run(dir: &Path) {
    std::fs::create_dir_all(dir).expect("folder");
    let trace = dir.join("trace.txt");
    let _ = std::fs::remove_file(&trace);
    let lang = std::env::args().nth(3).unwrap_or_else(|| "pt-BR".into());
    let mut child = spawn(&[
        "--demo",
        "--demo-cycle",
        "--seconds",
        "46",
        "--lang",
        &lang,
        "--trace",
        trace.to_str().expect("utf-8 path"),
    ]);
    let t0 = std::time::Instant::now();
    sleep(Duration::from_secs(2));
    println!("tray icon rect: {:?}", tray_rect());
    capture_tray(&dir.join("1-tray-scene.png"));

    // The hotkey opens the flyout; again, it closes it.
    hotkey();
    sleep(Duration::from_millis(1500));
    println!("flyout after hotkey: {:?}", flyout_rect());
    capture_flyout(&dir.join("2-flyout-hotkey.png"));
    hotkey();
    sleep(Duration::from_millis(700));
    println!("flyout after second hotkey: {:?}", flyout_rect());

    // A left click on the tray icon opens it; clicking the icon again closes it and it stays closed.
    if let Some(r) = tray_rect() {
        let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        click_at(cx, cy, false);
        sleep(Duration::from_millis(1500));
        println!("flyout after tray click: {:?}", flyout_rect());
        click_at(cx, cy, false);
        sleep(Duration::from_millis(1500));
        println!("flyout after clicking the icon again: {:?}", flyout_rect());
        // A right click shows the menu; capture it, then close it with Escape.
        click_at(cx, cy, true);
        sleep(Duration::from_millis(800));
        let (sw, sh) = screen();
        capture(sw - 640, sh - 400, 640, 400, &dir.join("3-menu.png"));
        escape();
        sleep(Duration::from_millis(500));
    }

    // A second launch asks the first to open its flyout and exits by itself.
    let mut second = spawn(&[]);
    let _ = second.wait();
    sleep(Duration::from_millis(1500));
    println!("flyout after a second launch: {:?}", flyout_rect());
    capture_flyout(&dir.join("4-flyout-second-launch.png"));
    hotkey();
    sleep(Duration::from_millis(600));

    // The demo cycle: normal at 10 s, warning at 20 s, charging at 30 s, full at 40 s.
    // The toast of the warning notice appears at 20 s, the full-charge one at 40 s.
    let (sw, sh) = screen();
    for (at, name) in [
        (12, "5-tray-normal"),
        (21, "6-tray-warning"),
        (23, "6b-notice-warning-2s"),
        (32, "7-tray-charging"),
        (41, "8-notice-full-1s"),
        (43, "8b-notice-full-3s"),
    ] {
        let wait = Duration::from_secs(at).saturating_sub(t0.elapsed());
        sleep(wait);
        if name.contains("notice") {
            capture(
                sw - 640,
                sh - 400,
                640,
                400,
                &dir.join(format!("{name}.png")),
            );
        } else {
            capture_tray(&dir.join(format!("{name}.png")));
        }
    }
    let status = child.wait().expect("wait");
    println!("JuiceBar exited: {status}");
    println!("--- trace ---");
    println!("{}", std::fs::read_to_string(&trace).unwrap_or_default());
}

fn main() {
    juicebar_ui::flyout::ensure_dpi_awareness();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("measure") => measure(),
        Some("empty") => {
            // The app without --demo: the "no devices yet" flyout.
            let png = args.next().expect("png path");
            let lang = args.next().unwrap_or_else(|| "pt-BR".into());
            let mut child = spawn(&["--seconds", "8", "--lang", &lang]);
            sleep(Duration::from_secs(2));
            hotkey();
            sleep(Duration::from_millis(1500));
            capture_flyout(Path::new(&png));
            let _ = child.wait();
        }
        Some("shot") => {
            let (sw, sh) = screen();
            capture(
                sw - 640,
                sh - 400,
                640,
                400,
                Path::new(&args.next().expect("png path")),
            );
        }
        Some("capture") => capture_run(Path::new(&args.next().expect("output folder"))),
        _ => eprintln!(
            "usage: drive capture <folder> [lang] | drive measure | drive empty <png> [lang] | drive shot <png>"
        ),
    }
}
