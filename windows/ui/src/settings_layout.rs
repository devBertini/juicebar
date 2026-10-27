// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Pure layout of the Settings page, in DIPs like `layout`: the header with the back arrow, then the
//! sections as cards of rows, in "content coordinates" (the page as if it were not scrolled; the
//! header occupies `0..header_h` and stays put, the rest moves up by the scroll offset).
//!
//! Text sizes the layout cannot know come in as `SettingsMetrics`, measured with DirectWrite by
//! `settings_draw::measure_settings`; with none, the defaults below stand in (one line of text).

use crate::layout::{Rect, WIDTH};
use crate::settings::{
    PART_DOWN, PART_MINUS, PART_PLUS, PART_RENAME, PART_ROW, PART_SEGMENT, PART_SHOW, PART_UP,
    RowId, RowKind, UiSettings,
};

pub const PAD_X: f32 = 14.0;
pub const CARD_W: f32 = WIDTH - 2.0 * PAD_X;
pub const ROW_PAD_X: f32 = 14.0;
const ROW_PAD_Y: f32 = 11.0;
const ROW_MIN: f32 = 46.0;
const NOTE_PAD_Y: f32 = 12.0;
pub const HEADER_H: f32 = 50.0;
const FIRST_GAP: f32 = 2.0;
const SECTION_GAP: f32 = 16.0;
const TITLE_H: f32 = 18.0;
const TITLE_GAP: f32 = 6.0;
const BOTTOM_PAD: f32 = 18.0;
const CONTROL_GAP: f32 = 12.0;
pub const BACK: f32 = 30.0;
pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 20.0;
pub const BUTTON: f32 = 28.0;
pub const STEPPER_VALUE_W: f32 = 44.0;
pub const STEPPER_GAP: f32 = 2.0;
pub const STEPPER_W: f32 = 2.0 * BUTTON + STEPPER_VALUE_W + 2.0 * STEPPER_GAP;
pub const SEGMENT_PAD: f32 = 2.0;
pub const SEGMENT_MIN_W: f32 = 56.0;
pub const SEGMENT_TEXT_PAD: f32 = 22.0;
pub const TEXT_FIELD_W: f32 = 168.0;
pub const VALUE_H: f32 = 24.0;
pub const ARROW: f32 = 12.0;
/// A device row: the glyph's square, the gap after it, and the small buttons at the right end.
pub const DEVICE_GLYPH: f32 = 22.0;
pub const DEVICE_GLYPH_GAP: f32 = 10.0;
pub const DEVICE_BUTTON: f32 = 26.0;
pub const DEVICE_GAP: f32 = 4.0;
/// The gap between the last button and the switch.
pub const DEVICE_SWITCH_GAP: f32 = 8.0;
/// Pencil, up and down, then the switch.
pub const DEVICE_CONTROL_W: f32 =
    3.0 * DEVICE_BUTTON + 2.0 * DEVICE_GAP + DEVICE_SWITCH_GAP + TOGGLE_W;
const IDENTITY_H: f32 = 156.0;
/// The scroll step of one notch of the mouse wheel.
pub const WHEEL_STEP: f32 = 48.0;

/// What the painter measured for one row.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RowMeasure {
    /// Width of the control at the right end; 0 when the row has none.
    pub control_w: f32,
    /// Width of each segment of a choice.
    pub seg_w: Vec<f32>,
    /// Height of the label wrapped at `label_width(control_w)`.
    pub label_h: f32,
    /// Height of the second line, wrapped the same way.
    pub detail_h: f32,
}

/// The measures of every row of the page, in page order (sections top to bottom, rows top to bottom).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SettingsMetrics {
    pub rows: Vec<RowMeasure>,
}

/// The width the label and the second line wrap at.
pub fn label_width(control_w: f32) -> f32 {
    let gap = if control_w > 0.0 {
        control_w + CONTROL_GAP
    } else {
        0.0
    };
    CARD_W - 2.0 * ROW_PAD_X - gap
}

/// How far a row's label is pushed right (a device row keeps its glyph there).
pub fn label_inset(kind: &RowKind) -> f32 {
    match kind {
        RowKind::Device { .. } => DEVICE_GLYPH + DEVICE_GLYPH_GAP,
        _ => 0.0,
    }
}

/// The width the label of a row of `kind` with a control `control_w` wide wraps at.
pub fn label_width_of(kind: &RowKind, control_w: f32) -> f32 {
    label_width(control_w) - label_inset(kind)
}

/// The control's width and its segments' widths, given a function that measures a text in the
/// control font.
pub fn control_size(kind: &RowKind, text_w: &dyn Fn(&str) -> f32) -> (f32, Vec<f32>) {
    match kind {
        RowKind::Toggle { .. } => (TOGGLE_W, Vec::new()),
        RowKind::Stepper { .. } => (STEPPER_W, Vec::new()),
        RowKind::Choice { options, .. } => {
            let seg: Vec<f32> = options
                .iter()
                .map(|o| (text_w(o) + SEGMENT_TEXT_PAD).max(SEGMENT_MIN_W).ceil())
                .collect();
            (seg.iter().sum::<f32>() + 2.0 * SEGMENT_PAD, seg)
        }
        RowKind::Link { action } => {
            let text = if action.is_empty() {
                0.0
            } else {
                text_w(action).ceil() + 8.0
            };
            (text + ARROW, Vec::new())
        }
        RowKind::Value { text } => ((text_w(text).ceil() + 20.0).max(40.0), Vec::new()),
        RowKind::Text { .. } => (TEXT_FIELD_W, Vec::new()),
        RowKind::Device { .. } => (DEVICE_CONTROL_W, Vec::new()),
        RowKind::Identity { .. } | RowKind::Note => (0.0, Vec::new()),
    }
}

/// The measure a row gets when nothing was measured: one text line each.
fn default_measure(kind: &RowKind, has_detail: bool) -> RowMeasure {
    let (control_w, seg_w) = control_size(kind, &|s: &str| s.chars().count() as f32 * 7.0);
    RowMeasure {
        control_w,
        seg_w,
        label_h: 18.0,
        detail_h: if has_detail { 15.0 } else { 0.0 },
    }
}

/// One row, placed.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsRowGeom {
    pub id: RowId,
    /// The whole row, edge to edge of its card.
    pub bounds: Rect,
    pub label: Rect,
    pub detail: Option<Rect>,
    /// The control at the right end (the whole stepper, the whole choice, the switch...).
    pub control: Rect,
    pub minus: Rect,
    pub plus: Rect,
    pub segments: Vec<Rect>,
    /// Where a click acts, with the part it stands for, in priority order.
    pub hits: Vec<(Rect, u8)>,
    /// A hairline above (every row but the first of a card).
    pub rule: bool,
}

/// A titled card.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionGeom {
    pub title: Rect,
    pub card: Rect,
    pub rows: Vec<SettingsRowGeom>,
}

/// The page.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsGeom {
    /// The back arrow's button.
    pub back: Rect,
    pub title: Rect,
    /// The header ends here; what scrolls lies below.
    pub header_h: f32,
    /// The bottom of the content (the page's natural height).
    pub content_bottom: f32,
    /// The window's height for this page (the content, or less when it does not fit).
    pub height: f32,
    pub sections: Vec<SectionGeom>,
}

/// The parts of the About block inside its row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdentityParts {
    pub mark: Rect,
    pub name: Rect,
    pub version: Rect,
    /// Y of the hairline between the product and its author.
    pub rule_y: f32,
    pub bar: Rect,
    pub credit: Rect,
    pub author: Rect,
}

pub fn identity_parts(b: Rect) -> IdentityParts {
    let x = b.x + ROW_PAD_X + 2.0;
    let mark = Rect::new(x, b.y + 18.0, 52.0, 52.0);
    let tx = mark.right() + 16.0;
    let tw = b.right() - ROW_PAD_X - tx;
    let rule_y = b.y + 88.0;
    let credit_y = rule_y + 14.0;
    IdentityParts {
        mark,
        name: Rect::new(tx, mark.y + 4.0, tw, 24.0),
        version: Rect::new(tx, mark.y + 30.0, tw, 18.0),
        rule_y,
        bar: Rect::new(x, credit_y + 2.0, 3.0, 40.0),
        credit: Rect::new(x + 14.0, credit_y, b.right() - ROW_PAD_X - x - 14.0, 24.0),
        author: Rect::new(
            x + 14.0,
            credit_y + 26.0,
            b.right() - ROW_PAD_X - x - 14.0,
            18.0,
        ),
    }
}

impl SettingsGeom {
    /// The farthest the page can scroll.
    pub fn max_scroll(&self) -> f32 {
        (self.content_bottom - self.height).max(0.0)
    }

    /// The height of what scrolls.
    pub fn view_h(&self) -> f32 {
        self.height - self.header_h
    }

    /// `scroll` kept inside `0..=max_scroll`.
    pub fn clamp_scroll(&self, scroll: f32) -> f32 {
        scroll.clamp(0.0, self.max_scroll())
    }

    pub fn row(&self, id: RowId) -> Option<&SettingsRowGeom> {
        self.sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .find(|r| r.id == id)
    }

    /// The scroll that shows row `id` entirely (with a little air), moving as little as possible from `scroll`.
    pub fn scroll_to_reveal(&self, id: RowId, scroll: f32) -> f32 {
        let Some(r) = self.row(id) else {
            return self.clamp_scroll(scroll);
        };
        let air = 8.0;
        let top = r.bounds.y - air - self.header_h;
        let bottom = r.bounds.bottom() + air - self.height;
        let want = if scroll > top {
            top
        } else if scroll < bottom {
            bottom
        } else {
            scroll
        };
        self.clamp_scroll(want)
    }

    /// The scroll thumb as `(y, h)` inside the viewport, or `None` when the page fits.
    pub fn thumb(&self, scroll: f32) -> Option<(f32, f32)> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return None;
        }
        let view = self.view_h();
        let total = view + max;
        let h = (view * view / total).max(28.0);
        let y = self.header_h + 4.0 + (view - 8.0 - h) * (scroll / max);
        Some((y, h))
    }
}

/// Places the page. `max_height` caps the window (the work area minus its margins); a taller page scrolls.
pub fn compute(ui: &UiSettings, m: &SettingsMetrics, max_height: f32) -> SettingsGeom {
    let back = Rect::new(PAD_X - 2.0, 14.0, BACK, BACK);
    let title = Rect::new(
        back.right() + 6.0,
        14.0,
        WIDTH - back.right() - 6.0 - PAD_X,
        BACK,
    );
    let mut y = HEADER_H + FIRST_GAP;
    let mut flat = 0usize;
    let mut sections = Vec::with_capacity(ui.sections.len());
    for (si, s) in ui.sections.iter().enumerate() {
        if si > 0 {
            y += SECTION_GAP;
        }
        let title_r = Rect::new(PAD_X + 4.0, y, CARD_W - 8.0, TITLE_H);
        y += TITLE_H + TITLE_GAP;
        let card_top = y;
        let mut rows = Vec::with_capacity(s.rows.len());
        for (ri, r) in s.rows.iter().enumerate() {
            let measure = m
                .rows
                .get(flat)
                .cloned()
                .unwrap_or_else(|| default_measure(&r.kind, r.detail.is_some()));
            flat += 1;
            let g = place_row(r, &measure, PAD_X, y, ri > 0);
            y = g.bounds.bottom();
            rows.push(g);
        }
        sections.push(SectionGeom {
            title: title_r,
            card: Rect::new(PAD_X, card_top, CARD_W, y - card_top),
            rows,
        });
    }
    let content_bottom = y + BOTTOM_PAD;
    SettingsGeom {
        back,
        title,
        header_h: HEADER_H,
        content_bottom,
        height: content_bottom.min(max_height.max(HEADER_H + 120.0)),
        sections,
    }
}

fn place_row(
    r: &crate::settings::SettingsRow,
    m: &RowMeasure,
    x: f32,
    y: f32,
    rule: bool,
) -> SettingsRowGeom {
    let empty = Rect::default();
    if matches!(r.kind, RowKind::Identity { .. }) {
        let bounds = Rect::new(x, y, CARD_W, IDENTITY_H);
        return SettingsRowGeom {
            id: r.id,
            bounds,
            label: empty,
            detail: None,
            control: empty,
            minus: empty,
            plus: empty,
            segments: Vec::new(),
            hits: Vec::new(),
            rule,
        };
    }
    let note = matches!(r.kind, RowKind::Note);
    let has_detail = r.detail.is_some();
    let text_h = m.label_h + if has_detail { 2.0 + m.detail_h } else { 0.0 };
    let h = if note {
        text_h + 2.0 * NOTE_PAD_Y
    } else {
        (text_h + 2.0 * ROW_PAD_Y).max(ROW_MIN)
    };
    let bounds = Rect::new(x, y, CARD_W, h);
    let x0 = x + ROW_PAD_X;
    let x1 = bounds.right() - ROW_PAD_X;
    let lw = label_width_of(&r.kind, m.control_w);
    let lx = x0 + label_inset(&r.kind);
    let top = y + (h - text_h) / 2.0;
    let label = Rect::new(lx, top, lw, m.label_h);
    let detail = has_detail.then(|| Rect::new(lx, top + m.label_h + 2.0, lw, m.detail_h));
    let cy = y + h / 2.0;
    let ch = match r.kind {
        RowKind::Toggle { .. } => TOGGLE_H,
        RowKind::Value { .. } => VALUE_H,
        RowKind::Link { .. } => ARROW + 8.0,
        RowKind::Device { .. } => DEVICE_BUTTON,
        _ => BUTTON,
    };
    let control = if m.control_w > 0.0 {
        Rect::new(x1 - m.control_w, cy - ch / 2.0, m.control_w, ch)
    } else {
        empty
    };
    let mut minus = empty;
    let mut plus = empty;
    let mut segments = Vec::new();
    let mut hits: Vec<(Rect, u8)> = Vec::new();
    if r.enabled {
        match &r.kind {
            RowKind::Toggle { .. } | RowKind::Link { .. } | RowKind::Text { .. } => {
                hits.push((bounds, PART_ROW));
            }
            RowKind::Stepper {
                value, min, max, ..
            } => {
                minus = Rect::new(control.x, control.y, BUTTON, BUTTON);
                plus = Rect::new(control.right() - BUTTON, control.y, BUTTON, BUTTON);
                let tall = |b: Rect| Rect::new(b.x - 2.0, y, b.w + 4.0, h);
                if value > min {
                    hits.push((tall(minus), PART_MINUS));
                }
                if value < max {
                    hits.push((tall(plus), PART_PLUS));
                }
            }
            RowKind::Device {
                can_up, can_down, ..
            } => {
                let (show, rename, up, down) = device_buttons(control);
                // The hit zones are as tall as the row; the pencil and the arrows answer where they are drawn.
                let tall = |b: Rect| Rect::new(b.x - 1.0, y, b.w + 2.0, h);
                hits.push((tall(show), PART_SHOW));
                hits.push((tall(rename), PART_RENAME));
                if *can_up {
                    hits.push((tall(up), PART_UP));
                }
                if *can_down {
                    hits.push((tall(down), PART_DOWN));
                }
                minus = up;
                plus = down;
            }
            RowKind::Choice { .. } => {
                let mut sx = control.x + SEGMENT_PAD;
                for (k, w) in m.seg_w.iter().enumerate() {
                    let seg = Rect::new(
                        sx,
                        control.y + SEGMENT_PAD,
                        *w,
                        control.h - 2.0 * SEGMENT_PAD,
                    );
                    sx += w;
                    segments.push(seg);
                    hits.push((
                        Rect::new(seg.x, y, seg.w, h),
                        PART_SEGMENT + u8::try_from(k).unwrap_or(0),
                    ));
                }
            }
            _ => {}
        }
    } else if let RowKind::Device { .. } = r.kind {
        let (_, _, up, down) = device_buttons(control);
        minus = up;
        plus = down;
    } else if let RowKind::Stepper { .. } = r.kind {
        minus = Rect::new(control.x, control.y, BUTTON, BUTTON);
        plus = Rect::new(control.right() - BUTTON, control.y, BUTTON, BUTTON);
    }
    if let (RowKind::Choice { .. }, true) = (&r.kind, segments.is_empty()) {
        let mut sx = control.x + SEGMENT_PAD;
        for w in &m.seg_w {
            segments.push(Rect::new(
                sx,
                control.y + SEGMENT_PAD,
                *w,
                control.h - 2.0 * SEGMENT_PAD,
            ));
            sx += w;
        }
    }
    SettingsRowGeom {
        id: r.id,
        bounds,
        label,
        detail,
        control,
        minus,
        plus,
        segments,
        hits,
        rule,
    }
}

impl SettingsGeom {
    /// What lies under the point `(x, y)` of the window when the page is scrolled by `scroll`.
    pub fn hit(&self, x: f32, y: f32, scroll: f32) -> SettingsHit {
        if y < self.header_h {
            return if self.back.contains(x, y) || self.title.contains(x, y) {
                SettingsHit::Back
            } else {
                SettingsHit::None
            };
        }
        if y >= self.height {
            return SettingsHit::None;
        }
        let cy = y + scroll;
        for s in &self.sections {
            for r in &s.rows {
                if !r.bounds.contains(x, cy) {
                    continue;
                }
                for (rect, part) in &r.hits {
                    if rect.contains(x, cy) {
                        return SettingsHit::Row(r.id, *part);
                    }
                }
                return SettingsHit::None;
            }
        }
        SettingsHit::None
    }
}

/// The parts of a device row's control, left to right: the pencil, the up arrow, the down arrow and the switch
/// (returned as switch, pencil, up, down).
pub fn device_buttons(control: Rect) -> (Rect, Rect, Rect, Rect) {
    let cy = control.cy();
    let button = |x: f32| Rect::new(x, cy - DEVICE_BUTTON / 2.0, DEVICE_BUTTON, DEVICE_BUTTON);
    let rename = button(control.x);
    let up = button(rename.right() + DEVICE_GAP);
    let down = button(up.right() + DEVICE_GAP);
    let show = Rect::new(
        control.right() - TOGGLE_W,
        cy - TOGGLE_H / 2.0,
        TOGGLE_W,
        TOGGLE_H,
    );
    (show, rename, up, down)
}

/// The glyph's square inside a device row.
pub fn device_glyph(bounds: Rect) -> Rect {
    Rect::new(
        bounds.x + ROW_PAD_X,
        bounds.cy() - DEVICE_GLYPH / 2.0,
        DEVICE_GLYPH,
        DEVICE_GLYPH,
    )
}

/// What a point on the Settings page lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsHit {
    Back,
    Row(RowId, u8),
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Glyph;
    use crate::settings::{SettingsRow, SettingsSection};

    fn page() -> UiSettings {
        UiSettings::new(
            "Configurações",
            vec![
                SettingsSection::new(
                    "Aparência",
                    vec![
                        SettingsRow::choice(1, "Tema", &["Automático", "Claro", "Escuro"], 0),
                        SettingsRow::toggle(2, "Animação do ícone", true)
                            .with_detail("O suco se mexe"),
                    ],
                ),
                SettingsSection::new(
                    "Ícone",
                    vec![
                        SettingsRow::stepper(
                            3,
                            "Verde quando todos acima de",
                            50,
                            (25, 95, 5),
                            "50%",
                        ),
                        SettingsRow::stepper(4, "Vermelho", 20, (5, 50, 5), "20%"),
                    ],
                ),
                SettingsSection::new(
                    "Sobre",
                    vec![
                        SettingsRow::identity(
                            5,
                            "JuiceBar",
                            "Versão 0.1.0",
                            "Idealizado por Claudio Bertini",
                            "devBertini",
                        ),
                        SettingsRow::link(6, "Código no GitHub", ""),
                        SettingsRow::note(7, "Sem rede, sem telemetria"),
                    ],
                ),
            ],
        )
    }

    fn geom(max: f32) -> SettingsGeom {
        compute(&page(), &SettingsMetrics::default(), max)
    }

    #[test]
    fn sections_stack_and_rows_stay_inside_their_cards() {
        let g = geom(10_000.0);
        let mut prev_bottom = g.header_h;
        for s in &g.sections {
            assert!(s.title.y >= prev_bottom && s.title.bottom() <= s.card.y);
            let mut y = s.card.y;
            for r in &s.rows {
                assert!((r.bounds.y - y).abs() < 0.01, "rows touch");
                assert!(r.bounds.x >= s.card.x && r.bounds.right() <= s.card.right());
                assert!(r.label.right() <= r.bounds.right());
                if r.control.w > 0.0 {
                    assert!(r.label.right() < r.control.x, "label clears the control");
                    assert!(r.control.y >= r.bounds.y && r.control.bottom() <= r.bounds.bottom());
                }
                y = r.bounds.bottom();
            }
            assert!((y - s.card.bottom()).abs() < 0.01);
            prev_bottom = s.card.bottom();
        }
        assert_eq!(g.content_bottom, prev_bottom + BOTTOM_PAD);
        assert_eq!(g.height, g.content_bottom, "a roomy window shows it all");
        assert_eq!(g.max_scroll(), 0.0);
        assert!(g.thumb(0.0).is_none());
    }

    #[test]
    fn a_short_window_scrolls_exactly_the_missing_height() {
        let g = geom(400.0);
        assert_eq!(g.height, 400.0);
        assert!((g.max_scroll() - (g.content_bottom - 400.0)).abs() < 0.01);
        assert_eq!(g.clamp_scroll(-30.0), 0.0);
        assert_eq!(g.clamp_scroll(10_000.0), g.max_scroll());
        let (ty, th) = g.thumb(0.0).unwrap();
        let (by, bh) = g.thumb(g.max_scroll()).unwrap();
        assert!(ty >= g.header_h && th >= 28.0 && by + bh <= g.height);
        assert!(by > ty);
    }

    #[test]
    fn a_window_cap_below_the_minimum_still_shows_a_usable_page() {
        let g = geom(10.0);
        assert!(g.height >= HEADER_H + 120.0);
    }

    #[test]
    fn a_row_with_a_second_line_is_taller_and_a_long_label_grows_its_row() {
        let g = geom(10_000.0);
        let plain = g.row(1).unwrap().bounds.h;
        let with_detail = g.row(2).unwrap().bounds.h;
        assert!(with_detail > plain);
        let mut m = SettingsMetrics {
            rows: vec![RowMeasure::default(); 7],
        };
        let base = compute(&page(), &m, 10_000.0).content_bottom;
        m.rows[1] = RowMeasure {
            control_w: 40.0,
            seg_w: Vec::new(),
            label_h: 36.0,
            detail_h: 15.0,
        };
        let tall = compute(&page(), &m, 10_000.0);
        assert!(tall.row(2).unwrap().bounds.h > with_detail);
        assert!(tall.content_bottom > base, "the page grows with its row");
    }

    #[test]
    fn the_stepper_buttons_answer_until_their_bound() {
        let g = geom(10_000.0);
        let r = g.row(3).unwrap();
        let (mx, my) = (r.minus.x + 5.0, r.minus.cy());
        assert_eq!(g.hit(mx, my, 0.0), SettingsHit::Row(3, PART_MINUS));
        let (px, py) = (r.plus.x + 5.0, r.plus.cy());
        assert_eq!(g.hit(px, py, 0.0), SettingsHit::Row(3, PART_PLUS));
        // The value between them is not a button.
        assert_eq!(
            g.hit(r.control.x + BUTTON + 20.0, r.control.cy(), 0.0),
            SettingsHit::None
        );
        let mut ui = page();
        if let RowKind::Stepper { value, .. } = &mut ui.sections[1].rows[0].kind {
            *value = 25;
        }
        let low = compute(&ui, &SettingsMetrics::default(), 10_000.0);
        let r = low.row(3).unwrap();
        assert_eq!(
            low.hit(r.minus.x + 5.0, r.minus.cy(), 0.0),
            SettingsHit::None
        );
        assert_eq!(
            low.hit(r.plus.x + 5.0, r.plus.cy(), 0.0),
            SettingsHit::Row(3, PART_PLUS)
        );
    }

    #[test]
    fn each_segment_of_a_choice_names_its_option() {
        let g = geom(10_000.0);
        let r = g.row(1).unwrap();
        assert_eq!(r.segments.len(), 3);
        for (k, seg) in r.segments.iter().enumerate() {
            assert_eq!(
                g.hit(seg.x + seg.w / 2.0, seg.cy(), 0.0),
                SettingsHit::Row(1, PART_SEGMENT + k as u8)
            );
        }
        for w in r.segments.windows(2) {
            assert!((w[0].right() - w[1].x).abs() < 0.01, "segments touch");
        }
        assert!(r.segments[2].right() <= r.control.right());
    }

    #[test]
    fn a_toggle_answers_on_the_whole_row_and_a_disabled_row_on_nothing() {
        let g = geom(10_000.0);
        let r = g.row(2).unwrap();
        assert_eq!(
            g.hit(r.label.x + 4.0, r.label.y + 2.0, 0.0),
            SettingsHit::Row(2, PART_ROW)
        );
        let mut ui = page();
        ui.sections[0].rows[1].enabled = false;
        let g = compute(&ui, &SettingsMetrics::default(), 10_000.0);
        let r = g.row(2).unwrap();
        assert_eq!(
            g.hit(r.control.x + 3.0, r.control.cy(), 0.0),
            SettingsHit::None
        );
    }

    #[test]
    fn the_header_answers_for_back_and_the_scroll_moves_the_rows_under_the_pointer() {
        let g = geom(180.0);
        assert_eq!(g.hit(g.back.x + 4.0, g.back.cy(), 0.0), SettingsHit::Back);
        assert_eq!(g.hit(g.title.x + 4.0, g.title.cy(), 0.0), SettingsHit::Back);
        assert_eq!(g.hit(WIDTH - 3.0, 20.0, 0.0), SettingsHit::None);
        let r = g.row(4).unwrap();
        // Far below the first screen until the page scrolls to it.
        assert!(r.bounds.y > g.height);
        let scroll = g.scroll_to_reveal(4, 0.0);
        assert!(scroll > 0.0 && scroll <= g.max_scroll());
        let sy = r.bounds.cy() - scroll;
        assert!(sy > g.header_h && sy < g.height);
        assert_eq!(
            g.hit(r.plus.x + 4.0, sy, scroll),
            SettingsHit::Row(4, PART_PLUS)
        );
        // Below the window nothing answers.
        assert_eq!(g.hit(100.0, g.height + 5.0, scroll), SettingsHit::None);
    }

    #[test]
    fn revealing_a_row_moves_as_little_as_possible() {
        let g = geom(300.0);
        let r = g.row(3).unwrap();
        let s = g.scroll_to_reveal(3, 0.0);
        assert!(r.bounds.bottom() - s <= g.height && r.bounds.y - s >= g.header_h);
        // Already visible: unchanged.
        assert_eq!(g.scroll_to_reveal(3, s), s);
        // Scrolled past the row: it comes back at the top of the view.
        let back = g.scroll_to_reveal(3, g.max_scroll());
        assert!(back <= g.max_scroll() && r.bounds.y - back >= g.header_h - 0.01);
        assert_eq!(g.scroll_to_reveal(99, 12.0), 12.0_f32.min(g.max_scroll()));
    }

    #[test]
    fn the_about_block_stacks_its_parts_inside_its_row() {
        let g = geom(10_000.0);
        let r = g.row(5).unwrap();
        assert_eq!(r.bounds.h, IDENTITY_H);
        let p = identity_parts(r.bounds);
        let inside = |a: Rect| {
            a.x >= r.bounds.x
                && a.right() <= r.bounds.right()
                && a.y >= r.bounds.y
                && a.bottom() <= r.bounds.bottom()
        };
        for part in [p.mark, p.name, p.version, p.bar, p.credit, p.author] {
            assert!(inside(part), "{part:?}");
        }
        assert!(p.version.bottom() <= p.rule_y && p.rule_y < p.credit.y);
        assert!(p.credit.bottom() <= p.author.y);
        assert!(
            g.hit(p.mark.x + 3.0, p.mark.cy(), 0.0) == SettingsHit::None,
            "the block does not act"
        );
    }

    #[test]
    fn widths_follow_the_text_and_the_label_gets_what_is_left() {
        let w = |s: &str| s.chars().count() as f32 * 7.0;
        let choice = RowKind::Choice {
            options: vec!["Automático".into(), "Claro".into(), "Escuro".into()],
            selected: 0,
        };
        let (cw, seg) = control_size(&choice, &w);
        assert_eq!(seg.len(), 3);
        assert!(seg[1] >= SEGMENT_MIN_W && seg[0] > seg[1]);
        assert_eq!(cw, seg.iter().sum::<f32>() + 2.0 * SEGMENT_PAD);
        assert_eq!(control_size(&RowKind::Toggle { on: true }, &w).0, TOGGLE_W);
        assert_eq!(label_width(0.0), CARD_W - 28.0);
        assert!(label_width(116.0) < label_width(40.0));
        let (lw, _) = control_size(
            &RowKind::Link {
                action: "Abrir".into(),
            },
            &w,
        );
        assert!(lw > ARROW);
    }

    #[test]
    fn the_windows_widest_choice_still_leaves_room_for_its_label() {
        // "Automático / Português / English" at 12.5 px medium is about 6.3 DIPs a letter.
        let w = |s: &str| s.chars().count() as f32 * 6.3;
        let c = RowKind::Choice {
            options: vec!["Automático".into(), "Português".into(), "English".into()],
            selected: 0,
        };
        let (cw, _) = control_size(&c, &w);
        assert!(label_width(cw) > 60.0, "{}", label_width(cw));
    }

    fn device_ui() -> UiSettings {
        UiSettings::new(
            "Configurações",
            vec![SettingsSection::new(
                "Dispositivos",
                vec![
                    SettingsRow::device(10, "G703", Glyph::Mouse, true, (false, true))
                        .with_detail("Receptor"),
                    SettingsRow::device(11, "G915 X", Glyph::Keyboard, false, (true, true)),
                    SettingsRow::device(12, "Xbox Controller", Glyph::Gamepad, true, (true, false)),
                    SettingsRow::note(13, "ajuda"),
                ],
            )],
        )
    }

    #[test]
    fn a_device_row_keeps_its_glyph_its_label_and_its_four_controls_apart_inside_the_card() {
        let g = compute(&device_ui(), &SettingsMetrics::default(), 10_000.0);
        let r = g.row(10).unwrap();
        let glyph = device_glyph(r.bounds);
        let (show, rename, up, down) = device_buttons(r.control);
        assert!(glyph.x >= r.bounds.x && glyph.bottom() <= r.bounds.bottom());
        assert!(glyph.right() < r.label.x, "the label clears the glyph");
        assert!(r.label.right() < rename.x, "the label clears the buttons");
        assert!(rename.right() <= up.x && up.right() <= down.x && down.right() <= show.x);
        assert!(show.right() <= r.bounds.right() - ROW_PAD_X + 0.01);
        for b in [show, rename, up, down] {
            assert!(b.y >= r.bounds.y && b.bottom() <= r.bounds.bottom());
        }
        assert_eq!(r.control.w, DEVICE_CONTROL_W);
        assert!((label_inset(&RowKind::Toggle { on: true })).abs() < 0.01);
        assert!(
            (label_width_of(&device_ui().row(10).unwrap().kind, DEVICE_CONTROL_W)
                - (label_width(DEVICE_CONTROL_W) - DEVICE_GLYPH - DEVICE_GLYPH_GAP))
                .abs()
                < 0.01
        );
    }

    #[test]
    fn each_device_control_answers_for_its_own_part_and_a_blocked_arrow_for_none() {
        let g = compute(&device_ui(), &SettingsMetrics::default(), 10_000.0);
        let at = |id: u16, f: &dyn Fn((Rect, Rect, Rect, Rect)) -> Rect| {
            let r = g.row(id).unwrap();
            let b = f(device_buttons(r.control));
            g.hit(b.x + b.w / 2.0, b.cy(), 0.0)
        };
        assert_eq!(at(11, &|b| b.0), SettingsHit::Row(11, PART_SHOW));
        assert_eq!(at(11, &|b| b.1), SettingsHit::Row(11, PART_RENAME));
        assert_eq!(at(11, &|b| b.2), SettingsHit::Row(11, PART_UP));
        assert_eq!(at(11, &|b| b.3), SettingsHit::Row(11, PART_DOWN));
        // The first device cannot go up, the last cannot go down.
        assert_eq!(at(10, &|b| b.2), SettingsHit::None);
        assert_eq!(at(10, &|b| b.3), SettingsHit::Row(10, PART_DOWN));
        assert_eq!(at(12, &|b| b.3), SettingsHit::None);
        assert_eq!(at(12, &|b| b.2), SettingsHit::Row(12, PART_UP));
        // The name and the glyph are not buttons.
        let r = g.row(11).unwrap();
        assert_eq!(
            g.hit(r.label.x + 4.0, r.label.y + 2.0, 0.0),
            SettingsHit::None
        );
        let glyph = device_glyph(r.bounds);
        assert_eq!(g.hit(glyph.x + 3.0, glyph.cy(), 0.0), SettingsHit::None);
    }

    #[test]
    fn a_device_row_with_a_second_line_is_taller_than_one_without() {
        let g = compute(&device_ui(), &SettingsMetrics::default(), 10_000.0);
        assert!(g.row(10).unwrap().bounds.h > g.row(11).unwrap().bounds.h);
        assert!(g.row(11).unwrap().bounds.h >= ROW_MIN);
    }

    #[test]
    fn the_widest_device_control_still_leaves_a_name_room() {
        // The longest catalog name ("Xbox Controller") at about 7 DIPs a letter beside the controls and the glyph.
        let w = |s: &str| s.chars().count() as f32 * 7.0;
        let kind = RowKind::Device {
            glyph: Glyph::Gamepad,
            shown: true,
            can_up: true,
            can_down: true,
        };
        let (cw, _) = control_size(&kind, &w);
        assert!(label_width_of(&kind, cw) > w("Xbox Controller"));
    }
}
