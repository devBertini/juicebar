// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Win32 HID access: enumeration with CM_Get_Device_Interface_List, opening with shared access,
//! overlapped reads with a timeout, overlapped writes of full-length reports.

use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_ListW, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid,
    HidD_GetPreparsedData, HidD_GetProductString, HidP_GetCaps, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{
    CreateEventW, ResetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::core::PCWSTR;

pub const LOGITECH_VID: u16 = 0x046D;

#[derive(Clone, Debug, Default)]
pub struct Caps {
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
}

/// One HID top-level collection (a device interface). The `path` is used only to open the
/// interface and is never printed or written to a trace.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub interface: Option<u8>,
    pub collection: Option<u8>,
    pub product: String,
    /// `Ok` when the interface opened read+write with shared access, else the Win32 error code.
    pub open_rw: Result<(), u32>,
    pub caps: Option<Caps>,
}

impl Entry {
    pub fn label(&self) -> String {
        let mi = self
            .interface
            .map(|v| format!("MI_{v:02X}"))
            .unwrap_or_else(|| "-".into());
        let col = self
            .collection
            .map(|v| format!("COL{v:02}"))
            .unwrap_or_else(|| "-".into());
        format!("{mi} {col}")
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hex_after(path: &str, key: &str, digits: usize) -> Option<u32> {
    let lower = path.to_ascii_lowercase();
    let at = lower.find(key)? + key.len();
    u32::from_str_radix(lower.get(at..at + digits)?, 16).ok()
}

fn dec_after(path: &str, key: &str) -> Option<u8> {
    let lower = path.to_ascii_lowercase();
    let at = lower.find(key)? + key.len();
    lower.get(at..at + 2)?.parse().ok()
}

fn open_handle(path: &str, access: u32, overlapped: bool) -> Result<HANDLE, u32> {
    let w = wide(path);
    let flags = if overlapped {
        FILE_FLAG_OVERLAPPED
    } else {
        Default::default()
    };
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
        .map_err(|e| (e.code().0 as u32) & 0xFFFF)
    }
}

fn read_caps(h: HANDLE) -> Option<Caps> {
    unsafe {
        let mut pre = PHIDP_PREPARSED_DATA::default();
        if !HidD_GetPreparsedData(h, &mut pre) {
            return None;
        }
        let mut caps = HIDP_CAPS::default();
        let st = HidP_GetCaps(pre, &mut caps);
        let _ = HidD_FreePreparsedData(pre);
        if st.0 < 0 {
            return None;
        }
        Some(Caps {
            usage_page: caps.UsagePage,
            usage: caps.Usage,
            input_len: caps.InputReportByteLength,
            output_len: caps.OutputReportByteLength,
            feature_len: caps.FeatureReportByteLength,
        })
    }
}

fn product_string(h: HANDLE) -> String {
    let mut buf = [0u16; 128];
    unsafe {
        if HidD_GetProductString(h, buf.as_mut_ptr() as *mut _, (buf.len() * 2) as u32) {
            let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            return String::from_utf16_lossy(&buf[..end]);
        }
    }
    String::new()
}

/// List the HID device interfaces present for a vendor ID.
pub fn enumerate(vid_filter: u16) -> Vec<Entry> {
    let guid = unsafe { HidD_GetHidGuid() };
    let mut len = 0u32;
    unsafe {
        if CM_Get_Device_Interface_List_SizeW(
            &mut len,
            &guid,
            PCWSTR::null(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
        {
            return vec![];
        }
    }
    let mut buf = vec![0u16; len as usize];
    unsafe {
        if CM_Get_Device_Interface_ListW(
            &guid,
            PCWSTR::null(),
            &mut buf,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
        {
            return vec![];
        }
    }
    let mut out = Vec::new();
    for chunk in buf.split(|c| *c == 0).filter(|c| !c.is_empty()) {
        let path = String::from_utf16_lossy(chunk);
        let Some(vid) = hex_after(&path, "vid_", 4) else {
            continue;
        };
        if vid as u16 != vid_filter {
            continue;
        }
        let pid = hex_after(&path, "pid_", 4).unwrap_or(0) as u16;
        let interface = hex_after(&path, "&mi_", 2).map(|v| v as u8);
        let collection = dec_after(&path, "&col");
        // Try the open the probe needs. Keyboard and mouse collections are held exclusively by Windows.
        let rw = match open_handle(&path, GENERIC_READ.0 | GENERIC_WRITE.0, true) {
            Ok(h) => {
                let caps = read_caps(h);
                let product = product_string(h);
                let mut attrs = HIDD_ATTRIBUTES {
                    Size: size_of::<HIDD_ATTRIBUTES>() as u32,
                    ..Default::default()
                };
                unsafe {
                    let _ = HidD_GetAttributes(h, &mut attrs);
                    let _ = CloseHandle(h);
                }
                (Ok(()), caps, product)
            }
            Err(code) => match open_handle(&path, 0, false) {
                Ok(h) => {
                    let caps = read_caps(h);
                    let product = product_string(h);
                    unsafe {
                        let _ = CloseHandle(h);
                    }
                    (Err(code), caps, product)
                }
                Err(_) => (Err(code), None, String::new()),
            },
        };
        out.push(Entry {
            path,
            vid: vid as u16,
            pid,
            interface,
            collection,
            product: rw.2,
            open_rw: rw.0,
            caps: rw.1,
        });
    }
    out.sort_by_key(|a| (a.pid, a.interface, a.collection));
    out
}

/// An opened collection with one outstanding overlapped read.
pub struct Port {
    pub label: String,
    pub caps: Caps,
    handle: HANDLE,
    read_event: HANDLE,
    write_event: HANDLE,
    read_ov: Box<OVERLAPPED>,
    buf: Box<[u8]>,
    pending: bool,
    pub dead: bool,
}

impl Port {
    pub fn open(entry: &Entry) -> Result<Port, u32> {
        let caps = entry.caps.clone().ok_or(0u32)?;
        let handle = open_handle(&entry.path, GENERIC_READ.0 | GENERIC_WRITE.0, true)?;
        let read_event =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|_| 1u32)?;
        let write_event =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|_| 1u32)?;
        let mut p = Port {
            label: entry.label(),
            handle,
            read_event,
            write_event,
            read_ov: Box::new(OVERLAPPED::default()),
            buf: vec![0u8; caps.input_len.max(1) as usize].into_boxed_slice(),
            caps,
            pending: false,
            dead: false,
        };
        p.start_read();
        Ok(p)
    }

    fn start_read(&mut self) {
        unsafe {
            let _ = ResetEvent(self.read_event);
            *self.read_ov = OVERLAPPED::default();
            self.read_ov.hEvent = self.read_event;
            match ReadFile(
                self.handle,
                Some(&mut self.buf[..]),
                None,
                Some(&mut *self.read_ov),
            ) {
                Ok(()) => self.pending = true,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => self.pending = true,
                Err(_) => {
                    self.pending = false;
                    self.dead = true;
                }
            }
        }
    }

    fn finish_read(&mut self) -> Option<Vec<u8>> {
        let mut n = 0u32;
        let ok = unsafe { GetOverlappedResult(self.handle, &*self.read_ov, &mut n, false) };
        self.pending = false;
        match ok {
            Ok(()) => {
                let frame = self.buf[..n as usize].to_vec();
                self.start_read();
                Some(frame)
            }
            Err(_) => {
                self.dead = true;
                None
            }
        }
    }

    /// Write one full-length report (report ID first). Waits at most one second.
    pub fn write(&mut self, report: &[u8]) -> Result<(), String> {
        unsafe {
            let _ = ResetEvent(self.write_event);
            let mut ov = OVERLAPPED {
                hEvent: self.write_event,
                ..Default::default()
            };
            match WriteFile(self.handle, Some(report), None, Some(&mut ov)) {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {}
                Err(e) => return Err(format!("WriteFile failed: {e}")),
            }
            if WaitForSingleObject(self.write_event, 1000) != WAIT_OBJECT_0 {
                let _ = CancelIoEx(self.handle, Some(&ov));
                let mut n = 0u32;
                let _ = GetOverlappedResult(self.handle, &ov, &mut n, true);
                return Err("write timed out".into());
            }
            let mut n = 0u32;
            GetOverlappedResult(self.handle, &ov, &mut n, false)
                .map_err(|e| format!("write failed: {e}"))?;
            if n as usize != report.len() {
                return Err(format!("short write: {n} of {} bytes", report.len()));
            }
        }
        Ok(())
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        unsafe {
            if self.pending {
                let _ = CancelIoEx(self.handle, Some(&*self.read_ov));
                let mut n = 0u32;
                let _ = GetOverlappedResult(self.handle, &*self.read_ov, &mut n, true);
            }
            let _ = CloseHandle(self.handle);
            let _ = CloseHandle(self.read_event);
            let _ = CloseHandle(self.write_event);
        }
    }
}

/// All the HID++ collections of one receiver.
pub struct Hub {
    pub ports: Vec<Port>,
}

impl Hub {
    /// Write a frame to the collection whose output report length equals the frame length.
    pub fn send(&mut self, frame: &[u8]) -> Result<usize, String> {
        let idx = self
            .ports
            .iter()
            .position(|p| p.caps.output_len as usize == frame.len() && !p.dead)
            .ok_or_else(|| format!("no collection takes a {}-byte output report", frame.len()))?;
        self.ports[idx].write(frame)?;
        Ok(idx)
    }

    /// Wait up to `timeout` for an input report on any collection.
    pub fn poll(&mut self, timeout: Duration) -> Option<(usize, Vec<u8>)> {
        let deadline = Instant::now() + timeout;
        loop {
            let live: Vec<usize> = (0..self.ports.len())
                .filter(|i| self.ports[*i].pending)
                .collect();
            if live.is_empty() {
                return None;
            }
            let events: Vec<HANDLE> = live.iter().map(|i| self.ports[*i].read_event).collect();
            let left = deadline.saturating_duration_since(Instant::now());
            let r = unsafe {
                WaitForMultipleObjects(
                    &events,
                    false,
                    left.as_millis().min(u32::MAX as u128) as u32,
                )
            };
            if r == WAIT_TIMEOUT {
                return None;
            }
            let k = (r.0 - WAIT_OBJECT_0.0) as usize;
            if k >= live.len() {
                return None;
            }
            let i = live[k];
            if let Some(frame) = self.ports[i].finish_read() {
                return Some((i, frame));
            }
            if Instant::now() >= deadline {
                return None;
            }
        }
    }
}
