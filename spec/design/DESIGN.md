# JuiceBar design: the Native direction

The owner chose option 1, Native, with one change: a single icon in the menu bar and in the tray, never one per device, so JuiceBar takes the least room in anyone's bar. The reference drawing is `docs/design/options/1-native/` (render it with `scripts/render_design.ps1 -Option 1-native`); where the drawing and this file differ, this file wins.

## The bar item and the tray icon
- One icon only: the JuiceBar mark, a juice glass with a straw (never one icon per device, so JuiceBar takes the least room in anyone's bar).
- The glass outline and the straw take the bar's own color (the taskbar's foreground on Windows, redrawn when the theme changes; the menu bar's label color on the Mac); the juice is colored by the state: green when every shown device is above the high limit (default 50 %), red when any is below the low limit (default 20 %) or a smart warning is active, purple like grape juice otherwise. Both limits are settings.
- Motion, gentle and rare: the liquid sloshes and the straw sways for about 2 seconds when the state changes and once in a while (about every 15 minutes); while a device charges, a few bubbles rise now and then. No motion when the system's animation effects are off. Nothing else changes in the icon.
- Hover shows every device in one line each (Windows tooltip; on the Mac the panel is one click away). Left click opens the panel; right click offers only Abrir, Configurações and Sair.

## The panel (Mac) and the flyout (Windows)
- Looks like part of the system: on the Mac a Liquid Glass sheet like Control Center (system material, 18 to 22 pt corner radius); on Windows a Quick Settings-like flyout (real acrylic backdrop showing the blurred desktop, 8 px radius, 1 px subtle stroke, Segoe UI Variable). A soft glow at the top of the panel takes the state's juice color (purple, green or red); the device rows sit on light glass cards over it.
- Device names are short (the catalog's model, such as "G703"; the user may rename them); a device's tile uses a neutral juice tint, never the system accent (a red accent read as an alarm).
- Header: "JuiceBar" and the gear (settings).
- One row per device: glyph in a tinted circle, name, connection label, percent (large, tabular digits), a slim capsule, and one line of status: "~3 h de uso", "cheio em ~1 h 10 min", or "desligado · 71% às 14:02". Devices that are on come first; off devices after a quiet "Desligados" separator.
- A click on a row expands it (one at a time; a device with a warning opens expanded): the smart warning (if any), a 7-day level chart (area line) with a dashed forecast to 0%, "Hoje: <tempo> de uso", hours of use per day for the last 7 days as small bars, and "Saúde da bateria: <n>%" ("aprendendo" until three cycles). A device without a reading shows "–", no capsule fill and no red.
- Footer: the "Economia de Energia" switch (English "Power saver") with a small (i) whose hover or click says it turns off the devices' LEDs so their batteries last longer, and the hotkey hint.
- Colors come from the system: on the Mac `labelColor`, `secondaryLabelColor`, `tertiaryLabelColor`, `controlAccentColor`, `systemRed`, `systemGreen`, `systemOrange`; on Windows the theme's foreground and the user's accent color. The mockup's values (below) are only the approximation used in the drawing.

| Token (mockup) | Light | Dark |
|---|---|---|
| text | `#1d1d1f` | `#f5f5f7` |
| secondary text | `rgba(29,29,31,.62)` | `rgba(245,245,247,.64)` |
| tertiary text | `rgba(29,29,31,.42)` | `rgba(245,245,247,.42)` |
| accent | `#0a74e8` | `#3b9bff` |
| red (low, warning) | `#e5322d` | `#ff5a52` |
| green (charging) | `#1fa84a` | `#32d36b` |
| orange (warning text) | `#c9700a` | `#ffa940` |
| glass | `rgba(246,246,249,.74)` | `rgba(44,44,52,.68)` |

## Widget (Mac) and notifications
- Medium widget: the devices as compact rows (glyph, name, capsule, percent), "<n> ligados" in the header.
- Notifications use the system's own banner and toast; title "<device> com <n>%", body the warning or "pode tirar do cabo".

## Glyphs
Our own 24 x 24 line drawings (stroke 1.75, round caps), never a brand's logo: mouse, keyboard, headset, gamepad, earbuds, bolt, Bluetooth, wireless, warning, gear, leaf. Source: `docs/design/harness/glyphs.js`; each app draws them from these paths (Core Graphics on the Mac, Direct2D on Windows), so both apps show the same shapes.
