// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The provider's clock: request timeouts, the dongle ping, the slow refresh, the grace after a reconnection and the
//! Wake actions that ask the shell for each of them.

mod protocol_rig;

use juicebar_core::hidpp::catalog::Match;
use juicebar_core::registry::logitech_key;
use juicebar_core::types::{Kind, ProviderEvent, Quality};
use protocol_rig::*;

fn count_presence(ev: &[ProviderEvent]) -> Vec<bool> {
    ev.iter()
        .filter_map(|e| match e {
            ProviderEvent::Presence(_, p) => Some(p.online),
            _ => None,
        })
        .collect()
}

fn batteries(ev: &[ProviderEvent]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, ProviderEvent::Battery(..)))
        .count()
}

#[test]
fn the_first_request_to_a_device_waits_four_seconds() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |f| f.mute = vec![1]);
    // The receiver's own registers answered; the ping to the sleeping device did not.
    let ping = rig.sent.last().unwrap();
    assert_eq!(hex_of(&ping.frame), short("1001001d00005a"));
    let ping_at = ping.at;
    rig.take_events();
    let sent = rig.sent.len();
    rig.advance(ping_at + 3.99);
    assert_eq!(rig.sent.len(), sent);
    assert!(
        rig.take_events().is_empty(),
        "not offline before the 4 s are over"
    );
    rig.advance(ping_at + 4.0);
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![false]);
    assert!(matches!(&ev[0], ProviderEvent::Appeared(i) if i.name == "G703 LIGHTSPEED"));
    // A late ping 600 s after, with the same 4 s patience.
    let next = rig.next_wake().unwrap();
    assert!((next - (ping_at + 4.0 + 600.0)).abs() < 0.01, "{next}");
    rig.advance(next);
    assert_eq!(rig.sent.len(), sent + 1);
    let again = rig.sent.last().unwrap();
    assert_eq!(hex_of(&again.frame), short("1001001d00005a"));
    assert!((rig.next_wake().unwrap() - (again.at + 4.0)).abs() < 1e-9);
}

#[test]
fn later_requests_wait_one_second_and_a_timeout_means_offline() {
    // Only the ping is answered; the device then goes quiet.
    let mut rig = start_receiver(G703_TRACE, 0xC539, |f| f.budget = Some(1));
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![true]);
    let request = rig.sent.last().unwrap();
    assert_eq!(hex_of(&request.frame), long("1101000d1004"));
    let at = request.at;
    // The device is awake: patience is 1 s, not 4.
    rig.advance(at + 0.99);
    assert!(rig.take_events().is_empty());
    rig.advance(at + 1.0);
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![false], "{ev:?}");
    assert_eq!(batteries(&ev), 0);
    // The discovery chain is dropped: no further request to the device.
    let sent = rig.sent.len();
    rig.advance(at + 100.0);
    assert_eq!(rig.sent.len(), sent);
}

#[test]
fn a_dongle_whose_headset_is_off_is_pinged_at_most_every_sixty_seconds() {
    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(G733_TRACE, 1, 1);
    fake.mute_all = true;
    rig.add_fake(fake, &[1]);
    rig.arrive(endpoint(1, 0x0AFE, 0xFF43, 0x0202, 20));
    assert_eq!(rig.sent_hex(), vec![long("11ff001d00005a")]);
    assert!((rig.next_wake().unwrap() - 4.0).abs() < 1e-9);
    // The headset is off: after the 4 s the dongle's device is offline.
    rig.advance(4.0);
    let ev = rig.take_events();
    assert!(matches!(&ev[0], ProviderEvent::Appeared(i) if i.kind == Kind::Headset));
    assert_eq!(count_presence(&ev), vec![false]);
    // The next ping is 60 s after it was declared offline.
    rig.advance(63.9);
    assert_eq!(rig.sent.len(), 1);
    rig.advance(64.0);
    assert_eq!(rig.sent.len(), 2);
    // Still off: no event repeats, and the next ping is another 60 s on.
    rig.advance(68.0);
    assert!(rig.take_events().is_empty());
    rig.advance(127.9);
    assert_eq!(rig.sent.len(), 2);
    // The headset is switched on: the next ping finds it and the whole discovery follows.
    rig.fakes[0].mute_all = false;
    rig.advance(128.0);
    assert!(rig.sent.len() > 2);
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![true]);
    assert_eq!(batteries(&ev), 1);
    // Every ping while off was at least a minute from the last.
    let pings: Vec<f64> = rig
        .sent
        .iter()
        .filter(|s| hex_of(&s.frame).starts_with("11ff001d"))
        .map(|s| s.at)
        .collect();
    assert_eq!(pings.len(), 3);
    assert!(pings.windows(2).all(|w| w[1] - w[0] >= 60.0), "{pings:?}");
    rig.assert_protocol_only(&[(3, 0), (3, 1), (8, 0)], &[]);
}

#[test]
fn a_dongle_that_answers_with_an_error_is_offline_at_once() {
    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(G733_TRACE, 1, 1);
    fake.refuse = vec![0xFF];
    rig.add_fake(fake, &[1]);
    rig.arrive(endpoint(1, 0x0AFE, 0xFF43, 0x0202, 20));
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![false]);
    assert!(rig.mono < 0.1, "no wait for a timeout");
    assert_eq!(rig.sent.len(), 1);
    // (The request's own 4 s wake is stale; the shell fires it and the core asks for the real one.)
    let offline_at = rig.mono;
    rig.advance(5.0);
    let next = rig.next_wake().unwrap();
    assert!((next - (offline_at + 60.0)).abs() < 1e-9, "{next}");
    // Switched on at last.
    rig.fakes[0].refuse.clear();
    rig.advance(next);
    assert_eq!(count_presence(&rig.take_events()), vec![true]);
}

#[test]
fn the_slow_refresh_runs_every_600_seconds_for_an_online_device_only() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    let ev = rig.take_events();
    assert_eq!(batteries(&ev), 1);
    let sent = rig.sent.len();
    let read_at = rig.sent.last().unwrap().at;
    rig.advance(read_at + 599.0);
    assert_eq!(rig.sent.len(), sent, "nothing before the 600 s");
    let wake = rig.next_wake().unwrap();
    assert!((wake - (read_at + 600.0)).abs() < 0.01);
    rig.advance(wake);
    assert_eq!(rig.sent.len(), sent + 1);
    assert_eq!(hex_of(&rig.sent.last().unwrap().frame), long("1101060d00"));
    assert_eq!(batteries(&rig.take_events()), 1);
    // And again 600 s after that reading.
    rig.advance(wake + 5.0);
    let again = rig.next_wake().unwrap();
    assert!(
        (again - (wake + 0.002 + 600.0)).abs() < 0.01,
        "{again} after {wake}"
    );
    // The device switches off: the refresh stops.
    rig.report(1, "1001410c628640");
    assert_eq!(count_presence(&rig.take_events()), vec![false]);
    let sent = rig.sent.len();
    rig.advance(wake + 5000.0);
    assert_eq!(rig.sent.len(), sent);
    assert!(rig.wakes.is_empty());
}

#[test]
fn a_battery_read_is_matched_to_its_software_id() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |f| f.mute_all = false);
    rig.take_events();
    let sent = rig.sent.len();
    rig.fakes[0].mute_all = true;
    let read_at = rig.sent.last().unwrap().at;
    rig.advance(read_at + 599.0);
    let wake = rig.next_wake().unwrap();
    rig.advance(wake);
    assert_eq!(
        rig.sent.len(),
        sent + 1,
        "the refresh read is out and unanswered"
    );
    // G HUB's answers carry its own software IDs; another device's answer has another index.
    rig.report(2, &long("1101060e0f7e"));
    rig.report(2, &long("1101060f0f7e"));
    rig.report(2, &long("1102060d0f7e"));
    assert!(rig.take_events().is_empty());
    // Ours is read.
    rig.report(2, &long("1101060d0f80"));
    let ev = rig.take_events();
    assert_eq!(batteries(&ev), 1);
    match &ev[0] {
        ProviderEvent::Battery(_, r) => assert_eq!(r.millivolts, Some(3968)),
        other => panic!("{other:?}"),
    }
    // The request is done: it does not time out later.
    rig.advance(rig.mono + 10.0);
    assert!(count_presence(&rig.take_events()).is_empty());
}

#[test]
fn the_registry_may_ask_for_a_refresh_but_not_sooner_than_the_minimum() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    rig.take_events();
    let key = logitech_key(Match::Wpid, 0x4086);
    let sent = rig.sent.len();
    rig.advance(rig.mono + 30.0);
    let now = rig.now();
    let actions = rig.core.refresh(&key, now);
    assert!(
        actions
            .iter()
            .all(|a| !matches!(a, juicebar_core::types::Action::Send(..)))
    );
    rig.advance(rig.mono + 600.0);
    let after_timer = rig.sent.len();
    assert!(after_timer > sent, "the timer read it");
    // Just read: a refresh now sends nothing.
    let now = rig.now();
    let actions = rig.core.refresh(&key, now);
    assert!(
        actions
            .iter()
            .all(|a| !matches!(a, juicebar_core::types::Action::Send(..)))
    );
    // An unknown key is ignored.
    let now = rig.now();
    let none = rig.core.refresh(&logitech_key(Match::Wpid, 0x1234), now);
    assert!(none.is_empty());
    // After the minimum it reads.
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    rig.take_events();
    let sent = rig.sent.len();
    rig.mono += 601.0;
    let now = rig.now();
    let actions = rig.core.refresh(&key, now);
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, juicebar_core::types::Action::Send(..)))
    );
    assert_eq!(rig.sent.len(), sent);
}

#[test]
fn a_reconnection_is_read_once_unless_the_device_pushes_first() {
    // The G703's own recorded pushes: link lost, link established, and its battery 4 ms later.
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    rig.take_events();
    let sent = rig.sent.len();
    let base = rig.mono;
    for (t, hex) in listen_frames(G703_LISTEN) {
        rig.advance(base + t);
        rig.report(if hex.len() == 14 { 1 } else { 2 }, &hex);
    }
    rig.advance(base + 60.0);
    assert_eq!(
        rig.sent.len(),
        sent,
        "the push spared the read, and no discovery was repeated"
    );
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![false, true]);
    match ev.last().unwrap() {
        ProviderEvent::Battery(_, r) => {
            assert_eq!(r.millivolts, Some(3962));
            assert_eq!(r.quality, Quality::Derived);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_reconnection_without_a_push_costs_one_battery_read_after_two_seconds() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    rig.take_events();
    let sent = rig.sent.len();
    rig.advance(rig.mono + 20.0);
    rig.report(1, "1001410c628640");
    rig.advance(rig.mono + 19.5);
    rig.fakes[0].mute_all = true;
    let established = rig.mono;
    rig.report(1, "1001410ca28640");
    assert_eq!(count_presence(&rig.take_events()), vec![false, true]);
    // The grace runs two seconds before anything is asked.
    rig.advance(established + 1.99);
    assert_eq!(rig.sent.len(), sent);
    rig.advance(established + 2.0);
    assert_eq!(rig.sent.len(), sent + 1, "exactly one request");
    let (read_frame, read_at) = {
        let read = rig.sent.last().unwrap();
        (hex_of(&read.frame), read.at)
    };
    assert_eq!(
        read_frame,
        long("1101060d00"),
        "the cached feature index, no discovery"
    );
    // The device woke just now: the first request waits four seconds.
    assert!((rig.next_wake().unwrap() - (read_at + 4.0)).abs() < 1e-9);
    rig.advance(read_at + 3.99);
    assert!(rig.take_events().is_empty());
    rig.advance(read_at + 4.0);
    assert_eq!(count_presence(&rig.take_events()), vec![false]);
}

#[test]
fn events_before_the_battery_feature_is_known_are_kept() {
    // The receiver is slow; the keyboard announces itself and pushes its battery before discovery runs.
    let mut rig = start_receiver(G915X_TRACE, 0xC547, |f| f.mute_all = true);
    assert_eq!(rig.sent.len(), 1, "the register read is out, unanswered");
    rig.report(1, "10014111a1b540");
    rig.report(2, &long("1101080032040000"));
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![true]);
    assert_eq!(batteries(&ev), 0, "the feature index 8 is not known yet");
    rig.fakes[0].mute_all = false;
    rig.advance(1.0);
    let ev = rig.take_events();
    let readings: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            ProviderEvent::Battery(_, r) => Some(r.percent),
            _ => None,
        })
        .collect();
    assert_eq!(readings, vec![Some(50.0)]);
    // The pushed battery spared the read at the end of the grace.
    let sent = rig.sent.len();
    rig.advance(10.0);
    assert!(
        !rig.sent_hex().contains(&long("1101081d00")),
        "{:?}",
        rig.sent_hex()
    );
    assert_eq!(rig.sent.len(), sent);
}

#[test]
fn a_link_lost_drops_the_requests_waiting_for_that_device() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |f| f.mute = vec![1]);
    rig.take_events();
    let sent = rig.sent.len();
    // The ping is in flight; the receiver says the link is lost.
    rig.report(1, "1001410c628640");
    let ev = rig.take_events();
    assert_eq!(count_presence(&ev), vec![false]);
    rig.advance(100.0);
    assert_eq!(
        rig.sent.len(),
        sent,
        "no ping after a link lost, only the receiver's notification brings it back"
    );
    assert!(rig.wakes.is_empty());
    // A late answer to the dropped ping changes nothing.
    rig.report(2, &long("1101001d04025a"));
    assert!(rig.take_events().is_empty());
}

#[test]
fn a_device_the_catalog_does_not_know_is_still_shown_without_a_forecast() {
    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(G703_TRACE, 1, 2);
    // Slot 1 holds a wireless product id 1234, a mouse.
    fake.script("10ff83b5200000", &long("11ff83b520070812340c0102b3"));
    rig.add_fake(fake, &[1, 2, 3]);
    for ep in receiver_endpoints(0xC539, 1) {
        rig.arrive(ep);
    }
    let ev = rig.take_events();
    match &ev[0] {
        ProviderEvent::Appeared(i) => {
            assert_eq!(i.key.0, "logitech-hidpp:1234");
            assert_eq!(i.kind, Kind::Mouse);
        }
        other => panic!("{other:?}"),
    }
    match ev.last().unwrap() {
        ProviderEvent::Battery(_, r) => {
            assert_eq!(r.percent, None);
            assert_eq!(r.quality, Quality::Coarse);
            assert_eq!(r.millivolts, Some(3966));
        }
        other => panic!("{other:?}"),
    }
}

fn hex_of(frame: &[u8]) -> String {
    juicebar_core::hidpp::hex(frame)
}
