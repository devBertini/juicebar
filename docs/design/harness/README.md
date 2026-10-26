# JuiceBar mockup harness

Draws the desktop around an option so five options can be compared on equal ground. Everything is local (no network, no CDN). Do not edit files in `harness/`; draw inside your option folder.

## Option layout

```
docs/design/options/<n>-<slug>/
  mac.html      Mac scene (required)
  win.html      Windows scene (required)
  option.css    optional, shared by both pages
  option.js     optional, shared by both pages (branch on JuiceBarFrame.platform)
```

Start by copying `options/0-empty/`. Its two pages are the include pattern; keep the order:

```html
<link rel="stylesheet" href="../../harness/frame.css">
<link rel="stylesheet" href="option.css">
...
<body class="scene-mac">            <!-- or scene-win -->
<script src="../../harness/glyphs.js"></script>
<script src="../../harness/sample-state.js"></script>
<script src="../../harness/frame.js"></script>   <!-- builds the scene and the slots -->
<script src="option.js"></script>                 <!-- runs after the slots exist -->
</body>
```

The scripts must be at the start of `<body>`, synchronous. After `frame.js` runs, every slot element exists. Fill slots from JS (`JuiceBarFrame.slot("panel").innerHTML = ...`) or with static markup: `<template data-slot="panel">...</template>` anywhere in the body is moved into the slot. Call `JuiceBarFrame.reposition()` if you change content after load (a ResizeObserver also does it).

## Scenes and slots

| Scene | CSS px | Render scale | PNG |
|---|---|---|---|
| Mac (`scene-mac`) | 1200 x 750 | 2 | 2400 x 1500 |
| Windows (`scene-win`) | 1600 x 1000 | 1.5 | 2400 x 1500 |

Mac (top-right of a MacBook Air 15" desktop, menu bar 24 px, translucent):
- `#bar-item`: in the menu bar, left of the system battery glyph. Height 24 px, width is yours. Everything you draw here must read on a translucent bar in both themes.
- `#panel`: the open panel, centered under `#bar-item`, 4 px below the bar, clamped 8 px inside the scene. Up to 380 x 600 px.
- `#widget`: macOS medium widget on the left, 329 x 155 px, corner radius 22 px (you draw the rounded shape and shadow).
- `#notification`: macOS banner under the widget, 344 px wide, height yours.

Windows (bottom-right of a Windows 11 desktop, taskbar 48 px, Mica-like):
- `#tray`: 32 px cells for your 16 px icon(s), left of the Wi-Fi/volume/battery glyphs. Every direct child of `#tray` becomes a 32 x 32 flex cell; put the icon inside. Several icons are allowed.
- `#flyout`: above the taskbar with a 12 px gap, centered on the last tray cell, clamped 12 px inside the scene. Up to 380 x 640 px.
- `#toast`: Windows 11 toast, left half of the screen, 364 px wide, height yours.

The harness draws the rest (wallpaper, clock, system glyphs, placeholders, bottom-left caption). Slots start empty and unstyled; the panel, flyout, widget, notification and toast get no background from the harness.

## Themes

The URL query `?theme=light|dark` (default light) sets `data-theme` on `<html>`. Style both with CSS variables:

```css
:root[data-theme="light"] { --accent: ...; }
:root[data-theme="dark"]  { --accent: ...; }
```

`JuiceBarFrame.theme` and `JuiceBarFrame.platform` ("mac" | "win") are available to scripts. Harness variables you may read: `--hx-fg`, `--hx-muted`. Fonts: Mac stack `-apple-system, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", sans-serif` (SF Pro does not exist on this PC; the Mac captions say so), Windows `"Segoe UI Variable Text", "Segoe UI", sans-serif`. Use `backdrop-filter` for glass; the wallpapers are colorful on purpose.

## Glyphs (`glyphs.js`)

Our own 24 x 24 line drawings, stroke 1.75, round caps: `g-mouse`, `g-keyboard`, `g-headset`, `g-bolt`, `g-bluetooth`, `g-wireless`, `g-warning`, `g-gear`, `g-leaf` (RGB economy), `g-gamepad`, `g-earbuds`. Use `<svg class="g"><use href="#g-mouse"/></svg>` or `JuiceBarGlyph("g-mouse", 18)`. Size with `font-size`/width/height; recolor with `color`; restyle on `svg.g` (`fill`, `stroke-width`). Never use any brand's logos or marks. You may draw your own glyphs.

## Sample data (`sample-state.js`)

`window.JUICEBAR_SAMPLE` (pt-BR, "now" is seg. 26/10/2026 14:32):
- `clock.mac`, `clock.winTime`, `clock.winDate`, `now`, `historyHours` (168), `lowThreshold` (20), `rgbEconomy` (false), `hotkey.win` ("Ctrl+Alt+B"), `hotkey.mac` ("⌃⌥B").
- `devices[]`: `id`, `name`, `kind`, `glyph`, `on`, `connection` ("LIGHTSPEED", "Bluetooth" or null), `platforms` (absent = both scenes; `["mac"]` = Mac only), `connectionGlyph`, `level` (percent), `state` ("discharging" | "charging" | "off"), `forecast`, `eta`, `etaClock`, `warning` (G703 only), `lastSeen` (G733 only), `usageToday`, `health` (percent), `history` (`[hourOffset, percent]` points, offsets from -168 to 0, one per hour; sawtooth of discharges and charges).
- Devices: G703 on, 23%, discharging, "~3 h de uso", "acaba hoje ~17:40", warning "Não chega ao fim do dia: carregue antes das 18h", 4 h 10 min, health 94. G915 X on, 46%, charging, "cheio em ~1 h 10 min", 6 h 05 min, health 98. G733 off, last seen "71% às 14:02", 1 h 30 min, health 89. Controle Xbox on, Bluetooth, 64%, "~18 h de uso", health null (learning). AirPods Pro (Mac only) on, Bluetooth, 80%, "~4 h de uso", health null.
- `pinned`: the ids the bar item shows by default (`g703`, `g915x`, `g733`); the panel lists every device.
- `notifications.warning` and `notifications.full` (`title`, `body`, `text`; full text: "G915 X carregado: pode tirar do cabo").

## What every option must show

JuiceBar is not Logitech-only: devices come from providers (Logitech HID++, any Bluetooth device that reports battery, game controllers, other brands). Every design must hold three to eight devices: the Mac scene has five, the Windows scene four. The bar item shows the pinned devices (or a summary), never all of them by force.

Mac: bar item; panel with every device and their states (on, charging, off, low with warning) and, for a device, chart of the level history, usage today, battery health, the RGB economy toggle and a settings entry (gear); widget; one notification (smart warning or full charge).
Windows: tray icon(s); flyout with the same content as the panel; one toast.
Both themes, both platforms, same data. Text in pt-BR, no emoji, no em dash.

## Rendering

```
powershell -NoProfile -File scripts\render_design.ps1 -Option <n-slug>
```

No `-Option` renders every folder under `docs/design/options/`. Output: `docs/design/renders/<n-slug>/{mac-light,mac-dark,win-light,win-dark}.png` (ignored by git) and the gallery `docs/design/index.html`. The script uses headless Chrome (`--headless=new --force-device-scale-factor=S --window-size=W,H`; measured: these flags give exactly 2400 x 1500 for both scenes), fails if a PNG is not 2400 x 1500 or if Chrome logs a console error or uncaught exception for the page. Open the PNGs and check nothing is clipped before handing over.
