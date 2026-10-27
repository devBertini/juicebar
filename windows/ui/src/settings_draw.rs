// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Paints the Settings page onto a `Canvas` and measures its texts for the layout.
//!
//! The look is Windows 11 Settings on the flyout's glass: a section title, a card of rows, a label with
//! a quiet second line and the control at the right end. The one memorable piece is the About block:
//! the mark, the name, and "Idealizado por Claudio Bertini" set large beside a bar in the juice color.

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    IDWriteTextFormat,
};
use windows::core::Result;

use crate::draw::{Palette, draw_glyph, paint_switch};
use crate::gfx::{Align, Canvas, Gfx, IDENTITY, scale_translate};
use crate::glyphs;
use crate::icon::{self, Frame, IconLook, Juice};
use crate::layout::{FlyoutHit, Rect, WIDTH};
use crate::model::UiState;
use crate::settings::{
    PART_DOWN, PART_MINUS, PART_PLUS, PART_RENAME, PART_SEGMENT, PART_SHOW, PART_UP, RowKind,
    SettingsRow, UiSettings,
};
use crate::settings_layout::{
    self, ARROW, CARD_W, RowMeasure, STEPPER_GAP, STEPPER_VALUE_W, SettingsGeom, SettingsMetrics,
    SettingsRowGeom, device_buttons, device_glyph, identity_parts,
};

struct SFonts {
    title: IDWriteTextFormat,
    section: IDWriteTextFormat,
    label: IDWriteTextFormat,
    detail: IDWriteTextFormat,
    control: IDWriteTextFormat,
    value: IDWriteTextFormat,
    note: IDWriteTextFormat,
    name: IDWriteTextFormat,
    version: IDWriteTextFormat,
    credit: IDWriteTextFormat,
}

impl SFonts {
    fn new(g: &Gfx) -> Result<SFonts> {
        let (n, m, s) = (
            DWRITE_FONT_WEIGHT_NORMAL.0,
            DWRITE_FONT_WEIGHT_MEDIUM.0,
            DWRITE_FONT_WEIGHT_SEMI_BOLD.0,
        );
        Ok(SFonts {
            title: g.text_format(false, s, 15.0)?,
            section: g.text_format(false, s, 12.5)?,
            label: g.text_format(false, n, 13.0)?,
            detail: g.text_format(false, n, 11.5)?,
            control: g.text_format(false, m, 12.5)?,
            value: g.text_format(false, s, 13.0)?,
            note: g.text_format(false, n, 12.0)?,
            name: g.text_format(true, s, 19.0)?,
            version: g.text_format(false, n, 12.0)?,
            credit: g.text_format(true, s, 17.0)?,
        })
    }
}

/// Measures the control widths and the wrapped heights of every row, in page order.
pub fn measure_settings(g: &Gfx, ui: &UiSettings) -> Result<SettingsMetrics> {
    let f = SFonts::new(g)?;
    let mut rows = Vec::new();
    for r in ui.rows() {
        let err = std::cell::RefCell::new(None);
        let text_w = |s: &str| match g.measure(s, &f.control) {
            Ok((w, _)) => w,
            Err(e) => {
                *err.borrow_mut() = Some(e);
                0.0
            }
        };
        let (control_w, seg_w) = settings_layout::control_size(&r.kind, &text_w);
        if let Some(e) = err.into_inner() {
            return Err(e);
        }
        let lw = settings_layout::label_width_of(&r.kind, control_w);
        let (label_h, detail_h) = if matches!(r.kind, RowKind::Identity { .. }) {
            (0.0, 0.0)
        } else {
            let font = if matches!(r.kind, RowKind::Note) {
                &f.note
            } else {
                &f.label
            };
            let label_h = g.measure_wrapped(&r.label, font, lw)?.ceil();
            let detail_h = match &r.detail {
                Some(d) => g.measure_wrapped(d, &f.detail, lw)?.ceil(),
                None => 0.0,
            };
            (label_h, detail_h)
        };
        rows.push(RowMeasure {
            control_w,
            seg_w,
            label_h,
            detail_h,
        });
    }
    Ok(SettingsMetrics { rows })
}

fn shift(r: Rect, dy: f32) -> Rect {
    Rect::new(r.x, r.y - dy, r.w, r.h)
}

fn inflate(r: Rect, by: f32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.w + 2.0 * by, r.h + 2.0 * by)
}

/// Paints the page: the fixed header, then the sections clipped below it.
#[allow(clippy::too_many_arguments)]
pub fn paint_settings(
    c: &Canvas,
    state: &UiState,
    ui: &UiSettings,
    g: &SettingsGeom,
    p: &Palette,
    hover: FlyoutHit,
    scroll: f32,
    height: f32,
) -> Result<()> {
    let f = SFonts::new(c.gfx)?;

    // Header.
    if hover == FlyoutHit::Back {
        c.fill_rrect(g.back, 6.0, p.hover);
    }
    let (cx, cy) = (g.back.x + g.back.w / 2.0, g.back.cy());
    let arrow = [(cx + 6.0, cy), (cx - 6.0, cy)];
    c.line(arrow[0], arrow[1], 1.6, p.fg, None);
    let head = c.polyline(
        &[(cx - 1.0, cy - 5.5), (cx - 6.5, cy), (cx - 1.0, cy + 5.5)],
        false,
        false,
    )?;
    c.stroke_geometry(&head.into(), 1.6, p.fg, None);
    c.text_in(&ui.title, &f.title, g.title, Align::Left, p.fg)?;
    if scroll > 0.5 {
        let y = c.snap(g.header_h - 0.5);
        c.line((0.0, y), (WIDTH, y), c.hairline(), p.hair, None);
    }

    // The sections, under the header.
    c.push_clip(Rect::new(0.0, g.header_h, WIDTH, height - g.header_h));
    let painted = (|| -> Result<()> {
        for (sg, section) in g.sections.iter().zip(&ui.sections) {
            let t = shift(sg.title, scroll);
            if t.bottom() < g.header_h || t.y > height {
                // The title is off screen; its card may still show.
            } else {
                c.text_in(&section.title, &f.section, t, Align::Left, p.fg2)?;
            }
            let card = shift(sg.card, scroll);
            if card.bottom() < g.header_h || card.y > height {
                continue;
            }
            c.fill_rrect(card, 8.0, p.card);
            for (rg, row) in sg.rows.iter().zip(&section.rows) {
                let b = shift(rg.bounds, scroll);
                if b.bottom() < g.header_h || b.y > height {
                    continue;
                }
                paint_row(
                    c,
                    state,
                    p,
                    &f,
                    row,
                    rg,
                    scroll,
                    hover,
                    ui.focus == Some(row.id),
                )?;
            }
            c.stroke_rrect(card, 8.0, c.hairline(), p.card_edge);
        }
        Ok(())
    })();
    c.pop_clip();
    painted?;

    // The scroll thumb.
    if let Some((ty, th)) = g.thumb(scroll) {
        c.fill_rrect(Rect::new(WIDTH - 6.0, ty, 3.0, th), 1.5, p.fg3);
    }
    Ok(())
}

fn row_hovered(hover: FlyoutHit, id: u16) -> Option<u8> {
    match hover {
        FlyoutHit::Setting { id: h, part } if h == id => Some(part),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &SFonts,
    row: &SettingsRow,
    g: &SettingsRowGeom,
    scroll: f32,
    hover: FlyoutHit,
    focused: bool,
) -> Result<()> {
    let b = shift(g.bounds, scroll);
    let hovered = row_hovered(hover, row.id);
    if let RowKind::Identity {
        name,
        version,
        credit,
        author,
    } = &row.kind
    {
        return paint_identity(c, state, p, f, b, name, version, credit, author);
    }
    if g.rule {
        let y = c.snap(b.y);
        c.line(
            (b.x + 14.0, y),
            (b.right() - 14.0, y),
            c.hairline(),
            p.hair,
            None,
        );
    }
    let whole_row = matches!(
        row.kind,
        RowKind::Toggle { .. } | RowKind::Link { .. } | RowKind::Text { .. }
    );
    if whole_row && hovered.is_some() {
        c.fill_rrect(
            Rect::new(b.x + 3.0, b.y + 3.0, b.w - 6.0, b.h - 6.0),
            5.0,
            p.hover,
        );
    }
    let quiet = if row.enabled { 1.0 } else { 0.45 };
    let note = matches!(row.kind, RowKind::Note);
    let hidden_device = matches!(row.kind, RowKind::Device { shown: false, .. });
    let (font, color) = if note {
        (&f.note, p.fg2)
    } else if hidden_device {
        (&f.label, p.fg2)
    } else {
        (&f.label, p.fg.fade(quiet))
    };
    c.text_wrapped(&row.label, font, shift(g.label, scroll), Align::Left, color)?;
    if let (Some(d), Some(r)) = (&row.detail, g.detail) {
        c.text_wrapped(
            d,
            &f.detail,
            shift(r, scroll),
            Align::Left,
            p.fg2.fade(quiet),
        )?;
    }
    let ctl = shift(g.control, scroll);
    let ring = |r: Rect, radius: f32| {
        if focused {
            c.stroke_rrect(inflate(r, 3.0), radius + 3.0, 2.0, p.accent);
        }
    };
    match &row.kind {
        RowKind::Toggle { on } => {
            paint_switch(c, p, ctl, *on, quiet);
            ring(ctl, 10.0);
        }
        RowKind::Stepper {
            value,
            min,
            max,
            text,
            ..
        } => {
            let minus = shift(g.minus, scroll);
            let plus = shift(g.plus, scroll);
            for (btn, part, usable, plus_sign) in [
                (minus, PART_MINUS, *value > *min, false),
                (plus, PART_PLUS, *value < *max, true),
            ] {
                c.fill_rrect(btn, 6.0, p.tile.fade(quiet));
                c.stroke_rrect(btn, 6.0, c.hairline(), p.card_edge);
                if hovered == Some(part) && usable {
                    c.fill_rrect(btn, 6.0, p.hover);
                }
                let col = if usable && row.enabled { p.fg } else { p.fg3 };
                let (bx, by) = (btn.x + btn.w / 2.0, btn.cy());
                c.line((bx - 5.0, by), (bx + 5.0, by), 1.5, col, None);
                if plus_sign {
                    c.line((bx, by - 5.0), (bx, by + 5.0), 1.5, col, None);
                }
            }
            let mid = Rect::new(minus.right() + STEPPER_GAP, ctl.y, STEPPER_VALUE_W, ctl.h);
            c.text_in(text, &f.value, mid, Align::Center, p.fg.fade(quiet))?;
            ring(ctl, 6.0);
        }
        RowKind::Choice { options, selected } => {
            c.fill_rrect(ctl, 7.0, p.off_bg.fade(quiet));
            c.stroke_rrect(ctl, 7.0, c.hairline(), p.card_edge);
            for (k, (seg, label)) in g.segments.iter().zip(options).enumerate() {
                let seg = shift(*seg, scroll);
                let on = k == *selected;
                let part = PART_SEGMENT + u8::try_from(k).unwrap_or(0);
                if on {
                    c.fill_rrect(seg, 5.0, p.accent.fade(quiet));
                } else if hovered == Some(part) {
                    c.fill_rrect(seg, 5.0, p.hover);
                }
                let col = if on {
                    p.on(p.accent)
                } else {
                    p.fg2.fade(quiet)
                };
                c.text_in(label, &f.control, seg, Align::Center, col)?;
            }
            ring(ctl, 7.0);
        }
        RowKind::Link { action } => {
            let ax = ctl.right() - ARROW;
            let ay = ctl.cy() - ARROW / 2.0;
            let col = if row.enabled { p.fg2 } else { p.fg3 };
            c.line(
                (ax + 2.0, ay + ARROW - 2.0),
                (ax + ARROW - 2.0, ay + 2.0),
                1.4,
                col,
                None,
            );
            let head = c.polyline(
                &[
                    (ax + 3.5, ay + 1.5),
                    (ax + ARROW - 1.5, ay + 1.5),
                    (ax + ARROW - 1.5, ay + ARROW - 3.5),
                ],
                false,
                false,
            )?;
            c.stroke_geometry(&head.into(), 1.4, col, None);
            if !action.is_empty() {
                let r = Rect::new(ctl.x, ctl.y, ctl.w - ARROW - 6.0, ctl.h);
                c.text_in(action, &f.control, r, Align::Right, p.accent)?;
            }
            if focused {
                c.stroke_rrect(
                    Rect::new(b.x + 2.0, b.y + 2.0, b.w - 4.0, b.h - 4.0),
                    6.0,
                    2.0,
                    p.accent,
                );
            }
        }
        RowKind::Value { text } => {
            c.fill_rrect(ctl, 4.0, p.tile);
            c.stroke_rrect(ctl, 4.0, c.hairline(), p.stroke);
            c.text_in(text, &f.control, ctl, Align::Center, p.fg2)?;
        }
        RowKind::Text {
            value,
            placeholder,
            editing,
            selected,
        } => {
            c.fill_rrect(ctl, 6.0, p.tile);
            let edge = if *editing || focused {
                p.accent
            } else {
                p.card_edge
            };
            c.stroke_rrect(ctl, 6.0, if *editing { 1.5 } else { c.hairline() }, edge);
            let inner = Rect::new(ctl.x + 9.0, ctl.y, ctl.w - 18.0, ctl.h);
            let text_w = c.measure(value, &f.control)?.0;
            // A name wider than the field shows its end, where the caret is.
            let over = text_w > inner.w - 2.0;
            let at = if over {
                Rect::new(inner.right() - text_w - 2.0, inner.y, text_w + 4.0, inner.h)
            } else {
                inner
            };
            if over {
                c.push_clip(inner);
            }
            let drawn = (|| -> Result<()> {
                if value.is_empty() {
                    c.text_in(placeholder, &f.control, inner, Align::Left, p.fg3)?;
                } else if *editing && *selected {
                    // The whole text is selected: a highlight behind it, the text on the accent.
                    let w = text_w.min(inner.w);
                    let hl = Rect::new(inner.x - 2.0, ctl.y + 4.0, w + 4.0, ctl.h - 8.0);
                    c.fill_rrect(hl, 3.0, p.accent);
                    c.text_in(value, &f.control, at, Align::Left, p.on(p.accent))?;
                } else {
                    c.text_in(value, &f.control, at, Align::Left, p.fg)?;
                }
                Ok(())
            })();
            if over {
                c.pop_clip();
            }
            drawn?;
            if *editing && !*selected {
                let x = if over {
                    inner.right() + 1.0
                } else {
                    inner.x + text_w + 1.0
                };
                c.line((x, ctl.y + 6.0), (x, ctl.bottom() - 6.0), 1.2, p.fg, None);
            }
        }
        RowKind::Device {
            glyph,
            shown,
            can_up,
            can_down,
        } => {
            let ink = if *shown { p.fg } else { p.fg3 };
            let gr = device_glyph(b);
            draw_glyph(c, *glyph, gr, ink, false, glyphs::STROKE)?;
            let (show, rename, up, down) = device_buttons(ctl);
            for (btn, part, usable) in [
                (rename, PART_RENAME, true),
                (up, PART_UP, *can_up),
                (down, PART_DOWN, *can_down),
            ] {
                c.fill_rrect(btn, 6.0, p.tile.fade(quiet));
                c.stroke_rrect(btn, 6.0, c.hairline(), p.card_edge);
                if hovered == Some(part) && usable {
                    c.fill_rrect(btn, 6.0, p.hover);
                }
                let col = if usable { p.fg } else { p.fg3 };
                let (bx, by) = (btn.x + btn.w / 2.0, btn.cy());
                match part {
                    PART_RENAME => {
                        // A pencil: its outline on the diagonal, the tip at the lower left.
                        let body = c.polyline(
                            &[
                                (bx - 5.5, by + 5.5),
                                (bx - 4.5, by + 1.5),
                                (bx + 2.5, by - 5.5),
                                (bx + 5.5, by - 2.5),
                                (bx - 1.5, by + 4.5),
                            ],
                            true,
                            false,
                        )?;
                        c.stroke_geometry(&body.into(), 1.4, col, None);
                        c.line((bx + 0.5, by - 3.5), (bx + 3.5, by - 0.5), 1.2, col, None);
                    }
                    _ => {
                        let d = if part == PART_UP { -1.0 } else { 1.0 };
                        let head = c.polyline(
                            &[
                                (bx - 4.5, by - d * 2.0),
                                (bx, by + d * 2.5),
                                (bx + 4.5, by - d * 2.0),
                            ],
                            false,
                            false,
                        )?;
                        c.stroke_geometry(&head.into(), 1.6, col, None);
                    }
                }
            }
            paint_switch(c, p, show, *shown, quiet);
            if hovered == Some(PART_SHOW) {
                c.stroke_rrect(inflate(show, 2.0), 12.0, c.hairline(), p.fg3);
            }
            if focused {
                c.stroke_rrect(
                    Rect::new(b.x + 2.0, b.y + 2.0, b.w - 4.0, b.h - 4.0),
                    6.0,
                    2.0,
                    p.accent,
                );
            }
        }
        RowKind::Identity { .. } | RowKind::Note => {}
    }
    debug_assert!(g.bounds.w <= CARD_W + 0.01);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn paint_identity(
    c: &Canvas,
    state: &UiState,
    p: &Palette,
    f: &SFonts,
    b: Rect,
    name: &str,
    version: &str,
    credit: &str,
    author: &str,
) -> Result<()> {
    let parts = identity_parts(b);
    c.set_transform(scale_translate(1.0, parts.mark.x, parts.mark.y));
    let look = IconLook {
        juice: state.juice,
        charging: false,
    };
    let drawn = icon::draw_mark(c, look, &Frame::STILL, p.dark, parts.mark.w);
    c.set_transform(IDENTITY);
    drawn?;
    c.text_in(name, &f.name, parts.name, Align::Left, p.fg)?;
    c.text_in(version, &f.version, parts.version, Align::Left, p.fg2)?;
    let y = c.snap(parts.rule_y);
    c.line(
        (b.x + 16.0, y),
        (b.right() - 16.0, y),
        c.hairline(),
        p.hair,
        None,
    );
    c.fill_rrect(parts.bar, 1.5, p.juice(Juice::Purple));
    c.text_in(credit, &f.credit, parts.credit, Align::Left, p.fg)?;
    c.text_in(author, &f.version, parts.author, Align::Left, p.fg2)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Theme, sample_settings_page};

    #[test]
    fn every_row_is_measured_once_in_page_order() {
        let g = Gfx::new().unwrap();
        let ui = sample_settings_page();
        let m = measure_settings(&g, &ui).unwrap();
        assert_eq!(m.rows.len(), ui.rows().count());
        for (r, mr) in ui.rows().zip(&m.rows) {
            match &r.kind {
                RowKind::Identity { .. } => assert_eq!(mr.label_h, 0.0),
                _ => assert!(mr.label_h >= 12.0, "{} {}", r.label, mr.label_h),
            }
            if let RowKind::Choice { options, .. } = &r.kind {
                assert_eq!(mr.seg_w.len(), options.len());
            }
        }
        // The long hint wraps onto more than one line, the short labels do not.
        let hint = &m.rows[2];
        let short = &m.rows[0];
        assert!(
            hint.label_h > short.label_h * 1.5,
            "{} vs {}",
            hint.label_h,
            short.label_h
        );
    }

    #[test]
    fn the_widest_controls_leave_their_labels_a_single_line() {
        let g = Gfx::new().unwrap();
        let ui = sample_settings_page();
        let m = measure_settings(&g, &ui).unwrap();
        let f = SFonts::new(&g).unwrap();
        for (r, mr) in ui.rows().zip(&m.rows) {
            if matches!(r.id, 1 | 4 | 5 | 6 | 11) {
                // Tema, the three steppers and Idioma: short or fitting labels stay on one line.
                assert!(
                    mr.label_h < 22.0,
                    "{} wraps ({} high) beside a {} wide control",
                    r.label,
                    mr.label_h,
                    mr.control_w
                );
                let w = g.measure(&r.label, &f.label).unwrap().0;
                assert!(
                    w <= settings_layout::label_width_of(&r.kind, mr.control_w),
                    "{}",
                    r.label
                );
            }
        }
    }

    #[test]
    fn the_whole_page_paints_in_both_themes_with_ink() {
        for theme in [Theme::Light, Theme::Dark] {
            let mut s = UiState::sample_settings(theme);
            s.settings.as_mut().unwrap().focus = Some(4);
            let px = crate::render::flyout_pixels(&s, 1.0).unwrap();
            assert!(px.height > 600, "{}", px.height);
            // Not blank: more than a handful of distinct colors.
            let mut seen = std::collections::HashSet::new();
            for c in px.data.as_chunks::<4>().0.iter().step_by(7) {
                seen.insert([c[0], c[1], c[2]]);
            }
            assert!(seen.len() > 40, "{theme:?} {}", seen.len());
        }
    }
}
