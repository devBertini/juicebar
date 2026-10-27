# Providers

A provider finds the devices of one protocol or system service and reports their presence and battery. Everything after it (the registry of registered devices, the history store, the estimator, usage, health, warnings, the bar icon and the panel) knows nothing about where a reading came from. Both apps implement this contract with the same names: `Provider` (Rust trait in `windows/core`, Swift protocol in `Packages/JuiceBarCore`).

## What a provider reports

| Type | Fields |
|---|---|
| `DeviceInfo` | `key` (stable, see Identity), `provider` (id below), `name` (as the device or the system names it), `kind` (`mouse`, `keyboard`, `headset`, `earbuds`, `gamepad`, `other`), `connection` (`receiver`, `dongle`, `bluetooth`, `usb`), `brand` |
| `BatteryReading` | `at` (monotonic and wall time), `percent` (0 to 100, or none), `coarse` (`full`, `good`, `low`, `critical`, or none), `millivolts` (or none), `charging` (`discharging`, `charging`, `full`, `unknown`), `quality` (`exact`: the device gives a percentage; `derived`: computed from voltage by a calibrated curve; `coarse`: only a level) |
| `Presence` | `online` (true or false), `at` |

Events, pushed by the provider to the registry: `appeared(DeviceInfo)`, `disappeared(key)`, `presence(key, Presence)`, `battery(key, BatteryReading)`. The registry asks a provider for one thing only: `refresh(key)`, a single battery read.

Each provider declares its `capabilities`: `pushesPresence` (the OS or the device tells us when it comes and goes), `pushesBattery` (the device sends battery events), `minRefreshSeconds` (the shortest interval its reads may be asked at; a read wakes a sleeping wireless device, so the default is 600 s, and a provider that pushes battery may say none).

## Rules every provider follows
- **Event-driven, never a loop.** Presence comes from OS notifications (device arrival and removal, a receiver's connection notification, a controller added or removed); battery from the device's own events where it sends them, otherwise from `refresh` at the pace the registry sets (never below `minRefreshSeconds`; no read at all while the device is offline). No thread spins, no timer ticks while nothing changes.
- **Devices that stream input are sampled, never followed.** A game pad sends input reports hundreds of times a second; its provider opens it, takes one report (or asks the OS for its battery), and closes it, at its refresh interval and on its arrival, so JuiceBar never wakes for its input.
- **Read only.** A provider never writes a device setting, except a feature the owner turned on in the app (the RGB economy), through its own explicit call, never as a side effect of reading.
- **Coexists with vendor software.** Shared opens only (never exclusive), its own HID++ software ID, answers matched to its own requests; other programs' traffic ignored.
- **Releases what it holds** on `disappeared`, on sleep and on quit.
- **Identity without secrets.** `key` = provider id + the fields that keep a device apart (a receiver's product id and slot, a wireless product id, a Bluetooth address), hashed (SHA-256, first 16 hex characters) when any of them is a serial number or an address, so no store, log, trace or fixture holds one in the clear.
- **No network, no telemetry.**

## Providers

| Id | What | Windows | Mac | Status |
|---|---|---|---|---|
| `logitech-hidpp` | Logitech devices over HID++: LIGHTSPEED receivers, headset dongles, Bluetooth | HID API (shared open, overlapped reads), `CM_Register_Notification` | IOHIDManager | v1, the owner's G703, G733 and G915 X |
| `os-bluetooth` | Bluetooth devices of any brand that report battery to the system | to be measured (W6): the GATT Battery Service, or the property Windows Settings shows | to be measured (M8): `IORegistry` for Apple accessories, Core Bluetooth Battery Service for others | v1 if the probes find a read-only path |
| `game-controllers` | Xbox, PlayStation, Switch Pro and other controllers | Xbox: `Windows.Gaming.Input` battery report (exact), else `XInputGetBatteryInformation` (coarse); DualSense: one HID input report sampled (USB report 0x01, Bluetooth 0x31; `docs/research/controllers.md`) | `GameController` (`GCController.battery`) | v1, built from documented facts; proven when the owner connects an Xbox controller or a DualSense |
| others (`steelseries-arctis`, `hyperx-cloud`, `razer`, `corsair-icue`, ...) | other brands' own protocols | | | a contributor's provider, shipped only when proven on a real device (`docs/CONTRIBUTING.md`) |

Which of these a device appears under is decided once: when two providers see the same device (a Logitech mouse over Bluetooth that the system also reports), the more exact one wins (`logitech-hidpp` over `os-bluetooth`), by connection and name.

## Proof
Each provider ships sanitized traces of real devices in `spec/fixtures/` and decoder tests that read them, in each app that implements it. A provider proven on one platform only ships on that platform.
