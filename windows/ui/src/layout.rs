// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Pure layout math of the flyout: every rectangle in device-independent pixels (DIPs, 96 dpi),
//! hit-testing, DPI scaling and the placement of the window in a monitor's work area.
//! The numbers follow the mockup's Windows scene (`docs/design/options/1-native/option.css`).

use crate::model::UiState;

/// A rectangle in DIPs.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}

/// What a click landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutHit {
    /// A device row, by its index in `UiState::devices`.
    Row(usize),
    Gear,
    RgbSwitch,
    None,
}

pub const WIDTH: f32 = 380.0;
const PAD_X: f32 = 14.0;
const PAD_TOP: f32 = 14.0;
const PAD_BOTTOM: f32 = 12.0;
const HEAD_H: f32 = 28.0;
const GEAR: f32 = 26.0;
const LIST_GAP: f32 = 2.0;
const ICON: f32 = 34.0;
const COL_GAP: f32 = 11.0;
const PCT_W: f32 = 52.0;
const L1_H: f32 = 18.0;
const CAP_H: f32 = 4.0;
const L3_H: f32 = 14.0;
const MID_H: f32 = L1_H + 4.0 + CAP_H + 4.0 + L3_H;
const ROW_H: f32 = 58.0;
const CARD_ROW_H: f32 = 62.0;
const WARN_H: f32 = 30.0;
const CHART_TITLE_H: f32 = 15.0;
const CHART_H: f32 = 98.0;
const STATS_H: f32 = 33.0;
const SEP_H: f32 = 15.0;
const FOOT_GAP: f32 = 8.0;
const FOOT_PAD: f32 = 10.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 20.0;
/// Margin between the flyout and the work area's edges.
pub const EDGE_MARGIN: f32 = 12.0;

/// Text widths the layout cannot know; the painter measures them with DirectWrite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub rgb_label_w: f32,
    pub hotkey_w: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            rgb_label_w: 112.0,
            hotkey_w: 62.0,
        }
    }
}

/// The chart's coordinate mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartGeom {
    /// The whole chart box (axis labels included).
    pub area: Rect,
    /// The plot rectangle: 0 to 100 % vertically, -168 to +6 hours horizontally.
    pub plot: Rect,
}

impl ChartGeom {
    pub const HOURS_MIN: f32 = -168.0;
    pub const HOURS_MAX: f32 = 6.0;

    pub fn x_of(&self, hours: f32) -> f32 {
        self.plot.x + (hours - Self::HOURS_MIN) / (Self::HOURS_MAX - Self::HOURS_MIN) * self.plot.w
    }
    pub fn y_of(&self, percent: f32) -> f32 {
        self.plot.y + (1.0 - percent / 100.0) * self.plot.h
    }
}

/// The expanded part of a row.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailGeom {
    pub warn: Option<Rect>,
    pub chart_title: Rect,
    pub chart: ChartGeom,
    /// Y of the hairline above the two stats.
    pub stats_top: f32,
    pub today: Rect,
    pub health: Rect,
}

/// One device row.
#[derive(Clone, Debug, PartialEq)]
pub struct RowGeom {
    /// Index into `UiState::devices`.
    pub index: usize,
    /// The click area: the card for an expanded row, else the row.
    pub bounds: Rect,
    pub card: Option<Rect>,
    pub icon: Rect,
    pub title: Rect,
    pub capsule: Rect,
    pub status: Rect,
    pub percent: Rect,
    pub detail: Option<DetailGeom>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FooterGeom {
    /// Y of the hairline above the footer.
    pub line_y: f32,
    pub icon: Rect,
    pub label: Rect,
    pub switch: Rect,
    /// Click area of the whole "Economia de RGB" control.
    pub switch_hit: Rect,
    pub hotkey: Rect,
}

/// The whole flyout, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub title: Rect,
    pub gear: Rect,
    /// Rows in display order (devices that are on first).
    pub rows: Vec<RowGeom>,
    /// "Desligados" separator, present when any device is off.
    pub separator: Option<Rect>,
    pub footer: FooterGeom,
}

impl Layout {
    pub fn compute(state: &UiState, m: Metrics) -> Layout {
        let list_x = PAD_X;
        let list_w = WIDTH - 2.0 * PAD_X;
        let title = Rect::new(PAD_X + 4.0, PAD_TOP, 200.0, HEAD_H - 6.0);
        let gear_cy = PAD_TOP + (HEAD_H - 6.0) / 2.0;
        let gear = Rect::new(WIDTH - PAD_X - 4.0 - GEAR, gear_cy - GEAR / 2.0, GEAR, GEAR);

        let mut y = PAD_TOP + HEAD_H;
        let mut rows = Vec::with_capacity(state.devices.len());
        let mut separator = None;
        let mut first = true;
        let mut sep_done = false;
        for index in state.display_order() {
            let d = &state.devices[index];
            if !d.on && !sep_done {
                sep_done = true;
                if !first {
                    y += LIST_GAP;
                }
                y += 6.0;
                separator = Some(Rect::new(PAD_X + 8.0, y, list_w - 16.0, SEP_H));
                y += SEP_H + 2.0;
                first = false;
            }
            if !first {
                y += LIST_GAP;
            }
            first = false;
            let expanded = d.expanded;
            if expanded {
                y += 2.0;
                let (row, detail, card_h) = Self::card(state, index, list_x, y, list_w);
                let card = Rect::new(list_x, y, list_w, card_h);
                rows.push(RowGeom {
                    bounds: card,
                    card: Some(card),
                    detail: Some(detail),
                    ..row
                });
                y += card_h + 4.0;
            } else {
                let g = Self::row(
                    index,
                    list_x + 8.0,
                    list_x + list_w - 8.0,
                    y,
                    ROW_PAD_TOP_PLAIN,
                );
                rows.push(RowGeom {
                    bounds: Rect::new(list_x, y, list_w, ROW_H),
                    ..g
                });
                y += ROW_H;
            }
        }
        let foot_top = y + FOOT_GAP;
        let content_y = foot_top + FOOT_PAD;
        let icon = Rect::new(PAD_X + 8.0, content_y + 2.0, 16.0, 16.0);
        let label = Rect::new(icon.right() + 9.0, content_y, m.rgb_label_w, SWITCH_H);
        let switch = Rect::new(label.right() + 13.0, content_y, SWITCH_W, SWITCH_H);
        let hk_w = m.hotkey_w + 14.0;
        let hotkey = Rect::new(WIDTH - PAD_X - 8.0 - hk_w, content_y + 0.5, hk_w, 19.0);
        let switch_hit = Rect::new(
            PAD_X + 4.0,
            content_y - 4.0,
            switch.right() - PAD_X - 4.0 + 4.0,
            SWITCH_H + 8.0,
        );
        let footer = FooterGeom {
            line_y: foot_top,
            icon,
            label,
            switch,
            switch_hit,
            hotkey,
        };
        let height = content_y + SWITCH_H + PAD_BOTTOM;
        Layout {
            width: WIDTH,
            height,
            title,
            gear,
            rows,
            separator,
            footer,
        }
    }

    fn row(index: usize, x0: f32, x1: f32, top: f32, pad_top: f32) -> RowGeom {
        let mid_top = top + pad_top;
        let icon = Rect::new(x0, mid_top + (MID_H - ICON) / 2.0, ICON, ICON);
        let mid_x = x0 + ICON + COL_GAP;
        let mid_w = x1 - PCT_W - COL_GAP - mid_x;
        RowGeom {
            index,
            bounds: Rect::default(),
            card: None,
            icon,
            title: Rect::new(mid_x, mid_top, mid_w, L1_H),
            capsule: Rect::new(mid_x, mid_top + L1_H + 4.0, mid_w, CAP_H),
            status: Rect::new(mid_x, mid_top + L1_H + 4.0 + CAP_H + 4.0, mid_w, L3_H),
            percent: Rect::new(x1 - PCT_W, mid_top, PCT_W, MID_H),
            detail: None,
        }
    }

    fn card(state: &UiState, index: usize, x: f32, top: f32, w: f32) -> (RowGeom, DetailGeom, f32) {
        let d = &state.devices[index];
        let row = Self::row(index, x + 12.0, x + w - 12.0, top, 10.0);
        let dx = x + 12.0;
        let dw = w - 24.0;
        let mut y = top + CARD_ROW_H;
        let warn = d.warning.as_ref().map(|_| {
            let r = Rect::new(dx, y, dw, WARN_H);
            y += WARN_H + 9.0;
            r
        });
        let chart_title = Rect::new(dx, y, dw, CHART_TITLE_H);
        let area = Rect::new(dx, y + CHART_TITLE_H + 2.0, dw, CHART_H);
        let plot = Rect::new(
            area.x + 27.0,
            area.y + 6.0,
            dw - 27.0 - 6.0,
            CHART_H - 6.0 - 17.0,
        );
        y = area.bottom() + 9.0;
        let stats_top = y;
        let half = (dw - 12.0) / 2.0;
        let today = Rect::new(dx, y + 8.0, half, STATS_H);
        let health = Rect::new(dx + half + 12.0, y + 8.0, half, STATS_H);
        let card_h = stats_top + 8.0 + STATS_H + 10.0 - top;
        (
            row,
            DetailGeom {
                warn,
                chart_title,
                chart: ChartGeom { area, plot },
                stats_top,
                today,
                health,
            },
            card_h,
        )
    }

    /// What lies under the point (DIPs, relative to the flyout's top-left corner).
    pub fn hit(&self, x: f32, y: f32) -> FlyoutHit {
        if self.gear.contains(x, y) {
            return FlyoutHit::Gear;
        }
        if self.footer.switch_hit.contains(x, y) {
            return FlyoutHit::RgbSwitch;
        }
        for r in &self.rows {
            if r.bounds.contains(x, y) {
                return FlyoutHit::Row(r.index);
            }
        }
        FlyoutHit::None
    }
}

const ROW_PAD_TOP_PLAIN: f32 = 7.0;

// ---- DPI and placement ----

/// Pixels per DIP for a monitor's effective dpi (96 is 1.0).
pub fn scale_from_dpi(dpi: u32) -> f32 {
    dpi as f32 / 96.0
}

/// The window's size in pixels for a layout size in DIPs.
pub fn size_px(width: f32, height: f32, scale: f32) -> (i32, i32) {
    (
        (width * scale).ceil() as i32,
        (height * scale).ceil() as i32,
    )
}

/// A rectangle in physical pixels (screen coordinates), like Win32's `RECT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Top-left of a `w` x `h` flyout above (or, when there is no room above, below) the anchor,
/// centered on it, kept `EDGE_MARGIN` DIPs inside the monitor's work area.
pub fn place(anchor: PxRect, w: i32, h: i32, work: PxRect, scale: f32) -> (i32, i32) {
    let margin = (EDGE_MARGIN * scale).round() as i32;
    let cx = (anchor.left + anchor.right) / 2;
    let x = (cx - w / 2)
        .min(work.right - margin - w)
        .max(work.left + margin);
    let above = anchor.top.min(work.bottom) - margin - h;
    let y = if above >= work.top + margin {
        above
    } else {
        (anchor.bottom + margin).min(work.bottom - margin - h)
    };
    let y = y.max(work.top + margin);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Theme;

    fn lay() -> (UiState, Layout) {
        let s = UiState::sample(Theme::Light);
        let l = Layout::compute(&s, Metrics::default());
        (s, l)
    }

    #[test]
    fn height_matches_the_mockup_flyout() {
        let (_, l) = lay();
        // The mockup's flyout measures about 580 DIPs.
        assert!((l.height - 580.0).abs() < 6.0, "height {}", l.height);
        assert_eq!(l.width, 380.0);
    }

    #[test]
    fn rows_follow_display_order_and_never_overlap() {
        let (_, l) = lay();
        let order: Vec<usize> = l.rows.iter().map(|r| r.index).collect();
        assert_eq!(order, vec![0, 1, 3, 2]);
        for w in l.rows.windows(2) {
            assert!(
                w[0].bounds.bottom() <= w[1].bounds.y,
                "{:?} overlaps {:?}",
                w[0].bounds,
                w[1].bounds
            );
        }
        let sep = l.separator.unwrap();
        assert!(sep.y >= l.rows[2].bounds.bottom() && sep.bottom() <= l.rows[3].bounds.y);
        assert!(l.rows[3].bounds.bottom() < l.footer.line_y);
        assert!(l.footer.switch.bottom() < l.height);
    }

    #[test]
    fn everything_stays_inside_the_flyout() {
        let (_, l) = lay();
        let inside =
            |r: Rect| r.x >= 0.0 && r.y >= 0.0 && r.right() <= l.width && r.bottom() <= l.height;
        assert!(inside(l.gear) && inside(l.footer.hotkey) && inside(l.footer.switch_hit));
        for r in &l.rows {
            assert!(inside(r.bounds) && inside(r.icon) && inside(r.percent) && inside(r.status));
            if let Some(d) = &r.detail {
                assert!(inside(d.chart.area) && inside(d.today) && inside(d.health));
                assert!(d.chart.area.contains(d.chart.plot.x, d.chart.plot.y));
            }
        }
    }

    #[test]
    fn chart_maps_hours_and_percent_into_the_plot() {
        let (_, l) = lay();
        let c = l.rows[0].detail.as_ref().unwrap().chart;
        assert_eq!(c.x_of(-168.0), c.plot.x);
        assert_eq!(c.x_of(6.0), c.plot.right());
        assert_eq!(c.y_of(100.0), c.plot.y);
        assert_eq!(c.y_of(0.0), c.plot.bottom());
        assert!(c.x_of(0.0) < c.x_of(3.1));
    }

    #[test]
    fn hit_testing_finds_each_control() {
        let (_, l) = lay();
        let c = |r: Rect| (r.x + r.w / 2.0, r.cy());
        let (gx, gy) = c(l.gear);
        assert_eq!(l.hit(gx, gy), FlyoutHit::Gear);
        let (sx, sy) = c(l.footer.switch);
        assert_eq!(l.hit(sx, sy), FlyoutHit::RgbSwitch);
        // Clicking the label toggles too.
        let (lx, ly) = c(l.footer.label);
        assert_eq!(l.hit(lx, ly), FlyoutHit::RgbSwitch);
        // Display order is [0, 1, 3, 2]; hits report the device index.
        for (pos, idx) in [0usize, 1, 3, 2].into_iter().enumerate() {
            let (rx, ry) = c(l.rows[pos].bounds);
            assert_eq!(l.hit(rx, ry), FlyoutHit::Row(idx));
        }
        // The expanded card answers for its detail too.
        let d = l.rows[0].detail.as_ref().unwrap();
        let (cx, cy) = c(d.chart.area);
        assert_eq!(l.hit(cx, cy), FlyoutHit::Row(0));
        assert_eq!(l.hit(-1.0, 5.0), FlyoutHit::None);
        assert_eq!(l.hit(5.0, 5.0), FlyoutHit::None);
        let sep = l.separator.unwrap();
        assert_eq!(l.hit(sep.x + 5.0, sep.cy()), FlyoutHit::None);
        assert_eq!(l.hit(l.width + 1.0, 100.0), FlyoutHit::None);
    }

    #[test]
    fn collapsing_the_card_shrinks_the_flyout() {
        let (mut s, l) = lay();
        s.devices[0].expanded = false;
        let l2 = Layout::compute(&s, Metrics::default());
        assert!(l2.height < l.height - 200.0);
        assert!(l2.rows[0].detail.is_none());
    }

    #[test]
    fn no_off_device_means_no_separator() {
        let (mut s, _) = lay();
        s.devices[2].on = true;
        assert!(Layout::compute(&s, Metrics::default()).separator.is_none());
    }

    #[test]
    fn dpi_scaling_rounds_up() {
        assert_eq!(scale_from_dpi(96), 1.0);
        assert_eq!(scale_from_dpi(144), 1.5);
        assert_eq!(scale_from_dpi(192), 2.0);
        assert_eq!(size_px(380.0, 579.5, 1.0), (380, 580));
        assert_eq!(size_px(380.0, 579.5, 1.5), (570, 870));
        assert_eq!(size_px(380.0, 579.5, 2.0), (760, 1159));
    }

    const WORK: PxRect = PxRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };

    #[test]
    fn bottom_tray_places_the_flyout_above_the_taskbar() {
        // Tray icon in the taskbar below the work area.
        let anchor = PxRect {
            left: 1700,
            top: 1040,
            right: 1724,
            bottom: 1080,
        };
        let (x, y) = place(anchor, 380, 580, WORK, 1.0);
        assert_eq!(y, 1032 - 12 - 580);
        assert!(x >= 12 && x + 380 <= 1920 - 12);
        assert_eq!(x, 1712 - 190);
    }

    #[test]
    fn right_edge_is_clamped_with_the_margin() {
        let anchor = PxRect {
            left: 1890,
            top: 1040,
            right: 1914,
            bottom: 1080,
        };
        let (x, _) = place(anchor, 380, 580, WORK, 1.0);
        assert_eq!(x, 1920 - 12 - 380);
    }

    #[test]
    fn left_edge_is_clamped_with_the_margin() {
        let anchor = PxRect {
            left: 2,
            top: 400,
            right: 40,
            bottom: 440,
        };
        let (x, _) = place(anchor, 380, 580, WORK, 1.0);
        assert_eq!(x, 12);
    }

    #[test]
    fn top_taskbar_places_the_flyout_below() {
        let work = PxRect {
            left: 0,
            top: 48,
            right: 1920,
            bottom: 1080,
        };
        let anchor = PxRect {
            left: 1700,
            top: 4,
            right: 1724,
            bottom: 44,
        };
        let (_, y) = place(anchor, 380, 580, work, 1.0);
        assert_eq!(y, 48 + 12, "below the taskbar, inside the work area margin");
    }

    #[test]
    fn secondary_monitor_with_negative_origin_and_150_percent() {
        let work = PxRect {
            left: -2560,
            top: -200,
            right: 0,
            bottom: 1240,
        };
        let anchor = PxRect {
            left: -60,
            top: 1260,
            right: -20,
            bottom: 1320,
        };
        let (w, h) = size_px(380.0, 580.0, 1.5);
        let (x, y) = place(anchor, w, h, work, 1.5);
        assert!(x >= work.left + 18 && x + w <= work.right - 18, "x {x}");
        assert_eq!(y, work.bottom - 18 - h);
    }

    #[test]
    fn a_flyout_taller_than_the_work_area_sticks_to_the_top_margin() {
        let work = PxRect {
            left: 0,
            top: 0,
            right: 800,
            bottom: 400,
        };
        let anchor = PxRect {
            left: 700,
            top: 410,
            right: 740,
            bottom: 440,
        };
        let (_, y) = place(anchor, 380, 580, work, 1.0);
        assert_eq!(y, 12);
    }
}
