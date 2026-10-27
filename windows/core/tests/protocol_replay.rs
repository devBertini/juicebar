// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The Logitech provider against the probe traces of the owner's devices (spec/fixtures): a fake device answers each
//! request from the trace, and the provider's frames and events are asserted.

mod protocol_rig;

use juicebar_core::hidpp::catalog::Match;
use juicebar_core::registry::{Change, Registry, logitech_key};
use juicebar_core::types::{
    BatteryReading, Charging, Coarse, Connection, DeviceInfo, Kind, ProviderEvent, Quality,
};
use protocol_rig::*;

fn appeared(ev: &ProviderEvent) -> &DeviceInfo {
    match ev {
        ProviderEvent::Appeared(i) => i,
        other => panic!("not Appeared: {other:?}"),
    }
}

fn battery(ev: &ProviderEvent) -> &BatteryReading {
    match ev {
        ProviderEvent::Battery(_, r) => r,
        other => panic!("not Battery: {other:?}"),
    }
}

fn presence_online(ev: &ProviderEvent) -> bool {
    match ev {
        ProviderEvent::Presence(_, p) => p.online,
        other => panic!("not Presence: {other:?}"),
    }
}

fn pairing_reads() -> Vec<String> {
    (0x20..=0x25)
        .map(|s| short(&format!("10ff83b5{s:02x}")))
        .collect()
}

#[test]
fn g703_receiver_trace() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    let mut expected = vec![short("10ff8100")];
    expected.extend(pairing_reads());
    expected.extend([
        short("1001001d00005a"),
        long("1101000d1004"),
        long("1101000d1001"),
        long("1101000d0005"),
        long("1101030d00"),
        long("1101031d00"),
        long("1101031d10"),
        long("1101031d20"),
        long("1101060d00"),
    ]);
    assert_eq!(rig.sent_hex(), expected);

    let key = logitech_key(Match::Wpid, 0x4086);
    let ev = rig.take_events();
    assert_eq!(ev.len(), 4, "{ev:?}");
    let first = appeared(&ev[0]);
    assert_eq!(first.key, key);
    assert_eq!(first.name, "G703 LIGHTSPEED");
    assert_eq!(
        (first.kind, first.connection),
        (Kind::Mouse, Connection::Receiver)
    );
    assert_eq!(first.brand, "Logitech");
    assert!(presence_online(&ev[1]));
    assert_eq!(
        appeared(&ev[2]).name,
        "G703 LIGHTSPEED Wireless Gaming Mouse w/ HERO"
    );
    // 3966 mV on the interim curve: between 3500 mV = 0 and 3968 mV = 77.
    let r = battery(&ev[3]);
    assert_eq!(r.millivolts, Some(3966));
    assert_eq!(r.quality, Quality::Derived);
    assert_eq!(r.charging, Charging::Discharging);
    assert!((r.percent.unwrap() - 77.0 * 466.0 / 468.0).abs() < 1e-9);
    assert_eq!(r.coarse, None);

    // Nothing else is pending but the slow refresh, 600 s after the reading.
    let read_at = r.at - 1_000_000.0;
    // (The requests' own timeout wakes are stale by now: the shell fires them early and the core asks again.)
    rig.advance(read_at + 599.0);
    assert_eq!(rig.wakes.len(), 1);
    assert!((rig.next_wake().unwrap() - (read_at + 600.0)).abs() < 1e-6);
    rig.assert_protocol_only(&[(3, 0), (3, 1), (6, 0)], &[3]);
}

#[test]
fn g915x_receiver_trace_then_the_listen_trace() {
    let mut rig = start_receiver(G915X_TRACE, 0xC547, |_| {});
    let mut expected = vec![short("10ff8100")];
    expected.extend(pairing_reads());
    expected.extend([
        short("1001001d00005a"),
        long("1101000d1004"),
        long("1101000d0005"),
        long("1101030d00"),
        long("1101031d00"),
        long("1101081d00"),
    ]);
    assert_eq!(rig.sent_hex(), expected);
    let key = logitech_key(Match::Wpid, 0x40B5);
    let ev = rig.take_events();
    assert_eq!(ev.len(), 4, "{ev:?}");
    assert_eq!(appeared(&ev[0]).name, "G915 X LIGHTSPEED");
    assert_eq!(appeared(&ev[0]).kind, Kind::Keyboard);
    assert!(presence_online(&ev[1]));
    assert_eq!(appeared(&ev[2]).name, "G915 X LS");
    let r = battery(&ev[3]);
    assert_eq!(r.percent, Some(49.0));
    assert_eq!(r.coarse, Some(Coarse::Good));
    assert_eq!(r.quality, Quality::Exact);
    assert_eq!(r.charging, Charging::Discharging);
    assert_eq!(r.millivolts, None);
    let sent_before = rig.sent.len();

    // Replay what the receiver and keyboard sent on their own while the owner switched the keyboard off and on and
    // plugged the cable: the link events and the battery events, at their recorded times.
    let base = rig.mono;
    let frames = listen_frames(G915X_LISTEN);
    let (before_last, last) = frames.split_at(frames.len() - 1);
    for (t, hex) in before_last {
        rig.advance(base + t);
        let ep = if hex.len() == 14 { 1 } else { 2 };
        rig.report(ep, hex);
    }
    // The keyboard pushed its battery: no slow refresh runs for it, even past 600 s.
    rig.advance(base + 700.0);
    assert_eq!(rig.sent.len(), sent_before);
    assert!(rig.wakes.is_empty(), "{:?}", rig.wakes);
    for (_, hex) in last {
        rig.report(1, hex);
    }
    // Not a single request: the link events and the pushed batteries carried everything.
    assert_eq!(rig.sent.len(), sent_before);
    let ev = rig.take_events();
    let kinds: Vec<String> = ev
        .iter()
        .map(|e| match e {
            ProviderEvent::Presence(_, p) => format!("presence {}", p.online),
            ProviderEvent::Battery(_, r) => {
                format!("battery {} {:?}", r.percent.unwrap(), r.charging)
            }
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "presence false",
            "presence true",
            "battery 50 Discharging",
            "battery 49 Discharging",
            "battery 49 Charging",
            "presence false",
        ]
    );
    // The devices in play are the one keyed 40B5.
    assert!(ev.iter().all(|e| match e {
        ProviderEvent::Presence(k, _) | ProviderEvent::Battery(k, _) => *k == key,
        _ => false,
    }));
    rig.advance(base + 3600.0);
    assert_eq!(
        rig.sent.len(),
        sent_before,
        "an offline device is never read"
    );
}

#[test]
fn g733_dongle_trace() {
    let mut rig = Rig::new();
    rig.add_fake(Fake::from_trace(G733_TRACE, 1, 1), &[1]);
    rig.arrive(endpoint(1, 0x0AFE, 0xFF43, 0x0202, 20));
    // The G733 dongle has long reports only and the device is at index 0xFF: no receiver registers.
    assert_eq!(
        rig.sent_hex(),
        vec![
            long("11ff001d00005a"),
            long("11ff000d1004"),
            long("11ff000d1001"),
            long("11ff000d1000"),
            long("11ff000d1f20"),
            long("11ff000d0005"),
            long("11ff030d00"),
            long("11ff031d00"),
            long("11ff031d10"),
            long("11ff080d00"),
        ]
    );
    let ev = rig.take_events();
    assert_eq!(ev.len(), 4, "{ev:?}");
    let first = appeared(&ev[0]);
    assert_eq!(first.name, "G733");
    assert_eq!(
        (first.kind, first.connection),
        (Kind::Headset, Connection::Dongle)
    );
    assert_eq!(first.key, logitech_key(Match::Dongle, 0x0AB5));
    assert!(presence_online(&ev[1]));
    assert_eq!(appeared(&ev[2]).name, "G733 Gaming Headset");
    // 3752 mV on the interim g733 curve; the flags byte (0x01) is not decoded as charging.
    let r = battery(&ev[3]);
    assert_eq!(r.millivolts, Some(3752));
    assert_eq!(r.quality, Quality::Derived);
    assert_eq!(r.charging, Charging::Unknown);
    assert!((r.percent.unwrap() - 45.0 * 352.0 / 353.0).abs() < 1e-9);
    rig.assert_protocol_only(&[(3, 0), (3, 1), (8, 0)], &[]);
}

#[test]
fn g915x_on_its_cable_trace() {
    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(G915X_CABLE, 1, 2);
    fake.alias = Some(1);
    rig.add_fake(fake, &[1, 2]);
    for ep in usb_endpoints(0xC356, 1) {
        rig.arrive(ep);
    }
    assert_eq!(
        rig.sent_hex(),
        vec![
            short("10ff001d00005a"),
            long("11ff000d1004"),
            long("11ff000d0005"),
            long("11ff030d00"),
            long("11ff031d00"),
            long("11ff071d00"),
        ]
    );
    let ev = rig.take_events();
    assert_eq!(ev.len(), 4, "{ev:?}");
    // The cable is the same device as the receiver's 40B5.
    assert_eq!(appeared(&ev[0]).key, logitech_key(Match::Wpid, 0x40B5));
    assert_eq!(appeared(&ev[0]).connection, Connection::Usb);
    let r = battery(&ev[3]);
    assert_eq!((r.percent, r.charging), (Some(49.0), Charging::Charging));
    assert_eq!(r.coarse, Some(Coarse::Good));
    rig.assert_protocol_only(&[(3, 0), (3, 1), (7, 1)], &[]);
}

#[test]
fn the_cable_continues_the_receivers_device() {
    // The receiver and the keyboard come up, then the keyboard is plugged in: the receiver reports the link lost
    // and the keyboard appears as USB C356.
    let mut rig = start_receiver(G915X_TRACE, 0xC547, |_| {});
    let mut registry = Registry::new();
    let key = logitech_key(Match::Wpid, 0x40B5);
    let apply = |rig: &mut Rig, registry: &mut Registry| -> Vec<Change> {
        rig.take_events()
            .iter()
            .flat_map(|e| registry.apply(e))
            .collect()
    };
    let first = apply(&mut rig, &mut registry);
    assert!(first.contains(&Change::Added(key.clone())));

    rig.advance(rig.mono + 10.0);
    rig.report(1, "1001411161b540");
    let lost = apply(&mut rig, &mut registry);
    assert_eq!(lost, vec![Change::Presence(key.clone())]);
    assert!(!registry.get(&key).unwrap().online());
    let sent_before = rig.sent.len();

    let mut cable = Fake::from_trace(G915X_CABLE, 11, 12);
    cable.alias = Some(1);
    rig.add_fake(cable, &[11, 12]);
    for ep in usb_endpoints(0xC356, 11) {
        rig.arrive(ep);
    }
    // The name is known from the receiver: the cable asks no name, only the ping, the battery feature and the read.
    let cable_sent: Vec<String> = rig.sent_hex()[sent_before..].to_vec();
    assert_eq!(
        cable_sent,
        vec![
            short("10ff001d00005a"),
            long("11ff000d1004"),
            long("11ff071d00"),
        ]
    );
    let merged = apply(&mut rig, &mut registry);
    assert!(merged.contains(&Change::Updated(key.clone())), "{merged:?}");
    assert!(merged.contains(&Change::Presence(key.clone())));
    assert!(merged.contains(&Change::Battery(key.clone())));
    assert!(!merged.iter().any(|c| matches!(c, Change::Added(_))));
    assert_eq!(registry.entries().len(), 1, "one device, not two");
    let e = registry.get(&key).unwrap();
    assert!(e.online());
    assert_eq!(e.info.connection, Connection::Usb);
    assert_eq!(e.info.name, "G915 X LS");
    assert_eq!(e.last.as_ref().unwrap().charging, Charging::Charging);

    // The cable comes out: the receiver still knows the keyboard, off; the device is not gone.
    rig.remove(11);
    rig.remove(12);
    let unplugged = apply(&mut rig, &mut registry);
    assert!(
        unplugged.contains(&Change::Presence(key.clone())),
        "{unplugged:?}"
    );
    assert!(
        !unplugged
            .iter()
            .any(|c| matches!(c, Change::Unavailable(_)))
    );
    assert!(!registry.get(&key).unwrap().online());
    assert!(registry.get(&key).unwrap().available);

    // The receiver link comes back before anything else: online through the receiver again.
    rig.advance(rig.mono + 5.0);
    rig.report(1, "10014111a1b540");
    let back = apply(&mut rig, &mut registry);
    assert!(back.contains(&Change::Presence(key.clone())));
    assert_eq!(
        registry.get(&key).unwrap().info.connection,
        Connection::Receiver
    );

    // The receiver goes: nothing reaches the keyboard now.
    for id in [1, 2, 3] {
        rig.remove(id);
    }
    let gone = apply(&mut rig, &mut registry);
    assert!(gone.contains(&Change::Unavailable(key.clone())), "{gone:?}");
    assert!(!registry.get(&key).unwrap().online());
}

#[test]
fn the_cable_unplugged_while_the_receiver_link_is_up_keeps_the_device_online() {
    let mut rig = start_receiver(G915X_TRACE, 0xC547, |_| {});
    let mut cable = Fake::from_trace(G915X_CABLE, 11, 12);
    cable.alias = Some(1);
    rig.add_fake(cable, &[11, 12]);
    for ep in usb_endpoints(0xC356, 11) {
        rig.arrive(ep);
    }
    rig.take_events();
    // Both paths are online; losing the cable changes nothing the registry should hear except the connection.
    rig.remove(11);
    rig.remove(12);
    let ev = rig.take_events();
    assert!(
        !ev.iter().any(|e| matches!(
            e,
            ProviderEvent::Presence(..) | ProviderEvent::Disappeared(_)
        )),
        "{ev:?}"
    );
}

#[test]
fn notifications_are_enabled_only_when_missing() {
    // Disabled: the one register write, keeping the other flags.
    let rig = start_receiver(G703_TRACE, 0xC539, |f| f.reg0 = [0x10, 0x01, 0x01]);
    assert_eq!(rig.sent_hex()[0], short("10ff8100"));
    assert_eq!(rig.sent_hex()[1], short("10ff8000100901"));
    assert_eq!(rig.fakes[0].reg0_writes, vec![[0x10, 0x09, 0x01]]);
    assert_eq!(rig.sent_hex()[2], short("10ff83b520"));
    rig.assert_protocol_only(&[(3, 0), (3, 1), (6, 0)], &[3]);

    // All zero.
    let rig = start_receiver(G703_TRACE, 0xC539, |f| f.reg0 = [0, 0, 0]);
    assert_eq!(rig.sent_hex()[1], short("10ff8000000900"));

    // Already enabled (G HUB did it): no write at all.
    let rig = start_receiver(G703_TRACE, 0xC539, |f| f.reg0 = [0x10, 0x0F, 0x01]);
    assert!(rig.fakes[0].reg0_writes.is_empty());
    assert!(rig.sent.iter().all(|s| s.frame[2] != 0x80));
    // The device is read all the same.
    assert!(
        rig.events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Battery(..)))
    );
}

#[test]
fn no_serial_register_and_no_device_info_is_ever_requested() {
    for (trace, pid, features) in [
        (G703_TRACE, 0xC539u16, vec![(3u8, 0u8), (3, 1), (6, 0)]),
        (G915X_TRACE, 0xC547, vec![(3, 0), (3, 1), (8, 1)]),
    ] {
        let rig = start_receiver(trace, pid, |_| {});
        for s in &rig.sent {
            let hex = juicebar_core::hidpp::hex(&s.frame);
            // Pairing sub-registers 0x30.. are serial numbers.
            assert!(!hex.starts_with("10ff83b53"), "{hex}");
        }
        rig.assert_protocol_only(&features, &[3]);
    }
}

#[test]
fn foreign_traffic_is_ignored() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    rig.take_events();
    let sent = rig.sent.len();
    // G HUB's answers (software IDs 0x0E and 0x0F) and a DJ report change nothing.
    rig.report(2, &long("1101060e0f7e"));
    rig.report(2, &long("1101060f0f7e"));
    rig.report(2, &long("2001060e0f7e"));
    rig.report(1, "1001ff0000");
    assert!(rig.take_events().is_empty());
    assert_eq!(rig.sent.len(), sent);
    // A report on a collection that was never announced.
    rig.report(99, &long("1101060e0f7e"));
    assert!(rig.take_events().is_empty());
}

#[test]
fn unknown_and_foreign_endpoints_are_ignored() {
    let mut rig = Rig::new();
    // Another vendor, a Bluetooth device, a non-HID++ page, an unknown product id of the right page.
    let mut other_vendor = endpoint(1, 0xC539, 0xFF00, 0x0001, 7);
    other_vendor.vid = 0x1234;
    let mut ble = endpoint(2, 0xC539, 0xFF00, 0x0001, 7);
    ble.bluetooth = true;
    for ep in [
        other_vendor,
        ble,
        endpoint(3, 0xC539, 0xFF13, 0x0001, 62),
        endpoint(4, 0xC52B, 0xFF00, 0x0001, 7),
        endpoint(5, 0xC52B, 0xFF00, 0x0002, 20),
    ] {
        rig.arrive(ep);
    }
    assert!(rig.sent.is_empty() && rig.events.is_empty() && rig.wakes.is_empty());
}

#[test]
fn removing_a_receiver_says_goodbye_and_stops_sending() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    rig.take_events();
    let key = logitech_key(Match::Wpid, 0x4086);
    rig.remove(2);
    let ev = rig.take_events();
    assert_eq!(ev, vec![ProviderEvent::Disappeared(key)]);
    let sent = rig.sent.len();
    rig.advance(10_000.0);
    assert_eq!(rig.sent.len(), sent);
    assert!(rig.wakes.is_empty());
    // The short collection alone cannot talk; when the long one comes back the receiver is read again.
    rig.arrive(endpoint(2, 0xC539, 0xFF00, 0x0002, 20));
    assert!(rig.sent.len() > sent);
}
