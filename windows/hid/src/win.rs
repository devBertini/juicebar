// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Win32 HID access: listing the present interfaces, reading a collection's capabilities, and
//! `Port`, an opened collection with one outstanding overlapped read and a queue of overlapped
//! writes. Nothing here blocks except `Drop`, which waits for cancelled I/O.

use std::collections::VecDeque;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_ListW, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDP_CAPS, HidD_FreePreparsedData, HidD_GetHidGuid, HidD_GetPreparsedData,
    HidD_GetProductString, HidP_GetCaps, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
use windows::core::PCWSTR;

use crate::endpoint::Endpoint;

/// Win32 error codes that mean the device is gone (not a transient write failure).
const GONE_CODES: [u32; 4] = [
    6,    // ERROR_INVALID_HANDLE
    433,  // ERROR_NO_SUCH_DEVICE
    1117, // ERROR_IO_DEVICE
    1167, // ERROR_DEVICE_NOT_CONNECTED
];

fn win32_code(e: &windows::core::Error) -> u32 {
    // HRESULT_FROM_WIN32 keeps the Win32 code in the low 16 bits.
    (e.code().0 as u32) & 0xFFFF
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Capabilities of a collection that the transport needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caps {
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
}

/// The device interface paths of every HID collection present now.
pub(crate) fn present_paths() -> Vec<String> {
    // SAFETY: HidD_GetHidGuid has no preconditions and returns the class GUID by value.
    let guid = unsafe { HidD_GetHidGuid() };
    let mut len = 0u32;
    // SAFETY: `len` and `guid` are valid for the call; a null device ID asks for all devices.
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

fn open_handle(path: &str, access: u32, overlapped: bool) -> Result<HANDLE, u32> {
    let w = wide(path);
    let flags = if overlapped {
        FILE_FLAG_OVERLAPPED
    } else {
        Default::default()
    };
    // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .map_err(|e| win32_code(&e))
}

fn read_caps(h: HANDLE) -> Option<Caps> {
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
    if status.0 < 0 {
        return None;
    }
    Some(Caps {
        usage_page: caps.UsagePage,
        usage: caps.Usage,
        input_len: caps.InputReportByteLength,
        output_len: caps.OutputReportByteLength,
    })
}

fn product_string(h: HANDLE) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: the buffer pointer and its byte length describe `buf` exactly.
    let ok = unsafe { HidD_GetProductString(h, buf.as_mut_ptr().cast(), size_of_val(&buf) as u32) };
    if !ok {
        return String::new();
    }
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Capabilities and product string of the collection at `path`, read through a handle with no
/// access rights (it works for keyboards and mice too and touches nothing).
pub(crate) fn query(path: &str) -> Option<(Caps, String)> {
    let h = open_handle(path, 0, false).ok()?;
    let caps = read_caps(h);
    let product = product_string(h);
    // SAFETY: `h` was opened above and is closed exactly once.
    let _ = unsafe { CloseHandle(h) };
    caps.map(|c| (c, product))
}

fn new_event() -> Result<HANDLE, u32> {
    // SAFETY: plain event creation: manual reset, initially not signaled, unnamed.
    unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| win32_code(&e))
}

/// An opened collection with one outstanding overlapped read and a queue of writes (one in
/// flight at a time). All its buffers are boxed so their addresses stay fixed while the kernel
/// owns them.
pub(crate) struct Port {
    pub endpoint: Endpoint,
    pub path_lower: String,
    pub dead: bool,
    handle: HANDLE,
    read_event: HANDLE,
    read_ov: Box<OVERLAPPED>,
    read_buf: Box<[u8]>,
    read_pending: bool,
    write_event: HANDLE,
    write_ov: Box<OVERLAPPED>,
    write_buf: Box<[u8]>,
    write_pending: bool,
    write_queue: VecDeque<Vec<u8>>,
}

impl Port {
    /// Open `path` read+write, shared, overlapped, and start the first read.
    pub(crate) fn open(path: &str, endpoint: Endpoint) -> Result<Port, u32> {
        let handle = open_handle(path, GENERIC_READ.0 | GENERIC_WRITE.0, true)?;
        let read_event = match new_event() {
            Ok(e) => e,
            Err(code) => {
                // SAFETY: `handle` was opened above and is closed exactly once.
                let _ = unsafe { CloseHandle(handle) };
                return Err(code);
            }
        };
        let write_event = match new_event() {
            Ok(e) => e,
            Err(code) => {
                // SAFETY: both handles were created above and are closed exactly once.
                unsafe {
                    let _ = CloseHandle(read_event);
                    let _ = CloseHandle(handle);
                }
                return Err(code);
            }
        };
        let mut port = Port {
            path_lower: path.to_ascii_lowercase(),
            dead: false,
            handle,
            read_event,
            read_ov: Box::new(OVERLAPPED::default()),
            read_buf: vec![0u8; endpoint.input_len.max(1) as usize].into_boxed_slice(),
            read_pending: false,
            write_event,
            write_ov: Box::new(OVERLAPPED::default()),
            write_buf: Box::default(),
            write_pending: false,
            write_queue: VecDeque::new(),
            endpoint,
        };
        port.start_read();
        if port.dead {
            return Err(0);
        }
        Ok(port)
    }

    fn start_read(&mut self) {
        // SAFETY: the event is a valid handle owned by this port.
        let _ = unsafe { ResetEvent(self.read_event) };
        *self.read_ov = OVERLAPPED::default();
        self.read_ov.hEvent = self.read_event;
        // SAFETY: the buffer and the OVERLAPPED are boxed and live until the read completes or
        // is cancelled in `Drop`.
        let r = unsafe {
            ReadFile(
                self.handle,
                Some(&mut self.read_buf[..]),
                None,
                Some(&mut *self.read_ov),
            )
        };
        match r {
            Ok(()) => self.read_pending = true,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => self.read_pending = true,
            Err(_) => {
                self.read_pending = false;
                self.dead = true;
            }
        }
    }

    /// The event the I/O thread waits on for the read, when a read is outstanding.
    pub(crate) fn read_handle(&self) -> Option<HANDLE> {
        self.read_pending.then_some(self.read_event)
    }

    /// The event for the write in flight, if any.
    pub(crate) fn write_handle(&self) -> Option<HANDLE> {
        self.write_pending.then_some(self.write_event)
    }

    fn signaled(event: HANDLE) -> bool {
        // SAFETY: a zero-timeout wait on a valid event handle.
        unsafe { WaitForSingleObject(event, 0) == WAIT_OBJECT_0 }
    }

    /// A completed read, if one is ready (and the next read started). Marks the port dead when
    /// the read failed.
    pub(crate) fn take_report(&mut self) -> Option<Vec<u8>> {
        if !self.read_pending || !Self::signaled(self.read_event) {
            return None;
        }
        let mut n = 0u32;
        // SAFETY: the OVERLAPPED is the one used by the completed read on this handle.
        let r = unsafe { GetOverlappedResult(self.handle, &*self.read_ov, &mut n, false) };
        self.read_pending = false;
        match r {
            Ok(()) => {
                let frame = self.read_buf[..(n as usize).min(self.read_buf.len())].to_vec();
                self.start_read();
                (!frame.is_empty()).then_some(frame)
            }
            Err(_) => {
                self.dead = true;
                None
            }
        }
    }

    /// Queue one full-length report; it is written as soon as no other write is in flight.
    pub(crate) fn queue_write(&mut self, report: Vec<u8>) {
        self.write_queue.push_back(report);
        if !self.write_pending {
            self.start_next_write();
        }
    }

    fn start_next_write(&mut self) {
        while let Some(report) = self.write_queue.pop_front() {
            self.write_buf = report.into_boxed_slice();
            // SAFETY: the event is a valid handle owned by this port.
            let _ = unsafe { ResetEvent(self.write_event) };
            *self.write_ov = OVERLAPPED::default();
            self.write_ov.hEvent = self.write_event;
            // SAFETY: the write buffer and the OVERLAPPED are boxed and stay untouched until the
            // write completes or is cancelled in `Drop`.
            let r = unsafe {
                WriteFile(
                    self.handle,
                    Some(&self.write_buf[..]),
                    None,
                    Some(&mut *self.write_ov),
                )
            };
            match r {
                Ok(()) => {
                    self.write_pending = true;
                    return;
                }
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    self.write_pending = true;
                    return;
                }
                Err(e) => {
                    // A refused write is dropped: the core times the request out. Only a
                    // vanished device ends the port.
                    if GONE_CODES.contains(&win32_code(&e)) {
                        self.dead = true;
                        return;
                    }
                }
            }
        }
    }

    /// Finish the write in flight when it is done and start the next queued one.
    pub(crate) fn service_write(&mut self) {
        if !self.write_pending || !Self::signaled(self.write_event) {
            return;
        }
        let mut n = 0u32;
        // SAFETY: the OVERLAPPED is the one used by the completed write on this handle.
        let r = unsafe { GetOverlappedResult(self.handle, &*self.write_ov, &mut n, false) };
        self.write_pending = false;
        if let Err(e) = r
            && GONE_CODES.contains(&win32_code(&e))
        {
            self.dead = true;
            return;
        }
        self.start_next_write();
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        // SAFETY: cancel, wait for the cancelled I/O to finish (so the kernel no longer touches
        // the boxed buffers), then close each handle exactly once.
        unsafe {
            if self.read_pending || self.write_pending {
                let _ = CancelIoEx(self.handle, None);
            }
            let mut n = 0u32;
            if self.read_pending {
                let _ = GetOverlappedResult(self.handle, &*self.read_ov, &mut n, true);
            }
            if self.write_pending {
                let _ = GetOverlappedResult(self.handle, &*self.write_ov, &mut n, true);
            }
            let _ = CloseHandle(self.handle);
            let _ = CloseHandle(self.read_event);
            let _ = CloseHandle(self.write_event);
        }
    }
}
