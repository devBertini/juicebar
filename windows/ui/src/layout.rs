// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Pure layout math of the flyout: every rectangle in device-independent pixels (DIPs, 96 dpi),
//! hit-testing, DPI scaling and the placement of the window in a monitor's work area.
//! The numbers follow the mockup's Windows scene (`docs/design/options/1-native/option.css`), with
//! every device on its own glass card and the first-open welcome page.

use crate::model::UiState;
use crate::settings::{EditKey, RowId};
use crate::settings_layout::{self, SettingsGeom, SettingsHit, SettingsMetrics};

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
    /// A device card, by its index in `UiState::devices`.
    Row(usize),
    Gear,
    /// The small (i) next to the power saver switch.
    Info,
    RgbSwitch,
    /// The welcome page's button.
    Start,
    /// The Settings page's back arrow.
    Back,
    /// A control of the Settings page: the row's id and the part of the control (`settings::PART_*`).
    Setting {
        id: RowId,
        part: u8,
    },
    /// A key typed into a text field of the Settings page that is being edited.
    Edit(EditKey),
    None,
}

pub const WIDTH: f32 = 380.0;
const PAD_X: f32 = 14.0;
const PAD_TOP: f32 = 14.0;
const PAD_BOTTOM: f32 = 12.0;
const HEAD_H: f32 = 28.0;
const GEAR: f32 = 26.0;
const CARD_GAP: f32 = 6.0;
const ICON: f32 = 34.0;
const COL_GAP: f32 = 11.0;
const PCT_W: f32 = 52.0;
const L1_H: f32 = 18.0;
const CAP_H: f32 = 4.0;
const L3_H: f32 = 14.0;
const MID_H: f32 = L1_H + 4.0 + CAP_H + 4.0 + L3_H;
const CARD_PAD: f32 = 9.0;
/// A closed card, and the head of an open one.
const CARD_H: f32 = MID_H + 2.0 * CARD_PAD;
const WARN_H: f32 = 30.0;
const CHART_TITLE_H: f32 = 15.0;
const CHART_H: f32 = 88.0;
const STATS_H: f32 = 33.0;
const BARS_TITLE_H: f32 = 14.0;
const BARS_H: f32 = 52.0;
const SEP_H: f32 = 15.0;
const EMPTY_H: f32 = 64.0;
const FOOT_GAP: f32 = 6.0;
const FOOT_PAD: f32 = 10.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 20.0;
const INFO: f32 = 16.0;
const GHUB_H: f32 = 15.0;
/// Margin between the flyout and the work area's edges.
pub const EDGE_MARGIN: f32 = 12.0;
/// Width the (i) text wraps at inside its bubble.
pub const INFO_TEXT_W: f32 = 296.0;
/// Width the welcome page text wraps at (the page pads 22 on each side).
pub const WELCOME_TEXT_W: f32 = WIDTH - 44.0;
/// Width of the tray hint text inside its card (the card pads 14 on each side).
pub const WELCOME_STEPS_W: f32 = WELCOME_TEXT_W - 28.0;

/// Text sizes the layout cannot know; the painter measures them with DirectWrite.
#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub rgb_label_w: f32,
    pub hotkey_w: f32,
    /// Height of the (i) text wrapped at `INFO_TEXT_W`.
    pub info_h: f32,
    /// Height of the welcome page body wrapped at `WELCOME_TEXT_W`.
    pub welcome_body_h: f32,
    /// Height of the welcome page tray steps wrapped at `WELCOME_STEPS_W`.
    pub welcome_steps_h: f32,
    /// The text of every row of the Settings page.
    pub settings: SettingsMetrics,
    /// The tallest the Settings page may be (the work area minus the flyout's margins), in DIPs.
    pub max_height: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            rgb_label_w: 124.0,
            hotkey_w: 62.0,
            info_h: 34.0,
            welcome_body_h: 60.0,
            welcome_steps_h: 52.0,
            settings: SettingsMetrics::default(),
            max_height: 720.0,
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

/// The expanded part of a card.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailGeom {
    pub warn: Option<Rect>,
    pub chart_title: Rect,
    pub chart: ChartGeom,
    /// Y of the hairline above the two stats.
    pub stats_top: f32,
    pub today: Rect,
    pub health: Rect,
    /// "Horas de uso por dia", with the scale at its right end.
    pub bars_title: Rect,
    /// The seven bars and the weekday labels under them.
    pub bars: Rect,
}

impl DetailGeom {
    /// The column of day `i` (0 to 6, oldest first) inside `bars`, weekday label included.
    pub fn bar_slot(&self, i: usize) -> Rect {
        let w = self.bars.w / 7.0;
        Rect::new(self.bars.x + w * i as f32, self.bars.y, w, self.bars.h)
    }
}

/// One device card.
#[derive(Clone, Debug, PartialEq)]
pub struct RowGeom {
    /// Index into `UiState::devices`.
    pub index: usize,
    /// The card, and the click area.
    pub bounds: Rect,
    pub icon: Rect,
    pub title: Rect,
    pub capsule: Rect,
    pub status: Rect,
    pub percent: Rect,
    pub detail: Option<DetailGeom>,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct FooterGeom {
    /// Y of the hairline above the footer.
    pub line_y: f32,
    pub icon: Rect,
    pub label: Rect,
    /// The small (i) and its click area.
    pub info: Rect,
    pub info_hit: Rect,
    /// Where the (i) text opens: a bubble above the footer.
    pub bubble: Rect,
    pub switch: Rect,
    /// Click area of the whole "Economia de Energia" control.
    pub switch_hit: Rect,
    pub hotkey: Rect,
    /// The G HUB notice, on its own small line under the switch; present only when there is one.
    pub ghub: Option<Rect>,
}

/// The first-open page.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct WelcomeGeom {
    pub mark: Rect,
    pub title: Rect,
    pub body: Rect,
    pub tray_card: Rect,
    pub tray_title: Rect,
    pub tray_steps: Rect,
    pub credit: Rect,
    pub button: Rect,
}

/// The whole flyout, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub title: Rect,
    pub gear: Rect,
    /// Cards in display order (devices that are on first).
    pub rows: Vec<RowGeom>,
    /// "Desligados" separator, present when any device is off.
    pub separator: Option<Rect>,
    /// Where the "no devices yet" text goes; present only when the list is empty.
    pub empty: Option<Rect>,
    pub footer: FooterGeom,
    /// Present instead of the list and the footer on the first open.
    pub welcome: Option<WelcomeGeom>,
    /// Present instead of the list and the footer on the Settings page.
    pub settings: Option<SettingsGeom>,
}

impl Layout {
    pub fn compute(state: &UiState, m: Metrics) -> Layout {
        if state.welcome.is_some() {
            return Self::welcome_page(&m);
        }
        if let Some(ui) = &state.settings {
            return Self::settings_page(ui, &m);
        }
        let list_x = PAD_X;
        let list_w = WIDTH - 2.0 * PAD_X;
        let title = Rect::new(PAD_X + 4.0, PAD_TOP, 200.0, HEAD_H - 6.0);
        let gear_cy = PAD_TOP + (HEAD_H - 6.0) / 2.0;
        let gear = Rect::new(WIDTH - PAD_X - 4.0 - GEAR, gear_cy - GEAR / 2.0, GEAR, GEAR);

        let mut y = PAD_TOP + HEAD_H;
        let mut rows = Vec::with_capacity(state.devices.len());
        let mut separator = None;
        let mut sep_done = false;
        for index in state.display_order() {
            let d = &state.devices[index];
            if !d.on && !sep_done {
                sep_done = true;
                y += 2.0;
                separator = Some(Rect::new(PAD_X + 8.0, y, list_w - 16.0, SEP_H));
                y += SEP_H + 4.0;
            }
            let (detail, card_h) = if d.expanded {
                let (detail, h) = Self::detail(d.warning.is_some(), list_x, y, list_w);
                (Some(detail), h)
            } else {
                (None, CARD_H)
            };
            let card = Rect::new(list_x, y, list_w, card_h);
            let g = Self::row(index, card.x + 12.0, card.right() - 12.0, card.y, CARD_PAD);
            rows.push(RowGeom {
                bounds: card,
                detail,
                ..g
            });
            y += card_h + CARD_GAP;
        }
        if !rows.is_empty() {
            y -= CARD_GAP;
        }
        let empty = state.devices.is_empty().then(|| {
            let r = Rect::new(list_x, y + 4.0, list_w, EMPTY_H);
            y += EMPTY_H + 4.0;
            r
        });
        let foot_top = y + FOOT_GAP;
        let content_y = foot_top + FOOT_PAD;
        let icon = Rect::new(PAD_X + 8.0, content_y + 2.0, 16.0, 16.0);
        let label = Rect::new(icon.right() + 9.0, content_y, m.rgb_label_w, SWITCH_H);
        let info = Rect::new(label.right() + 6.0, content_y + 2.0, INFO, INFO);
        let info_hit = Rect::new(info.x - 3.0, content_y, INFO + 6.0, SWITCH_H);
        let switch = Rect::new(info.right() + 12.0, content_y, SWITCH_W, SWITCH_H);
        let hk_w = m.hotkey_w + 14.0;
        let hotkey = Rect::new(WIDTH - PAD_X - 8.0 - hk_w, content_y + 0.5, hk_w, 19.0);
        let switch_hit = Rect::new(
            PAD_X + 4.0,
            content_y - 4.0,
            switch.right() - PAD_X - 4.0 + 4.0,
            SWITCH_H + 8.0,
        );
        let bubble_h = m.info_h + 20.0;
        let bubble_w = INFO_TEXT_W + 24.0;
        let bubble = Rect::new(
            (WIDTH - bubble_w) / 2.0,
            foot_top - 8.0 - bubble_h,
            bubble_w,
            bubble_h,
        );
        let mut bottom = content_y + SWITCH_H;
        let ghub = state.ghub_notice.as_ref().map(|_| {
            let r = Rect::new(PAD_X + 8.0, bottom + 5.0, list_w - 16.0, GHUB_H);
            bottom = r.bottom();
            r
        });
        let footer = FooterGeom {
            line_y: foot_top,
            icon,
            label,
            info,
            info_hit,
            bubble,
            switch,
            switch_hit,
            hotkey,
            ghub,
        };
        Layout {
            width: WIDTH,
            height: bottom + PAD_BOTTOM,
            title,
            gear,
            rows,
            separator,
            empty,
            footer,
            welcome: None,
            settings: None,
        }
    }

    fn settings_page(ui: &crate::settings::UiSettings, m: &Metrics) -> Layout {
        let g = settings_layout::compute(ui, &m.settings, m.max_height);
        Layout {
            width: WIDTH,
            height: g.height,
            title: Rect::default(),
            gear: Rect::default(),
            rows: Vec::new(),
            separator: None,
            empty: None,
            footer: FooterGeom::default(),
            welcome: None,
            settings: Some(g),
        }
    }

    fn welcome_page(m: &Metrics) -> Layout {
        let x = 22.0;
        let w = WELCOME_TEXT_W;
        let mut y = 28.0;
        let mark = Rect::new(x, y, 52.0, 52.0);
        let title = Rect::new(x + 52.0 + 14.0, y, w - 66.0, 52.0);
        y += 52.0 + 18.0;
        let body = Rect::new(x, y, w, m.welcome_body_h);
        y += m.welcome_body_h + 16.0;
        let card_h = 14.0 + 17.0 + 6.0 + m.welcome_steps_h + 14.0;
        let tray_card = Rect::new(x, y, w, card_h);
        let tray_title = Rect::new(x + 14.0, y + 14.0, w - 28.0, 17.0);
        let tray_steps = Rect::new(x + 14.0, y + 14.0 + 17.0 + 6.0, w - 28.0, m.welcome_steps_h);
        y += card_h + 16.0;
        let credit = Rect::new(x, y, w, 16.0);
        y += 16.0 + 18.0;
        let button = Rect::new(x, y, w, 36.0);
        y += 36.0 + 22.0;
        Layout {
            width: WIDTH,
            height: y,
            title: Rect::default(),
            gear: Rect::default(),
            rows: Vec::new(),
            separator: None,
            empty: None,
            footer: FooterGeom::default(),
            welcome: Some(WelcomeGeom {
                mark,
                title,
                body,
                tray_card,
                tray_title,
                tray_steps,
                credit,
                button,
            }),
            settings: None,
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
            icon,
            title: Rect::new(mid_x, mid_top, mid_w, L1_H),
            capsule: Rect::new(mid_x, mid_top + L1_H + 4.0, mid_w, CAP_H),
            status: Rect::new(mid_x, mid_top + L1_H + 4.0 + CAP_H + 4.0, mid_w, L3_H),
            percent: Rect::new(x1 - PCT_W, mid_top, PCT_W, MID_H),
            detail: None,
        }
    }

    /// The open part of a card that starts at `top`, and the card's whole height.
    fn detail(has_warning: bool, x: f32, top: f32, w: f32) -> (DetailGeom, f32) {
        let dx = x + 12.0;
        let dw = w - 24.0;
        let mut y = top + CARD_H;
        let warn = has_warning.then(|| {
            let r = Rect::new(dx, y, dw, WARN_H);
            y += WARN_H + 8.0;
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
        y = area.bottom() + 8.0;
        let stats_top = y;
        let half = (dw - 12.0) / 2.0;
        let today = Rect::new(dx, y + 8.0, half, STATS_H);
        let health = Rect::new(dx + half + 12.0, y + 8.0, half, STATS_H);
        y = stats_top + 8.0 + STATS_H + 8.0;
        let bars_title = Rect::new(dx, y, dw, BARS_TITLE_H);
        let bars = Rect::new(dx, y + BARS_TITLE_H + 3.0, dw, BARS_H);
        let card_h = bars.bottom() + 10.0 - top;
        (
            DetailGeom {
                warn,
                chart_title,
                chart: ChartGeom { area, plot },
                stats_top,
                today,
                health,
                bars_title,
                bars,
            },
            card_h,
        )
    }

    /// What lies under the point (DIPs, relative to the flyout's top-left corner).
    pub fn hit(&self, x: f32, y: f32) -> FlyoutHit {
        self.hit_scrolled(x, y, 0.0)
    }

    /// `hit` for a page scrolled by `scroll` DIPs (only the Settings page scrolls).
    pub fn hit_scrolled(&self, x: f32, y: f32, scroll: f32) -> FlyoutHit {
        if let Some(g) = &self.settings {
            return match g.hit(x, y, scroll) {
                SettingsHit::Back => FlyoutHit::Back,
                SettingsHit::Row(id, part) => FlyoutHit::Setting { id, part },
                SettingsHit::None => FlyoutHit::None,
            };
        }
        if let Some(w) = &self.welcome {
            return if w.button.contains(x, y) {
                FlyoutHit::Start
            } else {
                FlyoutHit::None
            };
        }
        if self.gear.contains(x, y) {
            return FlyoutHit::Gear;
        }
        if self.footer.info_hit.contains(x, y) {
            return FlyoutHit::Info;
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
    fn the_open_flyout_fits_a_768_line_screen() {
        let (_, l) = lay();
        // One open card with warning, three closed ones and the footer: under what a 768 px high
        // laptop screen leaves above the taskbar (about 720 DIPs) with the edge margins.
        assert!(l.height > 560.0 && l.height < 696.0, "height {}", l.height);
        assert_eq!(l.width, 380.0);
    }

    #[test]
    fn every_device_sits_on_its_own_card_and_cards_never_overlap() {
        let (_, l) = lay();
        let order: Vec<usize> = l.rows.iter().map(|r| r.index).collect();
        assert_eq!(order, vec![0, 1, 3, 2]);
        for w in l.rows.windows(2) {
            assert!(
                w[0].bounds.bottom() + 5.0 <= w[1].bounds.y,
                "{:?} too close to {:?}",
                w[0].bounds,
                w[1].bounds
            );
        }
        for r in &l.rows[1..] {
            assert_eq!(r.bounds.h, 62.0, "a closed card");
        }
        assert!(l.rows[0].bounds.h > 250.0, "the open card");
        let sep = l.separator.unwrap();
        assert!(sep.y >= l.rows[2].bounds.bottom() && sep.bottom() <= l.rows[3].bounds.y);
        assert!(l.rows[3].bounds.bottom() < l.footer.line_y);
        assert!(l.footer.switch.bottom() < l.height);
    }

    #[test]
    fn everything_stays_inside_the_flyout() {
        let (mut s, _) = lay();
        s.ghub_notice = Some("o G HUB está reaplicando a iluminação".into());
        let l = Layout::compute(&s, Metrics::default());
        let inside =
            |r: Rect| r.x >= 0.0 && r.y >= 0.0 && r.right() <= l.width && r.bottom() <= l.height;
        assert!(inside(l.gear) && inside(l.footer.hotkey) && inside(l.footer.switch_hit));
        assert!(inside(l.footer.info_hit) && inside(l.footer.bubble));
        assert!(inside(l.footer.ghub.unwrap()));
        for r in &l.rows {
            assert!(inside(r.bounds) && inside(r.icon) && inside(r.percent) && inside(r.status));
            if let Some(d) = &r.detail {
                assert!(inside(d.chart.area) && inside(d.today) && inside(d.health));
                assert!(inside(d.bars) && inside(d.bars_title));
                assert!(d.chart.area.contains(d.chart.plot.x, d.chart.plot.y));
                assert!(d.bars.bottom() < r.bounds.bottom());
            }
        }
    }

    #[test]
    fn the_footer_controls_do_not_collide() {
        let (_, l) = lay();
        let f = l.footer;
        assert!(f.icon.right() < f.label.x);
        assert!(f.label.right() <= f.info.x);
        assert!(f.info.right() < f.switch.x);
        assert!(
            f.switch.right() < f.hotkey.x,
            "hotkey must clear the switch"
        );
    }

    #[test]
    fn the_g_hub_notice_gets_its_own_line_and_grows_the_flyout() {
        let (mut s, l) = lay();
        assert!(l.footer.ghub.is_none());
        s.ghub_notice = Some("o G HUB está reaplicando a iluminação".into());
        let l2 = Layout::compute(&s, Metrics::default());
        let g = l2.footer.ghub.unwrap();
        assert!(g.y >= l2.footer.switch.bottom() && g.bottom() < l2.height);
        assert!(l2.height > l.height + 10.0);
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
    fn the_week_bars_have_seven_equal_slots_across_the_card() {
        let (_, l) = lay();
        let d = l.rows[0].detail.as_ref().unwrap();
        let (first, last) = (d.bar_slot(0), d.bar_slot(6));
        assert_eq!(first.x, d.bars.x);
        assert!((last.right() - d.bars.right()).abs() < 0.01);
        assert!((first.w - d.bars.w / 7.0).abs() < 1e-4);
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
        // The (i) is its own control, even though it sits inside the switch's area.
        let (ix, iy) = c(l.footer.info);
        assert_eq!(l.hit(ix, iy), FlyoutHit::Info);
        // Display order is [0, 1, 3, 2]; hits report the device index.
        for (pos, idx) in [0usize, 1, 3, 2].into_iter().enumerate() {
            let (rx, ry) = c(l.rows[pos].bounds);
            assert_eq!(l.hit(rx, ry), FlyoutHit::Row(idx));
        }
        // The open card answers for its detail too.
        let d = l.rows[0].detail.as_ref().unwrap();
        let (cx, cy) = c(d.chart.area);
        assert_eq!(l.hit(cx, cy), FlyoutHit::Row(0));
        let (bx, by) = c(d.bars);
        assert_eq!(l.hit(bx, by), FlyoutHit::Row(0));
        assert_eq!(l.hit(-1.0, 5.0), FlyoutHit::None);
        assert_eq!(l.hit(5.0, 5.0), FlyoutHit::None);
        // The gap between two cards is not a card.
        let gap = l.rows[1].bounds.y - 2.0;
        assert_eq!(l.hit(100.0, gap), FlyoutHit::None);
        let sep = l.separator.unwrap();
        assert_eq!(l.hit(sep.x + 5.0, sep.cy()), FlyoutHit::None);
        assert_eq!(l.hit(l.width + 1.0, 100.0), FlyoutHit::None);
    }

    #[test]
    fn collapsing_the_card_shrinks_the_flyout() {
        let (mut s, l) = lay();
        s.devices[0].expanded = false;
        let l2 = Layout::compute(&s, Metrics::default());
        assert!(l2.height < l.height - 180.0);
        assert!(l2.rows[0].detail.is_none());
    }

    #[test]
    fn an_open_card_without_a_warning_is_shorter_by_the_warning_line() {
        let (mut s, l) = lay();
        s.devices[0].warning = None;
        let l2 = Layout::compute(&s, Metrics::default());
        assert!((l.rows[0].bounds.h - l2.rows[0].bounds.h - 38.0).abs() < 0.01);
        assert!(l2.rows[0].detail.as_ref().unwrap().warn.is_none());
    }

    #[test]
    fn no_off_device_means_no_separator() {
        let (mut s, _) = lay();
        s.devices[2].on = true;
        assert!(Layout::compute(&s, Metrics::default()).separator.is_none());
    }

    #[test]
    fn the_welcome_page_stacks_its_parts_and_only_the_button_answers() {
        let mut s = UiState::sample_welcome(Theme::Light);
        let l = Layout::compute(&s, Metrics::default());
        let w = l.welcome.unwrap();
        assert!(l.rows.is_empty() && l.separator.is_none());
        let stack = [w.mark, w.body, w.tray_card, w.credit, w.button];
        for p in stack.windows(2) {
            assert!(p[0].bottom() <= p[1].y, "{:?} overlaps {:?}", p[0], p[1]);
        }
        assert!(w.tray_steps.bottom() < w.tray_card.bottom());
        assert!(w.button.bottom() < l.height);
        assert!(w.button.right() <= l.width && w.button.x >= 0.0);
        let (bx, by) = (w.button.x + 5.0, w.button.cy());
        assert_eq!(l.hit(bx, by), FlyoutHit::Start);
        assert_eq!(l.hit(w.mark.x + 2.0, w.mark.y + 2.0), FlyoutHit::None);
        // Taller text makes a taller page.
        let tall = Metrics {
            welcome_body_h: 90.0,
            ..Metrics::default()
        };
        assert!(Layout::compute(&s, tall).height > l.height + 20.0);
        // Without the page the list comes back.
        s.welcome = None;
        assert!(Layout::compute(&s, Metrics::default()).welcome.is_none());
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
