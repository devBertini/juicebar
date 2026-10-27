// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The `game-controllers` provider (spec/PROVIDERS.md, docs/research/controllers.md): DualSense and Xbox pads.
//!
//! A game pad streams input reports hundreds of times a second, so this provider never follows one. It listens for
//! the arrival and removal of HID interfaces of the catalog's products (its own `CM_Register_Notification`, a
//! callback with no thread of ours) and, for a pad that is present, samples its battery once on arrival and then
//! every [`REFRESH_SECONDS`]: a DualSense is opened shared for a moment, one input report is taken (1 s at most)
//! and the handle closed; an Xbox pad is asked of `Windows.Gaming.Input`, else of XInput. With no pad present
//! nothing runs: no thread, no timer, no WinRT, no XInput.
//!
//! [`Pads`] is the bookkeeping, free of Win32 and tested alone; [`Controllers`] is the shell that does the I/O.
//! Not yet proven on a device (docs/research/controllers.md).

use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_ListW, CM_NOTIFY_ACTION, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
    CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_Register_Notification, CM_Unregister_Notification,
    CR_SUCCESS, HCMNOTIFICATION,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDP_CAPS, HidD_FreePreparsedData, HidD_GetHidGuid, HidD_GetPreparsedData, HidP_GetCaps,
    PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::UI::Input::XboxController::{
    BATTERY_DEVTYPE, BATTERY_DEVTYPE_GAMEPAD, XINPUT_BATTERY_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{PCWSTR, s, w};

use juicebar_core::controllers::{
    self, DS_BT_REPORT_ID, DS_USB_REPORT_ID, GamingStatus, Model, Protocol, REFRESH_SECONDS,
};
use juicebar_core::types::{BatteryReading, DeviceKey, Presence, ProviderEvent};
use juicebar_hid::parse_interface_path;

/// How long a DualSense is given to send one input report.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(1);
/// Seconds before the next try after a pad that has never given a reading fails to: a pad that has just connected
/// is often not ready yet. After these, the slow refresh.
const RETRY_SECONDS: [f64; 3] = [5.0, 15.0, 60.0];
/// XInput serves four users.
const XINPUT_USERS: u32 = 4;

// ---------------------------------------------------------------- bookkeeping (no Win32)

/// A read the shell is asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Sample one input report of a DualSense.
    Hid {
        id: u32,
        path: String,
        bluetooth: bool,
    },
    /// Ask the system's controller APIs for an Xbox pad's battery.
    Xbox { id: u32 },
}

#[derive(Debug)]
struct Pad {
    id: u32,
    path_lower: String,
    model: &'static Model,
    bluetooth: bool,
    key: DeviceKey,
    path: String,
    /// The registry knows it: set by the first reading.
    announced: bool,
    /// A read is in flight.
    busy: bool,
    /// Monotonic moment of the next read.
    due: Option<f64>,
    failures: usize,
}

/// The pads present, when each is read next, and the events a reading turns into.
#[derive(Debug, Default)]
pub struct Pads {
    pads: Vec<Pad>,
    next_id: u32,
}

impl Pads {
    /// A HID interface arrived. `None` when it is not a pad of the catalog, not the pad's first collection, or
    /// already known; otherwise the pad's id. It is read at once.
    pub fn arrived(&mut self, path: &str, now: f64) -> Option<u32> {
        let parsed = parse_interface_path(path)?;
        let model = controllers::find_model(parsed.vid, parsed.pid)?;
        // A pad that splits into collections shows one interface per collection: only the first is the pad.
        if parsed.collection.is_some_and(|c| c != 1) {
            return None;
        }
        let lower = path.to_ascii_lowercase();
        if self.pads.iter().any(|p| p.path_lower == lower) {
            return None;
        }
        let base = controllers::controller_key(model.vid, model.pid, None);
        let key = if self.pads.iter().any(|p| p.key == base) {
            // A second pad of the same product at the same time: told apart by its (hashed) path.
            controllers::controller_key(model.vid, model.pid, Some(&lower))
        } else {
            base
        };
        self.next_id += 1;
        self.pads.push(Pad {
            id: self.next_id,
            path_lower: lower,
            model,
            bluetooth: parsed.bluetooth,
            key,
            path: path.to_string(),
            announced: false,
            busy: false,
            due: Some(now),
            failures: 0,
        });
        Some(self.next_id)
    }

    /// A HID interface went away: the pad leaves the registry if it was in it.
    pub fn removed(&mut self, path: &str) -> Vec<ProviderEvent> {
        let lower = path.to_ascii_lowercase();
        let Some(at) = self.pads.iter().position(|p| p.path_lower == lower) else {
            return Vec::new();
        };
        let pad = self.pads.remove(at);
        if pad.announced {
            vec![ProviderEvent::Disappeared(pad.key)]
        } else {
            Vec::new()
        }
    }

    /// The reads that are due at `now`; each pad asked is busy until [`Pads::outcome`].
    pub fn take_due(&mut self, now: f64) -> Vec<Ask> {
        let mut asks = Vec::new();
        for pad in &mut self.pads {
            if pad.busy || pad.due.is_none_or(|d| d > now) {
                continue;
            }
            pad.busy = true;
            pad.due = None;
            asks.push(match pad.model.protocol {
                Protocol::DualSense => Ask::Hid {
                    id: pad.id,
                    path: pad.path.clone(),
                    bluetooth: pad.bluetooth,
                },
                Protocol::Xbox => Ask::Xbox { id: pad.id },
            });
        }
        asks
    }

    /// A read finished. A reading announces the pad the first time (appeared, on, battery) and is a battery event
    /// after; no reading schedules a retry while the pad has never answered, and the slow refresh after.
    pub fn outcome(
        &mut self,
        id: u32,
        reading: Option<BatteryReading>,
        now: f64,
        wall: f64,
    ) -> Vec<ProviderEvent> {
        let Some(pad) = self.pads.iter_mut().find(|p| p.id == id) else {
            return Vec::new();
        };
        pad.busy = false;
        let Some(reading) = reading else {
            let wait = if pad.announced {
                None
            } else {
                RETRY_SECONDS.get(pad.failures).copied()
            };
            pad.failures += 1;
            pad.due = Some(now + wait.unwrap_or_else(|| f64::from(REFRESH_SECONDS)));
            return Vec::new();
        };
        pad.failures = 0;
        pad.due = Some(now + f64::from(REFRESH_SECONDS));
        let mut events = Vec::new();
        if !pad.announced {
            pad.announced = true;
            events.push(ProviderEvent::Appeared(controllers::device_info(
                pad.model,
                pad.key.clone(),
                pad.bluetooth,
            )));
            events.push(ProviderEvent::Presence(
                pad.key.clone(),
                Presence {
                    online: true,
                    at: wall,
                },
            ));
        }
        events.push(ProviderEvent::Battery(pad.key.clone(), reading));
        events
    }

    /// The system's controller list changed: Xbox pads that have not answered yet are tried again now.
    pub fn xbox_list_changed(&mut self, now: f64) {
        for pad in &mut self.pads {
            if pad.model.protocol == Protocol::Xbox && !pad.announced && !pad.busy {
                pad.due = Some(now);
            }
        }
    }

    /// The earliest moment a read is due, if any pad waits for one.
    pub fn next_due(&self) -> Option<f64> {
        self.pads
            .iter()
            .filter_map(|p| p.due)
            .min_by(f64::total_cmp)
    }

    pub fn xbox_count(&self) -> usize {
        self.pads
            .iter()
            .filter(|p| p.model.protocol == Protocol::Xbox)
            .count()
    }

    /// Where a pad stands among the pads of its vendor and product, and its model.
    fn position_of(&self, id: u32) -> Option<(&'static Model, usize)> {
        let pad = self.pads.iter().find(|p| p.id == id)?;
        let ordinal = self
            .pads
            .iter()
            .take_while(|p| p.id != id)
            .filter(|p| p.model.vid == pad.model.vid && p.model.pid == pad.model.pid)
            .count();
        Some((pad.model, ordinal))
    }

    fn is_bluetooth(&self, id: u32) -> Option<bool> {
        self.pads.iter().find(|p| p.id == id).map(|p| p.bluetooth)
    }

    #[cfg(test)]
    fn key_of(&self, id: u32) -> Option<DeviceKey> {
        self.pads.iter().find(|p| p.id == id).map(|p| p.key.clone())
    }
}

// ---------------------------------------------------------------- the inbox

/// What the notification callback, the sampling threads and the WinRT handlers hand to the window.
#[derive(Debug)]
enum Note {
    Arrived(String),
    Removed(String),
    Sampled(u32, Option<Vec<u8>>),
    XboxListChanged,
}

/// A queue and the way to wake the window: one message is posted when the queue goes from empty to not empty.
struct Inbox {
    notes: Mutex<Vec<Note>>,
    hwnd: isize,
    message: u32,
}

impl Inbox {
    fn push(&self, note: Note) {
        let first = {
            let mut notes = self.notes.lock().unwrap_or_else(|e| e.into_inner());
            notes.push(note);
            notes.len() == 1
        };
        if first {
            // SAFETY: posting to a window by handle from any thread is allowed; a failure (the window is gone) is
            // harmless.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut c_void)),
                    self.message,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    fn take(&self) -> Vec<Note> {
        std::mem::take(&mut *self.notes.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

// ---------------------------------------------------------------- the device notification

/// The registration of the HID interface notification and the context the callback reads.
struct Registration {
    notify: HCMNOTIFICATION,
    context: *const Inbox,
}

impl Registration {
    fn new(inbox: &Arc<Inbox>) -> Option<Registration> {
        let context = Arc::into_raw(inbox.clone());
        let mut filter = CM_NOTIFY_FILTER {
            cbSize: size_of::<CM_NOTIFY_FILTER>() as u32,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            ..Default::default()
        };
        // SAFETY: HidD_GetHidGuid has no preconditions; the union field written is the one `FilterType` selects.
        unsafe {
            filter.u.DeviceInterface.ClassGuid = HidD_GetHidGuid();
        }
        let mut notify = HCMNOTIFICATION::default();
        // SAFETY: `filter` and `notify` are valid for the call; `context` stays valid until the unregistration in
        // `Drop`, which waits for a running callback.
        let cr = unsafe {
            CM_Register_Notification(
                &filter,
                Some(context.cast()),
                Some(on_interface),
                &mut notify,
            )
        };
        if cr != CR_SUCCESS {
            // SAFETY: `context` came from Arc::into_raw above and was never handed out.
            drop(unsafe { Arc::from_raw(context) });
            return None;
        }
        Some(Registration { notify, context })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: `notify` came from a successful CM_Register_Notification and is unregistered once; this waits for
        // a running callback.
        let _ = unsafe { CM_Unregister_Notification(self.notify) };
        // SAFETY: `context` came from Arc::into_raw in `new`; no callback runs any more.
        drop(unsafe { Arc::from_raw(self.context) });
    }
}

/// The callback. Runs on a system thread: it parses the path, keeps what could be a pad and posts a message.
unsafe extern "system" fn on_interface(
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
    // SAFETY: `context` is the pointer from Arc::into_raw; the Arc is released only after the registration is
    // removed, which waits for running callbacks.
    let inbox = unsafe { &*(context as *const Inbox) };
    // SAFETY: for a device-interface event the union holds DeviceInterface and `event_data_size` bytes are
    // readable; the path is the NUL-terminated string at `SymbolicLink`.
    let path = unsafe {
        let link = std::ptr::addr_of!((*event_data).u.DeviceInterface.SymbolicLink) as *const u16;
        let offset = link as usize - event_data as usize;
        let units = (event_data_size as usize).saturating_sub(offset) / 2;
        let slice = std::slice::from_raw_parts(link, units);
        let end = slice.iter().position(|c| *c == 0).unwrap_or(slice.len());
        String::from_utf16_lossy(&slice[..end])
    };
    let is_pad = parse_interface_path(&path).is_some_and(|p| {
        controllers::is_candidate_vendor(p.vid) && controllers::find_model(p.vid, p.pid).is_some()
    });
    if is_pad {
        inbox.push(if arrival {
            Note::Arrived(path)
        } else {
            Note::Removed(path)
        });
    }
    0
}

/// The HID interfaces present now.
fn present_paths() -> Vec<String> {
    // SAFETY: HidD_GetHidGuid has no preconditions and returns the class GUID by value.
    let guid = unsafe { HidD_GetHidGuid() };
    let mut len = 0u32;
    // SAFETY: `len` and `guid` are valid for the call; a null device id asks for all devices.
    let size_ok = unsafe {
        CM_Get_Device_Interface_List_SizeW(
            &mut len,
            &guid,
            PCWSTR::null(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    } == CR_SUCCESS;
    if !size_ok || len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` has the length the size query returned for the same filter.
    let list_ok = unsafe {
        CM_Get_Device_Interface_ListW(
            &guid,
            PCWSTR::null(),
            &mut buf,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    } == CR_SUCCESS;
    if !list_ok {
        return Vec::new();
    }
    buf.split(|c| *c == 0)
        .filter(|c| !c.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

// ---------------------------------------------------------------- sampling a DualSense

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateFileW or CreateEventW and is closed once here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn caps_of(h: HANDLE) -> Option<HIDP_CAPS> {
    let mut pre = PHIDP_PREPARSED_DATA::default();
    // SAFETY: `h` is a valid HID handle and `pre` receives the preparsed data pointer.
    if !unsafe { HidD_GetPreparsedData(h, &mut pre) } {
        return None;
    }
    let mut caps = HIDP_CAPS::default();
    // SAFETY: `pre` was just returned by HidD_GetPreparsedData and is freed right after.
    let status = unsafe { HidP_GetCaps(pre, &mut caps) };
    // SAFETY: `pre` is the pointer obtained above and is not used again.
    let _ = unsafe { HidD_FreePreparsedData(pre) };
    (status.0 >= 0).then_some(caps)
}

/// Opens the pad shared, reads one input report within [`SAMPLE_TIMEOUT`] and closes it. Nothing is written or
/// asked of the pad (a feature read would switch a Bluetooth pad out of the simple mode a game uses): a
/// Bluetooth pad that is not already sending report 0x31 gives no reading. `None` when the pad cannot be opened,
/// is not a game pad collection, or sends nothing in time. Runs on a short-lived thread.
fn sample_dualsense(path: &str, bluetooth: bool) -> Option<Vec<u8>> {
    let wide = windows::core::HSTRING::from(path);
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call. Read access and a shared open: the
    // pad stays usable by the game that has it.
    let file = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .ok()?;
    let file = Handle(file);
    let caps = caps_of(file.0)?;
    // Usage page 1 (generic desktop), usage 4 (joystick) or 5 (game pad).
    if caps.UsagePage != 1 || !matches!(caps.Usage, 4 | 5) {
        return None;
    }
    let want = if bluetooth {
        DS_BT_REPORT_ID
    } else {
        DS_USB_REPORT_ID
    };
    read_report(file.0, usize::from(caps.InputReportByteLength), want)
}

/// Whether a report with this id ends the sampling: the wanted one, or, for a Bluetooth pad, the short
/// "simple" report (0x01) it sends until a program switches it: that pad gives no reading, and waiting out
/// the timeout would change nothing.
fn sampling_done(id: u8, want: u8) -> bool {
    id == want || (want == DS_BT_REPORT_ID && id == DS_USB_REPORT_ID)
}

/// One input report of an opened HID handle within [`SAMPLE_TIMEOUT`]: reports of another id are passed over
/// until `want` (or a Bluetooth pad's short mode, see `sampling_done`) comes or the time is up, and the last one read is
/// returned. The read is always ended before this returns, so no buffer is left in the kernel's hands.
fn read_report(file: HANDLE, input_len: usize, want: u8) -> Option<Vec<u8>> {
    // SAFETY: plain event creation: manual reset, initially not signaled, unnamed.
    let event = Handle(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?);
    let mut buf = vec![0u8; input_len.max(1)];
    let deadline = Instant::now() + SAMPLE_TIMEOUT;
    let mut last = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut ov = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        // SAFETY: `buf` and `ov` live until the read completes or is cancelled and waited for below.
        let started = unsafe { ReadFile(file, Some(&mut buf[..]), None, Some(&mut ov)) };
        match started {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
            Err(_) => break,
        }
        // SAFETY: a wait on the event created above.
        let waited = unsafe { WaitForSingleObject(event.0, left.as_millis().max(1) as u32) };
        let mut n = 0u32;
        if waited == WAIT_OBJECT_0 {
            // SAFETY: the OVERLAPPED is the one used by the completed read on this handle.
            if unsafe { GetOverlappedResult(file, &ov, &mut n, false) }.is_err() {
                break;
            }
            let report = buf[..(n as usize).min(buf.len())].to_vec();
            let right = report.first().is_some_and(|id| sampling_done(*id, want));
            last = Some(report);
            if right {
                break;
            }
        } else {
            // SAFETY: cancels the one read this thread started, then waits for it to end so `buf` and `ov` are no
            // longer in the kernel's hands.
            unsafe {
                let _ = CancelIoEx(file, Some(&ov));
                let _ = GetOverlappedResult(file, &ov, &mut n, true);
            }
            break;
        }
    }
    last
}

// ---------------------------------------------------------------- the Xbox side

type XInputGetBattery =
    unsafe extern "system" fn(u32, BATTERY_DEVTYPE, *mut XINPUT_BATTERY_INFORMATION) -> u32;

/// `Windows.Gaming.Input`, started when the first Xbox pad arrives, and XInput, loaded when it is first needed.
/// Neither is touched while no Xbox pad is present. The WinRT runtime, once started, stays for the process.
#[derive(Default)]
struct XboxApi {
    watching: Option<(i64, i64)>,
    xinput: Option<Option<XInputGetBattery>>,
}

impl XboxApi {
    /// Listens for the system's controller list to change (a new pad is often seen by the system after its HID
    /// interface arrives).
    fn watch(&mut self, inbox: &Arc<Inbox>) {
        use windows::Foundation::EventHandler;
        use windows::Gaming::Input::RawGameController;
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        if self.watching.is_some() {
            return;
        }
        // SAFETY: initializes the WinRT runtime for this thread; a different mode already set is fine (the calls
        // below work in either) and reported as an error that is ignored.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let handler = |inbox: &Arc<Inbox>| {
            let inbox = inbox.clone();
            EventHandler::<RawGameController>::new(move |_, _| {
                inbox.push(Note::XboxListChanged);
                Ok(())
            })
        };
        let added = RawGameController::RawGameControllerAdded(&handler(inbox));
        let removed = RawGameController::RawGameControllerRemoved(&handler(inbox));
        if let (Ok(a), Ok(r)) = (added, removed) {
            self.watching = Some((a, r));
        }
    }

    fn unwatch(&mut self) {
        use windows::Gaming::Input::RawGameController;
        if let Some((a, r)) = self.watching.take() {
            let _ = RawGameController::RemoveRawGameControllerAdded(a);
            let _ = RawGameController::RemoveRawGameControllerRemoved(r);
        }
    }

    /// The battery of the `ordinal`-th pad of this vendor and product (the system lists them in a stable order):
    /// the Windows.Gaming.Input report, else XInput when exactly one Xbox pad is present (XInput numbers its
    /// slots by user, not by device, so with more it cannot say which slot is which pad).
    fn read(
        &mut self,
        model: &Model,
        ordinal: usize,
        xbox_pads: usize,
        wall: f64,
    ) -> Option<BatteryReading> {
        self.gaming_input(model, ordinal, wall)
            .or_else(|| (xbox_pads == 1).then(|| self.xinput(wall)).flatten())
    }

    fn gaming_input(&mut self, model: &Model, ordinal: usize, wall: f64) -> Option<BatteryReading> {
        use windows::Gaming::Input::RawGameController;
        let list = RawGameController::RawGameControllers().ok()?;
        let mut seen = 0;
        for pad in list {
            if pad.HardwareVendorId().ok()? != model.vid
                || pad.HardwareProductId().ok()? != model.pid
            {
                continue;
            }
            if seen < ordinal {
                seen += 1;
                continue;
            }
            let report = pad.TryGetBatteryReport().ok()?;
            let capacity = |v: windows::core::Result<windows::Foundation::IReference<i32>>| {
                v.ok().and_then(|r| r.Value().ok())
            };
            return controllers::gaming_reading(
                capacity(report.RemainingCapacityInMilliwattHours()),
                capacity(report.FullChargeCapacityInMilliwattHours()),
                GamingStatus::from_code(report.Status().ok()?.0)?,
                wall,
            );
        }
        None
    }

    /// XInput's battery level of the one connected slot; none when no slot or several are connected.
    fn xinput(&mut self, wall: f64) -> Option<BatteryReading> {
        let get = (*self.xinput.get_or_insert_with(load_xinput))?;
        let mut found = None;
        for user in 0..XINPUT_USERS {
            let mut info = XINPUT_BATTERY_INFORMATION::default();
            // SAFETY: `get` is XInputGetBatteryInformation from xinput1_4.dll; `info` is a valid out structure.
            let rc = unsafe { get(user, BATTERY_DEVTYPE_GAMEPAD, &mut info) };
            if rc == 0 && info.BatteryType.0 != controllers::xinput::TYPE_DISCONNECTED {
                if found.is_some() {
                    return None;
                }
                found = Some(info);
            }
        }
        let info = found?;
        controllers::xinput_reading(info.BatteryType.0, info.BatteryLevel.0, wall)
    }
}

/// Loads xinput1_4.dll on first use, so the process does not carry it while no Xbox pad is present.
fn load_xinput() -> Option<XInputGetBattery> {
    // SAFETY: loading a system library by name; the module stays loaded for the process.
    let module = unsafe { LoadLibraryW(w!("xinput1_4.dll")) }.ok()?;
    // SAFETY: looking a named export up in the module just loaded.
    let proc = unsafe { GetProcAddress(module, s!("XInputGetBatteryInformation")) }?;
    // SAFETY: the export has exactly this signature (xinput.h).
    Some(unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, XInputGetBattery>(proc)
    })
}

// ---------------------------------------------------------------- the shell

/// The provider as the app drives it: it owns the notification, the bookkeeping and the Xbox APIs.
pub struct Controllers {
    inbox: Arc<Inbox>,
    registration: Option<Registration>,
    pads: Pads,
    xbox: XboxApi,
}

impl Controllers {
    /// Registers for HID interface arrivals and removals (before listing, so none is missed) and queues the pads
    /// already present. Messages `message` are posted to `hwnd` when there is something to take.
    pub fn start(hwnd: HWND, message: u32) -> Controllers {
        let inbox = Arc::new(Inbox {
            notes: Mutex::new(Vec::new()),
            hwnd: hwnd.0 as isize,
            message,
        });
        let registration = Registration::new(&inbox);
        for path in present_paths() {
            if parse_interface_path(&path)
                .is_some_and(|p| controllers::find_model(p.vid, p.pid).is_some())
            {
                inbox.push(Note::Arrived(path));
            }
        }
        Controllers {
            inbox,
            registration,
            pads: Pads::default(),
            xbox: XboxApi::default(),
        }
    }

    /// Takes what arrived and does the reads that are due. `mono` and `wall` are the app's clocks. The events go
    /// to the registry the way the Logitech provider's do.
    pub fn run(&mut self, mono: f64, wall: f64) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        for note in self.inbox.take() {
            match note {
                Note::Arrived(path) => {
                    self.pads.arrived(&path, mono);
                }
                Note::Removed(path) => events.extend(self.pads.removed(&path)),
                Note::Sampled(id, report) => {
                    let reading = report.and_then(|r| {
                        let bluetooth = self.pads.is_bluetooth(id)?;
                        controllers::decode_dualsense(&r, bluetooth, wall)
                    });
                    events.extend(self.pads.outcome(id, reading, mono, wall));
                }
                Note::XboxListChanged => self.pads.xbox_list_changed(mono),
            }
        }
        if self.pads.xbox_count() == 0 {
            self.xbox.unwatch();
        } else {
            self.xbox.watch(&self.inbox);
        }
        for ask in self.pads.take_due(mono) {
            match ask {
                Ask::Hid {
                    id,
                    path,
                    bluetooth,
                } => self.sample_in_thread(id, path, bluetooth),
                Ask::Xbox { id } => {
                    let reading = self.pads.position_of(id).and_then(|(model, ordinal)| {
                        self.xbox.read(model, ordinal, self.pads.xbox_count(), wall)
                    });
                    events.extend(self.pads.outcome(id, reading, mono, wall));
                }
            }
        }
        events
    }

    /// The monotonic moment the app's pad timer should fire at, if any pad is waiting for a read.
    pub fn next_due(&self) -> Option<f64> {
        self.pads.next_due()
    }

    /// Releases the notification and the system's controller listeners.
    pub fn stop(&mut self) {
        self.registration = None;
        self.xbox.unwatch();
    }

    fn sample_in_thread(&self, id: u32, path: String, bluetooth: bool) {
        let inbox = self.inbox.clone();
        let spawned = std::thread::Builder::new()
            .name("juicebar-pad".into())
            .stack_size(256 * 1024)
            .spawn({
                let inbox = inbox.clone();
                move || {
                    let report = sample_dualsense(&path, bluetooth);
                    inbox.push(Note::Sampled(id, report));
                }
            });
        if spawned.is_err() {
            inbox.push(Note::Sampled(id, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use juicebar_core::types::{Charging, Coarse, Quality};

    const DS_USB: &str =
        r"\\?\hid#vid_054c&pid_0ce6#7&2c6e5d0&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const DS_USB_2: &str =
        r"\\?\hid#vid_054c&pid_0ce6#7&99aa11&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const DS_BT: &str = r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6#a&1b2c3d&0&a1b2c3d4e5f6_c00000000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const XBOX_COL1: &str =
        r"\\?\hid#vid_045e&pid_0b12&col01#7&1&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const XBOX_COL2: &str =
        r"\\?\hid#vid_045e&pid_0b12&col02#7&1&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const MOUSE: &str =
        r"\\?\hid#vid_045e&pid_082a#7&1&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
    const LOGITECH: &str = r"\\?\hid#vid_046d&pid_c539&mi_02&col01#8&2a8b&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";

    fn reading(percent: f64) -> BatteryReading {
        BatteryReading {
            at: 1.0,
            percent: Some(percent),
            coarse: None,
            millivolts: None,
            charging: Charging::Discharging,
            quality: Quality::Exact,
        }
    }

    #[test]
    fn only_catalog_pads_are_taken_and_a_pad_is_read_at_once() {
        let mut pads = Pads::default();
        assert_eq!(
            pads.arrived(MOUSE, 0.0),
            None,
            "a Microsoft mouse is not a pad"
        );
        assert_eq!(pads.arrived(LOGITECH, 0.0), None);
        assert_eq!(pads.arrived("not a path", 0.0), None);
        assert_eq!(pads.next_due(), None, "nothing is scheduled with no pad");
        let id = pads.arrived(DS_USB, 10.0).unwrap();
        assert_eq!(pads.next_due(), Some(10.0));
        assert_eq!(
            pads.arrived(DS_USB, 10.0),
            None,
            "the same interface twice is one pad"
        );
        assert_eq!(
            pads.arrived(&DS_USB.to_uppercase(), 10.0),
            None,
            "paths compare ignoring case"
        );
        assert_eq!(
            pads.take_due(10.0),
            vec![Ask::Hid {
                id,
                path: DS_USB.to_string(),
                bluetooth: false
            }]
        );
        assert!(
            pads.take_due(10.0).is_empty(),
            "a pad is not asked while a read is in flight"
        );
    }

    #[test]
    fn the_bluetooth_flag_follows_the_path() {
        let mut pads = Pads::default();
        let id = pads.arrived(DS_BT, 0.0).unwrap();
        match &pads.take_due(0.0)[0] {
            Ask::Hid {
                id: got, bluetooth, ..
            } => assert_eq!((*got, *bluetooth), (id, true)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_first_reading_announces_the_pad_and_later_ones_are_battery_events() {
        let mut pads = Pads::default();
        let id = pads.arrived(DS_USB, 0.0).unwrap();
        pads.take_due(0.0);
        let events = pads.outcome(id, Some(reading(75.0)), 0.5, 1000.0);
        let key = DeviceKey("game-controllers:054C:0CE6".into());
        match events.as_slice() {
            [
                ProviderEvent::Appeared(info),
                ProviderEvent::Presence(k, p),
                ProviderEvent::Battery(k2, b),
            ] => {
                assert_eq!(info.key, key);
                assert_eq!(info.name, "DualSense");
                assert_eq!((k, k2), (&key, &key));
                assert_eq!((p.online, p.at), (true, 1000.0));
                assert_eq!(b.percent, Some(75.0));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(pads.next_due(), Some(0.5 + 600.0), "the slow refresh");
        assert!(pads.take_due(100.0).is_empty());
        let again = pads.take_due(601.0);
        assert_eq!(again.len(), 1);
        let events = pads.outcome(id, Some(reading(70.0)), 601.0, 2000.0);
        assert!(matches!(events.as_slice(), [ProviderEvent::Battery(..)]));
    }

    #[test]
    fn a_pad_that_does_not_answer_is_retried_soon_and_then_slowly_and_stays_unannounced() {
        let mut pads = Pads::default();
        let id = pads.arrived(XBOX_COL1, 0.0).unwrap();
        let mut now = 0.0;
        for wait in [5.0, 15.0, 60.0, 600.0, 600.0] {
            assert_eq!(pads.take_due(now).len(), 1, "due at {now}");
            assert!(
                pads.outcome(id, None, now, 0.0).is_empty(),
                "no event for a pad that never answered"
            );
            assert_eq!(pads.next_due(), Some(now + wait));
            now += wait;
        }
        // It finally answers: now it appears.
        pads.take_due(now);
        let coarse = BatteryReading {
            at: 1.0,
            percent: None,
            coarse: Some(Coarse::Good),
            millivolts: None,
            charging: Charging::Unknown,
            quality: Quality::Coarse,
        };
        let events = pads.outcome(id, Some(coarse), now, 5.0);
        assert!(matches!(events.first(), Some(ProviderEvent::Appeared(_))));
        // An announced pad that fails once waits the slow refresh, not a retry.
        pads.take_due(now + 600.0);
        assert!(pads.outcome(id, None, now + 600.0, 6.0).is_empty());
        assert_eq!(pads.next_due(), Some(now + 1200.0));
    }

    #[test]
    fn removal_disappears_an_announced_pad_only_and_stops_its_reads() {
        let mut pads = Pads::default();
        let silent = pads.arrived(XBOX_COL1, 0.0).unwrap();
        let heard = pads.arrived(DS_USB, 0.0).unwrap();
        pads.take_due(0.0);
        pads.outcome(heard, Some(reading(50.0)), 0.0, 1.0);
        assert!(
            pads.removed(XBOX_COL1).is_empty(),
            "never announced, nothing to take away"
        );
        assert_eq!(
            pads.removed(DS_USB),
            vec![ProviderEvent::Disappeared(DeviceKey(
                "game-controllers:054C:0CE6".into()
            ))]
        );
        assert_eq!(pads.next_due(), None, "no pad, no read due");
        assert!(pads.removed(DS_USB).is_empty());
        // A read that finishes after the pad left changes nothing.
        assert!(
            pads.outcome(silent, Some(reading(1.0)), 3.0, 3.0)
                .is_empty()
        );
        assert!(pads.outcome(heard, Some(reading(1.0)), 3.0, 3.0).is_empty());
    }

    #[test]
    fn a_split_pad_is_one_pad_by_its_first_collection() {
        let mut pads = Pads::default();
        assert!(pads.arrived(XBOX_COL2, 0.0).is_none());
        assert!(pads.arrived(XBOX_COL1, 0.0).is_some());
        assert_eq!(pads.xbox_count(), 1);
    }

    #[test]
    fn a_second_pad_of_the_same_product_gets_a_hashed_key_and_the_first_keeps_its_own() {
        let mut pads = Pads::default();
        let a = pads.arrived(DS_USB, 0.0).unwrap();
        let b = pads.arrived(DS_USB_2, 0.0).unwrap();
        let ka = pads.key_of(a).unwrap();
        let kb = pads.key_of(b).unwrap();
        assert_eq!(ka.0, "game-controllers:054C:0CE6");
        assert!(kb.0.starts_with("game-controllers:054C:0CE6:") && kb.0.len() == ka.0.len() + 17);
        // A bluetooth pad of the same product is another pad again.
        let c = pads.arrived(DS_BT, 0.0).unwrap();
        let kc = pads.key_of(c).unwrap();
        assert!(kc != ka && kc != kb);
        assert!(
            !kc.0.contains("a1b2c3d4e5f6"),
            "an address never reaches a key"
        );
        assert_eq!(
            pads.position_of(b).map(|(m, o)| (m.pid, o)),
            Some((0x0CE6, 1))
        );
    }

    #[test]
    fn a_changed_system_list_retries_only_the_xbox_pads_that_have_not_answered() {
        let mut pads = Pads::default();
        let x = pads.arrived(XBOX_COL1, 0.0).unwrap();
        let d = pads.arrived(DS_USB, 0.0).unwrap();
        pads.take_due(0.0);
        pads.outcome(x, None, 0.0, 0.0);
        pads.outcome(d, None, 0.0, 0.0);
        pads.xbox_list_changed(2.0);
        assert_eq!(pads.take_due(2.0), vec![Ask::Xbox { id: x }]);
        // A busy pad is left alone.
        pads.xbox_list_changed(3.0);
        assert!(pads.take_due(3.0).is_empty());
    }

    #[test]
    fn the_core_constants_are_the_ones_the_windows_headers_define() {
        use windows::Win32::UI::Input::XboxController as x;
        assert_eq!(
            controllers::xinput::TYPE_DISCONNECTED,
            x::BATTERY_TYPE_DISCONNECTED.0
        );
        assert_eq!(controllers::xinput::TYPE_WIRED, x::BATTERY_TYPE_WIRED.0);
        assert_eq!(
            controllers::xinput::TYPE_ALKALINE,
            x::BATTERY_TYPE_ALKALINE.0
        );
        assert_eq!(controllers::xinput::TYPE_NIMH, x::BATTERY_TYPE_NIMH.0);
        assert_eq!(controllers::xinput::TYPE_UNKNOWN, x::BATTERY_TYPE_UNKNOWN.0);
        assert_eq!(controllers::xinput::LEVEL_EMPTY, x::BATTERY_LEVEL_EMPTY.0);
        assert_eq!(controllers::xinput::LEVEL_LOW, x::BATTERY_LEVEL_LOW.0);
        assert_eq!(controllers::xinput::LEVEL_MEDIUM, x::BATTERY_LEVEL_MEDIUM.0);
        assert_eq!(controllers::xinput::LEVEL_FULL, x::BATTERY_LEVEL_FULL.0);
        use windows::System::Power::BatteryStatus as s;
        assert_eq!(
            GamingStatus::from_code(s::NotPresent.0),
            Some(GamingStatus::NotPresent)
        );
        assert_eq!(
            GamingStatus::from_code(s::Discharging.0),
            Some(GamingStatus::Discharging)
        );
        assert_eq!(GamingStatus::from_code(s::Idle.0), Some(GamingStatus::Idle));
        assert_eq!(
            GamingStatus::from_code(s::Charging.0),
            Some(GamingStatus::Charging)
        );
    }

    #[test]
    fn the_device_notification_registers_and_releases_without_a_pad() {
        // A window handle that does not exist: posts to it fail harmlessly.
        let mut c = Controllers::start(HWND::default(), 0);
        assert!(c.registration.is_some(), "CM_Register_Notification refused");
        c.stop();
        assert!(c.registration.is_none());
        assert_eq!(
            Arc::strong_count(&c.inbox),
            1,
            "the callback context was released"
        );
    }

    /// Manual: reads from a real HID collection that is present (a Logitech vendor collection) for a report id that
    /// never comes, to prove that the overlapped read, its cancellation and the closing finish within the time
    /// allowed and leave nothing behind. Run with `--ignored`.
    #[test]
    #[ignore = "needs a HID device on this machine"]
    fn a_read_that_gets_nothing_ends_in_time_and_leaves_the_handle_usable() {
        for path in present_paths() {
            let Some(p) = parse_interface_path(&path) else {
                continue;
            };
            if p.vid != 0x046D {
                continue;
            }
            let wide = windows::core::HSTRING::from(path.as_str());
            // SAFETY: a NUL-terminated string that outlives the call; read access, shared.
            let Ok(file) = (unsafe {
                CreateFileW(
                    PCWSTR(wide.as_ptr()),
                    GENERIC_READ.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    None,
                )
            }) else {
                continue;
            };
            let file = Handle(file);
            let Some(caps) = caps_of(file.0) else {
                continue;
            };
            let began = Instant::now();
            let report = read_report(file.0, usize::from(caps.InputReportByteLength), 0xEE);
            let took = began.elapsed();
            println!("{path}: {:?} after {took:?}", report.as_ref().map(Vec::len));
            assert!(took < Duration::from_millis(1500), "{took:?}");
            // The handle still reads (a second round also ends in time).
            let began = Instant::now();
            let _ = read_report(file.0, usize::from(caps.InputReportByteLength), 0xEE);
            assert!(began.elapsed() < Duration::from_millis(1500));
            return;
        }
        panic!("no Logitech HID collection could be opened");
    }

    #[test]
    fn the_sampler_never_reads_a_feature_report() {
        // A feature read switches a Bluetooth pad out of the simple mode the game uses.
        let source = include_str!("controllers.rs");
        let call = ["HidD_Get", "Feature("].concat();
        assert_eq!(source.matches(&call).count(), 0);
    }

    #[test]
    fn a_bluetooth_pad_in_its_simple_mode_ends_the_sample_with_no_reading() {
        assert!(sampling_done(DS_BT_REPORT_ID, DS_BT_REPORT_ID));
        assert!(sampling_done(0x01, DS_BT_REPORT_ID), "simple mode report");
        assert!(!sampling_done(0x05, DS_BT_REPORT_ID));
        assert!(sampling_done(DS_USB_REPORT_ID, DS_USB_REPORT_ID));
        assert!(!sampling_done(DS_BT_REPORT_ID, DS_USB_REPORT_ID));
        let simple = [0x01, 0x80, 0x80, 0x80, 0x80, 0x08, 0, 0, 0, 0];
        assert!(controllers::decode_dualsense(&simple, true, 0.0).is_none());
    }
}
