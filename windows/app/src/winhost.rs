// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The Windows side of the live source: the HID transport, the one timer, the files and the log behind
//! [`Host`], the clock the core is given, and the translation of the transport's events into the core's.

use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SetTimer};

use juicebar_core::types::{DeviceKey, Endpoint, Now, TransportEvent};
use juicebar_hid::{StartError, Transport};

use crate::calib::Calibrator;
use crate::files::DataDir;
use crate::live::Host;
use crate::source::Clock;

/// The longest delay handed to `SetTimer` (it takes at most `USER_TIMER_MAXIMUM`, 0x7FFFFFFF ms).
const MAX_DELAY_MS: f64 = 2_000_000_000.0;

pub struct WinHost {
    hwnd: HWND,
    timer_id: usize,
    transport: Option<Rc<Transport>>,
    start: Instant,
    data: Option<DataDir>,
    log: Option<PathBuf>,
    /// The battery calibration, while it runs.
    calibrator: Option<Calibrator>,
}

impl WinHost {
    pub fn new(
        hwnd: HWND,
        timer_id: usize,
        transport: Option<Rc<Transport>>,
        start: Instant,
        data: Option<DataDir>,
        log: Option<PathBuf>,
    ) -> WinHost {
        WinHost {
            hwnd,
            timer_id,
            transport,
            start,
            data,
            log,
            calibrator: None,
        }
    }

    /// Takes calibration samples beside the voltage readings (`calib`).
    pub fn with_calibrator(mut self, calibrator: Option<Calibrator>) -> WinHost {
        self.calibrator = calibrator;
        self
    }

    fn log_error(&mut self, what: &str, key: &DeviceKey, e: &std::io::Error) {
        if self.logging() {
            self.log(&format!("           {what} {} failed: {e}", key.0));
        }
    }
}

impl Host for WinHost {
    fn send(&mut self, endpoint: u32, bytes: &[u8]) -> bool {
        self.transport
            .as_ref()
            .is_some_and(|t| t.send(juicebar_hid::EndpointId(endpoint), bytes).is_ok())
    }

    fn append(&mut self, key: &DeviceKey, lines: &[String]) {
        if let Some(Err(e)) = self.data.as_ref().map(|d| d.append(key, lines)) {
            self.log_error("append", key, &e);
        }
    }

    fn replace(&mut self, key: &DeviceKey, text: &str) {
        if let Some(Err(e)) = self.data.as_ref().map(|d| d.replace(key, text)) {
            self.log_error("rewrite", key, &e);
        }
    }

    fn set_aside(&mut self, key: &DeviceKey) {
        if let Some(Err(e)) = self.data.as_ref().map(|d| d.set_aside(key)) {
            self.log_error("set aside", key, &e);
        }
    }

    fn set_wake(&mut self, at_mono: f64) {
        let delay = ((at_mono - self.start.elapsed().as_secs_f64()) * 1000.0)
            .ceil()
            .clamp(1.0, MAX_DELAY_MS) as u32;
        // SAFETY: a timer on our own window; the same id replaces the one armed before.
        unsafe {
            SetTimer(Some(self.hwnd), self.timer_id, delay, None);
        }
    }

    fn calibrate(&mut self, key: &DeviceKey, millivolts: u16, charging: bool, wall: i64) {
        if let Some(c) = &self.calibrator {
            c.sample(&key.0, millivolts, charging, wall);
        }
    }

    fn logging(&self) -> bool {
        self.log.is_some()
    }

    fn log(&mut self, line: &str) {
        if let Some(path) = &self.log
            && let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Starts the transport; its waker posts `message` to `hwnd` (from the I/O thread, at most once per batch).
pub fn start_transport(hwnd: HWND, message: u32) -> Result<Rc<Transport>, StartError> {
    let target = hwnd.0 as isize;
    let waker = Arc::new(move || {
        // SAFETY: posting to a window by handle from another thread is allowed; a failure (the window is gone)
        // is harmless.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(target as *mut std::ffi::c_void)),
                message,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
    Transport::start(waker).map(Rc::new)
}

/// The moment now, as the core takes it: wall seconds, monotonic seconds since `start`, and the local offset
/// (the zone's bias plus the standard or daylight bias in force), read again at every call.
pub fn core_now(start: Instant) -> Now {
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    Now {
        wall,
        mono: start.elapsed().as_secs_f64(),
        utc_offset: Clock::now().utc_offset,
    }
}

/// The transport's event in the core's vocabulary. The report's own instant is dropped: the app stamps
/// everything with the time it handles it, which keeps the monotonic clock the core sees in order.
pub fn to_core(event: juicebar_hid::TransportEvent) -> TransportEvent {
    use juicebar_hid::{TransportEvent as Hid, TransportKind};
    match event {
        Hid::Arrived(e) => TransportEvent::Arrived(Endpoint {
            id: e.id.0,
            vid: e.vid,
            pid: e.pid,
            usage_page: e.usage_page,
            usage: e.usage,
            input_len: e.input_len,
            output_len: e.output_len,
            product: e.product,
            bluetooth: e.kind == TransportKind::Bluetooth,
        }),
        Hid::Removed(id) => TransportEvent::Removed(id.0),
        Hid::Report(id, bytes, _) => TransportEvent::Report(id.0, bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_hid::{EndpointId, TransportKind};

    #[test]
    fn endpoints_and_reports_cross_into_the_core_unchanged() {
        let ep = juicebar_hid::Endpoint {
            id: EndpointId(4),
            vid: 0x046D,
            pid: 0xC539,
            usage_page: 0xFF00,
            usage: 2,
            input_len: 20,
            output_len: 20,
            product: "USB Receiver".into(),
            kind: TransportKind::Usb,
        };
        match to_core(juicebar_hid::TransportEvent::Arrived(ep)) {
            TransportEvent::Arrived(e) => {
                assert_eq!(
                    (e.id, e.vid, e.pid, e.usage_page),
                    (4, 0x046D, 0xC539, 0xFF00)
                );
                assert_eq!((e.usage, e.input_len, e.output_len), (2, 20, 20));
                assert!(!e.bluetooth);
                assert_eq!(e.product, "USB Receiver");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            to_core(juicebar_hid::TransportEvent::Removed(EndpointId(9))),
            TransportEvent::Removed(9)
        );
        assert_eq!(
            to_core(juicebar_hid::TransportEvent::Report(
                EndpointId(2),
                vec![0x11, 1],
                Instant::now()
            )),
            TransportEvent::Report(2, vec![0x11, 1])
        );
    }

    #[test]
    fn the_core_clock_has_wall_time_a_running_monotonic_and_a_plausible_offset() {
        let start = Instant::now();
        let a = core_now(start);
        let b = core_now(start);
        assert!(a.wall > 1_700_000_000.0);
        assert!(b.mono >= a.mono);
        assert!(a.utc_offset.abs() <= 14 * 3600);
        assert_eq!(a.utc_offset % 60, 0);
    }
}
