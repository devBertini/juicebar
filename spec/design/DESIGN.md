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
- Header: "JuiceBar" and the gear, which opens the Settings page inside the same window (below).
- One row per device: glyph in a tinted circle, name, connection label, percent (large, tabular digits), a slim capsule, and one line of status: "~3 h de uso", "cheio em ~1 h 10 min", or "desligado · 71% às 14:02". Devices that are on come first; off devices after a quiet "Desligados" separator.
- A click on a row expands it (one at a time; a device with a warning opens expanded): the smart warning (if any), a 7-day level chart (area line) with a dashed forecast to 0%, "Hoje: <tempo> de uso", hours of use per day for the last 7 days as small bars, and "Saúde da bateria: <n>%" ("aprendendo" until three cycles). A device without a reading shows "–", no capsule fill and no red.
- Footer: the "Economia de Energia" switch (English "Power saver") with a small (i) whose hover or click says it turns off the devices' LEDs so their batteries last longer, and the hotkey hint, which shows the shortcut in use (Ctrl+Alt+B unless the user changed it).
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

## The Settings page (Windows)
The gear swaps the list for the Settings page in the same window: a header with a back arrow (Esc also goes back; while a field or a shortcut is being typed, Esc only ends that), then titled cards of rows like Windows' own Settings, scrolling when the work area is short. Each row has a label, an optional quiet second line and one control at its right end. The sections, top to bottom:
- **Aparência (Appearance):** theme (Automático, Claro, Escuro; the flyout's colors only), icon animation, and, only while Windows' transparency effects are off, a link that opens that Windows page.
- **Ícone (Icon):** the green limit (25 to 95 %) and the red limit (5 to 50 %), steppers that keep the red limit at least 5 under the green one; the row that was moved says so.
- **Alertas (Alerts):** the low-battery level (5 to 30 %), smart warning, full charge notice and do not disturb (no balloon at all; the icon still changes).
- **Dispositivos (Devices):** one row per known device, seen or hidden (a device hidden in an earlier run and not seen yet is listed too), showing its glyph, its name and its connection. Four controls per row: a pencil (rename), an up and a down arrow (order) and a switch (show in the panel, the tooltip and the icon's color rule). A hidden device is dimmed and its second line adds "oculto". The order is the user's: ordered devices first, then the rest with those that are on first. **Rename:** the pencil turns the row into a text field with the current name selected; typing replaces it, Backspace erases, Enter saves, Esc cancels; at most 24 characters; an empty name (or the catalog's own) goes back to the catalog's short name. The name applies everywhere: panel, tooltip, notifications and warnings. Keyboard: Tab reaches each row; Space shows or hides, Enter renames, Left and Right move the device up and down.
- **Geral (General):** start with Windows, language (Automático, Português, English) and the shortcut. The shortcut row reads "Atalho: Ctrl+Alt+B" with "Alterar": it waits for the next key combination (at least two of Ctrl, Alt, Shift and Win held with a letter, a digit or an F key, so Ctrl+C, Alt+F4 and Win+L stay the system's; Esc cancels; any other combination is refused with "Use pelo menos dois modificadores, como Ctrl+Alt" and the wait goes on), then registers it. When another program holds that combination the old shortcut stays and the row says "Esse atalho já está em uso". While the shortcut is not the default a second row, "Restaurar padrão (Ctrl+Alt+B)", brings it back.
- **Sobre (About):** the mark, the name and version, "Idealizado por Claudio Bertini" with the author's name, links to the code on GitHub and the license, and the line that nothing leaves the computer.

Every change takes effect at once and is saved in `settings.txt` (`device.<key>.shown`, `.name` and `.order`, `hotkey`; see `windows/app/src/settings.rs`). The tray icon's right-click menu holds three entries: Abrir (the list), Configurações (this page) and Sair.

## Widget (Mac) and notifications
- Medium widget: the devices as compact rows (glyph, name, capsule, percent), "<n> ligados" in the header.
- Notifications use the system's own banner and toast; title "<device> com <n>%", body the warning or "pode tirar do cabo".

## Glyphs
Our own 24 x 24 line drawings (stroke 1.75, round caps), never a brand's logo: mouse, keyboard, headset, gamepad, earbuds, bolt, Bluetooth, wireless, warning, gear, leaf. Source: `docs/design/harness/glyphs.js`; each app draws them from these paths (Core Graphics on the Mac, Direct2D on Windows), so both apps show the same shapes.
