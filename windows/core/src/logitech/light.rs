// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The RGB economy's driver: the explicit calls the app makes, and the chains of lighting requests they start
//! (spec/PROTOCOL.md, "Lighting"). Nothing here runs as a side effect of reading a battery: a chain starts only
//! when the app orders the economy on or off, when a device the economy is wanted on (re)connects, or when a
//! read-back falls due.
//!
//! Two chains, both one request at a time through the group's queue:
//! - apply: root getFeature, getInfo, getBrightness (the level to save), setBrightness(minimum), getBrightness
//!   (is it at the minimum?); then read-backs 3 s and 30 s after the write;
//! - restore: getBrightness (still at the minimum we set?), setBrightness(saved), getBrightness (is it back?).
//!
//! A read-back that finds the level put back is reported ([`LightingEvent::Reapplied`]) and never fought: no write
//! follows, so no loop with G HUB is possible.

use super::*;
use crate::lighting::{Chain, Failure, Purpose, SETTLE_SECONDS, VERIFY_AFTER, path_for};

impl Logitech {
    /// The levels the app kept from an earlier run (events `Saved` / `Cleared`). Call before the first event.
    pub fn set_saved_lights(&mut self, saved: Vec<(DeviceKey, SavedLight)>) {
        for (key, light) in saved {
            self.saved.insert(key, light);
        }
    }

    /// What the lighting driver has to say since the last call.
    pub fn take_lighting_events(&mut self) -> Vec<LightingEvent> {
        std::mem::take(&mut self.light_events)
    }

    /// Every device seen that has a shipped lighting path, online or not.
    pub fn lighting_keys(&self) -> Vec<DeviceKey> {
        let mut keys: Vec<DeviceKey> = self
            .groups
            .iter()
            .flat_map(|g| g.devs.iter())
            .filter(|d| path_for(d.row).is_some())
            .map(|d| d.key.clone())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Turn one device's lighting down: save its level, write the minimum, read it back. A device that is offline
    /// gets it when it comes back.
    pub fn economy_on(&mut self, key: &DeviceKey, now: Now) -> Vec<Action> {
        self.wanted.insert(key.clone());
        self.light_order(key, now)
    }

    /// Put one device's lighting back to the level that was saved, unless somebody changed it meanwhile.
    pub fn economy_off(&mut self, key: &DeviceKey, now: Now) -> Vec<Action> {
        self.wanted.remove(key);
        self.light_order(key, now)
    }

    /// The app's switch: the economy on (or off) for every device with lighting, those seen and those that appear
    /// later.
    pub fn set_economy(&mut self, on: bool, now: Now) -> Vec<Action> {
        self.all_wanted = on;
        let keys = self.lighting_keys();
        let mut out = Vec::new();
        for key in keys {
            if on {
                self.wanted.insert(key.clone());
            } else {
                self.wanted.remove(&key);
            }
            out.extend(self.light_order(&key, now));
        }
        out
    }

    /// The app is quitting: write the saved level back to every device the economy turned down, one request each
    /// and no read-back (there is no one left to read). The saved level stays kept, so a write that did not arrive
    /// is retried at the next start.
    pub fn release_lights(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        for g in &mut self.groups {
            for d in &mut g.devs {
                if !d.online || !d.light.applied {
                    continue;
                }
                let Some(saved) = self.saved.get(&d.key) else {
                    continue;
                };
                let req = d
                    .light
                    .request(d.index, LightStep::Restore(saved.brightness));
                let Some(req) =
                    req.filter(|r| lighting::is_allowed(r, d.light.feature_index, d.light.max))
                else {
                    continue;
                };
                if let Some(endpoint) = g.long {
                    out.push(Action::Send(endpoint, req.encode()));
                }
                d.light.connection_lost();
            }
        }
        out
    }

    // ------------------------------------------------------------ chains

    /// An explicit order for one key: start what is missing now, at every path to the device that is online.
    fn light_order(&mut self, key: &DeviceKey, now: Now) -> Vec<Action> {
        let mut out = Vec::new();
        for gi in 0..self.groups.len() {
            for di in 0..self.groups[gi].devs.len() {
                if &self.groups[gi].devs[di].key == key {
                    self.groups[gi].devs[di].light.start_at = None;
                    self.light_reconcile(gi, di);
                }
            }
            self.pump(gi, now, &mut out);
        }
        self.finish(&mut out);
        out
    }

    /// Start the chain a device needs, if it needs one and none runs.
    fn light_reconcile(&mut self, gi: usize, di: usize) {
        let d = &self.groups[gi].devs[di];
        if !d.online || d.light.chain.is_some() || path_for(d.row).is_none() {
            return;
        }
        let wanted = self.wanted.contains(&d.key);
        let chain = if wanted && !d.light.applied {
            Chain::Apply
        } else if !wanted && self.saved.contains_key(&d.key) {
            Chain::Restore
        } else {
            return;
        };
        let g = &mut self.groups[gi];
        let index = g.devs[di].index;
        g.devs[di].light.chain = Some(chain);
        g.devs[di].light.start_at = None;
        let step = first_step(&g.devs[di].light, chain);
        g.queue.push_back(Job {
            dev: index,
            step: Step::Light(step),
        });
    }

    /// A device just came online: the economy (or a pending restore) waits for the connection to settle.
    pub(super) fn light_on_online(&mut self, gi: usize, di: usize, now: Now) {
        let d = &self.groups[gi].devs[di];
        if path_for(d.row).is_none() {
            return;
        }
        let key = d.key.clone();
        if self.all_wanted {
            self.wanted.insert(key.clone());
        }
        if self.wanted.contains(&key) || self.saved.contains_key(&key) {
            self.groups[gi].devs[di].light.start_at = Some(now.mono + SETTLE_SECONDS);
        }
    }

    /// The timers of one device's lighting: a settled connection, and the read-backs.
    pub(super) fn light_timers(&mut self, gi: usize, di: usize, now: Now) {
        if !self.groups[gi].devs[di].online {
            return;
        }
        if self.groups[gi].devs[di]
            .light
            .start_at
            .is_some_and(|t| now.mono >= t)
        {
            self.light_reconcile(gi, di);
        }
        let d = &mut self.groups[gi].devs[di];
        let due = d.light.verify_at.iter().filter(|t| now.mono >= **t).count();
        if due == 0 {
            return;
        }
        d.light.verify_at.drain(..due);
        let key = d.key.clone();
        let idle = d.light.chain.is_none() && d.light.applied && d.light.feature_index.is_some();
        if idle && self.wanted.contains(&key) {
            let g = &mut self.groups[gi];
            let index = g.devs[di].index;
            g.queue.push_back(Job {
                dev: index,
                step: Step::Light(LightStep::Get(Purpose::Verify)),
            });
        }
    }

    /// The last gate for a lighting request: the lighting allow-list, with what the device has taught so far.
    pub(super) fn light_allowed(&self, gi: usize, dev: u8, req: &Request) -> bool {
        let g = &self.groups[gi];
        g.dev_pos(dev).is_some_and(|di| {
            let l = &g.devs[di].light;
            lighting::is_allowed(req, l.feature_index, l.max)
        })
    }

    fn light_fail(&mut self, key: DeviceKey, failure: Failure) {
        self.light_events.push(LightingEvent::Failed(key, failure));
    }

    /// An answer (or its absence) to a lighting request.
    pub(super) fn complete_light(
        &mut self,
        gi: usize,
        di: usize,
        step: LightStep,
        result: JobResult,
        now: Now,
        out: &mut Vec<Action>,
    ) {
        let key = self.groups[gi].devs[di].key.clone();
        let payload = match result {
            JobResult::Answer(p) => p,
            JobResult::Timeout => {
                self.light_fail(key, Failure::NoAnswer);
                self.dev_offline(gi, di, Cause::Timeout, now, out);
                return;
            }
            JobResult::Error => {
                let d = &mut self.groups[gi].devs[di];
                d.warm = true;
                d.light.chain = None;
                let failure = if step == LightStep::Feature {
                    Failure::NotSupported
                } else {
                    Failure::Refused
                };
                self.light_fail(key, failure);
                return;
            }
        };
        self.groups[gi].devs[di].warm = true;
        match step {
            LightStep::Feature => {
                let found = hidpp::decode_get_feature(&payload)
                    .filter(|f| f.0 != 0)
                    .map(|f| f.0);
                match found {
                    Some(index) => {
                        self.groups[gi].devs[di].light.feature_index = Some(index);
                        self.light_next(gi, di);
                    }
                    None => self.light_stop(gi, di, Failure::NotSupported),
                }
            }
            LightStep::Info => match lighting::decode_info(&payload) {
                Some((min, max)) => {
                    let l = &mut self.groups[gi].devs[di].light;
                    l.min = Some(min);
                    l.max = Some(max);
                    self.light_next(gi, di);
                }
                None => self.light_stop(gi, di, Failure::Refused),
            },
            LightStep::Get(purpose) => match lighting::decode_brightness(&payload) {
                Some(level) => self.light_read(gi, di, purpose, level, now),
                None => self.light_stop(gi, di, Failure::Refused),
            },
            LightStep::Off => {
                // The write was acknowledged: the read-backs count from here.
                let l = &mut self.groups[gi].devs[di].light;
                l.applied = true;
                l.reported_off = None;
                l.verify_at = VERIFY_AFTER.iter().map(|s| now.mono + s).collect();
                self.push_front(
                    gi,
                    Job {
                        dev: self.groups[gi].devs[di].index,
                        step: Step::Light(LightStep::Get(Purpose::Confirm)),
                    },
                );
            }
            LightStep::Restore(_) => {
                self.push_front(
                    gi,
                    Job {
                        dev: self.groups[gi].devs[di].index,
                        step: Step::Light(LightStep::Get(Purpose::ConfirmRestore)),
                    },
                );
            }
        }
    }

    /// The chain's next step after the feature index or the range arrived.
    fn light_next(&mut self, gi: usize, di: usize) {
        let l = &self.groups[gi].devs[di].light;
        let Some(chain) = l.chain else {
            return;
        };
        let step = first_step(l, chain);
        let dev = self.groups[gi].devs[di].index;
        self.push_front(
            gi,
            Job {
                dev,
                step: Step::Light(step),
            },
        );
    }

    /// End a chain on a failure that is not a timeout.
    fn light_stop(&mut self, gi: usize, di: usize, failure: Failure) {
        let d = &mut self.groups[gi].devs[di];
        d.light.chain = None;
        let key = d.key.clone();
        self.light_fail(key, failure);
    }

    /// A level read back from the device.
    fn light_read(&mut self, gi: usize, di: usize, purpose: Purpose, level: u16, now: Now) {
        let key = self.groups[gi].devs[di].key.clone();
        let dev = self.groups[gi].devs[di].index;
        let min = self.groups[gi].devs[di].light.min.unwrap_or(0);
        let wanted = self.wanted.contains(&key);
        match purpose {
            Purpose::Save => {
                if !wanted {
                    self.groups[gi].devs[di].light.chain = None;
                    self.light_reconcile(gi, di);
                    return;
                }
                // A level above the minimum is the one to give back: the first time, or after it was changed since
                // the economy was applied (or the app died before it could restore).
                if level > min {
                    let saved = SavedLight { brightness: level };
                    if self.saved.get(&key) != Some(&saved) {
                        self.saved.insert(key.clone(), saved);
                        self.light_events.push(LightingEvent::Saved(key, saved));
                    }
                    self.push_front(
                        gi,
                        Job {
                            dev,
                            step: Step::Light(LightStep::Off),
                        },
                    );
                } else {
                    // Already at the minimum (dark by the user's choice, or by an earlier run that kept its level):
                    // counts as applied, nothing is written, the read-backs still run.
                    let l = &mut self.groups[gi].devs[di].light;
                    l.chain = None;
                    l.applied = true;
                    l.reported_off = Some(true);
                    l.verify_at = VERIFY_AFTER.iter().map(|s| now.mono + s).collect();
                    self.light_events.push(LightingEvent::Off(key));
                }
            }
            Purpose::Confirm => {
                let l = &mut self.groups[gi].devs[di].light;
                l.chain = None;
                l.reported_off = Some(level <= min);
                self.light_events.push(if level <= min {
                    LightingEvent::Off(key)
                } else {
                    LightingEvent::Reapplied(key, level)
                });
                self.light_reconcile(gi, di);
            }
            Purpose::Verify => {
                let l = &mut self.groups[gi].devs[di].light;
                let off = level <= min;
                if l.reported_off != Some(off) {
                    l.reported_off = Some(off);
                    self.light_events.push(if off {
                        LightingEvent::Off(key)
                    } else {
                        LightingEvent::Reapplied(key, level)
                    });
                }
            }
            Purpose::BeforeRestore => {
                let Some(saved) = self.saved.get(&key).copied() else {
                    self.groups[gi].devs[di].light.chain = None;
                    return;
                };
                if wanted {
                    // Switched on again before the restore: leave it dark.
                    self.groups[gi].devs[di].light.chain = None;
                    self.light_reconcile(gi, di);
                } else if level <= min {
                    self.push_front(
                        gi,
                        Job {
                            dev,
                            step: Step::Light(LightStep::Restore(saved.brightness)),
                        },
                    );
                } else {
                    // Somebody (the user, G HUB) set a level since: that level stands.
                    self.saved.remove(&key);
                    let l = &mut self.groups[gi].devs[di].light;
                    l.chain = None;
                    l.applied = false;
                    l.reported_off = None;
                    l.verify_at.clear();
                    self.light_events.push(LightingEvent::Cleared(key));
                }
            }
            Purpose::ConfirmRestore => {
                let saved = self.saved.remove(&key);
                let l = &mut self.groups[gi].devs[di].light;
                l.chain = None;
                l.applied = false;
                l.reported_off = None;
                l.verify_at.clear();
                if saved.is_some_and(|s| s.brightness == level) {
                    self.light_events.push(LightingEvent::Restored(key.clone()));
                } else {
                    self.light_events
                        .push(LightingEvent::Failed(key.clone(), Failure::Mismatch));
                }
                self.light_events.push(LightingEvent::Cleared(key));
            }
        }
    }
}

/// The first step of a chain that still has something to learn.
fn first_step(l: &Lighting, chain: Chain) -> LightStep {
    if l.feature_index.is_none() {
        LightStep::Feature
    } else if l.min.is_none() {
        LightStep::Info
    } else {
        match chain {
            Chain::Apply => LightStep::Get(Purpose::Save),
            Chain::Restore => LightStep::Get(Purpose::BeforeRestore),
        }
    }
}
