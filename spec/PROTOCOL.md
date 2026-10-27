# The Logitech provider: HID++ as JuiceBar speaks it

What the `logitech-hidpp` provider (`spec/PROVIDERS.md`) sends and reads. Every fact here was measured on the owner's devices (`docs/research/probe-windows.md`, traces in `spec/fixtures/`) or read in the sources quoted in `docs/research/hidpp-and-lighting.md`; nothing is copied from GPL code.

## Transport
- A HID++ endpoint is a vendor-defined HID collection of VID 046D: usage page `0xFF00` (receivers and cabled devices: usage 0x0001 short reports, 0x0002 long, 0x0004 very long) or `0xFF43` (the G733 dongle: long reports only). Open it shared for read and write; G HUB and Windows' own lighting service keep their handles, and every open handle receives every input report.
- Reports: short `0x10` (7 bytes), long `0x11` (20 bytes). Send long when the endpoint has no short collection. A request is `[report id, device index, feature index, function << 4 | software id, params...]`, zero-padded to the report length.
- **Software ID `0x0D`.** An answer is ours when device index, feature index, function and software ID all match a pending request; everything else is someone else's traffic (G HUB uses 0x0E and 0x0F) or a device event (software ID 0).
- **Timeouts:** 4 s for the first request to a device that may be asleep (a sleeping keyboard answered its first ping after 1.47 s), 1 s afterwards. An answer with feature index `0xFF` is a HID++ 2.0 error (`[.., 0xFF, feature, function|swid, code]`); `0x8F` is a HID++ 1.0 error.

## Endpoints and device indexes
| Endpoint | Device index | How it is found |
|---|---|---|
| LIGHTSPEED receiver (`devices.tsv` match `receiver`) | 1 to 6, one per pairing slot | HID++ 1.0 long register read `0x83 0xB5 0x20+n`: the payload after the sub-register echo holds two bytes, the wireless product id (big-endian), then the kind in the low nibble (1 keyboard, 2 mouse), as `windows/probe/src/proto.rs` decodes it (G915 X: `20 07 08 40 B5 11 ...`); an empty slot answers a 1.0 error (0x03 on C539, 0x09 on C547) |
| A device's own dongle (`dongle`, the G733) | 0xFF | the endpoint is the device |
| A device on its cable (`usb`, the G915 X as C356) | 0xFF | the endpoint is the device; it is the same device as its `wpid` row and continues its history |

## Discovery, once per device connection
1. Root (feature index 0) function 1, ping: answers the HID++ protocol version (4.2 on all three devices).
2. Root function 0, getFeature, for each battery feature in the order `0x1004`, `0x1001`, `0x1000`, `0x1F20`: the first one present is the device's battery feature (index cached for this connection). Also `0x0005` DeviceName for the name.
3. Never read `0x0003` DeviceInfo or the receiver's serial registers (`0xB5 0x30+n`): they carry serial numbers.

## Battery reads and events
| Feature | Read | Decode | Event |
|---|---|---|---|
| `0x1004` Unified Battery | function 1 (status); function 0 gives capabilities | byte 0 percent, byte 1 level bits (8 full, 4 good, 2 low, 1 critical), byte 2 charging (0 discharging, 1 charging, 2 slow, 3 full, others per capability), quality `exact` | event 0, same layout: pushed on reconnect and on change (G915 X) |
| `0x1001` Battery Voltage | function 0 | bytes 0..1 millivolts big-endian, byte 2 flags (bit 7 charging), quality `derived` through `spec/curves.tsv` | event 0, same layout: pushed on reconnect only (G703), so the 600 s refresh stays |
| `0x1000` Battery Status | function 0 | byte 0 percent, byte 1 next level, byte 2 status | event 0 |
| `0x1F20` ADC (headsets) | function 0 | bytes 0..1 millivolts big-endian, byte 2 flags (meaning not yet measured: 0x01 while G HUB said not charging), quality `derived` | not measured |

## Presence
- **Receivers:** HID++ 1.0 notification `0x41` (`[0x10, slot, 0x41, b3, b4, wpid lo, wpid hi]`): bit 0x40 of byte 4 set means link lost, clear means link established; the wireless product id is little-endian (measured: `10 01 41 0c 62 86 40` is the G703, 0x4086, link lost; `... a2 86 40` established). They arrive only after the receiver's notifications are enabled: JuiceBar reads register `0x00` (short read `0x81 0x00`) and, when the wireless (`0x000100`) and software-present (`0x000800`) flags are not both set, writes them (short write `0x80 0x00`), the one register write the provider makes. G HUB enables them too; writing the same flags is harmless.
- **Cabled devices:** the OS device arrival and removal (`CM_Register_Notification` on Windows, IOHIDManager matching on the Mac).
- **Dongles (`dongle`, the G733):** the dongle answers the root ping at index 0xFF by itself, so a ping proves only that the dongle is plugged in. Presence comes from the battery request: a battery answer means online (and is a reading); a HID++ error answer (`11 FF FF 08 0D 05`) or a timeout means offline (the headset is off or out of range). While the dongle is present and the headset offline, read the battery at most every 60 s (the only periodic request JuiceBar makes for presence); a ping is repeated only while the battery feature is still unknown.
- On every link-established moment: discovery if needed, then one battery read unless the device pushed its battery within 2 s.
- **Pushing devices skip the slow refresh:** a `0x1004` device that has pushed a battery event in this connection is not refreshed every 600 s; every other device (`0x1001`, `0x1000`, `0x1F20`) is.
- A cabled device answers at index 0xFF (measured on the G915 X as C356) and also at 1 to 6; JuiceBar uses 0xFF.

## Coexistence
After a reconnect G HUB re-reads the whole device (dozens of requests in 200 ms). JuiceBar never repeats that: one ping, the cached feature index, one battery read at most.

## Lighting (the RGB economy)
The one device setting the provider writes besides the receiver's notification flags, and only on the app's explicit call (`Logitech::economy_on`, `economy_off`, `set_economy`) while the user's "RGB saver" switch is on. Measured on the G915 X (`docs/research/probe-windows.md`, "Lighting"; trace `spec/fixtures/windows-c547-g915x-lighting.hidtrace`).

- **Shipped for the G915 X only** (catalog model with canonical id 40B5, on a receiver slot and on its cable): BrightnessControl `0x8040`. The G703 (`0x8071`, effect not readable) and the G733 (`0x8070`, every read refused) are not touched.
- **Software ID `0x0C`**, long reports only, device index as everywhere. Not 0x0D: the general allow-list refuses every request with another software ID, so a lighting frame cannot pass as a read, and the lighting allow-list (`lighting::is_allowed`) admits only these:
  - Root function 0 with `80 40` (getFeature of `0x8040`);
  - on the brightness feature, function 0 `getInfo` and function 1 `getBrightness`, no parameters; function 2 `setBrightness` with exactly a big-endian 16-bit level, never above the maximum `getInfo` gave.
  - Never: the on/off functions (3, 4), any other feature, any other function.
- `getInfo` answer: maximum (bytes 0..1, big-endian), steps and flags, capabilities, minimum (bytes 4..5): `00 64 05 03 00 00` on the G915 X (0 to 100). `getBrightness` answer: the level in bytes 0..1 (`00 0d`, 13). The `setBrightness` answer has no payload.
- **Apply** (`economy_on`): getFeature, getInfo, getBrightness; a level above the minimum is saved (event `Saved`, which the app keeps in its settings) and `setBrightness(minimum)` is written; a level already at the minimum is left alone and nothing is saved. Then getBrightness (event `Off`, or `Reapplied` if it is not at the minimum), and read-backs 3 s and 30 s after the write.
- **Restore** (`economy_off`, or at start when a level was kept): getBrightness; at the minimum it is still ours, so `setBrightness(saved)` and getBrightness (events `Restored` and `Cleared`); above the minimum somebody changed it meanwhile, so nothing is written and the kept level is dropped (`Cleared`).
- **No fighting:** a read-back that finds the level put back (G HUB, the device waking) is reported (`Reapplied`; the app says "o G HUB está reaplicando a iluminação" in the panel's footer) and never answered with a write. The economy is applied again only after a reconnect.
- **Reconnect:** a device that comes online while the economy is wanted (or has a kept level) waits 3 s, then applies (or restores) with the same chains. A lighting request that times out marks the device offline like any other; lighting requests wait up to 4 s because a sleeping keyboard answered after 1.5 s.
- **Quit:** the saved level is written back to every device the economy turned down, with no read-back; the level stays kept so a write that did not arrive is restored at the next start.
- One request in flight per receiver, like every other request.
