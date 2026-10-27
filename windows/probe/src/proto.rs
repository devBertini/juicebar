// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! HID++ frame encoding, answer matching and decoders. Pure code: no Win32, unit tested.

/// Software ID the probe puts in the low nibble of the HID++ 2.0 function byte.
pub const SWID: u8 = 0x0D;
pub const SHORT_ID: u8 = 0x10;
pub const LONG_ID: u8 = 0x11;
pub const SHORT_LEN: usize = 7;
pub const LONG_LEN: usize = 20;
pub const RECEIVER: u8 = 0xFF;

/// HID++ usage pages: 0xFF00 on LIGHTSPEED receivers (short and long reports), 0xFF43 on devices
/// reached directly, like the G733 dongle (long reports only, device index 0xFF).
pub fn is_hidpp_page(usage_page: u16) -> bool {
    usage_page == 0xFF00 || usage_page == 0xFF43
}

/// Feature IDs the probe knows by name.
pub const F_FEATURE_SET: u16 = 0x0001;
pub const F_DEVICE_NAME: u16 = 0x0005;
pub const F_BATTERY_STATUS: u16 = 0x1000;
pub const F_BATTERY_VOLTAGE: u16 = 0x1001;
pub const F_UNIFIED_BATTERY: u16 = 0x1004;
pub const F_ADC_MEASUREMENT: u16 = 0x1F20;
pub const BATTERY_FEATURES: [u16; 4] = [
    F_BATTERY_STATUS,
    F_BATTERY_VOLTAGE,
    F_UNIFIED_BATTERY,
    F_ADC_MEASUREMENT,
];

/// A request the probe may send. The three bytes after the device index identify it in the answer:
/// `b2`/`b3` are sub-id and register (HID++ 1.0) or feature index and function|software ID (2.0).
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

    /// HID++ 2.0 request with the probe's software ID in the low nibble of the function byte.
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
        self.b2 != 0x81 && self.b2 != 0x83
    }

    /// The full-length report: 7 bytes for short, 20 for long, zero padded.
    pub fn encode(&self) -> Vec<u8> {
        let len = if self.report_id == LONG_ID {
            LONG_LEN
        } else {
            SHORT_LEN
        };
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
}

/// Which request kinds the probe is allowed to send. Everything else is refused before it is written.
pub fn is_allowed(req: &Request) -> bool {
    match req.b2 {
        0x81 => req.b3 == 0x02 && req.index == RECEIVER,
        0x83 => {
            req.b3 == 0xB5
                && req.index == RECEIVER
                && matches!(req.params[0], 0x20..=0x25 | 0x40..=0x45)
        }
        0x80 | 0x82 => false,
        _ => {
            let func = req.b3 >> 4;
            req.b3 & 0x0F == SWID && func <= 1
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The positive answer; the bytes after the four header bytes.
    Answer(Vec<u8>),
    /// An error answer. `v1` is true for a HID++ 1.0 error report (sub-id 0x8F).
    Error { v1: bool, code: u8 },
    /// Not the answer to this request (G HUB's traffic, events, other devices).
    Ignore,
}

/// Match a received frame to a request by device index, byte 2, byte 3 (feature index and
/// function|software ID, or sub-id and register) and nothing else.
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
    if frame[2] == 0xFF && frame[3] == req.b2 && frame[4] == req.b3 && req.is_hidpp20() {
        return Reply::Error {
            v1: false,
            code: frame[5],
        };
    }
    Reply::Ignore
}

pub fn hex(frame: &[u8]) -> String {
    frame
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
pub fn parse_hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

pub fn error_name_v1(code: u8) -> &'static str {
    match code {
        0x01 => "invalid sub-id (not a HID++ 1.0 request, or unsupported)",
        0x02 => "invalid address",
        0x03 => "invalid value",
        0x04 => "connect fail",
        0x05 => "too many devices",
        0x06 => "already exists",
        0x07 => "busy",
        0x08 => "unknown device",
        0x09 => "resource error",
        0x0A => "request unavailable",
        0x0B => "invalid parameter value",
        0x0C => "wrong PIN code",
        _ => "unknown error",
    }
}

pub fn error_name_v2(code: u8) -> &'static str {
    match code {
        0x01 => "unknown",
        0x02 => "invalid argument",
        0x03 => "out of range",
        0x04 => "hardware error",
        0x05 => "logitech internal",
        0x06 => "invalid feature index",
        0x07 => "invalid function id",
        0x08 => "busy",
        0x09 => "unsupported",
        _ => "unknown error",
    }
}

pub fn feature_name(id: u16) -> &'static str {
    match id {
        0x0000 => "Root",
        0x0001 => "FeatureSet",
        0x0002 => "FeatureInfo",
        0x0003 => "DeviceInfo",
        0x0005 => "DeviceName",
        0x0020 => "ConfigChange",
        0x1000 => "BatteryStatus",
        0x1001 => "BatteryVoltage",
        0x1004 => "UnifiedBattery",
        0x1F20 => "ADCMeasurement",
        _ => "",
    }
}

pub fn feature_type_flags(t: u8) -> String {
    let mut v = Vec::new();
    if t & 0x80 != 0 {
        v.push("obsolete");
    }
    if t & 0x40 != 0 {
        v.push("hidden");
    }
    if t & 0x20 != 0 {
        v.push("engineering");
    }
    if t & 0x10 != 0 {
        v.push("manuf-deactivated");
    }
    if t & 0x08 != 0 {
        v.push("compliance-deactivated");
    }
    if v.is_empty() {
        "-".into()
    } else {
        v.join("|")
    }
}

// ---------------------------------------------------------------- decoders

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battery {
    pub feature: u16,
    pub percent: Option<u8>,
    pub level: Option<String>,
    pub millivolts: Option<u16>,
    pub state: String,
}

/// 0x1000 function 0: percent, next-level percent, status.
pub fn decode_battery_status(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    let state = match p[2] {
        0 => "discharging",
        1 => "charging",
        2 => "almost full (charging)",
        3 => "full",
        4 => "slow charge",
        5 => "invalid battery",
        6 => "thermal error",
        _ => "unknown",
    };
    Some(Battery {
        feature: F_BATTERY_STATUS,
        percent: Some(p[0]),
        level: None,
        millivolts: None,
        state: state.into(),
    })
}

/// 0x1001 function 0: voltage in mV (big endian), flags (bit 7 = charging).
pub fn decode_battery_voltage(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    let mv = u16::from_be_bytes([p[0], p[1]]);
    let state = if p[2] & 0x80 != 0 {
        "charging"
    } else {
        "discharging"
    };
    Some(Battery {
        feature: F_BATTERY_VOLTAGE,
        percent: None,
        level: None,
        millivolts: Some(mv),
        state: state.into(),
    })
}

pub fn level_name(bits: u8) -> &'static str {
    match bits {
        1 => "critical",
        2 => "low",
        4 => "good",
        8 => "full",
        _ => "unknown",
    }
}

/// 0x1004 function 1: state of charge percent, level bit, charging status, external power.
pub fn decode_unified_status(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    let state = match p[2] {
        0 => "discharging",
        1 => "charging",
        2 => "slow charge",
        3 => "charge complete",
        4 => "error",
        _ => "unknown",
    };
    Some(Battery {
        feature: F_UNIFIED_BATTERY,
        percent: Some(p[0]),
        level: Some(level_name(p[1]).into()),
        millivolts: None,
        state: state.into(),
    })
}

/// 0x1004 function 0: supported level bits and capability flags. Returns a printable summary.
pub fn describe_unified_capabilities(p: &[u8]) -> String {
    if p.len() < 2 {
        return "short answer".into();
    }
    let mut levels = Vec::new();
    for (bit, name) in [
        (0x01, "critical"),
        (0x02, "low"),
        (0x04, "good"),
        (0x08, "full"),
    ] {
        if p[0] & bit != 0 {
            levels.push(name);
        }
    }
    format!(
        "supported levels [{}], flags 0x{:02x} (rechargeable {}, state-of-charge {})",
        levels.join(","),
        p[1],
        p[1] & 0x01 != 0,
        p[1] & 0x02 != 0
    )
}

/// 0x1F20 function 0: ADC reading in mV (big endian) and a flags byte. Measured on the G733: flags
/// 0x01 while G HUB reports "not charging", so the charging bit is not known; the flags are kept raw.
pub fn decode_adc(p: &[u8]) -> Option<Battery> {
    if p.len() < 3 {
        return None;
    }
    let mv = u16::from_be_bytes([p[0], p[1]]);
    Some(Battery {
        feature: F_ADC_MEASUREMENT,
        percent: None,
        level: None,
        millivolts: Some(mv),
        state: format!("flags 0x{:02x} (charging bit not decoded)", p[2]),
    })
}

pub fn format_battery(b: &Battery) -> String {
    let mut parts = Vec::new();
    if let Some(p) = b.percent {
        parts.push(format!("{p} %"));
    }
    if let Some(l) = &b.level {
        parts.push(format!("level {l}"));
    }
    if let Some(mv) = b.millivolts {
        parts.push(format!("{mv} mV"));
    }
    parts.push(b.state.clone());
    parts.join(", ")
}

/// Ping answer: protocol major and minor, and the echoed ping byte.
pub fn decode_ping(p: &[u8]) -> Option<(u8, u8, u8)> {
    if p.len() < 3 {
        None
    } else {
        Some((p[0], p[1], p[2]))
    }
}

/// Root getFeature answer: feature index, type flags, version.
pub fn decode_get_feature(p: &[u8]) -> Option<(u8, u8, u8)> {
    if p.len() < 3 {
        None
    } else {
        Some((p[0], p[1], p[2]))
    }
}

/// FeatureSet getFeatureID answer: feature ID, type flags, version.
pub fn decode_feature_id(p: &[u8]) -> Option<(u16, u8, u8)> {
    if p.len() < 4 {
        None
    } else {
        Some((u16::from_be_bytes([p[0], p[1]]), p[2], p[3]))
    }
}

/// Receiver register 0x02 and 0xB5 reads: receivers reply with the sub-register echoed in byte 0.
pub fn decode_connection_state(p: &[u8]) -> String {
    if p.len() < 3 {
        return "short answer".into();
    }
    format!("bytes {:02x} {:02x} {:02x}", p[0], p[1], p[2])
}

/// Pairing information (0xB5 sub 0x20..0x25): the wireless product ID and the device kind.
/// Layout read from the measured frames: `p[0]` echoes the sub-register, `p[3..5]` is the wireless
/// product ID (big endian), `p[7] & 0x0F` the kind.
pub fn decode_pairing(p: &[u8]) -> Option<(u16, u8)> {
    if p.len() < 8 {
        return None;
    }
    Some((u16::from_be_bytes([p[3], p[4]]), p[7] & 0x0F))
}

pub fn kind_name(kind: u8) -> &'static str {
    match kind {
        0 => "unknown",
        1 => "keyboard",
        2 => "mouse",
        3 => "numpad",
        4 => "presenter",
        8 => "trackball",
        9 => "touchpad",
        _ => "other",
    }
}

/// Device name register (0xB5 sub 0x40..0x45): `p[0]` echoes the sub-register, `p[1]` is the length.
pub fn decode_register_name(p: &[u8]) -> Option<String> {
    if p.len() < 2 {
        return None;
    }
    let n = (p[1] as usize).min(p.len().saturating_sub(2));
    Some(String::from_utf8_lossy(&p[2..2 + n]).into_owned())
}

pub fn decode_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Describe any input frame for the listen mode. `battery_index` maps a (device, feature index) to
/// the battery feature ID when known.
pub fn describe_event(frame: &[u8], battery_index: &dyn Fn(u8, u8) -> Option<u16>) -> String {
    if frame.len() < SHORT_LEN {
        return format!("short frame ({} bytes)", frame.len());
    }
    let (id, idx, b2, b3) = (frame[0], frame[1], frame[2], frame[3]);
    if id != SHORT_ID && id != LONG_ID {
        return format!("not HID++ (report id 0x{id:02x}, a DJ or other vendor report)");
    }
    match b2 {
        0x41 => {
            let flags = frame[4];
            let link_lost = flags & 0x40 != 0;
            let wpid = u16::from_be_bytes([frame[6], frame[5]]);
            format!(
                "HID++ 1.0 notification 0x41 device connection: device {idx}, link {}, kind {}, wireless pid {wpid:04x}",
                if link_lost {
                    "LOST (bit 0x40 set)"
                } else {
                    "established"
                },
                kind_name(flags & 0x0F)
            )
        }
        0x40 => format!(
            "HID++ 1.0 notification 0x40 device disconnection: device {idx}, data {:02x}",
            frame[3]
        ),
        0x07 | 0x0D => format!(
            "HID++ 1.0 battery notification 0x{b2:02x}: device {idx}, data {}",
            hex(&frame[3..7])
        ),
        0x8F => format!(
            "HID++ 1.0 error: device {idx}, request {:02x}{:02x}, code 0x{:02x}",
            frame[3], frame[4], frame[5]
        ),
        0xFF => format!(
            "HID++ 2.0 error: device {idx}, feature index {b3}, code 0x{:02x}",
            frame[5]
        ),
        _ => {
            if b2 == 0x81 || b2 == 0x83 || b2 == 0x80 || b2 == 0x82 {
                return format!(
                    "HID++ 1.0 register answer 0x{b2:02x} reg 0x{b3:02x}, device {idx}"
                );
            }
            let function = b3 >> 4;
            let swid = b3 & 0x0F;
            if swid != 0 {
                return format!(
                    "HID++ 2.0 answer to a request: device {idx}, feature index {b2}, function {function}, software ID 0x{swid:02x}"
                );
            }
            // swid 0: an event of feature index b2, function (event number) in the high nibble
            match battery_index(idx, b2) {
                Some(f) => {
                    let p = &frame[4..];
                    let decoded = match f {
                        F_BATTERY_STATUS if function == 0 => decode_battery_status(p),
                        F_BATTERY_VOLTAGE if function == 0 => decode_battery_voltage(p),
                        F_UNIFIED_BATTERY if function == 0 => decode_unified_status(p),
                        F_ADC_MEASUREMENT if function == 0 => decode_adc(p),
                        _ => None,
                    };
                    match decoded {
                        Some(b) => format!(
                            "HID++ 2.0 BATTERY EVENT 0x{f:04x} {}: device {idx}, {}",
                            feature_name(f),
                            format_battery(&b)
                        ),
                        None => format!(
                            "HID++ 2.0 event of battery feature 0x{f:04x}, event {function}: device {idx}"
                        ),
                    }
                }
                None => format!(
                    "HID++ 2.0 event: device {idx}, feature index {b2}, event {function} (feature not a known battery one)"
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_lengths_and_swid() {
        let ping = Request::feature(1, 0, 1, &[0, 0, 0x5A], false).encode();
        assert_eq!(ping.len(), SHORT_LEN);
        assert_eq!(hex(&ping), "1001001d00005a");
        let long = Request::feature(2, 7, 1, &[], true).encode();
        assert_eq!(long.len(), LONG_LEN);
        assert_eq!(long[0], LONG_ID);
        assert_eq!(long[3], 0x1D);
        assert_eq!(long[3] & 0x0F, SWID);
        let reg = Request::register_read(RECEIVER, 0x02, [0, 0, 0]).encode();
        assert_eq!(hex(&reg), "10ff8102000000");
        let pair = Request::register_read_long(RECEIVER, 0xB5, [0x20, 0, 0]).encode();
        assert_eq!(hex(&pair), "10ff83b5200000");
    }

    #[test]
    fn allowed_list_refuses_writes_and_serials() {
        assert!(is_allowed(&Request::register_read(RECEIVER, 0x02, [0; 3])));
        assert!(is_allowed(&Request::register_read_long(
            RECEIVER,
            0xB5,
            [0x25, 0, 0]
        )));
        assert!(is_allowed(&Request::register_read_long(
            RECEIVER,
            0xB5,
            [0x40, 0, 0]
        )));
        assert!(
            !is_allowed(&Request::register_read_long(RECEIVER, 0xB5, [0x30, 0, 0])),
            "serial"
        );
        assert!(
            !is_allowed(&Request::register_read_long(RECEIVER, 0xB5, [0x35, 0, 0])),
            "serial"
        );
        assert!(!is_allowed(&Request::register_read(RECEIVER, 0x00, [0; 3])));
        let write = Request {
            report_id: SHORT_ID,
            index: RECEIVER,
            b2: 0x80,
            b3: 0x00,
            params: vec![0, 9, 0],
        };
        assert!(!is_allowed(&write));
        let write_long = Request {
            report_id: SHORT_ID,
            index: RECEIVER,
            b2: 0x82,
            b3: 0xB2,
            params: vec![0, 0, 0],
        };
        assert!(!is_allowed(&write_long));
        assert!(is_allowed(&Request::feature(1, 5, 1, &[0], true)));
        assert!(
            !is_allowed(&Request::feature(1, 5, 2, &[0], true)),
            "function 2 is a setter on most features"
        );
        let wrong_swid = Request {
            report_id: LONG_ID,
            index: 1,
            b2: 5,
            b3: 0x10,
            params: vec![],
        };
        assert!(!is_allowed(&wrong_swid));
    }

    #[test]
    fn match_by_index_feature_and_swid() {
        let req = Request::feature(1, 4, 1, &[], true);
        // answer: device 1, feature index 4, function 1, swid 0xD
        let ans = parse_hex("11 01 04 1d 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &ans), Reply::Answer(ans[4..].to_vec()));
        // same feature, G HUB's software ID: ignored
        let other = parse_hex("11 01 04 1b 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &other), Reply::Ignore);
        // an event (swid 0) on the same feature index: ignored
        let event = parse_hex("11 01 04 00 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &event), Reply::Ignore);
        // other device index: ignored
        let dev2 = parse_hex("11 02 04 1d 55 04 01 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &dev2), Reply::Ignore);
        // not HID++ (DJ report)
        let dj = parse_hex("20 01 04 1d 55 04 01");
        assert_eq!(classify(&req, &dj), Reply::Ignore);
    }

    #[test]
    fn long_only_path() {
        assert!(is_hidpp_page(0xFF00) && is_hidpp_page(0xFF43));
        assert!(!is_hidpp_page(0xFF13) && !is_hidpp_page(0x000C));
        // Direct device: index 0xFF, ping sent as a long report (20 bytes), answered by index 0xFF.
        let ping = Request::feature(RECEIVER, 0, 1, &[0, 0, 0x5A], true);
        let frame = ping.encode();
        assert_eq!(frame.len(), LONG_LEN);
        assert_eq!(
            hex(&frame),
            "11ff001d00005a0000000000000000000000000000"
                .get(..40)
                .unwrap()
        );
        assert!(is_allowed(&ping));
        let ans = parse_hex("11 ff 00 1d 04 02 5a 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&ping, &ans), Reply::Answer(ans[4..].to_vec()));
        // Index 0xFF register reads still only allowed on the whitelist; a write never is.
        let write = Request {
            report_id: LONG_ID,
            index: RECEIVER,
            b2: 0x80,
            b3: 0x00,
            params: vec![],
        };
        assert!(!is_allowed(&write));
        // A 1.0 style answer for a different software ID stays ignored.
        let other = parse_hex("11 ff 00 1b 04 02 5a 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&ping, &other), Reply::Ignore);
    }

    #[test]
    fn match_errors() {
        let req = Request::feature(1, 0, 1, &[0, 0, 0x5A], false);
        // HID++ 1.0 device answers the 2.0 ping with a 1.0 error, invalid sub-id
        let e1 = parse_hex("10 01 8f 00 1d 01 00");
        assert_eq!(classify(&req, &e1), Reply::Error { v1: true, code: 1 });
        let e2 = parse_hex("11 01 ff 00 1d 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &e2), Reply::Error { v1: false, code: 2 });
        // error for another request (feature index 5): ignored
        let e3 = parse_hex("11 01 ff 05 1d 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&req, &e3), Reply::Ignore);
        // register read error from the receiver
        let reg = Request::register_read_long(RECEIVER, 0xB5, [0x22, 0, 0]);
        let er = parse_hex("10 ff 8f 83 b5 08 00");
        assert_eq!(classify(&reg, &er), Reply::Error { v1: true, code: 8 });
        // register answer
        let ok = parse_hex("11 ff 83 b5 22 00 00 40 70 00 00 02 00 00 00 00 00 00 00 00");
        assert_eq!(classify(&reg, &ok), Reply::Answer(ok[4..].to_vec()));
    }

    #[test]
    fn decode_battery_features() {
        let b = decode_battery_status(&[80, 50, 0]).unwrap();
        assert_eq!((b.percent, b.state.as_str()), (Some(80), "discharging"));
        let b = decode_battery_status(&[100, 0, 3]).unwrap();
        assert_eq!(b.state, "full");
        let b = decode_battery_voltage(&[0x0F, 0xA0, 0x80]).unwrap();
        assert_eq!((b.millivolts, b.state.as_str()), (Some(4000), "charging"));
        let b = decode_battery_voltage(&[0x0E, 0x10, 0x00]).unwrap();
        assert_eq!(
            (b.millivolts, b.state.as_str()),
            (Some(3600), "discharging")
        );
        let b = decode_unified_status(&[55, 4, 0]).unwrap();
        assert_eq!(
            (b.percent, b.level.as_deref(), b.state.as_str()),
            (Some(55), Some("good"), "discharging")
        );
        let b = decode_unified_status(&[100, 8, 1]).unwrap();
        assert_eq!(
            (b.level.as_deref(), b.state.as_str()),
            (Some("full"), "charging")
        );
        let b = decode_adc(&[0x0C, 0xE4, 0x00, 0]).unwrap();
        assert_eq!(b.millivolts, Some(3300));
        assert!(b.state.contains("flags 0x00"));
        assert!(decode_battery_status(&[1, 2]).is_none());
    }

    #[test]
    fn event_decoding() {
        let none = |_: u8, _: u8| None;
        // device connection notification: link established, keyboard (kind 1), wpid 4070
        let up = parse_hex("10 01 41 04 01 70 40");
        assert!(describe_event(&up, &none).contains("link established"));
        // same with the link-lost bit
        let down = parse_hex("10 01 41 04 41 70 40");
        let text = describe_event(&down, &none);
        assert!(text.contains("LOST"), "{text}");
        assert!(text.contains("4070"), "{text}");
        // battery event of feature index 6 mapped to 0x1004
        let map = |d: u8, i: u8| {
            if d == 2 && i == 6 {
                Some(F_UNIFIED_BATTERY)
            } else {
                None
            }
        };
        let ev = parse_hex("11 02 06 00 37 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        let text = describe_event(&ev, &map);
        assert!(
            text.contains("BATTERY EVENT 0x1004") && text.contains("55 %"),
            "{text}"
        );
        // answer to someone else's request (swid B)
        let theirs = parse_hex("11 02 06 1b 37 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
        assert!(describe_event(&theirs, &map).contains("software ID 0x0b"));
    }

    // ---- tests on frames captured from the owner's devices (spec/fixtures)

    const G703: &str = include_str!("../../../spec/fixtures/windows-c539-g703.hidtrace");
    const G915X: &str = include_str!("../../../spec/fixtures/windows-c547-g915x.hidtrace");

    /// (sent, received) pairs of a trace: each `>` line with the `<` line that follows it.
    fn pairs(trace: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        let mut sent: Option<Vec<u8>> = None;
        for line in trace.lines() {
            let mut it = line.split_whitespace();
            let (_ms, kind, rest) = (it.next(), it.next(), it.next());
            match (kind, rest) {
                (Some(">"), Some(h)) => sent = Some(parse_hex(h)),
                (Some("<"), Some(h)) => {
                    if let Some(s) = sent.take() {
                        out.push((s, parse_hex(h)));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Rebuild the request from the sent frame so `classify` is tested on real bytes.
    fn request_of(sent: &[u8]) -> Request {
        Request {
            report_id: sent[0],
            index: sent[1],
            b2: sent[2],
            b3: sent[3],
            params: sent[4..].to_vec(),
        }
    }

    #[test]
    fn every_captured_exchange_matches_and_is_allowed() {
        for (name, trace) in [("g703", G703), ("g915x", G915X)] {
            let p = pairs(trace);
            assert!(p.len() > 40, "{name}: {} pairs", p.len());
            for (sent, recv) in p {
                let req = request_of(&sent);
                assert!(
                    is_allowed(&req),
                    "{name}: sent frame not allowed: {}",
                    hex(&sent)
                );
                assert_eq!(req.encode(), sent, "{name}: re-encoding differs");
                assert_ne!(
                    classify(&req, &recv),
                    Reply::Ignore,
                    "{name}: {} / {}",
                    hex(&sent),
                    hex(&recv)
                );
            }
        }
    }

    #[test]
    fn captured_receiver_registers_g703() {
        let p = pairs(G703);
        let find = |sent: &str| {
            p.iter()
                .find(|(s, _)| hex(s) == sent)
                .map(|(_, r)| r.clone())
                .unwrap()
        };
        let reg02 = find("10ff8102000000");
        assert_eq!(decode_connection_state(&reg02[4..]), "bytes 00 01 00");
        let pair = find("10ff83b5200000");
        assert_eq!(decode_pairing(&pair[4..]), Some((0x4086, 2)));
        assert_eq!(kind_name(2), "mouse");
        let name = find("10ff83b5400000");
        assert_eq!(decode_register_name(&name[4..]).as_deref(), Some("G703 LS"));
        // empty slot: invalid value
        let empty = find("10ff83b5210000");
        let req = request_of(&parse_hex("10ff83b5210000"));
        assert_eq!(classify(&req, &empty), Reply::Error { v1: true, code: 3 });
    }

    #[test]
    fn captured_ping_and_offline_slots() {
        let p = pairs(G703);
        let ping = p.iter().find(|(s, _)| hex(s) == "1001001d00005a").unwrap();
        match classify(&request_of(&ping.0), &ping.1) {
            Reply::Answer(a) => assert_eq!(decode_ping(&a), Some((4, 2, 0x5A))),
            other => panic!("{other:?}"),
        }
        let off = p.iter().find(|(s, _)| hex(s) == "1002001d00005a").unwrap();
        assert_eq!(
            classify(&request_of(&off.0), &off.1),
            Reply::Error { v1: true, code: 8 }
        );
    }

    #[test]
    fn captured_g703_battery_voltage_0x1001() {
        let p = pairs(G703);
        let (s, r) = p
            .iter()
            .find(|(s, _)| hex(s) == "1101060d00000000000000000000000000000000")
            .unwrap();
        let Reply::Answer(a) = classify(&request_of(s), r) else {
            panic!()
        };
        let b = decode_battery_voltage(&a).unwrap();
        assert_eq!(b.feature, F_BATTERY_VOLTAGE);
        assert_eq!(b.millivolts, Some(3966));
        assert_eq!(b.state, "discharging");
        assert_eq!(b.percent, None);
    }

    #[test]
    fn captured_g915x_unified_battery_0x1004() {
        let p = pairs(G915X);
        let (s, r) = p
            .iter()
            .find(|(s, _)| hex(s) == "1101080d00000000000000000000000000000000")
            .unwrap();
        let Reply::Answer(a) = classify(&request_of(s), r) else {
            panic!()
        };
        assert_eq!(
            describe_unified_capabilities(&a),
            "supported levels [critical,low,good,full], flags 0x0f (rechargeable true, state-of-charge true)"
        );
        let (s, r) = p
            .iter()
            .find(|(s, _)| hex(s) == "1101081d00000000000000000000000000000000")
            .unwrap();
        let Reply::Answer(a) = classify(&request_of(s), r) else {
            panic!()
        };
        let b = decode_unified_status(&a).unwrap();
        assert_eq!(
            (b.percent, b.level.as_deref(), b.state.as_str()),
            (Some(49), Some("good"), "discharging")
        );
    }

    #[test]
    fn captured_g733_adc_long_only() {
        const G733: &str = include_str!("../../../spec/fixtures/windows-0afe-g733.hidtrace");
        let p = pairs(G733);
        assert!(p.len() >= 20, "{} pairs", p.len());
        for (sent, recv) in &p {
            let req = request_of(sent);
            assert_eq!(sent.len(), LONG_LEN, "long reports only");
            assert_eq!(sent[1], RECEIVER, "direct device index");
            assert!(is_allowed(&req));
            assert_ne!(classify(&req, recv), Reply::Ignore);
        }
        let (s, r) = p
            .iter()
            .find(|(s, _)| hex(s) == "11ff080d00000000000000000000000000000000")
            .unwrap();
        let Reply::Answer(a) = classify(&request_of(s), r) else {
            panic!()
        };
        let b = decode_adc(&a).unwrap();
        assert_eq!(b.feature, F_ADC_MEASUREMENT);
        assert_eq!(b.millivolts, Some(0x0EA8));
        assert_eq!(a[2], 0x01);
    }

    #[test]
    fn captured_device_names_decode() {
        let p = pairs(G703);
        let mut name = Vec::new();
        for (s, r) in p.iter().filter(|(s, _)| s[2] == 0x03 && s[3] == 0x1D) {
            if let Reply::Answer(a) = classify(&request_of(s), r) {
                name.extend_from_slice(&a[..a.len().min(16)]);
            }
        }
        assert_eq!(
            decode_text(&name[..45]),
            "G703 LIGHTSPEED Wireless Gaming Mouse w/ HERO"
        );
    }

    #[test]
    fn captured_foreign_battery_answer_is_ignored() {
        // Seen while listening on C539: G HUB's own 0x1001 poll (software ID 0x0E) answered on the same collection.
        let theirs = parse_hex("1101060e0f7e0000000000000000000000000000");
        let ours = Request::feature(1, 6, 0, &[], true);
        assert_eq!(classify(&ours, &theirs), Reply::Ignore);
        let map = |d: u8, i: u8| {
            if (d, i) == (1, 6) {
                Some(F_BATTERY_VOLTAGE)
            } else {
                None
            }
        };
        assert!(describe_event(&theirs, &map).contains("software ID 0x0e"));
    }
}
