// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Lighting: the requests, the answers and the allow-list of the RGB economy (spec/PROTOCOL.md, "Lighting").
//!
//! The economy is the one thing the Logitech provider writes to a device besides the receiver's notification
//! flags, and only because the owner turned it on in the app. Everything that can reach a wire for it is here:
//! the request builders are private to this module, the allow-list for them ([`is_allowed`]) is separate from
//! `hidpp::is_allowed` (which refuses every software ID but 0x0D, so a lighting frame can never pass as a read), and
//! the driver in `logitech::light` asks only through [`Lighting::request`].
//!
//! Pure code: no device, no clock.
//!
//! Which devices: only the ones whose lighting was measured, with a way to read the level back, to save it and to
//! restore exactly what was saved (`docs/research/probe-windows.md`, "Lighting"). That is the G915 X through the
//! brightness feature 0x8040. The G703 (0x8071, a write-only effect) and the G733 (0x8070, every read refused) are
//! known and not shipped: see [`Path`].

use crate::hidpp::catalog::{self, Match, Row};
use crate::hidpp::{DIRECT, LONG_ID, Request};
use crate::types::DeviceKey;

/// Software ID of every lighting request. Not 0x0D (the read-only requests of the provider, which the owner's other
/// JuiceBar instances or the tests share) and not 0x0E or 0x0F (G HUB): an answer to a lighting request can only be
/// ours.
pub const LIGHT_SWID: u8 = 0x0C;

/// BrightnessControl: Logitech's global brightness feature.
pub const F_BRIGHTNESS: u16 = 0x8040;

const FN_INFO: u8 = 0;
const FN_GET: u8 = 1;
const FN_SET: u8 = 2;

/// After a device reconnects the economy waits this long before it writes: a device that wakes, or G HUB reading it
/// again, may restore its own lighting first.
pub const SETTLE_SECONDS: f64 = 3.0;
/// The read-backs after the economy was applied: the lighting is read again this long after the write was
/// acknowledged, to see whether G HUB (or the device) put it back.
pub const VERIFY_AFTER: [f64; 2] = [3.0, 30.0];

/// How a device's lighting is turned down and back up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    /// 0x8040: read the global brightness (function 1), write it (function 2); the economy writes the minimum the
    /// device reports (0 on the G915 X) and restores the value it read.
    Brightness,
}

/// The lighting path of a catalog row, `None` when the model has none that is shipped.
///
/// Not shipped, measured on the owner's devices on 26/10/2026:
/// - G703 (wpid 4086): lighting is feature 0x8071 with a write-only effect (capabilities bit 0 clear): the effect
///   G HUB set cannot be read, so it can be neither saved nor restored exactly, nor read back.
/// - G733 (dongles 0AB5, 0AFE, 0B1F): lighting is feature 0x8070 version 3; getInfo, getZoneInfo, getEffectInfo and
///   getZoneEffect all answered the error NotAllowed (5): no state to save, no read-back.
pub fn path_for(row: Option<&Row>) -> Option<Path> {
    let row = row?;
    let usable = matches!(row.matcher, Match::Wpid | Match::Usb);
    (usable && catalog::canonical_id(row) == G915X_ID).then_some(Path::Brightness)
}

/// The canonical id of the G915 X (its wireless product id; the cable's C356 is the same device).
const G915X_ID: u16 = 0x40B5;

/// The level a device had before the economy: what `economy_off` writes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedLight {
    pub brightness: u16,
}

impl SavedLight {
    /// The text kept in the app's settings: `brightness:13`.
    pub fn to_text(&self) -> String {
        format!("brightness:{}", self.brightness)
    }

    pub fn from_text(text: &str) -> Option<SavedLight> {
        let value = text.trim().strip_prefix("brightness:")?;
        Some(SavedLight {
            brightness: value.parse().ok()?,
        })
    }
}

/// Why a lighting step did not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The device has no lighting feature of the shipped kind.
    NotSupported,
    /// The device answered a HID++ error.
    Refused,
    /// The device did not answer in time (it is treated as offline).
    NoAnswer,
    /// The level read back after a restore is not the one written.
    Mismatch,
}

/// What the lighting driver tells the app. Taken with `Logitech::take_lighting_events`.
#[derive(Clone, Debug, PartialEq)]
pub enum LightingEvent {
    /// The level to keep so `economy_off` can restore it, also after the app was closed.
    Saved(DeviceKey, SavedLight),
    /// The kept level is spent: forget it.
    Cleared(DeviceKey),
    /// Read back: the lighting is at its minimum.
    Off(DeviceKey),
    /// Read back: the saved level is back.
    Restored(DeviceKey),
    /// Read back after the economy: the lighting is at this level again (G HUB or the device put it back).
    /// Nothing is written in answer: JuiceBar does not fight another program.
    Reapplied(DeviceKey, u16),
    Failed(DeviceKey, Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Apply chain: the level before the economy.
    Save,
    /// Right after the economy write: is it at the minimum?
    Confirm,
    /// A timed read-back.
    Verify,
    /// Restore chain: is it still at the minimum we set?
    BeforeRestore,
    /// Right after the restore write.
    ConfirmRestore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightStep {
    /// Root getFeature(0x8040).
    Feature,
    /// BrightnessControl getInfo: the range.
    Info,
    /// getBrightness.
    Get(Purpose),
    /// setBrightness(minimum).
    Off,
    /// setBrightness(saved).
    Restore(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chain {
    Apply,
    Restore,
}

/// The lighting state of one device path (a receiver slot, a dongle or a cable).
#[derive(Clone, Debug, Default)]
pub struct Lighting {
    pub(crate) feature_index: Option<u8>,
    pub(crate) min: Option<u16>,
    pub(crate) max: Option<u16>,
    /// A chain of requests is queued or in flight.
    pub(crate) chain: Option<Chain>,
    /// The economy was applied (and acknowledged) in this connection: it is not applied again, whatever the
    /// read-backs say.
    pub(crate) applied: bool,
    /// When the apply chain may start (after a connection settles).
    pub(crate) start_at: Option<f64>,
    /// Read-backs still due, soonest first.
    pub(crate) verify_at: Vec<f64>,
    /// What the last read-back said: `Some(true)` at the minimum, `Some(false)` put back.
    pub(crate) reported_off: Option<bool>,
}

impl Lighting {
    /// The request of a step, always long, with software ID [`LIGHT_SWID`]. The only constructor of a lighting
    /// request; `None` when the step needs a feature index that is not known yet.
    pub(crate) fn request(&self, dev: u8, step: LightStep) -> Option<Request> {
        let build = |index: u8, function: u8, params: &[u8]| Request {
            report_id: LONG_ID,
            index: dev,
            b2: index,
            b3: (function << 4) | LIGHT_SWID,
            params: params.to_vec(),
        };
        Some(match step {
            LightStep::Feature => build(0, 0, &F_BRIGHTNESS.to_be_bytes()),
            LightStep::Info => build(self.feature_index?, FN_INFO, &[]),
            LightStep::Get(_) => build(self.feature_index?, FN_GET, &[]),
            LightStep::Off => build(self.feature_index?, FN_SET, &self.min?.to_be_bytes()),
            LightStep::Restore(value) => build(self.feature_index?, FN_SET, &value.to_be_bytes()),
        })
    }

    /// Forget what a connection taught that a new one may change; the feature index and the range stay (the same
    /// firmware answers them again).
    pub(crate) fn connection_lost(&mut self) {
        self.chain = None;
        self.applied = false;
        self.start_at = None;
        self.verify_at.clear();
        self.reported_off = None;
    }

    /// The soonest moment the driver needs `tick` for.
    pub(crate) fn next_due(&self) -> Option<f64> {
        self.start_at
            .into_iter()
            .chain(self.verify_at.first().copied())
            .min_by(f64::total_cmp)
    }
}

/// The allow-list of lighting requests (the only one that admits software ID 0x0C), checked for the request the
/// driver is about to send. `index` is the device's brightness feature index once discovered, `max` the top of its
/// range once read.
/// - the device is addressed as `hidpp` addresses them: 1 to 6 behind a receiver, 0xFF direct;
/// - root getFeature, for 0x8040 only;
/// - on the brightness feature: getInfo (function 0) and getBrightness (function 1) with no parameters, and
///   setBrightness (function 2) with exactly a 16-bit value, never above the device's maximum.
///
/// Anything else (the on/off write, every other feature, every other function) is refused.
pub fn is_allowed(req: &Request, index: Option<u8>, max: Option<u16>) -> bool {
    if req.report_id != LONG_ID || req.b3 & 0x0F != LIGHT_SWID {
        return false;
    }
    if !matches!(req.index, 1..=6 | DIRECT) {
        return false;
    }
    let function = req.b3 >> 4;
    let zeros_after = |n: usize| req.params.iter().skip(n).all(|b| *b == 0);
    if req.b2 == 0 {
        return function == 0
            && req.params.len() >= 2
            && req.params[..2] == F_BRIGHTNESS.to_be_bytes()
            && zeros_after(2);
    }
    if Some(req.b2) != index {
        return false;
    }
    match function {
        FN_INFO | FN_GET => zeros_after(0),
        FN_SET => {
            let value = u16::from_be_bytes([
                req.params.first().copied().unwrap_or(0),
                req.params.get(1).copied().unwrap_or(0),
            ]);
            req.params.len() >= 2 && zeros_after(2) && max.is_some_and(|m| value <= m)
        }
        _ => false,
    }
}

/// BrightnessControl getInfo answer: maximum (bytes 0..1), steps and flags (byte 2), capabilities (byte 3), minimum
/// (bytes 4..5), all big-endian. Measured on the G915 X: `00 64 05 03 00 00`, 0 to 100.
pub fn decode_info(payload: &[u8]) -> Option<(u16, u16)> {
    if payload.len() < 6 {
        return None;
    }
    let max = u16::from_be_bytes([payload[0], payload[1]]);
    let min = u16::from_be_bytes([payload[4], payload[5]]);
    (min <= max).then_some((min, max))
}

/// getBrightness answer: the level, big-endian in bytes 0..1.
pub fn decode_brightness(payload: &[u8]) -> Option<u16> {
    (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hidpp::{self, SWID};

    fn lit() -> Lighting {
        Lighting {
            feature_index: Some(14),
            min: Some(0),
            max: Some(100),
            ..Lighting::default()
        }
    }

    #[test]
    fn the_g915x_requests_are_these_bytes() {
        let l = lit();
        let hex = |s| hidpp::hex(&l.request(1, s).unwrap().encode());
        assert_eq!(
            hex(LightStep::Feature),
            "1101000c80400000000000000000000000000000"
        );
        assert_eq!(
            hex(LightStep::Info),
            "11010e0c00000000000000000000000000000000"
        );
        assert_eq!(
            hex(LightStep::Get(Purpose::Save)),
            "11010e1c00000000000000000000000000000000"
        );
        assert_eq!(
            hex(LightStep::Off),
            "11010e2c00000000000000000000000000000000"
        );
        assert_eq!(
            hex(LightStep::Restore(13)),
            "11010e2c000d0000000000000000000000000000"
        );
        // A cabled device is addressed 0xFF.
        assert_eq!(
            hidpp::hex(&l.request(0xFF, LightStep::Restore(100)).unwrap().encode()),
            "11ff0e2c00640000000000000000000000000000"
        );
    }

    #[test]
    fn a_step_that_needs_the_feature_index_waits_for_it() {
        let l = Lighting::default();
        assert!(l.request(1, LightStep::Feature).is_some());
        assert!(l.request(1, LightStep::Info).is_none());
        assert!(l.request(1, LightStep::Get(Purpose::Save)).is_none());
        assert!(l.request(1, LightStep::Off).is_none());
    }

    #[test]
    fn every_request_the_builders_make_passes_the_lighting_list_and_never_the_general_one() {
        let l = lit();
        for step in [
            LightStep::Feature,
            LightStep::Info,
            LightStep::Get(Purpose::Verify),
            LightStep::Off,
            LightStep::Restore(13),
            LightStep::Restore(100),
        ] {
            for dev in [1, 6, DIRECT] {
                let req = l.request(dev, step).unwrap();
                assert!(is_allowed(&req, Some(14), Some(100)), "{step:?} {dev}");
                assert!(!hidpp::is_allowed(&req), "{step:?} passes the general list");
            }
        }
    }

    #[test]
    fn the_lighting_list_refuses_what_is_not_the_economy() {
        let ok = |r: &Request| is_allowed(r, Some(14), Some(100));
        let mk = |fi: u8, f: u8, swid: u8, params: &[u8]| Request {
            report_id: LONG_ID,
            index: 1,
            b2: fi,
            b3: (f << 4) | swid,
            params: params.to_vec(),
        };
        assert!(ok(&mk(14, 2, LIGHT_SWID, &[0, 50])));
        // Software ID 0x0D is the read-only provider's, 0x0E and 0x0F are G HUB's.
        for swid in [SWID, 0x0E, 0x0F, 0] {
            assert!(!ok(&mk(14, 2, swid, &[0, 50])));
        }
        // The on/off write (function 4), the on/off read (3) and every other function.
        for f in [3, 4, 5, 8, 14, 15] {
            assert!(!ok(&mk(14, f, LIGHT_SWID, &[1])), "function {f}");
        }
        // Another feature index, or none known yet.
        assert!(!ok(&mk(9, 2, LIGHT_SWID, &[0, 50])));
        assert!(!is_allowed(
            &mk(14, 2, LIGHT_SWID, &[0, 50]),
            None,
            Some(100)
        ));
        // A value above the maximum, no maximum known, a short or padded-with-data value.
        assert!(!ok(&mk(14, 2, LIGHT_SWID, &[0, 101])));
        assert!(!is_allowed(
            &mk(14, 2, LIGHT_SWID, &[0, 50]),
            Some(14),
            None
        ));
        assert!(!ok(&mk(14, 2, LIGHT_SWID, &[50])));
        assert!(!ok(&mk(14, 2, LIGHT_SWID, &[0, 50, 7])));
        // Reads carry no parameters.
        assert!(!ok(&mk(14, 1, LIGHT_SWID, &[1])));
        // Root: only getFeature of 0x8040.
        assert!(!ok(&mk(0, 0, LIGHT_SWID, &[0x80, 0x71])));
        assert!(!ok(&mk(0, 1, LIGHT_SWID, &[0x80, 0x40])));
        assert!(!ok(&mk(0, 0, LIGHT_SWID, &[0x80, 0x40, 1])));
        // A short report, a device index nobody has.
        let mut short = mk(14, 2, LIGHT_SWID, &[0, 50]);
        short.report_id = hidpp::SHORT_ID;
        assert!(!ok(&short));
        let mut far = mk(14, 2, LIGHT_SWID, &[0, 50]);
        far.index = 7;
        assert!(!ok(&far));
    }

    #[test]
    fn the_general_list_does_not_admit_lighting_even_with_its_own_software_id_swapped() {
        // The general list passes functions 0 and 1 on any feature with software ID 0x0D, so a read of
        // 0x8040 would pass it; the writes (function 2) never do.
        let set = Request::feature(1, 14, 2, &[0, 0], true);
        assert!(!hidpp::is_allowed(&set));
    }

    #[test]
    fn the_answers_decode_as_measured() {
        // G915 X getInfo `00 64 05 03 00 00`: 0 to 100; getBrightness `00 0d`: 13.
        assert_eq!(decode_info(&[0, 0x64, 5, 3, 0, 0, 0]), Some((0, 100)));
        assert_eq!(decode_info(&[0, 0x64, 5]), None);
        assert_eq!(decode_info(&[0, 10, 5, 3, 0, 20]), None);
        assert_eq!(decode_brightness(&[0, 0x0d, 0, 0]), Some(13));
        assert_eq!(decode_brightness(&[7]), None);
    }

    #[test]
    fn saved_text_round_trips() {
        let s = SavedLight { brightness: 13 };
        assert_eq!(s.to_text(), "brightness:13");
        assert_eq!(SavedLight::from_text(&s.to_text()), Some(s));
        assert_eq!(
            SavedLight::from_text(" brightness:100 "),
            Some(SavedLight { brightness: 100 })
        );
        assert_eq!(SavedLight::from_text("brightness:x"), None);
        assert_eq!(SavedLight::from_text("13"), None);
    }

    #[test]
    fn only_the_g915x_has_a_shipped_path() {
        let path = |m, id| path_for(catalog::find(m, id));
        assert_eq!(path(Match::Wpid, 0x40B5), Some(Path::Brightness));
        assert_eq!(path(Match::Usb, 0xC356), Some(Path::Brightness));
        // The G703 and the G733 are measured and not shipped; receivers are not devices.
        assert_eq!(path(Match::Wpid, 0x4086), None);
        for pid in [0x0AB5, 0x0AFE, 0x0B1F] {
            assert_eq!(path(Match::Dongle, pid), None);
        }
        assert_eq!(path(Match::Receiver, 0xC547), None);
        assert_eq!(path_for(None), None);
    }
}
