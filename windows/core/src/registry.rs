// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The registry: the devices the app knows, which of them the user registered, who is on and the last battery
//! reading of each (spec/PROVIDERS.md). Providers push events in; the registry answers with what changed.
//!
//! Identity: a key is the provider id plus the fields that keep a device apart; this module builds them. No
//! serial number or address is ever an input, so a key never needs hashing here.

use crate::hidpp::catalog::{self, Match};
use crate::types::{BatteryReading, DeviceInfo, DeviceKey, Presence, ProviderEvent};

/// A key from a provider id and its identity fields: `provider:field:field`.
pub fn device_key(provider: &str, fields: &[&str]) -> DeviceKey {
    let mut key = String::from(provider);
    for f in fields {
        key.push(':');
        key.push_str(f);
    }
    DeviceKey(key)
}

/// The key of a Logitech device from a catalog id. Rows of one model share a key, so a G915 X as wireless product id
/// 40B5 on a receiver and as USB C356 on its cable are one device, and the three G733 dongles are one device.
/// An id the catalog does not know keys on itself.
pub fn logitech_key(matcher: Match, id: u16) -> DeviceKey {
    let canonical = catalog::find(matcher, id).map_or(id, catalog::canonical_id);
    device_key("logitech-hidpp", &[&format!("{canonical:04X}")])
}

/// One device the registry knows.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub info: DeviceInfo,
    /// On the user's list. A device seen for the first time is registered unless the user removed it before.
    pub registered: bool,
    /// False after the provider reported `Disappeared` (its receiver, dongle or cable is gone).
    pub available: bool,
    pub presence: Option<Presence>,
    pub last: Option<BatteryReading>,
}

impl Entry {
    pub fn online(&self) -> bool {
        self.presence.is_some_and(|p| p.online)
    }
}

/// What an applied event or a user action changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A device seen for the first time.
    Added(DeviceKey),
    /// Its name, kind or connection changed (a cable merged into a receiver's device shows here).
    Updated(DeviceKey),
    Presence(DeviceKey),
    Battery(DeviceKey),
    /// The user registered or removed it.
    Registration(DeviceKey),
    /// The provider lost every path to it.
    Unavailable(DeviceKey),
}

/// The user's list as the shell stores it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserList {
    pub registered: Vec<DeviceKey>,
    /// Devices the user removed: they do not register themselves again.
    pub removed: Vec<DeviceKey>,
}

#[derive(Debug, Default)]
pub struct Registry {
    entries: Vec<Entry>,
    removed: Vec<DeviceKey>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry that remembers the user's list from the last run.
    pub fn with_user_list(list: &UserList) -> Self {
        Registry {
            entries: Vec::new(),
            removed: list.removed.clone(),
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, key: &DeviceKey) -> Option<&Entry> {
        self.entries.iter().find(|e| &e.info.key == key)
    }

    /// The entries on the user's list, in the order they were first seen.
    pub fn registered(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.registered)
    }

    /// The list to store: registered keys of known devices and the keys the user removed.
    pub fn user_list(&self) -> UserList {
        UserList {
            registered: self
                .entries
                .iter()
                .filter(|e| e.registered)
                .map(|e| e.info.key.clone())
                .collect(),
            removed: self.removed.clone(),
        }
    }

    /// Register or remove a device by the user's choice. Removing keeps it from registering itself again.
    pub fn set_registered(&mut self, key: &DeviceKey, registered: bool) -> Vec<Change> {
        let Some(entry) = self.entries.iter_mut().find(|e| &e.info.key == key) else {
            return Vec::new();
        };
        if registered {
            self.removed.retain(|k| k != key);
        } else if !self.removed.contains(key) {
            self.removed.push(key.clone());
        }
        if entry.registered == registered {
            return Vec::new();
        }
        entry.registered = registered;
        vec![Change::Registration(key.clone())]
    }

    /// Apply one provider event; the result lists what changed (nothing when the event repeats what is known).
    /// Presence and battery events for a key never announced are dropped: providers announce first.
    pub fn apply(&mut self, event: &ProviderEvent) -> Vec<Change> {
        match event {
            ProviderEvent::Appeared(info) => self.appeared(info),
            ProviderEvent::Disappeared(key) => {
                let Some(entry) = self.entries.iter_mut().find(|e| &e.info.key == key) else {
                    return Vec::new();
                };
                let mut changes = Vec::new();
                if entry.available {
                    entry.available = false;
                    changes.push(Change::Unavailable(key.clone()));
                }
                if entry.online() {
                    entry.presence = entry.presence.map(|p| Presence {
                        online: false,
                        at: p.at,
                    });
                    changes.push(Change::Presence(key.clone()));
                }
                changes
            }
            ProviderEvent::Presence(key, presence) => {
                let Some(entry) = self.entries.iter_mut().find(|e| &e.info.key == key) else {
                    return Vec::new();
                };
                let was = entry.presence;
                entry.presence = Some(*presence);
                entry.available = true;
                if was.map(|p| p.online) == Some(presence.online) {
                    Vec::new()
                } else {
                    vec![Change::Presence(key.clone())]
                }
            }
            ProviderEvent::Battery(key, reading) => {
                let Some(entry) = self.entries.iter_mut().find(|e| &e.info.key == key) else {
                    return Vec::new();
                };
                if entry.last.as_ref() == Some(reading) {
                    return Vec::new();
                }
                entry.last = Some(reading.clone());
                vec![Change::Battery(key.clone())]
            }
        }
    }

    fn appeared(&mut self, info: &DeviceInfo) -> Vec<Change> {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.info.key == info.key) {
            let mut changes = Vec::new();
            if entry.info != *info {
                entry.info = info.clone();
                changes.push(Change::Updated(info.key.clone()));
            }
            entry.available = true;
            return changes;
        }
        let registered = !self.removed.contains(&info.key);
        self.entries.push(Entry {
            info: info.clone(),
            registered,
            available: true,
            presence: None,
            last: None,
        });
        vec![Change::Added(info.key.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Charging, Connection, Kind, Quality};

    fn info(key: &DeviceKey, connection: Connection) -> DeviceInfo {
        DeviceInfo {
            key: key.clone(),
            provider: "logitech-hidpp",
            name: "G915 X LS".into(),
            kind: Kind::Keyboard,
            connection,
            brand: "Logitech".into(),
        }
    }

    fn reading(at: f64, percent: f64) -> BatteryReading {
        BatteryReading {
            at,
            percent: Some(percent),
            coarse: None,
            millivolts: None,
            charging: Charging::Discharging,
            quality: Quality::Exact,
        }
    }

    #[test]
    fn keys_merge_by_catalog_model() {
        let wpid = logitech_key(Match::Wpid, 0x40B5);
        let cable = logitech_key(Match::Usb, 0xC356);
        assert_eq!(wpid, cable);
        assert_eq!(wpid.0, "logitech-hidpp:40B5");
        assert_ne!(wpid, logitech_key(Match::Wpid, 0x4086));
        let g733 = logitech_key(Match::Dongle, 0x0AFE);
        assert_eq!(g733, logitech_key(Match::Dongle, 0x0B1F));
        assert_eq!(g733.0, "logitech-hidpp:0AB5");
        // An id the catalog does not know keys on itself.
        assert_eq!(logitech_key(Match::Wpid, 0x1234).0, "logitech-hidpp:1234");
        assert_eq!(
            device_key("os-bluetooth", &["ab", "cd"]).0,
            "os-bluetooth:ab:cd"
        );
    }

    #[test]
    fn a_new_device_registers_itself() {
        let key = logitech_key(Match::Wpid, 0x40B5);
        let mut r = Registry::new();
        let ch = r.apply(&ProviderEvent::Appeared(info(&key, Connection::Receiver)));
        assert_eq!(ch, vec![Change::Added(key.clone())]);
        assert!(r.get(&key).unwrap().registered);
        assert_eq!(r.registered().count(), 1);
        // The same event again changes nothing.
        assert!(
            r.apply(&ProviderEvent::Appeared(info(&key, Connection::Receiver)))
                .is_empty()
        );
    }

    #[test]
    fn a_removed_device_does_not_register_itself_again() {
        let key = logitech_key(Match::Wpid, 0x40B5);
        let mut r = Registry::new();
        r.apply(&ProviderEvent::Appeared(info(&key, Connection::Receiver)));
        assert_eq!(
            r.set_registered(&key, false),
            vec![Change::Registration(key.clone())]
        );
        assert!(r.set_registered(&key, false).is_empty());
        let list = r.user_list();
        assert!(list.registered.is_empty());
        assert_eq!(list.removed, vec![key.clone()]);
        // A new run with the stored list: the device appears unregistered.
        let mut next = Registry::with_user_list(&list);
        next.apply(&ProviderEvent::Appeared(info(&key, Connection::Receiver)));
        assert!(!next.get(&key).unwrap().registered);
        // The user adds it back.
        assert_eq!(
            next.set_registered(&key, true),
            vec![Change::Registration(key.clone())]
        );
        assert!(next.user_list().removed.is_empty());
        assert!(
            next.set_registered(&DeviceKey("nope".into()), true)
                .is_empty()
        );
    }

    #[test]
    fn presence_and_battery_report_only_changes() {
        let key = logitech_key(Match::Wpid, 0x4086);
        let mut r = Registry::new();
        // Before the device is announced nothing is kept.
        assert!(
            r.apply(&ProviderEvent::Presence(
                key.clone(),
                Presence {
                    online: true,
                    at: 1.0
                }
            ))
            .is_empty()
        );
        r.apply(&ProviderEvent::Appeared(info(&key, Connection::Receiver)));
        let on = ProviderEvent::Presence(
            key.clone(),
            Presence {
                online: true,
                at: 2.0,
            },
        );
        assert_eq!(r.apply(&on), vec![Change::Presence(key.clone())]);
        assert!(r.get(&key).unwrap().online());
        // Same state, later time: stored, not a change.
        let on2 = ProviderEvent::Presence(
            key.clone(),
            Presence {
                online: true,
                at: 3.0,
            },
        );
        assert!(r.apply(&on2).is_empty());
        assert_eq!(r.get(&key).unwrap().presence.unwrap().at, 3.0);
        let b = ProviderEvent::Battery(key.clone(), reading(4.0, 77.0));
        assert_eq!(r.apply(&b), vec![Change::Battery(key.clone())]);
        assert!(r.apply(&b).is_empty());
        let b2 = ProviderEvent::Battery(key.clone(), reading(5.0, 77.0));
        assert_eq!(r.apply(&b2), vec![Change::Battery(key.clone())]);
        assert_eq!(r.get(&key).unwrap().last.as_ref().unwrap().at, 5.0);
        let off = ProviderEvent::Presence(
            key.clone(),
            Presence {
                online: false,
                at: 6.0,
            },
        );
        assert_eq!(r.apply(&off), vec![Change::Presence(key.clone())]);
        assert!(!r.get(&key).unwrap().online());
        // The last reading stays while the device is off.
        assert!(r.get(&key).unwrap().last.is_some());
    }

    #[test]
    fn a_cable_continues_the_same_device() {
        let key = logitech_key(Match::Wpid, 0x40B5);
        let cable_key = logitech_key(Match::Usb, 0xC356);
        let mut r = Registry::new();
        r.apply(&ProviderEvent::Appeared(info(&key, Connection::Receiver)));
        r.apply(&ProviderEvent::Battery(key.clone(), reading(1.0, 49.0)));
        r.apply(&ProviderEvent::Presence(
            key.clone(),
            Presence {
                online: false,
                at: 2.0,
            },
        ));
        // The cable's event carries the same key: an update, not a second device.
        let ch = r.apply(&ProviderEvent::Appeared(info(&cable_key, Connection::Usb)));
        assert_eq!(ch, vec![Change::Updated(key.clone())]);
        assert_eq!(r.entries().len(), 1);
        assert_eq!(r.get(&key).unwrap().info.connection, Connection::Usb);
        assert_eq!(
            r.get(&key).unwrap().last.as_ref().unwrap().percent,
            Some(49.0)
        );
    }

    #[test]
    fn disappeared_marks_the_device_off_and_unavailable() {
        let key = logitech_key(Match::Dongle, 0x0AFE);
        let mut r = Registry::new();
        r.apply(&ProviderEvent::Appeared(info(&key, Connection::Dongle)));
        r.apply(&ProviderEvent::Presence(
            key.clone(),
            Presence {
                online: true,
                at: 1.0,
            },
        ));
        let ch = r.apply(&ProviderEvent::Disappeared(key.clone()));
        assert_eq!(
            ch,
            vec![
                Change::Unavailable(key.clone()),
                Change::Presence(key.clone())
            ]
        );
        assert!(!r.get(&key).unwrap().available);
        assert!(!r.get(&key).unwrap().online());
        assert!(r.apply(&ProviderEvent::Disappeared(key.clone())).is_empty());
        // It comes back.
        r.apply(&ProviderEvent::Appeared(info(&key, Connection::Dongle)));
        assert!(r.get(&key).unwrap().available);
        assert!(
            r.apply(&ProviderEvent::Disappeared(DeviceKey("x".into())))
                .is_empty()
        );
    }
}
