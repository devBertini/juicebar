// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! JuiceBar's Windows HID transport (`spec/ARCHITECTURE.md`, row "transport").
//!
//! `Transport::start` opens every Logitech HID++ collection present (VID 046D, usage page
//! 0xFF00 or 0xFF43), shared for read and write, and watches the HID device interface class for
//! arrivals and removals. One I/O thread, blocked on its handles, reads reports into a channel
//! and writes the reports the app sends; the app is woken through a [`Waker`], at most once per
//! batch and not again until it calls [`Transport::drain`].
//!
//! Failure policy: a refused write is dropped (the core times its request out); a failed read or
//! a vanished device closes the endpoint and reports `Removed`.

#![cfg(windows)]

mod control;
mod endpoint;
mod io;
mod path;
mod win;

use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification,
    CM_Unregister_Notification, CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetHidGuid;
use windows::Win32::System::Threading::CreateEventW;
use windows::core::PCWSTR;

pub use control::SendError;
pub use endpoint::{
    Endpoint, EndpointId, LOGITECH_VID, TransportEvent, TransportKind, USAGE_PAGE_HIDPP,
    USAGE_PAGE_HIDPP_DONGLE, is_hidpp_collection,
};
pub use path::{ParsedPath, parse_interface_path};

use control::{Control, ControlQueue, pad_report};
use io::{Registration, Shared};

/// What the app supplies to be told that events are waiting (it posts a window message).
/// Called from the I/O thread; it must not block.
pub trait Waker: Send + Sync {
    fn wake(&self);
}

impl<F: Fn() + Send + Sync> Waker for F {
    fn wake(&self) {
        self()
    }
}

/// `Transport::start` failed.
#[derive(Debug)]
pub struct StartError {
    pub what: &'static str,
    pub source: windows::core::Error,
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.what, self.source)
    }
}

impl std::error::Error for StartError {}

struct Runtime {
    thread: JoinHandle<()>,
    registration: Registration,
}

/// The running transport. Dropping it stops it.
pub struct Transport {
    shared: Arc<Shared>,
    events: Mutex<Receiver<TransportEvent>>,
    runtime: Mutex<Option<Runtime>>,
}

impl Transport {
    /// Register for HID interface arrivals and removals and start the I/O thread, which
    /// enumerates the interfaces present and reports each as `Arrived`.
    pub fn start(waker: Arc<dyn Waker>) -> Result<Transport, StartError> {
        // SAFETY: plain event creation: manual reset, initially not signaled, unnamed.
        let control_event =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|source| {
                StartError {
                    what: "CreateEventW",
                    source,
                }
            })?;
        let shared = Arc::new(Shared {
            queue: ControlQueue::default(),
            control_event: io::SendHandle(control_event),
            live: Mutex::default(),
            wake_pending: false.into(),
            stopping: false.into(),
        });

        // Register before enumerating, as Microsoft documents, so no arrival is missed.
        let context = Arc::into_raw(shared.clone());
        let mut filter = CM_NOTIFY_FILTER {
            cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            ..Default::default()
        };
        // SAFETY: HidD_GetHidGuid has no preconditions; the union field written is the one
        // selected by `FilterType`.
        unsafe {
            filter.u.DeviceInterface.ClassGuid = HidD_GetHidGuid();
        }
        let mut notify = HCMNOTIFICATION::default();
        // SAFETY: `filter` and `notify` are valid for the call; `context` stays valid until
        // after the unregistration (see `stop`).
        let cr = unsafe {
            CM_Register_Notification(
                &filter,
                Some(context.cast()),
                Some(io::on_device_interface),
                &mut notify,
            )
        };
        if cr != CR_SUCCESS {
            // SAFETY: `context` came from Arc::into_raw above and was never handed out.
            drop(unsafe { Arc::from_raw(context) });
            return Err(StartError {
                what: "CM_Register_Notification",
                source: windows::core::Error::from_hresult(windows::core::HRESULT(cr.0 as i32)),
            });
        }
        let registration = Registration { notify, context };

        let (tx, rx) = channel();
        let thread_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("juicebar-hid-io".into())
            .spawn(move || io::run(thread_shared, tx, waker));
        let thread = match spawned {
            Ok(t) => t,
            Err(e) => {
                unregister(registration);
                return Err(StartError {
                    what: "thread spawn",
                    source: windows::core::Error::from(e),
                });
            }
        };
        Ok(Transport {
            shared,
            events: Mutex::new(rx),
            runtime: Mutex::new(Some(Runtime {
                thread,
                registration,
            })),
        })
    }

    /// Take every event queued so far, oldest first. Call it when the waker fires: it also
    /// re-arms the waker.
    pub fn drain(&self) -> Vec<TransportEvent> {
        self.shared.wake_pending.store(false, Ordering::SeqCst);
        let rx = self.events.lock().unwrap_or_else(|e| e.into_inner());
        rx.try_iter().collect()
    }

    /// Queue one output report for the endpoint, zero-padded to its output length. Returns once
    /// queued; the write happens on the I/O thread. A write the device refuses is dropped.
    pub fn send(&self, id: EndpointId, bytes: &[u8]) -> Result<(), SendError> {
        if self.shared.stopping.load(Ordering::SeqCst) {
            return Err(SendError::Stopped);
        }
        let output_len = self
            .shared
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .copied()
            .ok_or(SendError::UnknownEndpoint)?;
        let report = pad_report(bytes, output_len as usize)?;
        self.shared.post(Control::Send { id, report });
        Ok(())
    }

    /// Cancel all I/O, unregister the notification, close every handle and join the I/O
    /// thread. No `Removed` is reported for the endpoints closed this way. Safe to call twice.
    pub fn stop(&self) {
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let Some(runtime) = runtime else {
            return;
        };
        self.shared.stopping.store(true, Ordering::SeqCst);
        // Unregistering waits for a running callback, so none can post after this returns.
        unregister(runtime.registration);
        self.shared.post(Control::Stop);
        let _ = runtime.thread.join();
        self.shared
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

fn unregister(registration: Registration) {
    // SAFETY: `notify` came from a successful CM_Register_Notification and is unregistered once.
    let _ = unsafe { CM_Unregister_Notification(registration.notify) };
    // SAFETY: `context` came from Arc::into_raw in `start`; no callback runs any more.
    drop(unsafe { Arc::from_raw(registration.context) });
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.stop();
    }
}
