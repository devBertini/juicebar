// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Paints the flyout onto a `Canvas`. The window and the PNG seam both call `paint_flyout`, so a
//! render is what the user sees (apart from the system backdrop, replaced by a solid panel).
//!
//! The look: the panel is the system's acrylic (or, where Windows has transparency effects off, a
//! solid panel we paint), a soft glow at its top in the juice color of the tray mark (purple,
//! green or red), and one light glass card per device. Tiles, capsules and the chart take the
//! mark's juice colors, never the system accent; only the switch keeps the accent.

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::core::Result;

use crate::gfx::{Align, Canvas, Color, Gfx, IDENTITY, mul, rotate_about, scale_translate};
use crate::glyphs::{self, Shape};
use crate::icon::{self, Frame, IconLook, Juice};
use crate::layout::{
    DetailGeom, FlyoutHit, INFO_TEXT_W, Layout, Metrics, Rect, RowGeom, WELCOME_STEPS_W,
    WELCOME_TEXT_W, WIDTH,
};
use crate::model::{Glyph, Rgb, Theme, UiDevice, UiState, UiWelcome};

/// The product name in the header; every other label comes from `UiState::labels`.
pub const TITLE: &str = "JuiceBar";

/// The theme's colors (the mockup's Windows tokens, `spec/design/DESIGN.md`).
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub panel: Color,
    pub fg: Color,
    pub fg2: Color,
    pub fg3: Color,
    pub stroke: Color,
    pub tile: Color,
    pub fill: Color,
    pub hair: Color,
    pub hover: Color,
    pub accent: Color,
    pub red: Color,
    pub green: Color,
    pub orange: Color,
    pub off_bg: Color,
    pub off_fg: Color,
    /// Solid color under the card, for the "now" dot's ring.
    pub card_solid: Color,
    /// A device's glass card: white (light) or a faint white (dark) at low alpha.
    pub card: Color,
    pub card_open: Color,
    pub card_hover: Color,
    pub card_edge: Color,
    /// The opaque surface of the (i) bubble.
    pub bubble: Color,
}

impl Palette {
    pub fn new(theme: Theme, accent: Rgb) -> Palette {
        let accent =
            Color::hex(((accent.0 as u32) << 16) | ((accent.1 as u32) << 8) | accent.2 as u32);
        match theme {
            Theme::Light => Palette {
                dark: false,
                panel: Color::hex(0xf3f3f6),
                fg: Color::hex(0x1d1d1f),
                fg2: Color::rgba(0x1d1d1f, 0.62),
                fg3: Color::rgba(0x1d1d1f, 0.42),
                stroke: Color::rgba(0x000000, 0.10),
                tile: Color::rgba(0xffffff, 0.72),
                fill: Color::rgba(0x000000, 0.14),
                hair: Color::rgba(0x3c3c43, 0.12),
                hover: Color::rgba(0x000000, 0.045),
                accent,
                red: Color::hex(0xc42b1c),
                green: Color::hex(0x107c10),
                orange: Color::hex(0xc9700a),
                off_bg: Color::rgba(0x3c3c43, 0.14),
                off_fg: Color::rgba(0x1d1d1f, 0.55),
                card_solid: Color::hex(0xfbfbfc),
                card: Color::rgba(0xffffff, 0.62),
                card_open: Color::rgba(0xffffff, 0.78),
                card_hover: Color::rgba(0xffffff, 0.84),
                card_edge: Color::rgba(0x000000, 0.08),
                bubble: Color::hex(0xfbfbfc),
            },
            Theme::Dark => Palette {
                dark: true,
                panel: Color::hex(0x262629),
                fg: Color::hex(0xf5f5f7),
                fg2: Color::rgba(0xf5f5f7, 0.64),
                fg3: Color::rgba(0xf5f5f7, 0.42),
                stroke: Color::rgba(0xffffff, 0.10),
                tile: Color::rgba(0xffffff, 0.055),
                fill: Color::rgba(0xffffff, 0.20),
                hair: Color::rgba(0xffffff, 0.10),
                hover: Color::rgba(0xffffff, 0.06),
                accent,
                red: Color::hex(0xff99a4),
                green: Color::hex(0x6ccb5f),
                orange: Color::hex(0xffa940),
                off_bg: Color::rgba(0xffffff, 0.14),
                off_fg: Color::rgba(0xf5f5f7, 0.55),
                card_solid: Color::hex(0x2d2d31),
                card: Color::rgba(0xffffff, 0.065),
                card_open: Color::rgba(0xffffff, 0.09),
                card_hover: Color::rgba(0xffffff, 0.12),
                card_edge: Color::rgba(0xffffff, 0.10),
                bubble: Color::hex(0x38383e),
            },
        }
    }

    /// The juice colors of the tray mark, as the mark draws them on a taskbar of this theme.
    pub fn juice(&self, juice: Juice) -> Color {
        icon::juice_color(juice, self.dark)
    }

    /// Dark text on a light fill, white on a dark one.
    pub fn on(&self, bg: Color) -> Color {
        if bg.luminance() > 0.4 {
            Color::hex(0x0b1a24)
        } else {
            Color::hex(0xffffff)
        }
    }

    /// The glow at the top of the panel: the state's juice color at about 12 to 20 % alpha.
    pub fn glow(&self, juice: Juice) -> Color {
        self.juice(juice).fade(if self.dark { 0.20 } else { 0.17 })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    On,
    Low,
    Charging,
    Off,
}

fn tone(state: &UiState, d: &UiDevice) -> Tone {
    if !d.on {
        Tone::Off
    } else if state.is_low(d) {
        Tone::Low
    } else if d.charging {
        Tone::Charging
    } else {
        Tone::On
    }
}

/// What to paint besides the state.
#[derive(Clone, Copy)]
pub struct PaintOptions {
    /// Paint the panel itself (rounded, solid) instead of leaving it to the DWM backdrop.
    pub solid_panel: bool,
    /// Clear color behind the panel (the PNG seam's "wallpaper"); `None` clears to transparent.
    pub clear: Option<Color>,
    /// The control under the pointer.
    pub hover: FlyoutHit,
    /// How far the Settings page is scrolled, in DIPs (0 for every other page).
    pub scroll: f32,
}

struct Fonts {
    name: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    conn: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    pct: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    pct_small: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    title: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    small: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    axis: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    warn: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    stat: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    rgb: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    kbd: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    welcome: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    body: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
}

impl Fonts {
    fn new(g: &Gfx) -> Result<Fonts> {
        let (n, m, s) = (
            DWRITE_FONT_WEIGHT_NORMAL.0,
            DWRITE_FONT_WEIGHT_MEDIUM.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD.0,
        );
        Ok(Fonts {
            name: g.text_format(false, s, 13.5)?,
            conn: g.text_format(false, n, 11.5)?,
            pct: g.text_format(true, s, 20.0)?,
            pct_small: g.text_format(false, m, 12.0)?,
            title: g.text_format(false, s, 14.0)?,
            small: g.text_format(false, n, 11.0)?,
            axis: g.text_format(false, n, 8.5)?,
            warn: g.text_format(false, m, 12.0)?,
            stat: g.text_format(false, s, 12.5)?,
            rgb: g.text_format(false, m, 13.0)?,
            kbd: g.text_format(false, n, 11.5)?,
            welcome: g.text_format(true, s, 19.0)?,
            body: g.text_format(false, n, 13.0)?,
        })
    }
}

/// Measures what the layout needs: the footer label and hotkey widths and the heights of the
/// wrapped texts (the (i) bubble and, on the first open, the welcome page).
pub fn measure_metrics(g: &Gfx, state: &UiState) -> Result<Metrics> {
    let f = Fonts::new(g)?;
    let d = Metrics::default();
    let (welcome_body_h, welcome_steps_h) = match &state.welcome {
        Some(w) => (
            g.measure_wrapped(&w.body, &f.body, WELCOME_TEXT_W)?.ceil(),
            g.measure_wrapped(&w.tray_steps, &f.conn, WELCOME_STEPS_W)?
                .ceil(),
        ),
        None => (d.welcome_body_h, d.welcome_steps_h),
    };
    let settings = match &state.settings {
        Some(ui) => crate::settings_draw::measure_settings(g, ui)?,
        None => d.settings.clone(),
    };
    Ok(Metrics {
        settings,
        max_height: d.max_height,
        rgb_label_w: g.measure(&state.labels.rgb_economy, &f.rgb)?.0.ceil(),
        hotkey_w: g.measure(&state.hotkey_text, &f.kbd)?.0.ceil(),
        info_h: g
            .measure_wrapped(&state.labels.rgb_info, &f.small, INFO_TEXT_W)?
            .ceil(),
        welcome_body_h,
        welcome_steps_h,
    })
}

/// Draws a glyph into the square `r`, scaling the 24 x 24 view box.
pub fn draw_glyph(
    c: &Canvas,
    g: Glyph,
    r: Rect,
    color: Color,
    filled: bool,
    stroke: f32,
) -> Result<()> {
    let s = r.w / glyphs::VIEWBOX;
    let base = scale_translate(s, r.x, r.y);
    c.set_transform(base);
    let result = (|| -> Result<()> {
        for sh in glyphs::shapes(g) {
            match *sh {
                Shape::Path(d) => {
                    let geo = c.path(d, filled)?;
                    if filled {
                        c.fill_geometry(&geo.into(), color);
                    } else {
                        c.stroke_geometry(&geo.into(), stroke, color, None);
                    }
                }
                Shape::Rotated(d, deg) => {
                    c.set_transform(mul(rotate_about(deg, 12.0, 12.0), base));
                    let geo = c.path(d, false)?;
                    c.stroke_geometry(&geo.into(), stroke, color, None);
                    c.set_transform(base);
                }
                Shape::Rect { x, y, w, h, rx } => {
                    let rr = Rect::new(x, y, w, h);
                    if filled {
                        c.fill_rrect(rr, rx, color);
                    }
                    c.stroke_rrect(
                        Rect::new(x - stroke / 2.0, y - stroke / 2.0, w + stroke, h + stroke),
                        rx + stroke / 2.0,
                        stroke,
                        color,
                    );
                }
                Shape::Circle { cx, cy, r } => {
                    if filled {
                        c.fill_circle(cx, cy, r, color);
                    }
                    c.stroke_circle(cx, cy, r, stroke, color);
                }
            }
        }
        Ok(())
    })();
    c.set_transform(IDENTITY);
    result
}

/// Paints the whole flyout. `layout` must come from `Layout::compute(state, measure_metrics(..))`.
pub fn paint_flyout(
    c: &Canvas,
    state: &UiState,
    layout: &Layout,
    opts: PaintOptions,
) -> Result<()> {
    let p = Palette::new(state.theme, state.accent);
    let f = Fonts::new(c.gfx)?;
    c.clear(opts.clear.unwrap_or(Color::rgba(0, 0.0)));
    let full = Rect::new(0.0, 0.0, layout.width, layout.height);
    if opts.solid_panel {
        c.fill_rrect(full, 8.0, p.panel);
    }
    paint_glow(c, &p, state.juice, full)?;
    if opts.solid_panel {
        c.stroke_rrect(full, 8.0, c.hairline(), p.stroke);
    }
    if let (Some(w), Some(page)) = (&layout.welcome, &state.welcome) {
        return paint_welcome(c, state, &p, &f, w, page, opts.hover);
    }
    if let (Some(g), Some(ui)) = (&layout.settings, &state.settings) {
        return crate::settings_draw::paint_settings(
            c,
            state,
            ui,
            g,
            &p,
            opts.hover,
            opts.scroll,
            layout.height,
        );
    }

    // Header.
    c.text_in(TITLE, &f.title, layout.title, Align::Left, p.fg)?;
    if opts.hover == FlyoutHit::Gear {
        c.fill_rrect(layout.gear, 4.0, p.hover);
    }
    let g = layout.gear;
    draw_glyph(
        c,
        Glyph::Gear,
        Rect::new(g.x + 5.0, g.y + 5.0, 16.0, 16.0),
        p.fg2,
        false,
        glyphs::STROKE,
    )?;

    for row in &layout.rows {
        paint_row(c, state, &p, &f, row, opts.hover)?;
    }

    if let Some(r) = layout.empty {
        let mut lines = state.empty_text.lines();
        let (title, hint) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""));
        let half = r.h / 2.0;
        c.text_in(
            title,
            &f.stat,
            Rect::new(r.x, r.y, r.w, half),
            Align::Center,
            p.fg2,
        )?;
        c.text_in(
            hint,
            &f.small,
            Rect::new(r.x, r.y + half, r.w, half),
            Align::Center,
            p.fg3,
        )?;
    }

    if let Some(sep) = layout.separator {
        let t = c.text_layout(
            &state.labels.off_devices,
            &f.small,
            sep.w,
            sep.h,
            Align::Left,
        )?;
        c.draw_text_at(&t, sep.x, sep.y + (sep.h - t.h) / 2.0, p.fg3);
        let y = c.snap(sep.cy());
        c.line(
            (sep.x + t.w + 8.0, y),
            (sep.right(), y),
            c.hairline(),
            p.hair,
            None,
        );
    }

    paint_footer(c, state, layout, &p, &f, opts.hover)
}

/// The soft glow at the top of the panel: a wide ellipse centered on the top edge that fades out.
fn paint_glow(c: &Canvas, p: &Palette, juice: Juice, full: Rect) -> Result<()> {
    let glow = p.glow(juice);
    c.fill_rrect_radial(
        full,
        8.0,
        (full.w / 2.0, -6.0),
        full.w * 0.95,
        full.h.min(300.0),
        &[
            (0.0, glow),
            (0.4, glow.fade(0.5)),
            (0.75, glow.fade(0.14)),
            (1.0, glow.fade(0.0)),
        ],
    )
}

fn paint_footer(
    c: &Canvas,
    state: &UiState,
    layout: &Layout,
    p: &Palette,
    f: &Fonts,
    hover: FlyoutHit,
) -> Result<()> {
    let ft = &layout.footer;
    let y = c.snap(ft.line_y);
    c.line(
        (14.0, y),
        (layout.width - 14.0, y),
        c.hairline(),
        p.hair,
        None,
    );
    if hover == FlyoutHit::RgbSwitch {
        c.fill_rrect(ft.switch_hit, 6.0, p.hover);
    }
    draw_glyph(
        c,
        Glyph::Leaf,
        ft.icon,
        p.juice(Juice::Green),
        false,
        glyphs::STROKE,
    )?;
    c.text_in(
        &state.labels.rgb_economy,
        &f.rgb,
        ft.label,
        Align::Left,
        p.fg,
    )?;
    let info_on = hover == FlyoutHit::Info || state.info_open;
    paint_info_icon(c, p, ft.info, info_on);
    paint_switch(c, p, ft.switch, state.rgb_economy, 1.0);
    c.fill_rrect(ft.hotkey, 4.0, p.tile);
    c.stroke_rrect(ft.hotkey, 4.0, c.hairline(), p.stroke);
    c.text_in(&state.hotkey_text, &f.kbd, ft.hotkey, Align::Center, p.fg2)?;
    if let (Some(r), Some(text)) = (ft.ghub, state.ghub_notice.as_ref()) {
        c.text_in(text, &f.small, r, Align::Left, p.orange)?;
    }
    if info_on {
        paint_info_bubble(c, state, p, f, ft.bubble, ft.info)?;
    }
    Ok(())
}

/// The small (i): a ring with a dot and a stem.
fn paint_info_icon(c: &Canvas, p: &Palette, r: Rect, on: bool) {
    let color = if on { p.fg } else { p.fg3 };
    let (cx, cy) = (r.x + r.w / 2.0, r.cy());
    c.stroke_circle(cx, cy, r.w / 2.0 - 0.75, 1.2, color);
    c.fill_circle(cx, cy - 3.6, 0.95, color);
    c.line((cx, cy - 1.0), (cx, cy + 4.0), 1.4, color, None);
}

/// What the (i) says, in a bubble above the footer with its tip on the (i).
fn paint_info_bubble(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &Fonts,
    bubble: Rect,
    info: Rect,
) -> Result<()> {
    c.fill_rrect(bubble, 8.0, p.bubble);
    let tip_x = (info.x + info.w / 2.0).clamp(bubble.x + 16.0, bubble.right() - 16.0);
    let by = bubble.bottom();
    let caret = c.polyline(
        &[
            (tip_x - 6.0, by - 0.5),
            (tip_x + 6.0, by - 0.5),
            (tip_x, by + 5.5),
        ],
        true,
        true,
    )?;
    c.fill_geometry(&caret.into(), p.bubble);
    c.stroke_rrect(bubble, 8.0, c.hairline(), p.card_edge);
    c.text_wrapped(
        &state.labels.rgb_info,
        &f.small,
        Rect::new(
            bubble.x + 12.0,
            bubble.y + 10.0,
            INFO_TEXT_W,
            bubble.h - 20.0,
        ),
        Align::Left,
        p.fg,
    )?;
    Ok(())
}

/// The 40 x 20 switch; `quiet` (1.0 normal) fades a disabled one.
pub(crate) fn paint_switch(c: &Canvas, p: &Palette, r: Rect, on: bool, quiet: f32) {
    let knob_y = r.cy();
    if on {
        c.fill_rrect(r, r.h / 2.0, p.accent.fade(quiet));
        c.fill_circle(r.x + 30.0, knob_y, 6.0, p.on(p.accent).fade(quiet));
    } else {
        c.stroke_rrect(r, r.h / 2.0, 1.0, p.fg2.fade(quiet));
        c.fill_circle(r.x + 10.0, knob_y, 6.0, p.fg2.fade(quiet));
    }
}

/// The first-open page: the mark, a welcome, what JuiceBar does, how to keep its icon next to the
/// clock, who made it, and the button.
fn paint_welcome(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &Fonts,
    w: &crate::layout::WelcomeGeom,
    page: &UiWelcome,
    hover: FlyoutHit,
) -> Result<()> {
    c.set_transform(scale_translate(1.0, w.mark.x, w.mark.y));
    let look = IconLook {
        juice: state.juice,
        charging: false,
    };
    let drawn = icon::draw_mark(c, look, &Frame::STILL, p.dark, w.mark.w);
    c.set_transform(IDENTITY);
    drawn?;
    c.text_in(&page.title, &f.welcome, w.title, Align::Left, p.fg)?;
    c.text_wrapped(&page.body, &f.body, w.body, Align::Left, p.fg)?;

    c.fill_rrect(w.tray_card, 8.0, p.card_open);
    c.stroke_rrect(w.tray_card, 8.0, c.hairline(), p.card_edge);
    c.text_in(&page.tray_title, &f.stat, w.tray_title, Align::Left, p.fg)?;
    c.text_wrapped(&page.tray_steps, &f.conn, w.tray_steps, Align::Left, p.fg2)?;

    c.text_in(&page.credit, &f.small, w.credit, Align::Left, p.fg3)?;

    let fill = if hover == FlyoutHit::Start {
        Color::hex(0x9456bd)
    } else {
        Color::hex(0x7e3fa8)
    };
    c.fill_rrect(w.button, 6.0, fill);
    c.text_in(
        &page.start,
        &f.rgb,
        w.button,
        Align::Center,
        Color::hex(0xffffff),
    )?;
    debug_assert!(w.button.right() <= WIDTH);
    Ok(())
}

fn paint_row(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &Fonts,
    row: &RowGeom,
    hover: FlyoutHit,
) -> Result<()> {
    let d = &state.devices[row.index];
    let t = tone(state, d);

    // The glass card: every device has one; an open one is a little more present, an off one quieter.
    let card = if row.detail.is_some() {
        p.card_open
    } else if hover == FlyoutHit::Row(row.index) {
        p.card_hover
    } else {
        p.card
    };
    let quiet = if t == Tone::Off { 0.7 } else { 1.0 };
    c.fill_rrect(row.bounds, 8.0, card.fade(quiet));
    c.stroke_rrect(row.bounds, 8.0, c.hairline(), p.card_edge);

    // Glyph tile: a neutral tint of the mark's juice (grape for a device that is on, green while it
    // charges, red when low), never the system accent.
    let juice_of = |j| p.juice(j);
    let (bg, fg) = match t {
        Tone::On => (juice_of(Juice::Purple).fade(0.2), juice_of(Juice::Purple)),
        Tone::Charging => (juice_of(Juice::Green).fade(0.2), juice_of(Juice::Green)),
        Tone::Low => (juice_of(Juice::Red).fade(0.2), juice_of(Juice::Red)),
        Tone::Off => (p.off_bg, p.off_fg),
    };
    c.fill_rrect(row.icon, 8.0, bg);
    let ic = row.icon;
    draw_glyph(
        c,
        d.glyph,
        Rect::new(ic.x + 8.5, ic.y + 8.5, 17.0, 17.0),
        fg,
        false,
        glyphs::STROKE,
    )?;

    // Name and connection share a baseline.
    let name_color = if t == Tone::Off { p.fg2 } else { p.fg };
    let name = c.text_layout(&d.name, &f.name, row.title.w, row.title.h, Align::Left)?;
    let top = row.title.y + (row.title.h - name.h) / 2.0;
    c.draw_text_at(&name, row.title.x, top, name_color);
    if !d.connection.is_empty() {
        let cx = row.title.x + name.w + 7.0;
        let avail = row.title.right() - cx;
        if avail > 20.0 {
            let conn = c.text_layout(&d.connection, &f.conn, avail, row.title.h, Align::Left)?;
            c.draw_text_at(&conn, cx, top + name.baseline - conn.baseline, p.fg2);
        }
    }

    // Capsule: the track always, the fill only with a reading.
    let cap = row.capsule;
    c.fill_rrect(cap, 2.0, p.fill);
    let frac = if d.percent.is_some() {
        d.capsule_fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if frac > 0.0 {
        let fill = match t {
            Tone::On => juice_of(Juice::Purple),
            Tone::Low => juice_of(Juice::Red),
            Tone::Charging => juice_of(Juice::Green),
            Tone::Off => p.fg.fade(0.3),
        };
        c.fill_rrect(
            Rect::new(cap.x, cap.y, (cap.w * frac).max(cap.h), cap.h),
            2.0,
            fill,
        );
    }

    // Status line, with the bolt while charging.
    let mut sx = row.status.x;
    if d.charging && d.on {
        draw_glyph(
            c,
            Glyph::Bolt,
            Rect::new(sx, row.status.cy() - 5.5, 11.0, 11.0),
            p.green,
            true,
            2.0,
        )?;
        sx += 14.0;
    }
    c.text_in(
        &d.status_line,
        &f.conn,
        Rect::new(sx, row.status.y, row.status.right() - sx, row.status.h),
        Align::Left,
        p.fg2,
    )?;

    // Percent: big digits, a small "%" on the same baseline; without a reading a quiet dash.
    let pc = row.percent;
    match d.percent {
        Some(percent) => {
            let color = match t {
                Tone::Low => p.red,
                Tone::Off => p.fg2,
                _ => p.fg,
            };
            let digits = c.text_layout(&percent.to_string(), &f.pct, pc.w, pc.h, Align::Left)?;
            let sign = c.text_layout("%", &f.pct_small, pc.w, pc.h, Align::Left)?;
            let total = digits.w + 1.0 + sign.w;
            let x = pc.right() - total;
            let top = pc.cy() - digits.h / 2.0;
            c.draw_text_at(&digits, x, top, color);
            c.draw_text_at(
                &sign,
                x + digits.w + 1.0,
                top + digits.baseline - sign.baseline,
                p.fg2,
            );
        }
        None => c.text_in("\u{2013}", &f.pct, pc, Align::Right, p.fg3)?,
    }

    if let Some(detail) = &row.detail {
        paint_detail(c, state, p, f, d, detail)?;
    }
    Ok(())
}

/// Scale of the week's bars: the largest day rounded up to a whole hour, at least one.
pub fn week_scale(hours: &[f32; 7]) -> f32 {
    hours.iter().copied().fold(0.0f32, f32::max).ceil().max(1.0)
}

fn paint_detail(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &Fonts,
    d: &UiDevice,
    g: &DetailGeom,
) -> Result<()> {
    let purple = p.juice(Juice::Purple);
    if let (Some(w), Some(msg)) = (g.warn, d.warning.as_ref()) {
        c.fill_rrect(w, 4.0, p.orange.fade(0.14));
        draw_glyph(
            c,
            Glyph::Warning,
            Rect::new(w.x + 10.0, w.cy() - 7.5, 15.0, 15.0),
            p.orange,
            false,
            glyphs::STROKE,
        )?;
        c.text_in(
            msg,
            &f.warn,
            Rect::new(w.x + 32.0, w.y, w.w - 40.0, w.h),
            Align::Left,
            p.orange,
        )?;
    }

    // Chart title and the forecast key.
    c.text_in(
        &state.labels.last_7_days,
        &f.small,
        Rect::new(
            g.chart_title.x + 2.0,
            g.chart_title.y,
            150.0,
            g.chart_title.h,
        ),
        Align::Left,
        p.fg2,
    )?;
    let key = c.text_layout(
        &state.labels.forecast,
        &f.small,
        120.0,
        g.chart_title.h,
        Align::Left,
    )?;
    let kx = g.chart_title.right() - 2.0 - key.w;
    c.draw_text_at(
        &key,
        kx,
        g.chart_title.y + (g.chart_title.h - key.h) / 2.0,
        p.fg2,
    );
    let dash = c.dash_style(&[2.0, 2.0])?;
    let ky = g.chart_title.cy();
    c.line(
        (kx - 5.0 - 12.0, ky),
        (kx - 5.0, ky),
        1.5,
        p.red,
        Some(&dash),
    );

    // Chart.
    let ch = g.chart;
    let line_w = c.hairline();
    for pct in [0.0f32, 50.0, 100.0] {
        let y = c.snap(ch.y_of(pct));
        c.line((ch.plot.x, y), (ch.plot.right(), y), line_w, p.hair, None);
        c.text_in(
            &format!("{pct:.0}"),
            &f.axis,
            Rect::new(ch.area.x, y - 6.0, ch.plot.x - 5.0 - ch.area.x, 12.0),
            Align::Right,
            p.fg3,
        )?;
    }
    let dots = c.dash_style(&[0.01, 3.0])?;
    let ty = c.snap(ch.y_of(state.low_threshold as f32));
    c.line(
        (ch.plot.x, ty),
        (ch.plot.right(), ty),
        0.8,
        p.red.fade(0.55),
        Some(&dots),
    );

    if d.chart.len() >= 2 {
        let pts: Vec<(f32, f32)> = d
            .chart
            .iter()
            .map(|&(h, pc)| (ch.x_of(h), ch.y_of(pc)))
            .collect();
        let mut area = pts.clone();
        area.push((ch.x_of(0.0), ch.y_of(0.0)));
        area.push((pts[0].0, ch.y_of(0.0)));
        let ag = c.polyline(&area, true, true)?;
        c.push_clip(ch.plot);
        c.fill_geometry_vgradient(
            &ag.into(),
            ch.plot.y,
            ch.plot.bottom(),
            purple.fade(0.34),
            purple.fade(0.0),
        )?;
        c.pop_clip();
        let lg = c.polyline(&pts, false, false)?;
        c.stroke_geometry(&lg.into(), 1.4, purple, None);
    }
    if let Some((h, pc)) = d.forecast_from {
        let (x0, y0) = (ch.x_of(h), ch.y_of(pc));
        let x1 = ch.x_of(d.forecast_empty_hours);
        c.line((x0, y0), (x1, ch.y_of(0.0)), 1.5, p.red, Some(&dash));
        c.fill_circle(x0, y0, 3.0, purple);
        c.stroke_circle(x0, y0, 3.0, 1.5, p.card_solid);
        c.fill_circle(x0, y0, 2.25, purple);
    }
    for (i, label) in state.day_labels.iter().enumerate() {
        let x = ch.x_of(-168.0 + 12.0 + i as f32 * 24.0);
        c.text_in(
            label,
            &f.axis,
            Rect::new(x - 20.0, ch.area.bottom() - 11.0, 40.0, 12.0),
            Align::Center,
            p.fg3,
        )?;
    }

    // Stats.
    let y = c.snap(g.stats_top);
    c.line(
        (g.today.x, y),
        (g.health.right(), y),
        c.hairline(),
        p.hair,
        None,
    );
    let label_row = |r: Rect| Rect::new(r.x, r.y, r.w, 15.0);
    let value_row = |r: Rect| Rect::new(r.x, r.y + 16.0, r.w, 17.0);
    c.text_in(
        &state.labels.today,
        &f.small,
        label_row(g.today),
        Align::Left,
        p.fg2,
    )?;
    c.text_in(
        &format!("{} {}", d.usage_today, state.labels.of_use),
        &f.stat,
        value_row(g.today),
        Align::Left,
        p.fg,
    )?;
    c.text_in(
        &state.labels.battery_health,
        &f.small,
        label_row(g.health),
        Align::Left,
        p.fg2,
    )?;
    let health = d
        .health
        .map_or_else(|| state.labels.learning.clone(), |h| format!("{h}%"));
    c.text_in(&health, &f.stat, value_row(g.health), Align::Left, p.fg)?;

    paint_week_bars(c, state, p, f, d, g)
}

/// Hours of use on each of the last 7 days: small rounded bars on a common scale, today's solid,
/// the weekday under each, the scale (the tallest whole hour) at the right of the title.
fn paint_week_bars(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &Fonts,
    d: &UiDevice,
    g: &DetailGeom,
) -> Result<()> {
    let purple = p.juice(Juice::Purple);
    c.text_in(
        &state.labels.usage_by_day,
        &f.small,
        Rect::new(g.bars_title.x + 2.0, g.bars_title.y, 200.0, g.bars_title.h),
        Align::Left,
        p.fg2,
    )?;
    let scale = week_scale(&d.week_hours);
    c.text_in(
        &format!("{} h", scale as u32),
        &f.axis,
        Rect::new(
            g.bars_title.right() - 60.0,
            g.bars_title.y,
            58.0,
            g.bars_title.h,
        ),
        Align::Right,
        p.fg3,
    )?;
    let label_h = 12.0;
    let base = g.bars.bottom() - label_h - 3.0;
    let max_h = base - g.bars.y;
    for (i, hours) in d.week_hours.iter().enumerate() {
        let slot = g.bar_slot(i);
        let today = i == 6;
        let bar_w = 14.0;
        let bar_h = if *hours <= 0.0 {
            2.0
        } else {
            (hours / scale * max_h).clamp(3.0, max_h)
        };
        let r = Rect::new(slot.x + (slot.w - bar_w) / 2.0, base - bar_h, bar_w, bar_h);
        let color = if *hours <= 0.0 {
            p.fill
        } else if today {
            purple
        } else {
            purple.fade(0.5)
        };
        c.fill_rrect(r, 3.0_f32.min(bar_h / 2.0), color);
        if let Some(label) = state.usage_labels.get(i) {
            c.text_in(
                label,
                &f.axis,
                Rect::new(slot.x, base + 3.0, slot.w, label_h),
                Align::Center,
                if today { p.fg } else { p.fg3 },
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_week_scale_is_the_tallest_day_rounded_up_and_at_least_one_hour() {
        assert_eq!(week_scale(&[0.0; 7]), 1.0);
        assert_eq!(week_scale(&[0.2, 0.4, 0.0, 0.0, 0.0, 0.0, 0.1]), 1.0);
        assert_eq!(week_scale(&[3.2, 5.1, 4.6, 6.0, 1.4, 0.8, 4.2]), 6.0);
        assert_eq!(week_scale(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 7.01]), 8.0);
    }

    #[test]
    fn the_glow_is_subtle_in_both_themes_and_follows_the_juice() {
        for theme in [Theme::Light, Theme::Dark] {
            let p = Palette::new(theme, Rgb(0, 0x67, 0xc0));
            for j in [Juice::Purple, Juice::Green, Juice::Red] {
                let g = p.glow(j);
                assert!(
                    (0.12..=0.20).contains(&g.a),
                    "{theme:?} {j:?} alpha {}",
                    g.a
                );
            }
            let (pu, gr, re) = (
                p.glow(Juice::Purple),
                p.glow(Juice::Green),
                p.glow(Juice::Red),
            );
            assert!(pu != gr && gr != re && pu != re);
        }
    }

    #[test]
    fn glass_cards_are_faint_and_a_tile_never_uses_the_accent() {
        let light = Palette::new(Theme::Light, Rgb(0xe8, 0x11, 0x23));
        let dark = Palette::new(Theme::Dark, Rgb(0xe8, 0x11, 0x23));
        assert!(light.card.a < 0.7 && dark.card.a < 0.1);
        assert!(light.card_edge.a <= 0.1 && dark.card_edge.a <= 0.1);
        // The glyph tile tints come from the mark's juice colors: a red system accent cannot reach them.
        for p in [light, dark] {
            let purple = p.juice(Juice::Purple);
            assert!(purple != p.accent && purple != p.red);
        }
    }
}
