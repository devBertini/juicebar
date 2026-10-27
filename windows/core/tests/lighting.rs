// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The RGB economy through the Logitech provider: the requests it sends, the level it saves and restores, what it
//! does when something else puts the lighting back, and what it never does (spec/PROTOCOL.md, "Lighting").
//! The device is a fake with the G915 X's BrightnessControl as measured on 26/10/2026.

mod protocol_rig;

use juicebar_core::hidpp::{self, Request, catalog::Match};
use juicebar_core::lighting::{self, Failure, LIGHT_SWID, LightingEvent, SavedLight};
use juicebar_core::registry::logitech_key;
use juicebar_core::types::DeviceKey;
use protocol_rig::*;

fn g915x_key() -> DeviceKey {
    logitech_key(Match::Wpid, 0x40B5)
}

/// A G915 X behind its receiver, discovered and online, lighting at `brightness`.
fn keyboard(brightness: u16) -> Rig {
    start_receiver(G915X_TRACE, 0xC547, |f| {
        f.light = Some(LightFake::g915x(14, brightness));
    })
}

fn light(rig: &Rig) -> &LightFake {
    rig.fakes[0].light.as_ref().unwrap()
}

fn light_mut(rig: &mut Rig) -> &mut LightFake {
    rig.fakes[0].light.as_mut().unwrap()
}

/// The lighting frames sent so far, as hex.
fn light_frames(rig: &Rig) -> Vec<String> {
    rig.sent
        .iter()
        .filter(|s| s.frame[3] & 0x0F == LIGHT_SWID)
        .map(|s| hidpp::hex(&s.frame))
        .collect()
}

fn events(rig: &mut Rig) -> Vec<LightingEvent> {
    rig.core.take_lighting_events()
}

const FEATURE: &str = "1101000c80400000000000000000000000000000";
const INFO: &str = "11010e0c00000000000000000000000000000000";
const GET: &str = "11010e1c00000000000000000000000000000000";
const OFF: &str = "11010e2c00000000000000000000000000000000";
const BACK_13: &str = "11010e2c000d0000000000000000000000000000";

#[test]
fn nothing_is_written_to_the_lighting_unless_the_app_orders_it() {
    let mut rig = keyboard(13);
    rig.advance(7200.0);
    assert!(light_frames(&rig).is_empty());
    assert!(light(&rig).requests.is_empty());
    assert!(events(&mut rig).is_empty());
}

#[test]
fn economy_on_saves_the_level_turns_the_lighting_off_and_reads_it_back() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(light_frames(&rig), vec![FEATURE, INFO, GET, OFF, GET]);
    assert_eq!(light(&rig).brightness, 0);
    assert_eq!(light(&rig).writes, vec![0]);
    assert_eq!(
        events(&mut rig),
        vec![
            LightingEvent::Saved(key.clone(), SavedLight { brightness: 13 }),
            LightingEvent::Off(key.clone()),
        ]
    );
    // The two read-backs, 3 s and 30 s after the write, find it still off: no new event, no write.
    let before = light_frames(&rig).len();
    rig.advance(rig.mono + 3.5);
    assert_eq!(light_frames(&rig).len(), before + 1);
    assert_eq!(light_frames(&rig).last().unwrap(), GET);
    rig.advance(rig.mono + 30.0);
    assert_eq!(light_frames(&rig).len(), before + 2);
    assert!(events(&mut rig).is_empty());
    assert_eq!(light(&rig).writes, vec![0]);
    // Then it is quiet: no lighting request follows, only the battery's own refresh.
    rig.advance(rig.mono + 3600.0);
    assert_eq!(light_frames(&rig).len(), before + 2);
}

#[test]
fn economy_off_restores_exactly_what_was_saved() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    events(&mut rig);
    rig.call(|c, n| c.economy_off(&key, n));
    let frames = light_frames(&rig);
    assert_eq!(
        &frames[5..],
        &[GET.to_string(), BACK_13.to_string(), GET.to_string()]
    );
    assert_eq!(light(&rig).brightness, 13);
    assert_eq!(light(&rig).writes, vec![0, 13]);
    assert_eq!(
        events(&mut rig),
        vec![
            LightingEvent::Restored(key.clone()),
            LightingEvent::Cleared(key.clone())
        ]
    );
    // Nothing left to read back.
    let before = light_frames(&rig).len();
    rig.advance(rig.mono + 120.0);
    assert_eq!(light_frames(&rig).len(), before);
}

#[test]
fn another_level_survives_a_round_trip_too() {
    for level in [100, 50, 1] {
        let mut rig = keyboard(level);
        let key = g915x_key();
        rig.call(|c, n| c.economy_on(&key, n));
        rig.call(|c, n| c.economy_off(&key, n));
        assert_eq!(light(&rig).brightness, level);
        assert_eq!(light(&rig).writes, vec![0, level]);
    }
}

#[test]
fn g_hub_putting_the_level_back_is_reported_and_never_fought() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    events(&mut rig);
    // G HUB reapplies its lighting a moment later.
    light_mut(&mut rig).brightness = 13;
    rig.advance(rig.mono + 3.5);
    assert_eq!(
        events(&mut rig),
        vec![LightingEvent::Reapplied(key.clone(), 13)]
    );
    // The 30 s read-back sees the same: said once, and still no write.
    rig.advance(rig.mono + 30.0);
    assert!(events(&mut rig).is_empty());
    assert_eq!(
        light(&rig).writes,
        vec![0],
        "the economy never writes twice"
    );
    // And it stays quiet for good.
    let before = light_frames(&rig).len();
    rig.advance(rig.mono + 3600.0);
    assert_eq!(light_frames(&rig).len(), before);
    // Switching off: the level is not ours any more, so nothing is written and the kept level is dropped.
    rig.call(|c, n| c.economy_off(&key, n));
    assert_eq!(events(&mut rig), vec![LightingEvent::Cleared(key)]);
    assert_eq!(light(&rig).writes, vec![0]);
    assert_eq!(light(&rig).brightness, 13);
}

#[test]
fn a_level_set_by_the_user_while_the_economy_is_on_is_not_overwritten_by_the_restore() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    events(&mut rig);
    light_mut(&mut rig).brightness = 70;
    rig.call(|c, n| c.economy_off(&key, n));
    assert_eq!(light(&rig).brightness, 70);
    assert_eq!(light(&rig).writes, vec![0]);
    assert_eq!(events(&mut rig), vec![LightingEvent::Cleared(key)]);
}

#[test]
fn a_keyboard_the_user_already_darkened_is_left_dark_and_nothing_is_saved() {
    let mut rig = keyboard(0);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(light_frames(&rig), vec![FEATURE, INFO, GET]);
    assert_eq!(events(&mut rig), vec![LightingEvent::Off(key.clone())]);
    rig.call(|c, n| c.economy_off(&key, n));
    assert_eq!(light_frames(&rig).len(), 3, "nothing to restore");
    assert!(light(&rig).writes.is_empty());
}

#[test]
fn a_reconnected_keyboard_gets_the_economy_again_after_the_connection_settles() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.set_economy(true, n));
    events(&mut rig);
    assert_eq!(light(&rig).writes, vec![0]);
    // The keyboard goes off and comes back (receiver notification 0x41: link lost, then established) and, waking
    // up, restores its own lighting.
    rig.report(2, "1001410c61b540");
    light_mut(&mut rig).brightness = 13;
    let frames = light_frames(&rig).len();
    rig.report(2, "1001410c21b540");
    // Not at once: the connection settles first.
    assert_eq!(light_frames(&rig).len(), frames);
    rig.advance(rig.mono + 3.5);
    assert_eq!(light(&rig).writes, vec![0, 0]);
    assert_eq!(light(&rig).brightness, 0);
    // The level saved before the first time is still the one to give back.
    assert_eq!(rig.core.take_lighting_events().len(), 1);
    rig.call(|c, n| c.set_economy(false, n));
    assert_eq!(light(&rig).brightness, 13);
    let _ = key;
}

#[test]
fn a_device_that_appears_after_the_switch_is_turned_on_gets_the_economy_when_it_is_online() {
    let mut rig = Rig::new();
    rig.call(|c, n| c.set_economy(true, n));
    let mut fake = Fake::from_trace(G915X_TRACE, 1, 2);
    fake.light = Some(LightFake::g915x(14, 13));
    rig.add_fake(fake, &[1, 2, 3]);
    for ep in receiver_endpoints(0xC547, 1) {
        rig.arrive(ep);
    }
    assert!(light_frames(&rig).is_empty(), "not before it settles");
    rig.advance(rig.mono + 3.5);
    assert_eq!(light(&rig).writes, vec![0]);
}

#[test]
fn a_level_kept_by_an_earlier_run_is_restored_when_the_economy_is_off() {
    // The app died with the lighting down; it starts with the switch off and the level in its settings.
    let mut rig = Rig::new();
    let key = g915x_key();
    rig.core
        .set_saved_lights(vec![(key.clone(), SavedLight { brightness: 13 })]);
    let mut fake = Fake::from_trace(G915X_TRACE, 1, 2);
    fake.light = Some(LightFake::g915x(14, 0));
    rig.add_fake(fake, &[1, 2, 3]);
    for ep in receiver_endpoints(0xC547, 1) {
        rig.arrive(ep);
    }
    rig.advance(rig.mono + 3.5);
    assert_eq!(light(&rig).brightness, 13);
    assert_eq!(
        events(&mut rig),
        vec![
            LightingEvent::Restored(key.clone()),
            LightingEvent::Cleared(key)
        ]
    );
}

#[test]
fn a_level_kept_by_an_earlier_run_survives_the_economy_being_applied_again() {
    let mut rig = Rig::new();
    let key = g915x_key();
    rig.core
        .set_saved_lights(vec![(key.clone(), SavedLight { brightness: 13 })]);
    let mut fake = Fake::from_trace(G915X_TRACE, 1, 2);
    fake.light = Some(LightFake::g915x(14, 0));
    rig.add_fake(fake, &[1, 2, 3]);
    for ep in receiver_endpoints(0xC547, 1) {
        rig.arrive(ep);
    }
    // The switch is on again, the lighting is still dark: its level (0) must not replace the kept 13.
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(events(&mut rig), vec![LightingEvent::Off(key.clone())]);
    assert!(light(&rig).writes.is_empty());
    rig.call(|c, n| c.economy_off(&key, n));
    assert_eq!(light(&rig).brightness, 13);
}

#[test]
fn quitting_writes_the_saved_level_back_without_reading_and_keeps_it_kept() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    events(&mut rig);
    rig.call(|c, _| c.release_lights());
    assert_eq!(light(&rig).brightness, 13);
    assert_eq!(light(&rig).writes, vec![0, 13]);
    // No event: nobody is listening; the kept level stays for a write that did not arrive.
    assert!(events(&mut rig).is_empty());
}

#[test]
fn a_keyboard_without_the_feature_or_refusing_is_reported_and_asked_again_only_on_the_next_order() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    light_mut(&mut rig).no_feature = true;
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(
        events(&mut rig),
        vec![LightingEvent::Failed(key.clone(), Failure::NotSupported)]
    );
    rig.advance(rig.mono + 3600.0);
    assert_eq!(light_frames(&rig), vec![FEATURE], "no loop of retries");
    light_mut(&mut rig).no_feature = false;
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(light(&rig).writes, vec![0]);

    let mut rig = keyboard(13);
    light_mut(&mut rig).refuse = true;
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(
        events(&mut rig),
        vec![LightingEvent::Failed(key.clone(), Failure::NotSupported)]
    );
}

#[test]
fn a_keyboard_that_does_not_answer_is_treated_as_offline() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    light_mut(&mut rig).silent = true;
    rig.call(|c, n| c.economy_on(&key, n));
    rig.advance(rig.mono + 5.0);
    assert_eq!(
        events(&mut rig),
        vec![LightingEvent::Failed(key.clone(), Failure::NoAnswer)]
    );
    assert!(light(&rig).writes.is_empty());
}

#[test]
fn the_g703_and_the_g733_are_not_touched() {
    let mut rig = start_receiver(G703_TRACE, 0xC539, |_| {});
    assert!(rig.core.lighting_keys().is_empty());
    rig.call(|c, n| c.set_economy(true, n));
    rig.advance(rig.mono + 60.0);
    assert!(light_frames(&rig).is_empty());

    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(G733_TRACE, 1, 2);
    fake.alias = Some(1);
    rig.add_fake(fake, &[1]);
    rig.arrive(endpoint(1, 0x0AFE, 0xFF43, 0x0202, 20));
    rig.call(|c, n| c.set_economy(true, n));
    rig.advance(rig.mono + 60.0);
    assert!(rig.core.lighting_keys().is_empty());
    assert!(light_frames(&rig).is_empty());
}

#[test]
fn the_cable_path_works_with_its_own_feature_index() {
    let mut rig = Rig::new();
    let mut fake = Fake::from_trace(G915X_CABLE, 1, 2);
    fake.alias = Some(1);
    fake.light = Some(LightFake::g915x(13, 13));
    rig.add_fake(fake, &[1, 2]);
    for ep in usb_endpoints(0xC356, 1) {
        rig.arrive(ep);
    }
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    assert_eq!(
        light_frames(&rig)[3],
        "11ff0d2c00000000000000000000000000000000",
        "a cabled device is addressed 0xFF and its feature is at 13"
    );
    assert_eq!(light(&rig).brightness, 0);
    rig.call(|c, n| c.economy_off(&key, n));
    assert_eq!(light(&rig).brightness, 13);
}

#[test]
fn every_lighting_frame_passes_only_the_lighting_list_and_every_other_only_the_general_one() {
    let mut rig = keyboard(13);
    let key = g915x_key();
    rig.call(|c, n| c.economy_on(&key, n));
    rig.advance(rig.mono + 40.0);
    rig.call(|c, n| c.economy_off(&key, n));
    let mut lighting_frames = 0;
    for s in &rig.sent {
        let req = Request::parse(&s.frame).expect("a report");
        if req.b3 & 0x0F == LIGHT_SWID {
            lighting_frames += 1;
            assert!(
                !hidpp::is_allowed(&req, &[req.b2]),
                "{}",
                hidpp::hex(&s.frame)
            );
            assert!(
                lighting::is_allowed(&req, Some(14), Some(100)),
                "{}",
                hidpp::hex(&s.frame)
            );
            // Only the three functions of the brightness feature, and the root lookup.
            assert!(req.b2 == 0 || req.b2 == 14);
            assert!(req.b3 >> 4 <= 2);
        } else {
            assert!(
                hidpp::is_allowed(&req, &[req.b2]),
                "{}",
                hidpp::hex(&s.frame)
            );
            assert!(!lighting::is_allowed(&req, Some(14), Some(100)));
        }
    }
    assert!(lighting_frames >= 10, "{lighting_frames}");
}
