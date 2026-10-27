// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Paints the flyout onto a `Canvas`. The window and the PNG seam both call `paint_flyout`, so a
//! render is what the user sees (apart from the system backdrop, replaced by a solid panel).

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::core::Result;

use crate::gfx::{Align, Canvas, Color, Gfx, IDENTITY, mul, rotate_about, scale_translate};
use crate::glyphs::{self, Shape};
use crate::layout::{DetailGeom, FlyoutHit, Layout, Metrics, Rect, RowGeom};
use crate::model::{Glyph, Rgb, Theme, UiDevice, UiState};

/// The product name in the header; every other label comes from `UiState::labels`.
pub const TITLE: &str = "JuiceBar";

/// The theme's colors (the mockup's Windows tokens, `spec/design/DESIGN.md`).
#[derive(Clone, Copy)]
pub struct Palette {
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
}

impl Palette {
    pub fn new(theme: Theme, accent: Rgb) -> Palette {
        let accent =
            Color::hex(((accent.0 as u32) << 16) | ((accent.1 as u32) << 8) | accent.2 as u32);
        match theme {
            Theme::Light => Palette {
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
            },
            Theme::Dark => Palette {
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
            },
        }
    }

    /// Dark text on a light fill, white on a dark one.
    pub fn on(&self, bg: Color) -> Color {
        if bg.luminance() > 0.4 {
            Color::hex(0x0b1a24)
        } else {
            Color::hex(0xffffff)
        }
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
        })
    }
}

/// Measures the two texts the layout needs: the RGB label and the hotkey hint.
pub fn measure_metrics(g: &Gfx, state: &UiState) -> Result<Metrics> {
    let f = Fonts::new(g)?;
    Ok(Metrics {
        rgb_label_w: g.measure(&state.labels.rgb_economy, &f.rgb)?.0.ceil(),
        hotkey_w: g.measure(&state.hotkey_text, &f.kbd)?.0.ceil(),
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
        c.stroke_rrect(full, 8.0, c.hairline(), p.stroke);
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

    // Footer.
    let ft = &layout.footer;
    let y = c.snap(ft.line_y);
    c.line(
        (14.0, y),
        (layout.width - 14.0, y),
        c.hairline(),
        p.hair,
        None,
    );
    if opts.hover == FlyoutHit::RgbSwitch {
        c.fill_rrect(ft.switch_hit, 6.0, p.hover);
    }
    draw_glyph(c, Glyph::Leaf, ft.icon, p.green, false, glyphs::STROKE)?;
    c.text_in(
        &state.labels.rgb_economy,
        &f.rgb,
        ft.label,
        Align::Left,
        p.fg,
    )?;
    paint_switch(c, &p, ft.switch, state.rgb_economy);
    c.fill_rrect(ft.hotkey, 4.0, p.tile);
    c.stroke_rrect(ft.hotkey, 4.0, c.hairline(), p.stroke);
    c.text_in(&state.hotkey_text, &f.kbd, ft.hotkey, Align::Center, p.fg2)?;
    Ok(())
}

fn paint_switch(c: &Canvas, p: &Palette, r: Rect, on: bool) {
    let knob_y = r.cy();
    if on {
        c.fill_rrect(r, r.h / 2.0, p.accent);
        c.fill_circle(r.x + 30.0, knob_y, 6.0, p.on(p.accent));
    } else {
        c.stroke_rrect(r, r.h / 2.0, 1.0, p.fg2);
        c.fill_circle(r.x + 10.0, knob_y, 6.0, p.fg2);
    }
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
    if let Some(card) = row.card {
        c.fill_rrect(card, 8.0, p.tile);
        c.stroke_rrect(card, 8.0, c.hairline(), p.stroke);
    } else if hover == FlyoutHit::Row(row.index) {
        c.fill_rrect(row.bounds, 6.0, p.hover);
    }

    // Glyph tile.
    let (bg, fg) = match t {
        Tone::On => (p.accent, p.on(p.accent)),
        Tone::Charging => (p.green, p.on(p.green)),
        Tone::Low => (
            p.red,
            if state.theme == Theme::Dark {
                Color::hex(0x2a0a0e)
            } else {
                p.on(p.red)
            },
        ),
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

    // Capsule.
    let cap = row.capsule;
    c.fill_rrect(cap, 2.0, p.fill);
    let frac = d.capsule_fraction.clamp(0.0, 1.0);
    if frac > 0.0 {
        let fill = match t {
            Tone::On => p.fg.fade(0.82),
            Tone::Low => p.red,
            Tone::Charging => p.green,
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

    // Percent: big digits, a small "%" on the same baseline.
    let pc = row.percent;
    let color = match t {
        Tone::Low => p.red,
        Tone::Off => p.fg2,
        _ => p.fg,
    };
    let digits = c.text_layout(&d.percent.to_string(), &f.pct, pc.w, pc.h, Align::Left)?;
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

    if let Some(detail) = &row.detail {
        paint_detail(c, state, p, f, d, detail)?;
    }
    Ok(())
}

fn paint_detail(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &Fonts,
    d: &UiDevice,
    g: &DetailGeom,
) -> Result<()> {
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
            p.accent.fade(0.34),
            p.accent.fade(0.0),
        )?;
        c.pop_clip();
        let lg = c.polyline(&pts, false, false)?;
        c.stroke_geometry(&lg.into(), 1.4, p.accent, None);
    }
    if let Some((h, pc)) = d.forecast_from {
        let (x0, y0) = (ch.x_of(h), ch.y_of(pc));
        let x1 = ch.x_of(d.forecast_empty_hours);
        c.line((x0, y0), (x1, ch.y_of(0.0)), 1.5, p.red, Some(&dash));
        c.fill_circle(x0, y0, 3.0, p.accent);
        c.stroke_circle(x0, y0, 3.0, 1.5, p.card_solid);
        c.fill_circle(x0, y0, 2.25, p.accent);
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
    Ok(())
}
