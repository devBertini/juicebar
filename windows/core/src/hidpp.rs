// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! HID++ frames: building, answer matching, the send allow-list and the decoders (spec/PROTOCOL.md).
//! Pure code: no device, no clock. The Logitech provider (`logitech`) drives these.

pub mod catalog;

use crate::types::{Charging, Coarse};

/// Software ID in the low nibble of the function byte of every request JuiceBar sends.
pub const SWID: u8 = 0x0D;
pub const SHORT_ID: u8 = 0x10;
pub const LONG_ID: u8 = 0x11;
pub const SHORT_LEN: usize = 7;
pub const LONG_LEN: usize = 20;
/// Device index of a receiver itself, and of a device reached directly (a dongle or a cable).
pub const DIRECT: u8 = 0xFF;
/// The byte the ping carries; the device echoes it.
pub const PING_BYTE: u8 = 0x5A;

pub const F_DEVICE_NAME: u16 = 0x0005;
pub const F_BATTERY_STATUS: u16 = 0x1000;
pub const F_BATTERY_VOLTAGE: u16 = 0x1001;
pub const F_UNIFIED_BATTERY: u16 = 0x1004;
pub const F_ADC_MEASUREMENT: u16 = 0x1F20;
/// The order the provider looks for a battery feature in.
pub const BATTERY_FEATURES: [u16; 4] = [
    F_UNIFIED_BATTERY,
    F_BATTERY_VOLTAGE,
    F_BATTERY_STATUS,
    F_ADC_MEASUREMENT,
];

/// Receiver register 0x00 (notification flags) and the two flags JuiceBar needs: wireless (0x000100) and
/// software present (0x000800), as bytes of the 24-bit big-endian value: both live in the middle byte.
pub const REG_NOTIFICATIONS: u8 = 0x00;
pub const NOTIFY_MASK_MID: u8 = 0x09;

/// Receiver register 0xB5, pairing information of slot n at sub-register 0x20 + (n - 1).
pub const REG_PAIRING: u8 = 0xB5;

/// True for the usage pages of HID++ collections: 0xFF00 (receivers and cabled devices) and 0xFF43 (the G733 dongle).
pub fn is_hidpp_page(usage_page: u16) -> bool {
    usage_page == 0xFF00 || usage_page == 0xFF43
}

/// A request. `b2` and `b3` identify it in the answer: sub-id and register (HID++ 1.0) or feature index and
/// function|software ID (HID++ 2.0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub report_id: u8,
    pub index: u8,
    pub b2: u8,
    pub b3: u8,
    pub params: Vec<u8>,
}

impl Request {
    /// HID++ 1.0 short register read (sub-id 0x81), sent as a short report.
    pub fn register_read(index: u8, register: u8, params: [u8; 3]) -> Self {
        Request {
            report_id: SHORT_ID,
            index,
            b2: 0x81,
            b3: register,
            params: params.to_vec(),
        }
    }

    /// HID++ 1.0 long register read (sub-id 0x83), sent as a short report (the answer is long).
    pub fn register_read_long(index: u8, register: u8, params: [u8; 3]) -> Self {
        Request {
            report_id: SHORT_ID,
            index,
            b2: 0x83,
            b3: register,
            params: params.to_vec(),
        }
    }

    /// HID++ 1.0 short register write (sub-id 0x80). The provider makes exactly one such write (see `is_allowed`).
    pub fn register_write(index: u8, register: u8, params: [u8; 3]) -> Self {
        Request {
            report_id: SHORT_ID,
            index,
            b2: 0x80,
            b3: register,
            params: params.to_vec(),
        }
    }

    /// HID++ 2.0 request with JuiceBar's software ID.
    pub fn feature(index: u8, feature_index: u8, function: u8, params: &[u8], long: bool) -> Self {
        Request {
            report_id: if long { LONG_ID } else { SHORT_ID },
            index,
            b2: feature_index,
            b3: ((function & 0x0F) << 4) | SWID,
            params: params.to_vec(),
        }
    }

    pub fn is_hidpp20(&self) -> bool {
        self.b2 < 0x80
    }

    pub fn is_long(&self) -> bool {
        self.report_id == LONG_ID
    }

    /// The full-length report: 7 bytes for short, 20 for long, zero padded.
    pub fn encode(&self) -> Vec<u8> {
        let len = if self.is_long() { LONG_LEN } else { SHORT_LEN };
        let mut out = vec![0u8; len];
        out[0] = self.report_id;
        out[1] = self.index;
        out[2] = self.b2;
        out[3] = self.b3;
        for (i, p) in self.params.iter().enumerate() {
            if 4 + i < len {
                out[4 + i] = *p;
            }
        }
        out
    }

    /// Read a full report back as a request (used to check what was sent). Trailing zero padding stays in `params`.
    pub fn parse(frame: &[u8]) -> Option<Request> {
        let expected = match *frame.first()? {
            SHORT_ID => SHORT_LEN,
            LONG_ID => LONG_LEN,
            _ => return None,
        };
        if frame.len() != expected {
            return None;
        }
        Some(Request {
            report_id: frame[0],
            index: frame[1],
            b2: frame[2],
            b3: frame[3],
            params: frame[4..].to_vec(),
        })
    }
}

fn params_after(params: &[u8], used: usize) -> bool {
    params.iter().skip(used).all(|b| *b == 0)
}

/// The send allow-list (spec/PROTOCOL.md): everything the provider may put on a wire, and nothing else.
/// - receiver register 0x00 read, and one write of it that keeps the wireless and software-present flags set;
/// - receiver pairing reads, register 0xB5 sub-registers 0x20..=0x25 (never 0x30+, the serial numbers);
/// - HID++ 2.0 with software ID 0x0D, functions 0 and 1 only (no setters), to device indexes 1..=6 or 0xFF;
/// - on the Root feature (index 0) only getFeature for the battery features and DeviceName, and the ping;
/// - on any other feature only an index in `known`: the battery and DeviceName indexes this device itself
///   reported, never a guessed one.
pub fn is_allowed(req: &Request, known: &[u8]) -> bool {
    if req.report_id != SHORT_ID && req.report_id != LONG_ID {
        return false;
    }
    match req.b2 {
        0x81 => req.b3 == REG_NOTIFICATIONS && req.index == DIRECT && params_after(&req.params, 0),
        0x80 => {
            req.b3 == REG_NOTIFICATIONS
                && req.index == DIRECT
                && req.report_id == SHORT_ID
                && req.params.len() >= 3
                && req.params[1] & NOTIFY_MASK_MID == NOTIFY_MASK_MID
                && params_after(&req.params, 3)
        }
        0x83 => {
            req.b3 == REG_PAIRING
                && req.index == DIRECT
                && matches!(req.params.first(), Some(0x20..=0x25))
                && params_after(&req.params, 1)
        }
        b2 if b2 >= 0x80 => false,
        b2 => {
            let function = req.b3 >> 4;
            if req.b3 & 0x0F != SWID || function > 1 {
                return false;
            }
            if !matches!(req.index, 1..=6 | DIRECT) {
                return false;
            }
            if b2 != 0 {
                return known.contains(&b2);
            }
            // Root feature.
            match function {
                0 => {
                    let id = u16::from_be_bytes([
                        req.params.first().copied().unwrap_or(0),
                        req.params.get(1).copied().unwrap_or(0),
                    ]);
                    (BATTERY_FEATURES.contains(&id) || id == F_DEVICE_NAME)
                        && params_after(&req.params, 2)
                }
                _ => {
                    req.params.first() == Some(&0)
                        && req.params.get(1) == Some(&0)
                        && req.params.get(2) == Some(&PING_BYTE)
                        && params_after(&req.params, 3)
                }
            }
        }
    }
}

/// What a received frame is, against one pending request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The positive answer: the bytes after the four header bytes.
    Answer(Vec<u8>),
    /// An error answer. `v1` is true for a HID++ 1.0 error report (sub-id 0x8F).
    Error { v1: bool, code: u8 },
    /// Not the answer to this request: another program's traffic, an event, another device.
    Ignore,
}

/// Match a received frame to a request by device index, byte 2 and byte 3 (feature index and function|software ID,
/// or sub-id and register).
pub fn classify(req: &Request, frame: &[u8]) -> Reply {
    if frame.len() < SHORT_LEN || (frame[0] != SHORT_ID && frame[0] != LONG_ID) {
        return Reply::Ignore;
    }
    if frame[1] != req.index {
        return Reply::Ignore;
    }
    if frame[2] == req.b2 && frame[3] == req.b3 {
        return Reply::Answer(frame[4..].to_vec());
    }
    // HID++ 1.0 error: 8F, then the request's two bytes, then the code.
    if frame[2] == 0x8F && frame[3] == req.b2 && frame[4] == req.b3 {
        return Reply::Error {
            v1: true,
            code: frame[5],
        };
    }
    // HID++ 2.0 error: FF, feature index, function|software ID, code.
    if frame[2] == 0xFF && req.is_hidpp20() && frame[3] == req.b2 && frame[4] == req.b3 {
        return Reply::Error {
            v1: false,
            code: frame[5],
        };
    }
    Reply::Ignore
}

pub fn hex(frame: &[u8]) -> String {
    frame.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------- decoders

/// The pairing register answer (`payload` is the frame after its four header bytes): the wireless product id
/// (big-endian, bytes 3 and 4) and the device kind (low nibble of byte 7: 1 keyboard, 2 mouse). Byte 0 echoes the
/// sub-register.
pub fn decode_pairing(payload: &[u8]) -> Option<(u16, u8)> {
    if payload.len() < 8 {
        return None;
    }
    Some((
        u16::from_be_bytes([payload[3], payload[4]]),
        payload[7] & 0x0F,
    ))
}

/// Root getFeature answer: feature index (0 means not supported), type flags, version.
pub fn decode_get_feature(payload: &[u8]) -> Option<(u8, u8, u8)> {
    if payload.len() < 3 {
        None
    } else {
        Some((payload[0], payload[1], payload[2]))
    }
}

/// Ping answer: protocol major and minor, and the echoed byte.
pub fn decode_ping(payload: &[u8]) -> Option<(u8, u8, u8)> {
    if payload.len() < 3 {
        None
    } else {
        Some((payload[0], payload[1], payload[2]))
    }
}

/// Receiver register 0x00 answer: the three flag bytes.
pub fn decode_notification_flags(payload: &[u8]) -> Option<[u8; 3]> {
    if payload.len() < 3 {
        None
    } else {
        Some([payload[0], payload[1], payload[2]])
    }
}

/// The three flag bytes with the wireless and software-present flags set.
pub fn notification_flags_enabled(flags: [u8; 3]) -> [u8; 3] {
    [flags[0], flags[1] | NOTIFY_MASK_MID, flags[2]]
}

pub fn notifications_enabled(flags: [u8; 3]) -> bool {
    flags[1] & NOTIFY_MASK_MID == NOTIFY_MASK_MID
}

/// DeviceName getCount answer: the name's length.
pub fn decode_name_count(payload: &[u8]) -> Option<usize> {
    payload.first().map(|n| usize::from(*n))
}

/// A HID++ 1.0 device connection notification, sub-id 0x41:
/// `[0x10, slot, 0x41, protocol type, flags, wpid low, wpid high]`. Flags bit 0x40 set means the link is lost; the
/// low nibble is the device kind. (Measured: `1001410c628640` lost, `1001410ca28640` established, wpid 4086.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionNotice {
    pub index: u8,
    pub link_lost: bool,
    pub kind: u8,
    pub wpid: u16,
}

pub fn parse_connection(frame: &[u8]) -> Option<ConnectionNotice> {
    if frame.len() < SHORT_LEN || (frame[0] != SHORT_ID && frame[0] != LONG_ID) || frame[2] != 0x41
    {
        return None;
    }
    if !matches!(frame[1], 1..=6) {
        return None;
    }
    Some(ConnectionNotice {
        index: frame[1],
        link_lost: frame[4] & 0x40 != 0,
        kind: frame[4] & 0x0F,
        wpid: u16::from_le_bytes([frame[5], frame[6]]),
    })
}

/// A battery value as the device gave it, before any curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryData {
    pub feature: u16,
    /// The device's own percentage, when it gives one.
    pub percent: Option<u8>,
    pub coarse: Option<Coarse>,
    pub millivolts: Option<u16>,
    pub charging: Charging,
}

fn coarse_from_bits(bits: u8) -> Option<Coarse> {
    if bits & 0x08 != 0 {
        Some(Coarse::Full)
    } else if bits & 0x04 != 0 {
        Some(Coarse::Good)
    } else if bits & 0x02 != 0 {
        Some(Coarse::Low)
    } else if bits & 0x01 != 0 {
        Some(Coarse::Critical)
    } else {
        None
    }
}

/// Decode the payload of a battery answer or event (function 1 of 0x1004, function 0 of the others; event 0 has the
/// same layout). `payload` is the frame after its four header bytes.
pub fn decode_battery(feature: u16, payload: &[u8]) -> Option<BatteryData> {
    if payload.len() < 3 {
        return None;
    }
    match feature {
        F_UNIFIED_BATTERY => {
            let coarse = coarse_from_bits(payload[1]);
            // A device without a state-of-charge reports 0 beside a level that is not critical: no percentage.
            let percent =
                if payload[0] > 100 || (payload[0] == 0 && coarse != Some(Coarse::Critical)) {
                    None
                } else {
                    Some(payload[0])
                };
            let charging = match payload[2] {
                0 => Charging::Discharging,
                1 | 2 => Charging::Charging,
                3 => Charging::Full,
                _ => Charging::Unknown,
            };
            Some(BatteryData {
                feature,
                percent,
                coarse,
                millivolts: None,
                charging,
            })
        }
        F_BATTERY_VOLTAGE => {
            let mv = u16::from_be_bytes([payload[0], payload[1]]);
            if mv == 0 {
                return None;
            }
            Some(BatteryData {
                feature,
                percent: None,
                coarse: None,
                millivolts: Some(mv),
                charging: if payload[2] & 0x80 != 0 {
                    Charging::Charging
                } else {
                    Charging::Discharging
                },
            })
        }
        F_BATTERY_STATUS => {
            if payload[0] > 100 {
                return None;
            }
            let charging = match payload[2] {
                0 => Charging::Discharging,
                1 | 2 | 4 => Charging::Charging,
                3 => Charging::Full,
                _ => Charging::Unknown,
            };
            Some(BatteryData {
                feature,
                percent: Some(payload[0]),
                coarse: None,
                millivolts: None,
                charging,
            })
        }
        F_ADC_MEASUREMENT => {
            let mv = u16::from_be_bytes([payload[0], payload[1]]);
            if mv == 0 {
                return None;
            }
            // The flags byte has no known meaning yet (0x01 while G HUB said not charging).
            Some(BatteryData {
                feature,
                percent: None,
                coarse: None,
                millivolts: Some(mv),
                charging: Charging::Unknown,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn parse_hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_lengths_and_swid() {
        let ping = Request::feature(1, 0, 1, &[0, 0, PING_BYTE], false).encode();
        assert_eq!(hex(&ping), "1001001d00005a");
        let long = Request::feature(2, 7, 1, &[], true).encode();
        assert_eq!(long.len(), LONG_LEN);
        assert_eq!(long[0], LONG_ID);
        assert_eq!(long[3], 0x1D);
        assert_eq!(
            hex(&Request::register_read(DIRECT, 0, [0; 3]).encode()),
            "10ff8100000000"
        );
        assert_eq!(
            hex(&Request::register_read_long(DIRECT, REG_PAIRING, [0x20, 0, 0]).encode()),
            "10ff83b5200000"
        );
        assert_eq!(
            hex(&Request::register_write(DIRECT, 0, [0, 9, 0]).encode()),
            "10ff8000000900"
        );
        assert_eq!(
            Request::parse(&ping),
            Some(Request::feature(1, 0, 1, &[0, 0, PING_BYTE], false))
        );
        assert!(Request::parse(&[0x10, 1, 2]).is_none());
        assert!(Request::parse(&[0x12; 20]).is_none());
    }

    #[test]
    fn allow_list_has_exactly_the_protocol_requests() {
        assert!(is_allowed(&Request::register_read(DIRECT, 0, [0; 3]), &[]));
        assert!(!is_allowed(
            &Request::register_read(DIRECT, 0x02, [0; 3]),
            &[]
        ));
        assert!(!is_allowed(&Request::register_read(1, 0, [0; 3]), &[]));
        for sub in 0x20..=0x25 {
            assert!(is_allowed(
                &Request::register_read_long(DIRECT, REG_PAIRING, [sub, 0, 0]),
                &[]
            ));
        }
        for sub in [0x1F, 0x26, 0x30, 0x35, 0x40, 0x45] {
            assert!(
                !is_allowed(
                    &Request::register_read_long(DIRECT, REG_PAIRING, [sub, 0, 0]),
                    &[]
                ),
                "serial or name sub-register {sub:#x}"
            );
        }
        // The one write keeps the wireless and software-present flags set.
        assert!(is_allowed(
            &Request::register_write(DIRECT, 0, [0, 9, 0]),
            &[]
        ));
        assert!(is_allowed(
            &Request::register_write(DIRECT, 0, [0x10, 0x0F, 0x01]),
            &[]
        ));
        assert!(!is_allowed(
            &Request::register_write(DIRECT, 0, [0, 0, 0]),
            &[]
        ));
        assert!(!is_allowed(
            &Request::register_write(DIRECT, 0, [0, 1, 0]),
            &[]
        ));
        assert!(!is_allowed(
            &Request::register_write(DIRECT, 0x02, [0, 9, 0]),
            &[]
        ));
        assert!(!is_allowed(
            &Request::register_write(DIRECT, 0xB2, [0, 9, 0]),
            &[]
        ));
        let long_write = Request {
            report_id: LONG_ID,
            ..Request::register_write(DIRECT, 0, [0, 9, 0])
        };
        assert!(!is_allowed(&long_write, &[]));
        let sub_0x82 = Request {
            b2: 0x82,
            ..Request::register_write(DIRECT, 0, [0, 9, 0])
        };
        assert!(!is_allowed(&sub_0x82, &[]));
        // HID++ 2.0.
        assert!(is_allowed(
            &Request::feature(1, 0, 1, &[0, 0, PING_BYTE], false),
            &[]
        ));
        assert!(!is_allowed(
            &Request::feature(1, 0, 1, &[0, 0, 0], false),
            &[]
        ));
        for id in [0x1004u16, 0x1001, 0x1000, 0x1F20, 0x0005] {
            assert!(is_allowed(
                &Request::feature(1, 0, 0, &id.to_be_bytes(), true),
                &[]
            ));
        }
        for id in [0x0001u16, 0x0003, 0x1B04, 0x0000] {
            assert!(
                !is_allowed(&Request::feature(1, 0, 0, &id.to_be_bytes(), true), &[]),
                "{id:#x}"
            );
        }
        assert!(is_allowed(&Request::feature(DIRECT, 8, 1, &[], true), &[8]));
        assert!(is_allowed(&Request::feature(3, 3, 1, &[16], true), &[3]));
        assert!(
            !is_allowed(&Request::feature(1, 5, 2, &[0], true), &[5]),
            "function 2 is a setter on most features"
        );
        assert!(!is_allowed(&Request::feature(0, 5, 1, &[], true), &[5]));
        assert!(!is_allowed(&Request::feature(7, 5, 1, &[], true), &[5]));
        let wrong_swid = Request {
            report_id: LONG_ID,
            index: 1,
            b2: 5,
            b3: 0x1B,
            params: vec![],
        };
        assert!(!is_allowed(&wrong_swid, &[]));
        let not_hidpp = Request {
            report_id: 0x12,
            ..Request::feature(1, 5, 1, &[], true)
        };
        assert!(!is_allowed(&not_hidpp, &[]));
    }

    #[test]
    fn a_function_on_an_unknown_feature_index_is_refused() {
        // Only the root, and the battery and DeviceName indexes this device taught, take function 0 or 1.
        let known = [3u8, 7];
        for index in [3u8, 7] {
            for function in [0u8, 1] {
                assert!(is_allowed(
                    &Request::feature(1, index, function, &[], true),
                    &known
                ));
            }
        }
        for index in [1u8, 2, 4, 9, 0x7F] {
            for function in [0u8, 1] {
                assert!(
                    !is_allowed(&Request::feature(1, index, function, &[], true), &known),
                    "index {index} function {function}"
                );
                assert!(!is_allowed(
                    &Request::feature(DIRECT, index, function, &[], true),
                    &[]
                ));
            }
        }
        // The root needs no known index.
        assert!(is_allowed(
            &Request::feature(1, 0, 1, &[0, 0, PING_BYTE], false),
            &[]
        ));
    }

    #[test]
    fn match_by_index_feature_and_swid() {
        let req = Request::feature(1, 4, 1, &[], true);
        let ans = parse_hex("11 01 04 1d 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &ans), Reply::Answer(ans[4..].to_vec()));
        // G HUB's software ID on the same feature.
        let other = parse_hex("11 01 04 1b 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &other), Reply::Ignore);
        // An event (software ID 0).
        let event = parse_hex("11 01 04 00 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &event), Reply::Ignore);
        // Another device.
        let dev2 = parse_hex("11 02 04 1d 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &dev2), Reply::Ignore);
        // Another function of the same feature.
        let func0 = parse_hex("11 01 04 0d 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &func0), Reply::Ignore);
        // A DJ report.
        assert_eq!(
            classify(&req, &parse_hex("20 01 04 1d 55 04 01")),
            Reply::Ignore
        );
        assert_eq!(classify(&req, &parse_hex("11 01 04")), Reply::Ignore);
    }

    #[test]
    fn match_errors() {
        let req = Request::feature(1, 0, 1, &[0, 0, PING_BYTE], false);
        // The measured offline answer of a receiver slot: HID++ 1.0 error 0x08 (unknown device).
        let offline = parse_hex("10 02 8f 00 1d 08 00");
        let req2 = Request::feature(2, 0, 1, &[0, 0, PING_BYTE], false);
        assert_eq!(
            classify(&req2, &offline),
            Reply::Error { v1: true, code: 8 }
        );
        assert_eq!(classify(&req, &offline), Reply::Ignore);
        let e2 = parse_hex("11 01 ff 00 1d 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &e2), Reply::Error { v1: false, code: 2 });
        let e3 = parse_hex("11 01 ff 05 1d 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &e3), Reply::Ignore);
        let reg = Request::register_read_long(DIRECT, REG_PAIRING, [0x22, 0, 0]);
        assert_eq!(
            classify(&reg, &parse_hex("10 ff 8f 83 b5 03 00")),
            Reply::Error { v1: true, code: 3 }
        );
        // A 1.0 request is never matched by a 2.0 style error.
        assert_eq!(
            classify(&reg, &parse_hex("10 ff ff 83 b5 03 00")),
            Reply::Ignore
        );
    }

    #[test]
    fn pairing_answers_from_the_receivers() {
        // Measured: G915 X on C547 and G703 on C539.
        let g915 = parse_hex("11ff83b520070840b5110001b1000000002c0000");
        assert_eq!(decode_pairing(&g915[4..]), Some((0x40B5, 1)));
        let g703 = parse_hex("11ff83b520070840860c0102b300000000000000");
        assert_eq!(decode_pairing(&g703[4..]), Some((0x4086, 2)));
        assert_eq!(decode_pairing(&[0x20, 7, 8]), None);
    }

    #[test]
    fn connection_notices_from_the_listen_traces() {
        let lost = parse_connection(&parse_hex("1001410c628640")).unwrap();
        assert_eq!(
            lost,
            ConnectionNotice {
                index: 1,
                link_lost: true,
                kind: 2,
                wpid: 0x4086
            }
        );
        let up = parse_connection(&parse_hex("1001410ca28640")).unwrap();
        assert!(!up.link_lost && up.kind == 2 && up.wpid == 0x4086);
        let key_lost = parse_connection(&parse_hex("1001411161b540")).unwrap();
        assert!(key_lost.link_lost && key_lost.kind == 1 && key_lost.wpid == 0x40B5);
        let key_up = parse_connection(&parse_hex("10014111a1b540")).unwrap();
        assert!(!key_up.link_lost && key_up.wpid == 0x40B5);
        assert!(parse_connection(&parse_hex("1000410ca28640")).is_none());
        assert!(parse_connection(&parse_hex("1001400ca28640")).is_none());
        assert!(parse_connection(&parse_hex("1001410c")).is_none());
    }

    #[test]
    fn unified_battery_from_the_g915x_traces() {
        // Status answer: 49 %, level good, discharging.
        let d = decode_battery(F_UNIFIED_BATTERY, &parse_hex("310400000000")).unwrap();
        assert_eq!(d.percent, Some(49));
        assert_eq!(d.coarse, Some(Coarse::Good));
        assert_eq!(d.charging, Charging::Discharging);
        // The charging event on the cable: 49 %, charging.
        let c = decode_battery(F_UNIFIED_BATTERY, &parse_hex("31040101")).unwrap();
        assert_eq!((c.percent, c.charging), (Some(49), Charging::Charging));
        // Other statuses.
        let slow = decode_battery(F_UNIFIED_BATTERY, &[80, 4, 2]).unwrap();
        assert_eq!(slow.charging, Charging::Charging);
        let full = decode_battery(F_UNIFIED_BATTERY, &[100, 8, 3]).unwrap();
        assert_eq!(
            (full.coarse, full.charging),
            (Some(Coarse::Full), Charging::Full)
        );
        let err = decode_battery(F_UNIFIED_BATTERY, &[50, 4, 4]).unwrap();
        assert_eq!(err.charging, Charging::Unknown);
        // Level only (no state of charge): 0 beside a level that is not critical.
        let level_only = decode_battery(F_UNIFIED_BATTERY, &[0, 2, 0]).unwrap();
        assert_eq!(
            (level_only.percent, level_only.coarse),
            (None, Some(Coarse::Low))
        );
        let empty = decode_battery(F_UNIFIED_BATTERY, &[0, 1, 0]).unwrap();
        assert_eq!(
            (empty.percent, empty.coarse),
            (Some(0), Some(Coarse::Critical))
        );
        assert_eq!(
            decode_battery(F_UNIFIED_BATTERY, &[101, 4, 0])
                .unwrap()
                .percent,
            None
        );
        assert!(decode_battery(F_UNIFIED_BATTERY, &[50, 4]).is_none());
    }

    #[test]
    fn voltage_and_adc_batteries_from_the_traces() {
        // G703: 0x0F7E = 3966 mV, flags 0x00; the event 0x0F7A = 3962 mV.
        let g703 = decode_battery(F_BATTERY_VOLTAGE, &parse_hex("0f7e00000000")).unwrap();
        assert_eq!(g703.millivolts, Some(3966));
        assert_eq!((g703.percent, g703.charging), (None, Charging::Discharging));
        let ev = decode_battery(F_BATTERY_VOLTAGE, &parse_hex("0f7a0000")).unwrap();
        assert_eq!(ev.millivolts, Some(3962));
        let charging = decode_battery(F_BATTERY_VOLTAGE, &[0x0F, 0xA0, 0x80]).unwrap();
        assert_eq!(charging.charging, Charging::Charging);
        // G733: 0x0EA8 = 3752 mV, flags 0x01 (meaning unknown).
        let g733 = decode_battery(F_ADC_MEASUREMENT, &parse_hex("0ea801000000")).unwrap();
        assert_eq!(g733.millivolts, Some(3752));
        assert_eq!(g733.charging, Charging::Unknown);
        assert!(decode_battery(F_BATTERY_VOLTAGE, &[0, 0, 0]).is_none());
        assert!(decode_battery(F_ADC_MEASUREMENT, &[0, 0, 1]).is_none());
    }

    #[test]
    fn battery_status_feature() {
        let d = decode_battery(F_BATTERY_STATUS, &[80, 50, 0]).unwrap();
        assert_eq!((d.percent, d.charging), (Some(80), Charging::Discharging));
        assert_eq!(
            decode_battery(F_BATTERY_STATUS, &[100, 0, 3])
                .unwrap()
                .charging,
            Charging::Full
        );
        assert_eq!(
            decode_battery(F_BATTERY_STATUS, &[90, 0, 4])
                .unwrap()
                .charging,
            Charging::Charging
        );
        assert_eq!(
            decode_battery(F_BATTERY_STATUS, &[90, 0, 6])
                .unwrap()
                .charging,
            Charging::Unknown
        );
        assert!(decode_battery(F_BATTERY_STATUS, &[120, 0, 0]).is_none());
        assert!(decode_battery(0x1234, &[1, 2, 3]).is_none());
    }

    #[test]
    fn small_decoders() {
        assert_eq!(decode_ping(&[4, 2, PING_BYTE]), Some((4, 2, PING_BYTE)));
        assert_eq!(decode_get_feature(&[8, 0, 5]), Some((8, 0, 5)));
        assert_eq!(decode_get_feature(&[8]), None);
        assert_eq!(decode_name_count(&[19, 0]), Some(19));
        let flags = decode_notification_flags(&[0, 1, 0]).unwrap();
        assert!(!notifications_enabled(flags));
        assert_eq!(notification_flags_enabled(flags), [0, 9, 0]);
        assert!(notifications_enabled([0, 9, 0]));
        assert!(notifications_enabled([0x10, 0x0F, 0x01]));
        assert!(!notifications_enabled([0, 8, 0]));
        assert_eq!(
            notification_flags_enabled([0x10, 0x02, 0x01]),
            [0x10, 0x0B, 0x01]
        );
    }
}
