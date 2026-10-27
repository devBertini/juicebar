// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The device catalog (`spec/devices.tsv`) and the voltage-to-percent curves (`spec/curves.tsv`), embedded at build
//! time and parsed once. Both are shared with the Mac core, so the files stay the single source.

use std::sync::OnceLock;

use crate::types::Kind;

const DEVICES_TSV: &str = include_str!("../../../../spec/devices.tsv");
const CURVES_TSV: &str = include_str!("../../../../spec/curves.tsv");

/// What a catalog row's id identifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Match {
    /// USB product id of a receiver.
    Receiver,
    /// USB product id of a device's own dongle.
    Dongle,
    /// Wireless product id a receiver reports for a paired device.
    Wpid,
    /// USB product id of the device itself on its cable.
    Usb,
}

/// The battery source measured on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BatterySource {
    None,
    /// 0x1000 Battery Status.
    Status,
    /// 0x1001 Battery Voltage.
    Voltage,
    /// 0x1004 Unified Battery.
    Unified,
    /// 0x1F20 ADC measurement.
    Adc,
}

/// One row of `spec/devices.tsv` for the Logitech provider.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub matcher: Match,
    pub id: u16,
    pub model: &'static str,
    pub kind: Kind,
    pub battery: BatterySource,
    pub curve: Option<&'static str>,
    pub use_h: Option<f64>,
    pub use_h_rgb: Option<f64>,
    pub charge_h: Option<f64>,
}

fn parse_matcher(s: &str) -> Option<Match> {
    Some(match s {
        "receiver" => Match::Receiver,
        "dongle" => Match::Dongle,
        "wpid" => Match::Wpid,
        "usb" => Match::Usb,
        _ => return None,
    })
}

fn parse_kind(s: &str) -> Option<Kind> {
    Some(match s {
        "mouse" => Kind::Mouse,
        "keyboard" => Kind::Keyboard,
        "headset" => Kind::Headset,
        "earbuds" => Kind::Earbuds,
        "gamepad" => Kind::Gamepad,
        "other" => Kind::Other,
        _ => return None,
    })
}

fn parse_battery(s: &str) -> Option<BatterySource> {
    Some(match s {
        "-" => BatterySource::None,
        "hidpp-0x1000" => BatterySource::Status,
        "hidpp-0x1001" => BatterySource::Voltage,
        "hidpp-0x1004" => BatterySource::Unified,
        "hidpp-0x1f20" => BatterySource::Adc,
        _ => return None,
    })
}

fn optional(s: &str) -> Option<&str> {
    if s == "-" { None } else { Some(s) }
}

fn parse_row(line: &'static str) -> Option<Row> {
    let c: Vec<&'static str> = line.split('\t').collect();
    if c.len() < 10 || c[0] != "logitech-hidpp" {
        return None;
    }
    Some(Row {
        matcher: parse_matcher(c[1])?,
        id: u16::from_str_radix(c[2], 16).ok()?,
        model: c[3],
        kind: parse_kind(c[4])?,
        battery: parse_battery(c[5])?,
        curve: optional(c[6]),
        use_h: optional(c[7]).and_then(|v| v.parse().ok()),
        use_h_rgb: optional(c[8]).and_then(|v| v.parse().ok()),
        charge_h: optional(c[9]).and_then(|v| v.parse().ok()),
    })
}

/// Every Logitech row of the catalog, in file order.
pub fn rows() -> &'static [Row] {
    static ROWS: OnceLock<Vec<Row>> = OnceLock::new();
    ROWS.get_or_init(|| {
        DEVICES_TSV
            .lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with("provider\t"))
            .filter_map(parse_row)
            .collect()
    })
}

/// The row for a product id of the given kind.
pub fn find(matcher: Match, id: u16) -> Option<&'static Row> {
    rows().iter().find(|r| r.matcher == matcher && r.id == id)
}

/// The id every row of one device shares for its identity: the first `wpid` row of the same model, else the first row
/// of the model. The G915 X as wpid 40B5 on a receiver and as usb C356 on its cable are one device (40B5); the three
/// G733 dongles are one device (the first of them).
pub fn canonical_id(row: &Row) -> u16 {
    let same = || rows().iter().filter(|r| r.model == row.model);
    same()
        .find(|r| r.matcher == Match::Wpid)
        .or_else(|| same().next())
        .map_or(row.id, |r| r.id)
}

struct Curve {
    id: &'static str,
    points: Vec<(u16, f64)>,
}

fn curves() -> &'static [Curve] {
    static CURVES: OnceLock<Vec<Curve>> = OnceLock::new();
    CURVES.get_or_init(|| {
        let mut out: Vec<Curve> = Vec::new();
        for line in CURVES_TSV.lines() {
            if line.starts_with('#') || line.starts_with("curve\t") {
                continue;
            }
            let c: Vec<&'static str> = line.split('\t').collect();
            if c.len() < 3 {
                continue;
            }
            let (Ok(mv), Ok(pct)) = (c[1].parse::<u16>(), c[2].parse::<f64>()) else {
                continue;
            };
            match out.iter_mut().find(|k| k.id == c[0]) {
                Some(k) => k.points.push((mv, pct)),
                None => out.push(Curve {
                    id: c[0],
                    points: vec![(mv, pct)],
                }),
            }
        }
        for k in &mut out {
            k.points.sort_by_key(|p| p.0);
        }
        out
    })
}

/// Percent for a voltage on a curve: linear between points, clamped to the first and last point and to 0..=100.
/// `None` when the curve does not exist.
pub fn curve_percent(curve: &str, millivolts: u16) -> Option<f64> {
    let points = &curves().iter().find(|k| k.id == curve)?.points;
    let (first, last) = (points.first()?, points.last()?);
    let value = if millivolts <= first.0 {
        first.1
    } else if millivolts >= last.0 {
        last.1
    } else {
        let upper = points.iter().position(|p| p.0 >= millivolts)?;
        let (lo, hi) = (points[upper - 1], points[upper]);
        let t = f64::from(millivolts - lo.0) / f64::from(hi.0 - lo.0);
        lo.1 + (hi.1 - lo.1) * t
    };
    Some(value.clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_every_row_of_the_file() {
        let data_lines = DEVICES_TSV
            .lines()
            .filter(|l| !l.starts_with('#') && !l.starts_with("provider\t") && !l.is_empty())
            .count();
        assert_eq!(rows().len(), data_lines);
        assert_eq!(rows().len(), 8);
    }

    #[test]
    fn rows_carry_the_measured_fields() {
        let g703 = find(Match::Wpid, 0x4086).unwrap();
        assert_eq!(g703.model, "G703 LIGHTSPEED");
        assert_eq!(g703.kind, Kind::Mouse);
        assert_eq!(g703.battery, BatterySource::Voltage);
        assert_eq!(g703.curve, Some("g703"));
        assert_eq!(g703.use_h, Some(30.0));
        assert_eq!(g703.use_h_rgb, Some(22.0));
        assert_eq!(g703.charge_h, None);
        let g915 = find(Match::Wpid, 0x40B5).unwrap();
        assert_eq!(g915.battery, BatterySource::Unified);
        assert_eq!(g915.curve, None);
        assert_eq!(find(Match::Receiver, 0xC539).unwrap().kind, Kind::Other);
        assert_eq!(
            find(Match::Dongle, 0x0AFE).unwrap().battery,
            BatterySource::Adc
        );
        assert!(find(Match::Dongle, 0x1234).is_none());
    }

    #[test]
    fn cable_and_receiver_rows_share_one_identity() {
        let wpid = find(Match::Wpid, 0x40B5).unwrap();
        let usb = find(Match::Usb, 0xC356).unwrap();
        assert_eq!(canonical_id(wpid), 0x40B5);
        assert_eq!(canonical_id(usb), 0x40B5);
        // The three G733 dongles are one device.
        let ids: Vec<u16> = [0x0AB5, 0x0AFE, 0x0B1F]
            .iter()
            .map(|id| canonical_id(find(Match::Dongle, *id).unwrap()))
            .collect();
        assert_eq!(ids, vec![0x0AB5; 3]);
        // Different models stay apart.
        assert_ne!(
            canonical_id(find(Match::Wpid, 0x4086).unwrap()),
            canonical_id(wpid)
        );
    }

    #[test]
    fn curves_interpolate_and_clamp() {
        // The measured points are exact.
        assert_eq!(curve_percent("g703", 3968), Some(77.0));
        assert_eq!(curve_percent("g733", 3753), Some(45.0));
        // Between the empty point and the measured one.
        let p = curve_percent("g703", 3966).unwrap();
        assert!((p - 77.0 * 466.0 / 468.0).abs() < 1e-9, "{p}");
        // Clamped at both ends.
        assert_eq!(curve_percent("g703", 3000), Some(0.0));
        assert_eq!(curve_percent("g703", 4500), Some(100.0));
        assert_eq!(curve_percent("g703", 4200), Some(100.0));
        // Midpoint of the upper segment.
        let mid = curve_percent("g703", 4084).unwrap();
        assert!((mid - 88.5).abs() < 1e-9, "{mid}");
        assert_eq!(curve_percent("nope", 3900), None);
    }
}
