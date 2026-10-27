// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Parsing of Windows HID device interface paths (case-insensitive).

/// What a HID interface path says about the device behind it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedPath {
    pub vid: u16,
    pub pid: u16,
    /// The `MI_xx` interface number of a composite device.
    pub interface: Option<u8>,
    /// The `COLxx` collection number.
    pub collection: Option<u8>,
    /// Bluetooth classic HID or HID over GATT (service class in the path).
    pub bluetooth: bool,
}

const BT_HID_CLASSIC: &str = "{00001124-0000-1000-8000-00805f9b34fb}";
const BT_HID_GATT: &str = "{00001812-0000-1000-8000-00805f9b34fb}";

fn after<'a>(lower: &'a str, key: &str) -> Option<&'a str> {
    let at = lower.find(key)? + key.len();
    lower.get(at..)
}

fn hex_run(s: &str) -> &str {
    let end = s.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(s.len());
    &s[..end]
}

/// The 16-bit id after `key_usb` (`vid_046d`) or `key_bt` (`vid&0002046d`, where the low 16
/// bits are the id and the high ones the id source).
fn id16(lower: &str, key_usb: &str, key_bt: &str) -> Option<u16> {
    if let Some(rest) = after(lower, key_usb) {
        let run = hex_run(rest);
        if run.len() == 4 {
            return u16::from_str_radix(run, 16).ok();
        }
    }
    let run = hex_run(after(lower, key_bt)?);
    if run.is_empty() || run.len() > 8 {
        return None;
    }
    u32::from_str_radix(run, 16)
        .ok()
        .map(|v| (v & 0xFFFF) as u16)
}

/// Parse a path such as `\\?\HID#VID_046D&PID_C539&MI_02&COL01#8&2a8b&0&0000#{4d1e55b2-...}`.
/// Returns `None` when the path carries no vendor and product ID.
pub fn parse_interface_path(path: &str) -> Option<ParsedPath> {
    let lower = path.to_ascii_lowercase();
    let vid = id16(&lower, "vid_", "vid&")?;
    let pid = id16(&lower, "pid_", "pid&")?;
    let interface = after(&lower, "&mi_").and_then(|r| {
        let h = r.get(..2)?;
        u8::from_str_radix(h, 16).ok()
    });
    let collection = after(&lower, "&col").and_then(|r| r.get(..2)?.parse::<u8>().ok());
    let bluetooth = lower.contains(BT_HID_CLASSIC) || lower.contains(BT_HID_GATT);
    Some(ParsedPath {
        vid,
        pid,
        interface,
        collection,
        bluetooth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "{4d1e55b2-f16f-11cf-88cb-001111000030}";

    #[test]
    fn parses_a_lowercase_composite_path() {
        let p = parse_interface_path(&format!(
            r"\\?\hid#vid_046d&pid_c539&mi_02&col01#8&2a8b1c0e&0&0000#{GUID}"
        ))
        .unwrap();
        assert_eq!(p.vid, 0x046D);
        assert_eq!(p.pid, 0xC539);
        assert_eq!(p.interface, Some(2));
        assert_eq!(p.collection, Some(1));
        assert!(!p.bluetooth);
    }

    #[test]
    fn parses_an_uppercase_path() {
        let p = parse_interface_path(&format!(
            r"\\?\HID#VID_046D&PID_C547&MI_02&COL02#8&1A2B3C&0&0001#{}",
            GUID.to_ascii_uppercase()
        ))
        .unwrap();
        assert_eq!((p.vid, p.pid), (0x046D, 0xC547));
        assert_eq!(p.interface, Some(2));
        assert_eq!(p.collection, Some(2));
    }

    #[test]
    fn parses_a_mixed_case_path_with_hex_interface() {
        let p =
            parse_interface_path(r"\\?\Hid#Vid_046D&Pid_0aFe&Mi_0B&Col03#7&1&0&0002#{x}").unwrap();
        assert_eq!(p.pid, 0x0AFE);
        assert_eq!(p.interface, Some(0x0B));
        assert_eq!(p.collection, Some(3));
    }

    #[test]
    fn a_path_without_interface_or_collection_has_none() {
        let p = parse_interface_path(&format!(r"\\?\hid#vid_046d&pid_c232#7&abc&0&0000#{GUID}"))
            .unwrap();
        assert_eq!(p.interface, None);
        assert_eq!(p.collection, None);
        assert_eq!(p.pid, 0xC232);
    }

    #[test]
    fn parses_a_collection_without_an_interface() {
        let p = parse_interface_path(r"\\?\hid#vid_046d&pid_c539&col04#7&abc&0&0003#{x}").unwrap();
        assert_eq!(p.interface, None);
        assert_eq!(p.collection, Some(4));
    }

    #[test]
    fn parses_a_bluetooth_path() {
        let p = parse_interface_path(
            r"\\?\HID#{00001124-0000-1000-8000-00805F9B34FB}_VID&0002046D_PID&B023#9&1c&0&0000#{x}",
        )
        .unwrap();
        assert_eq!((p.vid, p.pid), (0x046D, 0xB023));
        assert!(p.bluetooth);
    }

    #[test]
    fn parses_a_gatt_bluetooth_path() {
        let p = parse_interface_path(
            r"\\?\hid#{00001812-0000-1000-8000-00805f9b34fb}_dev_vid&02046d_pid&b034_rev&0001#a&1&0&0000#{x}",
        )
        .unwrap();
        assert_eq!((p.vid, p.pid), (0x046D, 0xB034));
        assert!(p.bluetooth);
    }

    #[test]
    fn rejects_paths_without_ids() {
        assert_eq!(parse_interface_path(r"\\?\hid#nothing#1#{x}"), None);
        assert_eq!(parse_interface_path(""), None);
        assert_eq!(parse_interface_path(r"\\?\hid#vid_04"), None);
    }

    #[test]
    fn rejects_a_vid_with_a_wrong_digit_count() {
        assert_eq!(parse_interface_path(r"\\?\hid#vid_46d&pid_c539#x"), None);
    }
}
