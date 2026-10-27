# JuiceBar

**The battery of your wireless devices, next to the clock.** One small juice glass in the Windows tray (and soon in the Mac menu bar) tells you, at a glance, whether your mouse, keyboard, headset or controller needs a charge: purple when all is fine, green when everything is charged, red when something is about to run out. One click opens a panel with each device's battery, how many hours of use are left, when it will run out, a 7-day chart, today's use and the battery's health.

*Idealizado por Claudio Bertini ([devBertini](https://github.com/devBertini)).*

> Português: o JuiceBar mostra, ao lado do relógio, quais dos seus periféricos sem fio estão ligados, a bateria de cada um e quando ela vai acabar. Um único ícone (um copo de suco) muda de cor conforme as baterias; um clique abre o painel com a previsão em horas de uso, gráfico de 7 dias, uso de hoje e saúde da bateria. Leve, nada sai do seu computador, e de código aberto.

## Why

Vendor software that shows these batteries can keep more than a hundred megabytes resident and still does not tell you when a battery will run out. JuiceBar talks to the devices itself and stays out of the way:

- about **8 MB** of memory (working set; 4 MB private) and **0.01 %** of one CPU core while idle, measured over 10 minutes on a Windows 11 laptop where the vendor's software held 171 MB;
- no polling loop: devices report their state and JuiceBar sleeps until something changes;
- **nothing leaves your computer**: no internet access and no telemetry;
- one executable of about 1.2 MB; written in Rust, with Microsoft's `windows` crate as its only dependency.

## What it does

- Shows which registered devices are on, their battery now, hours of use left and the clock time it will run out ("runs out today ~17:40"), time to full while charging.
- Learns your own pace: the forecast starts from the maker's rated battery life and adapts to how you actually use each device.
- Smart warning: a notice only when a device will not last your usual day; a notice when a charge is complete.
- Battery health over the months, hours of use per day.
- **Power saver** ("Economia de Energia"): turns off the lights (LEDs) of supported devices so their batteries last longer, and puts them back exactly as they were.
- Settings: theme (automatic, light, dark), the icon's color limits, alerts and do-not-disturb, devices (show, rename, order), start with Windows, a global shortcut (Ctrl+Alt+B by default), English and Brazilian Portuguese.

## Devices

| Family | Status |
|---|---|
| Logitech over HID++ (LIGHTSPEED receivers, headset dongles, USB cable): G703, G915 X, G733 | proven on real devices |
| Xbox controllers and PlayStation DualSense | prepared from documented facts, not yet proven on a device |
| Other brands (Razer, Corsair, SteelSeries, HyperX and more) | open for contributions, each proven on a real device: see [CONTRIBUTING](docs/CONTRIBUTING.md) |

A Mac version (menu bar app) is on its way.

## Build and install (Windows 11)

Requirements: Rust (stable, MSVC) and the Visual Studio Build Tools with the C++ workload.

```
cargo build --release --manifest-path windows/Cargo.toml -p juicebar
powershell -NoProfile -File windows/scripts/install.ps1
```

`install.ps1` copies `JuiceBar.exe` to `%LOCALAPPDATA%\Programs\JuiceBar`, starts it with Windows and opens it; `uninstall.ps1` removes it (your history stays unless you pass `-RemoveData`). Windows 11 hides new tray icons behind the `^` arrow: drag the juice glass next to the clock, or turn it on in Settings > Personalization > Taskbar > Other system tray icons.

## How it is built

- `spec/`: the contract both apps follow: the HID++ protocol as JuiceBar speaks it, the providers, the device catalog, the forecast rules with an executable reference and its test cases, recorded traces of real devices.
- `windows/`: the Windows app: a platform-free core (protocol, forecast, history), the HID transport, the drawing of the panel and the icon, and the tray app.

## License

Copyright (c) 2026 Claudio Bertini (devBertini). JuiceBar is free software under the GNU General Public License, version 3 or (at your option) any later version: see [LICENSE](LICENSE).
