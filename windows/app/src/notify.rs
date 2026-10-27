// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Which notices a new snapshot calls for: the smart or low warning when a device starts to have
//! one, and "battery full" when a device reaches 100 %. Each fires once per episode and re-arms
//! when the condition ends. Pure: the tray shows what this returns.

use std::collections::HashMap;

use juicebar_core::snapshot::{DeviceView, Snapshot};
use juicebar_core::strings::{self, Lang, id};
use juicebar_core::types::DeviceKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Copy, Default)]
struct Seen {
    warned: bool,
    full: bool,
}

#[derive(Default)]
pub struct Tracker {
    started: bool,
    seen: HashMap<DeviceKey, Seen>,
    /// The user turned the smart warning's notice off.
    smart_off: bool,
    /// The user turned the "battery full" notice off.
    full_off: bool,
}

fn is_full(d: &DeviceView, lang: Lang) -> bool {
    d.on && d.status == strings::get(lang, id::STATUS_FULL)
}

impl Tracker {
    /// The user's switches for the two notices this tracker decides (the demo source has no estimators).
    pub fn set_alerts(&mut self, smart: bool, full: bool) {
        self.smart_off = !smart;
        self.full_off = !full;
    }

    /// Compares `snap` with what was seen before. The first snapshot after start only records the
    /// state (no notice for what was already true); a device that shows up later with a warning
    /// is announced, one that shows up full is not.
    pub fn observe(&mut self, snap: &Snapshot, lang: Lang) -> Vec<Notice> {
        let first = !self.started;
        self.started = true;
        let mut out = Vec::new();
        let mut next = HashMap::with_capacity(snap.devices.len());
        for d in &snap.devices {
            let known = self.seen.get(&d.key).copied();
            let prev = known.unwrap_or_default();
            let warned = d.on && d.warning.is_some();
            let full = is_full(d, lang);
            if warned && !prev.warned && !first && !self.smart_off {
                out.push(warning_notice(d, lang));
            }
            if full && !prev.full && known.is_some() && !first && !self.full_off {
                out.push(Notice {
                    title: strings::fmt(lang, id::NOTICE_FULL_TITLE, &[&d.name]),
                    body: strings::get(lang, id::NOTICE_FULL_BODY).to_string(),
                });
            }
            next.insert(d.key.clone(), Seen { warned, full });
        }
        self.seen = next;
        out
    }
}

/// Title "<device> com <n>%", body the warning (or, when the warning is just that line, the status).
fn warning_notice(d: &DeviceView, lang: Lang) -> Notice {
    let percent = d.percent.map_or(String::new(), |p| p.to_string());
    let title = strings::fmt(lang, id::LOW_WARNING, &[&d.name, &percent]);
    let warning = d.warning.clone().unwrap_or_default();
    let body = if warning == title {
        d.status.clone()
    } else {
        warning
    };
    Notice { title, body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Clock, DemoPhase, demo_snapshot};

    const CLOCK: Clock = Clock {
        wall: 1_793_035_800,
        utc_offset: -10800,
    };

    fn snap(p: DemoPhase, lang: Lang) -> Snapshot {
        demo_snapshot(p, &CLOCK, lang)
    }

    #[test]
    fn the_first_snapshot_only_records() {
        let mut t = Tracker::default();
        assert!(
            t.observe(&snap(DemoPhase::Scene, Lang::PtBr), Lang::PtBr)
                .is_empty()
        );
    }

    #[test]
    fn a_new_warning_fires_once_and_rearms_when_it_clears() {
        let mut t = Tracker::default();
        let lang = Lang::PtBr;
        t.observe(&snap(DemoPhase::Normal, lang), lang);
        let n = t.observe(&snap(DemoPhase::Warning, lang), lang);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "G703 com 23%");
        assert_eq!(n[0].body, "Não chega ao fim do dia: carregue antes das 18h");
        assert!(t.observe(&snap(DemoPhase::Warning, lang), lang).is_empty());
        assert!(t.observe(&snap(DemoPhase::Charging, lang), lang).is_empty());
        assert_eq!(t.observe(&snap(DemoPhase::Warning, lang), lang).len(), 1);
    }

    #[test]
    fn full_charge_fires_once_when_the_keyboard_reaches_100() {
        let mut t = Tracker::default();
        let lang = Lang::En;
        t.observe(&snap(DemoPhase::Charging, lang), lang);
        let n = t.observe(&snap(DemoPhase::Full, lang), lang);
        assert_eq!(
            n,
            vec![Notice {
                title: "G915 X: battery full".into(),
                body: "you can unplug it".into()
            }]
        );
        assert!(t.observe(&snap(DemoPhase::Full, lang), lang).is_empty());
        t.observe(&snap(DemoPhase::Normal, lang), lang);
        assert_eq!(t.observe(&snap(DemoPhase::Full, lang), lang).len(), 1);
    }

    #[test]
    fn the_switches_silence_their_notice_and_the_notice_comes_back_when_the_switch_does() {
        let lang = Lang::PtBr;
        let mut t = Tracker::default();
        t.set_alerts(false, false);
        t.observe(&snap(DemoPhase::Normal, lang), lang);
        assert!(t.observe(&snap(DemoPhase::Warning, lang), lang).is_empty());
        t.observe(&snap(DemoPhase::Charging, lang), lang);
        assert!(t.observe(&snap(DemoPhase::Full, lang), lang).is_empty());
        t.observe(&snap(DemoPhase::Normal, lang), lang);
        t.set_alerts(true, true);
        assert_eq!(t.observe(&snap(DemoPhase::Warning, lang), lang).len(), 1);
        t.observe(&snap(DemoPhase::Charging, lang), lang);
        assert_eq!(t.observe(&snap(DemoPhase::Full, lang), lang).len(), 1);
    }

    #[test]
    fn a_device_that_appears_full_is_not_announced_but_one_that_appears_warned_is() {
        let mut t = Tracker::default();
        let lang = Lang::PtBr;
        let mut s = snap(DemoPhase::Normal, lang);
        let mut extra = s.devices.pop().unwrap();
        t.observe(&s, lang);
        s.devices.push(extra.clone());
        assert!(t.observe(&s, lang).is_empty());
        t.observe(&snap(DemoPhase::Normal, lang), lang);
        extra.on = true;
        extra.key = DeviceKey("new".into());
        extra.status = strings::get(lang, id::STATUS_FULL).to_string();
        let mut s2 = snap(DemoPhase::Normal, lang);
        s2.devices.push(extra.clone());
        assert!(
            t.observe(&s2, lang).is_empty(),
            "a new full device is silent"
        );
        extra.key = DeviceKey("new2".into());
        extra.status = String::new();
        extra.warning = Some("G733 com 7%".into());
        extra.percent = Some(7);
        let mut s3 = s2.clone();
        s3.devices.push(extra);
        let n = t.observe(&s3, lang);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "G733 com 7%");
    }

    #[test]
    fn a_low_battery_warning_that_equals_the_title_uses_the_status_as_body() {
        let lang = Lang::En;
        let mut s = snap(DemoPhase::Normal, lang);
        let d = &mut s.devices[0];
        d.percent = Some(7);
        d.warning = Some(format!("{} at 7%", d.name));
        d.status = "~40 min of use".into();
        let n = warning_notice(&s.devices[0], lang);
        assert_eq!(n.title, format!("{} at 7%", s.devices[0].name));
        assert_eq!(n.body, "~40 min of use");
    }
}
