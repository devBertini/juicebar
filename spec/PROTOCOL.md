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
- **Dongles and cabled devices:** the OS device arrival and removal (`CM_Register_Notification` on Windows, IOHIDManager matching on the Mac). A dongle present with its headset off answers with a HID++ error or device index error: offline until a ping succeeds; while the dongle is present and the headset offline, ping it at most every 60 s (the only periodic request JuiceBar makes for presence).
- On every link-established moment: discovery if needed, then one battery read unless the device pushed its battery within 2 s.
- **Pushing devices skip the slow refresh:** a `0x1004` device that has pushed a battery event in this connection is not refreshed every 600 s; every other device (`0x1001`, `0x1000`, `0x1F20`) is.
- A cabled device answers at index 0xFF (measured on the G915 X as C356) and also at 1 to 6; JuiceBar uses 0xFF.

## Coexistence
After a reconnect G HUB re-reads the whole device (dozens of requests in 200 ms). JuiceBar never repeats that: one ping, the cached feature index, one battery read at most.
