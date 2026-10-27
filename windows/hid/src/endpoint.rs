// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The transport's vocabulary: what an endpoint is and what the transport reports.

use std::time::Instant;

/// Logitech's USB vendor ID.
pub const LOGITECH_VID: u16 = 0x046D;
/// Vendor usage page of receivers and cabled devices (usage 1 short, 2 long, 4 very long reports).
pub const USAGE_PAGE_HIDPP: u16 = 0xFF00;
/// Vendor usage page of the G733 dongle (long reports only).
pub const USAGE_PAGE_HIDPP_DONGLE: u16 = 0xFF43;

/// Session-local identifier of an opened endpoint. Never reused within one transport.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub struct EndpointId(pub u32);

/// How the HID interface reaches the PC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    Usb,
    Bluetooth,
}

/// One HID top-level collection the transport has open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub id: EndpointId,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// Input report length in bytes, report ID included.
    pub input_len: u16,
    /// Output report length in bytes, report ID included (0: the collection takes no output).
    pub output_len: u16,
    pub product: String,
    pub kind: TransportKind,
}

/// What the transport tells the app. Delivered in order through `Transport::drain`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportEvent {
    Arrived(Endpoint),
    Removed(EndpointId),
    /// A raw input report (report ID first) and the monotonic instant it was read.
    Report(EndpointId, Vec<u8>, Instant),
}

/// Whether a HID collection is one the transport opens: a Logitech vendor collection that
/// can be read.
pub fn is_hidpp_collection(vid: u16, usage_page: u16, input_len: u16) -> bool {
    vid == LOGITECH_VID
        && input_len > 0
        && (usage_page == USAGE_PAGE_HIDPP || usage_page == USAGE_PAGE_HIDPP_DONGLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_vendor_pages_of_logitech() {
        assert!(is_hidpp_collection(0x046D, 0xFF00, 20));
        assert!(is_hidpp_collection(0x046D, 0xFF43, 20));
    }

    #[test]
    fn rejects_other_pages() {
        for page in [0x0001, 0x000C, 0xFFBC, 0xFF13, 0xFF01, 0xFF42, 0] {
            assert!(!is_hidpp_collection(0x046D, page, 20), "page {page:#06x}");
        }
    }

    #[test]
    fn rejects_other_vendors() {
        assert!(!is_hidpp_collection(0x1532, 0xFF00, 20));
        assert!(!is_hidpp_collection(0x0000, 0xFF43, 20));
    }

    #[test]
    fn rejects_collections_that_cannot_be_read() {
        assert!(!is_hidpp_collection(0x046D, 0xFF00, 0));
    }
}
