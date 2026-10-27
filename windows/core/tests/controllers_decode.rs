// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The game-controller decoders on reports synthesized from the documented layout
//! (docs/research/controllers.md): USB report 0x01 of 64 bytes with the battery at byte 53, Bluetooth report 0x31
//! with it at byte 54.

use juicebar_core::controllers::{
    self, GamingStatus, Protocol, decode_dualsense, device_info, find_model, gaming_reading,
    xinput, xinput_reading,
};
use juicebar_core::types::{BatteryReading, Charging, Coarse, Connection, Kind, Quality};

const AT: f64 = 1_800_000_000.0;

fn usb(battery: u8) -> Vec<u8> {
    let mut r = vec![0u8; 64];
    r[0] = 0x01;
    r[53] = battery;
    r
}

fn bluetooth(battery: u8) -> Vec<u8> {
    let mut r = vec![0u8; 78];
    r[0] = 0x31;
    r[54] = battery;
    r
}

fn percent(r: Option<BatteryReading>) -> Option<f64> {
    r.and_then(|r| r.percent)
}

#[test]
fn a_discharging_dualsense_reads_level_times_ten_plus_five() {
    for (level, want) in [(0u8, 5.0), (1, 15.0), (7, 75.0), (9, 95.0)] {
        let r = decode_dualsense(&usb(level), false, AT).unwrap();
        assert_eq!(r.percent, Some(want), "usb level {level}");
        assert_eq!(r.charging, Charging::Discharging);
        assert_eq!(r.quality, Quality::Exact);
        assert_eq!((r.coarse, r.millivolts, r.at), (None, None, AT));
        let b = decode_dualsense(&bluetooth(level), true, AT).unwrap();
        assert_eq!(b, r, "the same battery byte reads the same over Bluetooth");
    }
}

#[test]
fn a_charging_dualsense_keeps_its_level_and_says_charging() {
    let r = decode_dualsense(&usb(0x13), false, AT).unwrap();
    assert_eq!((r.percent, r.charging), (Some(35.0), Charging::Charging));
    let b = decode_dualsense(&bluetooth(0x18), true, AT).unwrap();
    assert_eq!((b.percent, b.charging), (Some(85.0), Charging::Charging));
}

#[test]
fn a_full_dualsense_is_one_hundred_whatever_its_level() {
    for level in [0u8, 5, 0x0A, 0x0F] {
        let r = decode_dualsense(&usb(0x20 | level), false, AT).unwrap();
        assert_eq!((r.percent, r.charging), (Some(100.0), Charging::Full));
    }
}

#[test]
fn level_ten_and_above_is_capped_at_one_hundred() {
    assert_eq!(
        percent(decode_dualsense(&usb(0x0A), false, AT)),
        Some(100.0)
    );
    assert_eq!(
        percent(decode_dualsense(&usb(0x0F), false, AT)),
        Some(100.0)
    );
    assert_eq!(
        percent(decode_dualsense(&usb(0x1A), false, AT)),
        Some(100.0)
    );
    assert_eq!(
        percent(decode_dualsense(&bluetooth(0x0C), true, AT)),
        Some(100.0)
    );
}

#[test]
fn an_unknown_status_nibble_gives_no_reading() {
    for status in [3u8, 4, 0xA, 0xB, 0xF] {
        assert_eq!(decode_dualsense(&usb(status << 4 | 5), false, AT), None);
        assert_eq!(
            decode_dualsense(&bluetooth(status << 4 | 5), true, AT),
            None
        );
    }
}

#[test]
fn a_report_too_short_for_the_battery_byte_gives_no_reading() {
    // One byte short of the battery index, and exactly long enough.
    assert_eq!(decode_dualsense(&usb(5)[..53], false, AT), None);
    assert!(decode_dualsense(&usb(5)[..54], false, AT).is_some());
    assert_eq!(decode_dualsense(&bluetooth(5)[..54], true, AT), None);
    assert!(decode_dualsense(&bluetooth(5)[..55], true, AT).is_some());
    assert_eq!(decode_dualsense(&[], false, AT), None);
    assert_eq!(decode_dualsense(&[0x01], false, AT), None);
    assert_eq!(decode_dualsense(&[0x31], true, AT), None);
}

#[test]
fn a_report_of_the_other_layout_is_not_read() {
    // A Bluetooth report read as USB, and the other way round: the ids differ.
    assert_eq!(decode_dualsense(&bluetooth(5), false, AT), None);
    assert_eq!(decode_dualsense(&usb(5), true, AT), None);
    // The short "simple" Bluetooth report a pad sends before the feature read has no battery.
    let mut simple = vec![0u8; 10];
    simple[0] = 0x01;
    assert_eq!(decode_dualsense(&simple, true, AT), None);
    assert_eq!(decode_dualsense(&simple, false, AT), None);
    // A different report id of the right length.
    let mut other = usb(5);
    other[0] = 0x02;
    assert_eq!(decode_dualsense(&other, false, AT), None);
}

#[test]
fn the_offset_is_the_one_the_layout_names() {
    // A byte that is not the battery byte never moves the reading.
    let mut r = usb(0x07);
    r[52] = 0x2F;
    r[54] = 0x1F;
    assert_eq!(percent(decode_dualsense(&r, false, AT)), Some(75.0));
    let mut b = bluetooth(0x07);
    b[53] = 0x2F;
    b[55] = 0x1F;
    assert_eq!(percent(decode_dualsense(&b, true, AT)), Some(75.0));
}

#[test]
fn xinput_levels_map_to_coarse_readings() {
    for (level, want) in [
        (xinput::LEVEL_EMPTY, Coarse::Critical),
        (xinput::LEVEL_LOW, Coarse::Low),
        (xinput::LEVEL_MEDIUM, Coarse::Good),
        (xinput::LEVEL_FULL, Coarse::Full),
    ] {
        for kind in [xinput::TYPE_ALKALINE, xinput::TYPE_NIMH] {
            let r = xinput_reading(kind, level, AT).unwrap();
            assert_eq!(r.coarse, Some(want));
            assert_eq!(r.percent, None, "XInput gives no percentage");
            assert_eq!(r.quality, Quality::Coarse);
            assert_eq!(r.charging, Charging::Unknown);
        }
    }
}

#[test]
fn xinput_gives_nothing_for_a_pad_without_a_known_battery() {
    for kind in [
        xinput::TYPE_DISCONNECTED,
        xinput::TYPE_WIRED,
        xinput::TYPE_UNKNOWN,
        7,
    ] {
        assert_eq!(xinput_reading(kind, xinput::LEVEL_FULL, AT), None, "{kind}");
    }
    assert_eq!(xinput_reading(xinput::TYPE_NIMH, 4, AT), None);
    assert_eq!(xinput_reading(xinput::TYPE_NIMH, 0xFF, AT), None);
}

#[test]
fn a_gaming_input_report_is_remaining_over_full_capacity() {
    let r = gaming_reading(Some(2500), Some(5000), GamingStatus::Discharging, AT).unwrap();
    assert_eq!(r.percent, Some(50.0));
    assert_eq!(r.charging, Charging::Discharging);
    assert_eq!(r.quality, Quality::Exact);
    let c = gaming_reading(Some(1234), Some(5000), GamingStatus::Charging, AT).unwrap();
    assert_eq!((c.percent, c.charging), (Some(25.0), Charging::Charging));
    // A pad that reports 0 to 100 directly.
    let d = gaming_reading(Some(37), Some(100), GamingStatus::Discharging, AT).unwrap();
    assert_eq!(d.percent, Some(37.0));
}

#[test]
fn a_gaming_input_report_is_kept_within_zero_and_one_hundred() {
    let over = gaming_reading(Some(6000), Some(5000), GamingStatus::Discharging, AT).unwrap();
    assert_eq!(over.percent, Some(100.0));
    let under = gaming_reading(Some(-5), Some(5000), GamingStatus::Discharging, AT).unwrap();
    assert_eq!(under.percent, Some(0.0));
}

#[test]
fn an_idle_battery_is_full_at_one_hundred_and_unknown_below() {
    let full = gaming_reading(Some(5000), Some(5000), GamingStatus::Idle, AT).unwrap();
    assert_eq!((full.percent, full.charging), (Some(100.0), Charging::Full));
    let part = gaming_reading(Some(3000), Some(5000), GamingStatus::Idle, AT).unwrap();
    assert_eq!(
        (part.percent, part.charging),
        (Some(60.0), Charging::Unknown)
    );
}

#[test]
fn an_unusable_gaming_input_report_gives_nothing() {
    use GamingStatus::*;
    assert_eq!(gaming_reading(Some(100), Some(100), NotPresent, AT), None);
    assert_eq!(gaming_reading(None, Some(100), Discharging, AT), None);
    assert_eq!(gaming_reading(Some(50), None, Discharging, AT), None);
    assert_eq!(gaming_reading(Some(50), Some(0), Discharging, AT), None);
    assert_eq!(gaming_reading(Some(0), Some(0), Discharging, AT), None);
    assert_eq!(gaming_reading(Some(50), Some(-1), Discharging, AT), None);
    assert_eq!(GamingStatus::from_code(0), Some(NotPresent));
    assert_eq!(GamingStatus::from_code(3), Some(Charging));
    assert_eq!(GamingStatus::from_code(4), None);
    assert_eq!(GamingStatus::from_code(-1), None);
}

#[test]
fn the_catalog_names_the_dualsense_and_the_xbox_pads() {
    let ds = find_model(0x054C, 0x0CE6).unwrap();
    assert_eq!((ds.name, ds.protocol), ("DualSense", Protocol::DualSense));
    let edge = find_model(0x054C, 0x0DF2).unwrap();
    assert_eq!(edge.name, "DualSense Edge");
    let series = find_model(0x045E, 0x0B13).unwrap();
    assert_eq!(
        (series.brand, series.protocol),
        ("Microsoft", Protocol::Xbox)
    );
    // The same product id under another vendor, and a Microsoft product that is not a pad.
    assert_eq!(find_model(0x046D, 0x0CE6), None);
    assert_eq!(find_model(0x045E, 0x0CE6), None);
    assert_eq!(find_model(0x054C, 0x0B13), None);
    assert_eq!(find_model(0x045E, 0x082A), None);
    assert!(controllers::models().len() == 18);
    assert!(
        controllers::models()
            .iter()
            .all(|m| m.vid == 0x054C || m.vid == 0x045E)
    );
}

#[test]
fn keys_hold_the_vendor_and_product_and_hash_what_could_be_an_address() {
    assert_eq!(
        controllers::controller_key(0x054C, 0x0CE6, None).0,
        "game-controllers:054C:0CE6"
    );
    let path = r"\\?\hid#{00001124-0000-1000-8000-00805f9b34fb}_vid&0002054c_pid&0ce6#a&1b2c3d&0&a1b2c3d4e5f6_c00000000#{4d1e55b2}";
    let hashed = controllers::controller_key(0x054C, 0x0CE6, Some(path));
    let (head, tail) = hashed.0.rsplit_once(':').unwrap();
    assert_eq!(head, "game-controllers:054C:0CE6");
    assert_eq!(tail.len(), 16);
    assert!(tail.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!hashed.0.to_lowercase().contains("a1b2c3d4e5f6"));
    assert_eq!(
        hashed,
        controllers::controller_key(0x054C, 0x0CE6, Some(path))
    );
    assert_ne!(
        hashed,
        controllers::controller_key(0x054C, 0x0CE6, Some("other"))
    );
    assert!(hashed.0.starts_with(controllers::PROVIDER));
}

#[test]
fn a_pad_is_announced_as_a_gamepad_by_its_catalog_name() {
    let model = find_model(0x045E, 0x0B12).unwrap();
    let key = controllers::controller_key(0x045E, 0x0B12, None);
    let info = device_info(model, key.clone(), false);
    assert_eq!(info.key, key);
    assert_eq!(info.provider, "game-controllers");
    assert_eq!(
        (info.kind, info.connection),
        (Kind::Gamepad, Connection::Usb)
    );
    assert_eq!(
        (info.name.as_str(), info.brand.as_str()),
        ("Xbox Series Controller", "Microsoft")
    );
    assert_eq!(
        device_info(model, key, true).connection,
        Connection::Bluetooth
    );
}
