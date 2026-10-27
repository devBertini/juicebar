// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The `game-controllers` provider's pure side (spec/PROVIDERS.md, docs/research/controllers.md): which products are
//! game pads, the identity of one, and the decoders that turn what a pad or the system reports into a
//! [`BatteryReading`]. The app shell opens, reads and closes; nothing here touches a device.
//!
//! Three sources, three decoders:
//! - a DualSense input report, sampled once ([`decode_dualsense`]): a percentage in steps of ten;
//! - `XInputGetBatteryInformation` ([`xinput_reading`]): four levels, no percentage, no charging state;
//! - a `Windows.Gaming.Input` battery report ([`gaming_reading`]): remaining and full capacity, and a status.

use std::sync::OnceLock;

use crate::types::{
    BatteryReading, Charging, Coarse, Connection, DeviceInfo, DeviceKey, Kind, Quality,
};

/// The provider id in a [`DeviceInfo`] and in a device key.
pub const PROVIDER: &str = "game-controllers";

/// The slow refresh of a present pad, in seconds (spec/PROVIDERS.md: a read wakes a sleeping wireless device).
pub const REFRESH_SECONDS: u32 = 600;

/// Sony's USB vendor id (SDL `usb_ids.h`, `USB_VENDOR_SONY`).
pub const SONY_VID: u16 = 0x054C;
/// Microsoft's USB vendor id (`USB_VENDOR_MICROSOFT`).
pub const MICROSOFT_VID: u16 = 0x045E;

/// Whether a vendor id can belong to a game pad of the catalog. A cheap test for the always-registered device
/// notification, which must not look anything up for the vendors of a keyboard or a mouse.
pub fn is_candidate_vendor(vid: u16) -> bool {
    vid == SONY_VID || vid == MICROSOFT_VID
}

/// How a pad's battery is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// One HID input report sampled from the pad (catalog battery `hid-ds5`).
    DualSense,
    /// The system's controller APIs: `Windows.Gaming.Input`, else XInput (catalog battery `wgi`).
    Xbox,
}

/// One product of the catalog (`spec/devices.tsv`, provider `game-controllers`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    pub brand: &'static str,
    pub protocol: Protocol,
}

const DEVICES_TSV: &str = include_str!("../../../spec/devices.tsv");

fn parse_row(line: &'static str) -> Option<Model> {
    let c: Vec<&'static str> = line.split('\t').collect();
    if c.len() < 6 || c[0] != PROVIDER || c[4] != "gamepad" {
        return None;
    }
    let (vid, brand) = match c[1] {
        "sony" => (SONY_VID, "Sony"),
        "microsoft" => (MICROSOFT_VID, "Microsoft"),
        _ => return None,
    };
    let protocol = match c[5] {
        "hid-ds5" => Protocol::DualSense,
        "wgi" => Protocol::Xbox,
        _ => return None,
    };
    Some(Model {
        vid,
        pid: u16::from_str_radix(c[2], 16).ok()?,
        name: c[3],
        brand,
        protocol,
    })
}

/// Every game pad of the catalog, in file order.
pub fn models() -> &'static [Model] {
    static MODELS: OnceLock<Vec<Model>> = OnceLock::new();
    MODELS.get_or_init(|| {
        DEVICES_TSV
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(parse_row)
            .collect()
    })
}

/// The catalog row of a vendor and product id, if the product is a game pad JuiceBar reads.
pub fn find_model(vid: u16, pid: u16) -> Option<&'static Model> {
    if !is_candidate_vendor(vid) {
        return None;
    }
    models().iter().find(|m| m.vid == vid && m.pid == pid)
}

/// A pad's key: `game-controllers:054C:0CE6`. A second pad of the same product, present at the same time, passes
/// `unique` (something that tells it apart, such as its device path); that text may hold a Bluetooth address, so it
/// is hashed (SHA-256, first 16 hex characters) and never stored or logged in the clear.
pub fn controller_key(vid: u16, pid: u16, unique: Option<&str>) -> DeviceKey {
    let mut key = format!("{PROVIDER}:{vid:04X}:{pid:04X}");
    if let Some(u) = unique {
        key.push(':');
        key.push_str(&hash16(u));
    }
    DeviceKey(key)
}

/// What the registry is told when a pad shows up.
pub fn device_info(model: &Model, key: DeviceKey, bluetooth: bool) -> DeviceInfo {
    DeviceInfo {
        key,
        provider: PROVIDER,
        name: model.name.to_string(),
        kind: Kind::Gamepad,
        connection: if bluetooth {
            Connection::Bluetooth
        } else {
            Connection::Usb
        },
        brand: model.brand.to_string(),
    }
}

// ---------------------------------------------------------------- DualSense

/// Input report id over USB (SDL `k_EPS5ReportIdState`).
pub const DS_USB_REPORT_ID: u8 = 0x01;
/// Input report id over Bluetooth once the pad is in its full report mode (`k_EPS5ReportIdBluetoothState`).
pub const DS_BT_REPORT_ID: u8 = 0x31;
/// Length of the USB state report.
pub const DS_USB_REPORT_LEN: usize = 64;
/// The battery byte's index in the report: 52 bytes into the state packet, which starts at report byte 1 over USB
/// and at byte 2 over Bluetooth (docs/research/controllers.md section 1b).
const DS_USB_BATTERY: usize = 53;
const DS_BT_BATTERY: usize = 54;

/// The battery of a DualSense input report: `bluetooth` says which layout the bytes follow (report 0x31 over
/// Bluetooth, 0x01 of 64 bytes over USB). The high nibble of the battery byte is the status (0 discharging,
/// 1 charging, 2 full) and the low nibble the level 0 to 10, a step of ten percent: `level * 10 + 5`, capped at 100.
///
/// The percentage is `Exact`: the pad gives a percentage, only a coarse one (steps of ten), and the estimator
/// reads it as a percentage like any other. A full pad is 100 whatever its level. A status that is none of those
/// three (an error state such as temperature or voltage out of range) carries no usable level, so there is no
/// reading (SDL reports 0 percent for it, which would raise a false low-battery notice). A report of the other
/// transport's layout, the short Bluetooth "simple" report (0x01) a pad sends before it is switched, or one too
/// short to hold the battery byte gives none either. The Bluetooth CRC is not checked: a wrong byte only moves
/// the reading, and the next refresh replaces it.
pub fn decode_dualsense(report: &[u8], bluetooth: bool, at: f64) -> Option<BatteryReading> {
    let (id, index) = if bluetooth {
        (DS_BT_REPORT_ID, DS_BT_BATTERY)
    } else {
        (DS_USB_REPORT_ID, DS_USB_BATTERY)
    };
    if report.first() != Some(&id) {
        return None;
    }
    let byte = *report.get(index)?;
    let level = f64::from(byte & 0x0F);
    let stepped = (level * 10.0 + 5.0).min(100.0);
    let (percent, charging) = match byte >> 4 {
        0 => (stepped, Charging::Discharging),
        1 => (stepped, Charging::Charging),
        2 => (100.0, Charging::Full),
        _ => return None,
    };
    Some(BatteryReading {
        at,
        percent: Some(percent),
        coarse: None,
        millivolts: None,
        charging,
        quality: Quality::Exact,
    })
}

// ---------------------------------------------------------------- XInput

/// `XINPUT_BATTERY_INFORMATION.BatteryType` (xinput.h, `BATTERY_TYPE_*`).
pub mod xinput {
    pub const TYPE_DISCONNECTED: u8 = 0x00;
    pub const TYPE_WIRED: u8 = 0x01;
    pub const TYPE_ALKALINE: u8 = 0x02;
    pub const TYPE_NIMH: u8 = 0x03;
    pub const TYPE_UNKNOWN: u8 = 0xFF;
    /// `BatteryLevel` (`BATTERY_LEVEL_*`).
    pub const LEVEL_EMPTY: u8 = 0x00;
    pub const LEVEL_LOW: u8 = 0x01;
    pub const LEVEL_MEDIUM: u8 = 0x02;
    pub const LEVEL_FULL: u8 = 0x03;
}

/// The level XInput reports for a wireless pad with a known battery: four steps, no percentage, no charging
/// state, so a coarse reading the estimator does not forecast from. The level is "only valid for wireless devices
/// with a known battery type" (Microsoft), so a wired pad, a disconnected slot, an unknown battery type and a level
/// outside the four give no reading.
pub fn xinput_reading(battery_type: u8, level: u8, at: f64) -> Option<BatteryReading> {
    if !matches!(battery_type, xinput::TYPE_ALKALINE | xinput::TYPE_NIMH) {
        return None;
    }
    let coarse = match level {
        xinput::LEVEL_EMPTY => Coarse::Critical,
        xinput::LEVEL_LOW => Coarse::Low,
        xinput::LEVEL_MEDIUM => Coarse::Good,
        xinput::LEVEL_FULL => Coarse::Full,
        _ => return None,
    };
    Some(BatteryReading {
        at,
        percent: None,
        coarse: Some(coarse),
        millivolts: None,
        charging: Charging::Unknown,
        quality: Quality::Coarse,
    })
}

// ---------------------------------------------------------------- Windows.Gaming.Input

/// `Windows.Devices.Power.BatteryStatus` of a battery report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamingStatus {
    NotPresent,
    Discharging,
    Idle,
    Charging,
}

impl GamingStatus {
    /// From the enum's value (0 NotPresent, 1 Discharging, 2 Idle, 3 Charging).
    pub fn from_code(code: i32) -> Option<GamingStatus> {
        Some(match code {
            0 => GamingStatus::NotPresent,
            1 => GamingStatus::Discharging,
            2 => GamingStatus::Idle,
            3 => GamingStatus::Charging,
            _ => return None,
        })
    }
}

/// A `Windows.Gaming.Input` battery report as a reading: the percentage is remaining over full capacity (both in
/// milliwatt-hours), rounded and kept within 0 to 100. A report without a positive full capacity, without a
/// remaining capacity, or of a pad with no battery gives none (the caller then asks XInput). `Idle` is a battery
/// that neither charges nor discharges: full when it is at 100, otherwise the state is unknown.
pub fn gaming_reading(
    remaining_mwh: Option<i32>,
    full_mwh: Option<i32>,
    status: GamingStatus,
    at: f64,
) -> Option<BatteryReading> {
    if status == GamingStatus::NotPresent {
        return None;
    }
    let remaining = f64::from(remaining_mwh?.max(0));
    let full = f64::from(full_mwh.filter(|f| *f > 0)?);
    let percent = (remaining / full * 100.0).round().clamp(0.0, 100.0);
    let charging = match status {
        GamingStatus::Discharging => Charging::Discharging,
        GamingStatus::Charging => Charging::Charging,
        GamingStatus::Idle if percent >= 100.0 => Charging::Full,
        _ => Charging::Unknown,
    };
    Some(BatteryReading {
        at,
        percent: Some(percent),
        coarse: None,
        millivolts: None,
        charging,
        quality: Quality::Exact,
    })
}

// ---------------------------------------------------------------- hashing

/// SHA-256 of `text`, first 16 hex characters (spec/PROVIDERS.md, "Identity without secrets").
pub fn hash16(text: &str) -> String {
    sha256(text.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// FIPS 180-4 SHA-256, written out here because the core has no dependency.
fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = v.to_be_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_matches_the_published_vectors() {
        // FIPS 180-2 appendix B.
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Two blocks of padding: 64 bytes lands the length in a second block.
        assert_eq!(
            hex(&sha256(&[b'a'; 64])),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
        assert_eq!(hash16("abc"), "ba7816bf8f01cfea");
    }
}
