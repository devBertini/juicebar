# Architecture: the modules both apps share by name

Both apps are built the same way, so a change in one has an obvious place in the other. The core is pure: it never opens a device, a file or a timer; the app shell does I/O and hands the core events, and the core answers with actions. That keeps the core testable on recorded traces (`spec/fixtures/`) and the shells thin.

| Module | Windows (`windows/core`, `windows/app`) | Mac (`Packages/JuiceBarCore`, the app target) | Role |
|---|---|---|---|
| `hidpp` | core | core | frames, requests, answer matching (software ID 0x0D), decoders (`spec/PROTOCOL.md`) |
| `logitech` | core | core | the `logitech-hidpp` provider as a state machine: transport events in, actions out |
| `providers` | core (contract) + app (OS providers) | core (contract) + app | the `Provider` contract (`spec/PROVIDERS.md`); OS-backed providers (Bluetooth, controllers) live in the app shell |
| `registry` | core | core | registered devices, identity merge (wpid and cable PID are one device), presence |
| `store` | core (format) + app (file I/O) | core + app | per-device history as text lines; the shell reads and writes the file the core produces |
| `estimator` | core | core | `spec/estimator.md`, proven on `spec/estimator-cases.tsv` |
| `snapshot` | core | core | what the UI shows: one `DeviceView` per device and the bar icon's state, all texts formatted |
| `strings` | core | app (String Catalog) | every visible text, English source and Brazilian Portuguese, from `spec/strings.tsv` |
| transport | app (`hid`): SetupAPI/CfgMgr, `CM_Register_Notification`, one I/O thread with overlapped reads | app: IOHIDManager on the main run loop | raw reports in and out |
| UI | app: tray (`Shell_NotifyIcon`, Direct2D icon), flyout (Direct2D, DirectWrite, DWM backdrop) | app: `NSStatusItem`, `NSPanel` hosting SwiftUI, WidgetKit extension | `spec/design/DESIGN.md` |

## The sans-I/O loop
```
transport event (Arrived endpoint | Removed endpoint | Report bytes) + now
        -> core.handle(event, now) -> [Action]
Action  = Send(endpoint, bytes) | Emit(ProviderEvent) | Persist(device, lines) | Wake(at)
```
- `Endpoint`: session-local id, VID, PID, usage page, usage, input and output report lengths, product string, transport kind (`usb`, `bluetooth`).
- Time is always passed in (`now`: wall seconds UTC and a monotonic instant); the core never reads a clock.
- `Wake(at)` is the only way the core asks for time: the shell keeps one one-shot timer for the earliest requested moment (a request timeout, the slow battery refresh, a dongle ping) and calls `core.tick(now)` when it fires. No other timer exists; with nothing pending the process sleeps.
- `Snapshot` is rebuilt by the core after any event that changed what is shown, and the shell redraws only when it differs from the last one.

## Rules for the code
- Every source file starts with two comment lines in its language's form: `Copyright (c) 2026 Claudio Bertini (devBertini)` and `SPDX-License-Identifier: GPL-3.0-or-later`.
- Windows: Rust edition 2024, the `windows` crate only, `#![forbid(unsafe_code)]` in `core`; `unsafe` only in the app's Win32 calls, each block with a one-line reason. Mac: Swift 6, no package, the core free of AppKit and IOKit.
- No polling loops, no busy waits, no sleeping threads waiting for time; one I/O thread on Windows blocked on its handles.
