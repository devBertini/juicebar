// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The I/O thread: one thread blocked in `WaitForMultipleObjects` on the control event and on
//! each port's pending read (and write) event, plus the `CM_Register_Notification` callback
//! that feeds it hot-plug messages. No timeouts, no polling.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
    CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_EVENT_DATA, HCMNOTIFICATION,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED};
use windows::Win32::System::Threading::{INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects};

use crate::Waker;
use crate::control::{Control, ControlQueue};
use crate::endpoint::{
    Endpoint, EndpointId, LOGITECH_VID, TransportEvent, TransportKind, is_hidpp_collection,
};
use crate::path::parse_interface_path;
use crate::win::{Port, drain_writes, present_paths, query};

/// How long shutdown waits for queued writes to finish before cancelling them.
const SHUTDOWN_WRITE_BUDGET: std::time::Duration = std::time::Duration::from_millis(300);

/// `WaitForMultipleObjects` takes at most 64 handles: the control event, then up to two per port.
const MAX_PORTS: usize = 31;

/// A Win32 handle that may cross threads (events are kernel objects safe to signal anywhere).
pub(crate) struct SendHandle(pub HANDLE);
// SAFETY: a kernel event handle is a process-wide identifier; SetEvent/ResetEvent/Wait are thread-safe.
unsafe impl Send for SendHandle {}
// SAFETY: same as above; the handle is never mutated, only passed to thread-safe calls.
unsafe impl Sync for SendHandle {}

/// State shared by the app thread, the notification callback and the I/O thread.
pub(crate) struct Shared {
    pub queue: ControlQueue,
    pub control_event: SendHandle,
    /// Output report length of every open endpoint, for `send` to validate and pad.
    pub live: Mutex<HashMap<EndpointId, u16>>,
    /// True from a wake until the app drains; keeps the waker from firing twice for one drain.
    pub wake_pending: AtomicBool,
    pub stopping: AtomicBool,
}

impl Shared {
    /// Queue a message and signal the I/O thread when it may be asleep.
    pub(crate) fn post(&self, message: Control) {
        if self.queue.push(message) {
            // SAFETY: the event handle lives as long as `Shared`.
            let _ = unsafe { SetEvent(self.control_event.0) };
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: the event was created by `Transport::start` and is closed exactly once here.
        let _ = unsafe { CloseHandle(self.control_event.0) };
    }
}

/// A live device-notification registration.
pub(crate) struct Registration {
    pub notify: HCMNOTIFICATION,
    /// `Arc::into_raw` of the `Shared` the callback reads; reclaimed after unregistering.
    pub context: *const Shared,
}
// SAFETY: the registration is only used to unregister and reclaim the Arc from one thread at a time.
unsafe impl Send for Registration {}

/// Read a NUL-terminated UTF-16 string out of at most `max` units.
pub(crate) fn wide_until_nul(units: &[u16]) -> String {
    let end = units.iter().position(|c| *c == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// The device-interface callback. Runs on a system thread: it only parses the path and posts a message.
pub(crate) unsafe extern "system" fn on_device_interface(
    _hnotify: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    event_data: *const CM_NOTIFY_EVENT_DATA,
    event_data_size: u32,
) -> u32 {
    if context.is_null() || event_data.is_null() {
        return 0;
    }
    let arrival = action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL;
    if !arrival && action != CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
        return 0;
    }
    // SAFETY: `context` is the pointer from `Arc::into_raw`; the Arc is only released after the
    // registration is removed, which waits for running callbacks.
    let shared = unsafe { &*(context as *const Shared) };
    // SAFETY: for a device-interface event the union holds DeviceInterface and `event_data_size`
    // bytes are readable; the path is the NUL-terminated string at `SymbolicLink`.
    let path = unsafe {
        let link = std::ptr::addr_of!((*event_data).u.DeviceInterface.SymbolicLink) as *const u16;
        let offset = link as usize - event_data as usize;
        let units = (event_data_size as usize).saturating_sub(offset) / 2;
        wide_until_nul(std::slice::from_raw_parts(link, units))
    };
    if parse_interface_path(&path).is_some_and(|p| p.vid == LOGITECH_VID) {
        shared.post(if arrival {
            Control::Arrive(path)
        } else {
            Control::Remove(path)
        });
    }
    0
}

struct Io {
    shared: Arc<Shared>,
    tx: Sender<TransportEvent>,
    waker: Arc<dyn Waker>,
    ports: Vec<Port>,
    next_id: u32,
    queued: usize,
}

impl Io {
    fn emit(&mut self, event: TransportEvent) {
        if self.tx.send(event).is_ok() {
            self.queued += 1;
        }
    }

    /// Wake the app once for everything queued in this batch (and not again until it drains).
    fn flush(&mut self) {
        if self.queued > 0 {
            self.queued = 0;
            if !self.shared.wake_pending.swap(true, Ordering::SeqCst) {
                self.waker.wake();
            }
        }
    }

    fn arrive(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase();
        if self.ports.iter().any(|p| p.path_lower == lower) || self.ports.len() >= MAX_PORTS {
            return;
        }
        let Some(parsed) = parse_interface_path(path) else {
            return;
        };
        if parsed.vid != LOGITECH_VID {
            return;
        }
        let Some((caps, product)) = query(path) else {
            return;
        };
        if !is_hidpp_collection(parsed.vid, caps.usage_page, caps.input_len) {
            return;
        }
        let id = EndpointId(self.next_id);
        let endpoint = Endpoint {
            id,
            vid: parsed.vid,
            pid: parsed.pid,
            usage_page: caps.usage_page,
            usage: caps.usage,
            input_len: caps.input_len,
            output_len: caps.output_len,
            product,
            kind: if parsed.bluetooth {
                TransportKind::Bluetooth
            } else {
                TransportKind::Usb
            },
        };
        let Ok(port) = Port::open(path, endpoint.clone()) else {
            return;
        };
        self.next_id += 1;
        self.ports.push(port);
        self.shared
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, caps.output_len);
        self.emit(TransportEvent::Arrived(endpoint));
    }

    fn remove_at(&mut self, index: usize) {
        let port = self.ports.swap_remove(index);
        let id = port.endpoint.id;
        self.shared
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        drop(port);
        self.emit(TransportEvent::Removed(id));
    }

    fn remove(&mut self, path: &str) {
        let lower = path.to_ascii_lowercase();
        if let Some(i) = self.ports.iter().position(|p| p.path_lower == lower) {
            self.remove_at(i);
        }
    }

    fn send(&mut self, id: EndpointId, report: Vec<u8>) {
        if let Some(port) = self.ports.iter_mut().find(|p| p.endpoint.id == id) {
            port.queue_write(report);
        }
    }

    /// Collect every finished read and write; drop the ports that died.
    fn service_ports(&mut self) {
        let mut i = 0;
        while i < self.ports.len() {
            let report = self.ports[i].take_report();
            self.ports[i].service_write();
            if let Some(bytes) = report {
                let id = self.ports[i].endpoint.id;
                self.emit(TransportEvent::Report(id, bytes, Instant::now()));
            }
            if self.ports[i].dead {
                self.remove_at(i);
            } else {
                i += 1;
            }
        }
    }

    fn wait_handles(&self) -> Vec<HANDLE> {
        let mut handles = Vec::with_capacity(1 + self.ports.len() * 2);
        handles.push(self.shared.control_event.0);
        for p in &self.ports {
            handles.extend(p.read_handle());
            handles.extend(p.write_handle());
        }
        handles
    }

    /// Handle every queued control message. Returns false on `Stop`.
    fn process_control(&mut self) -> bool {
        // Reset before draining: a message posted after the drain sets the event again.
        // SAFETY: the control event lives as long as `Shared`.
        let _ = unsafe { ResetEvent(self.shared.control_event.0) };
        for message in self.shared.queue.drain() {
            match message {
                Control::Stop => return false,
                Control::Arrive(path) => self.arrive(&path),
                Control::Remove(path) => self.remove(&path),
                Control::Send { id, report } => self.send(id, report),
            }
        }
        true
    }
}

/// The body of the I/O thread. Enumerates the present interfaces, then serves until `Stop`.
pub(crate) fn run(shared: Arc<Shared>, tx: Sender<TransportEvent>, waker: Arc<dyn Waker>) {
    let mut io = Io {
        shared,
        tx,
        waker,
        ports: Vec::new(),
        next_id: 1,
        queued: 0,
    };
    for path in present_paths() {
        io.arrive(&path);
    }
    io.flush();
    loop {
        if !io.process_control() {
            break;
        }
        io.service_ports();
        io.flush();
        let handles = io.wait_handles();
        // SAFETY: `handles` holds valid event handles owned by `Shared` and the ports; INFINITE
        // blocks until one is signaled.
        let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        if r == WAIT_FAILED {
            break;
        }
    }
    // A request queued just before `Stop` (the lighting restore) gets a short while to reach the
    // device; dropping the ports then cancels what is still pending and closes the handles.
    drain_writes(&mut io.ports, SHUTDOWN_WRITE_BUDGET);
    io.ports.clear();
}

#[cfg(test)]
mod tests {
    use super::wide_until_nul;

    #[test]
    fn reads_up_to_the_nul() {
        let units: Vec<u16> = "hid#vid_046d\0trailing".encode_utf16().collect();
        assert_eq!(wide_until_nul(&units), "hid#vid_046d");
    }

    #[test]
    fn an_unterminated_buffer_is_read_whole() {
        let units: Vec<u16> = "abc".encode_utf16().collect();
        assert_eq!(wide_until_nul(&units), "abc");
        assert_eq!(wide_until_nul(&[]), "");
    }
}
