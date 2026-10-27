// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The glyph drawings (24 x 24, stroke 1.75, round caps) ported by hand from
//! `docs/design/harness/glyphs.js`, plus the tray mark's SVG paths from `spec/design/mark/`.

use crate::model::Glyph;

/// One drawn element of a glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// SVG path data.
    Path(&'static str),
    /// A path rotated by degrees around (12, 12) (the gear's teeth).
    Rotated(&'static str, f32),
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rx: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
}

/// The glyph's view box edge.
pub const VIEWBOX: f32 = 24.0;
/// The glyph stroke width in view box units.
pub const STROKE: f32 = 1.75;

const TOOTH: &str = "M12 2.5v2";

/// The shapes of a glyph, in drawing order.
pub fn shapes(g: Glyph) -> &'static [Shape] {
    use Shape::*;
    match g {
        Glyph::Mouse => &[
            Path(
                "M12 3c-3.3 0-5.5 2.2-5.5 5.5v7c0 3.3 2.2 5.5 5.5 5.5s5.5-2.2 5.5-5.5v-7C17.5 5.2 15.3 3 12 3z",
            ),
            Path("M6.5 11h11"),
            Path("M12 3v2.5"),
            Rect {
                x: 10.75,
                y: 5.5,
                w: 2.5,
                h: 4.5,
                rx: 1.25,
            },
        ],
        Glyph::Keyboard => &[
            Rect {
                x: 2.5,
                y: 5.5,
                w: 19.0,
                h: 13.0,
                rx: 2.5,
            },
            Path(
                "M6 9h.01M9 9h.01M12 9h.01M15 9h.01M18 9h.01M6.5 12h.01M9.5 12h.01M12.5 12h.01M15.5 12h.01M17.5 12h.01M8 15.5h8",
            ),
        ],
        Glyph::Headset => &[
            Path("M4.5 14v-2a7.5 7.5 0 0 1 15 0v2"),
            Rect {
                x: 3.0,
                y: 13.0,
                w: 4.0,
                h: 6.5,
                rx: 1.75,
            },
            Rect {
                x: 17.0,
                y: 13.0,
                w: 4.0,
                h: 6.5,
                rx: 1.75,
            },
            Path("M5 19.5c0 1.6 1.4 2 3 2h3.5"),
        ],
        Glyph::Bolt => &[Path("M13 2.5L5.5 13.5H11l-1 8 7.5-11H12l1-8z")],
        Glyph::Bluetooth => &[Path("M7 7.5l10 9-5 4.5V3l5 4.5-10 9")],
        Glyph::Wireless => &[
            Circle {
                cx: 12.0,
                cy: 18.0,
                r: 1.0,
            },
            Path("M8.5 14.5a5 5 0 0 1 7 0"),
            Path("M5.5 11.5a9 9 0 0 1 13 0"),
            Path("M2.5 8.5a13 13 0 0 1 19 0"),
        ],
        Glyph::Warning => &[
            Path("M12 3.5l9.5 16.5h-19z"),
            Path("M12 10v4.5"),
            Path("M12 17.5h.01"),
        ],
        Glyph::Gear => &[
            Circle {
                cx: 12.0,
                cy: 12.0,
                r: 6.5,
            },
            Circle {
                cx: 12.0,
                cy: 12.0,
                r: 2.5,
            },
            Rotated(TOOTH, 0.0),
            Rotated(TOOTH, 45.0),
            Rotated(TOOTH, 90.0),
            Rotated(TOOTH, 135.0),
            Rotated(TOOTH, 180.0),
            Rotated(TOOTH, 225.0),
            Rotated(TOOTH, 270.0),
            Rotated(TOOTH, 315.0),
        ],
        Glyph::Leaf => &[
            Path(
                "M19.5 4.5c-9 0-14 4-14 10.5 0 1.4.3 2.7 1 3.8 1.2.8 2.6 1.2 4 1.2 6 0 9-5 9-15.5z",
            ),
            Path("M5.5 20.5C8 15 11 12 15 9.5"),
        ],
        Glyph::Gamepad => &[
            Path(
                "M7.5 7h9c2.8 0 4.6 2.3 5 5.5l.4 3.5c.2 1.9-1.1 3.5-2.8 3.5-1.1 0-2-.6-2.5-1.6L15.8 16H8.2l-.8 1.9c-.5 1-1.4 1.6-2.5 1.6-1.7 0-3-1.6-2.8-3.5l.4-3.5C2.9 9.3 4.7 7 7.5 7z",
            ),
            Path("M7.5 10.5v3M6 12h3"),
            Path("M15.5 11h.01M17.5 13h.01"),
        ],
        Glyph::Earbuds => &[
            Path(
                "M7 4.5a3 3 0 0 1 3 3v2.5a3 3 0 0 1-2 2.8V19a1.5 1.5 0 0 1-3 0v-9.5a5 5 0 0 1 2-5z",
            ),
            Path(
                "M17 4.5a3 3 0 0 0-3 3v2.5a3 3 0 0 0 2 2.8V19a1.5 1.5 0 0 0 3 0v-9.5a5 5 0 0 0-2-5z",
            ),
        ],
    }
}

/// Every glyph, for tests.
pub const ALL: [Glyph; 11] = [
    Glyph::Mouse,
    Glyph::Keyboard,
    Glyph::Headset,
    Glyph::Gamepad,
    Glyph::Earbuds,
    Glyph::Bolt,
    Glyph::Bluetooth,
    Glyph::Wireless,
    Glyph::Warning,
    Glyph::Gear,
    Glyph::Leaf,
];

// The tray mark, 16 x 16 view box (`spec/design/mark/juicebar-mark*.svg`).
pub const MARK_VIEWBOX: f32 = 16.0;
/// The juice, filled (red for the warning state, cut by the bolt for the charging state).
pub const MARK_JUICE: &str = "M4.2 9.5 H11.55 L10.75 14 H5 Z";
/// The glass outline, stroked 1.5 with round joins and caps.
pub const MARK_GLASS: &str = "M3.5 4.75 L5 14.25 H10.75 L12.25 4.75";
/// The straw, stroked 1.5 with round joins and caps.
pub const MARK_STRAW: &str = "M7.25 9.5 L9 2 H13";
/// The bolt cut out of the juice in the charging state.
pub const MARK_BOLT: &str = "M8.6 10.2 L6.5 12.4 H7.7 L7.1 13.8 L9.3 11.6 H8.1 Z";
pub const MARK_STROKE: f32 = 1.5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svgpath;

    #[test]
    fn every_path_parses() {
        for g in ALL {
            for sh in shapes(g) {
                if let Shape::Path(d) | Shape::Rotated(d, _) = sh {
                    assert!(svgpath::parse(d).is_ok(), "{g:?}: {d}");
                }
            }
        }
        for d in [MARK_JUICE, MARK_GLASS, MARK_STRAW, MARK_BOLT] {
            assert!(svgpath::parse(d).is_ok(), "{d}");
        }
    }

    #[test]
    fn gear_has_eight_teeth() {
        let teeth = shapes(Glyph::Gear)
            .iter()
            .filter(|s| matches!(s, Shape::Rotated(..)))
            .count();
        assert_eq!(teeth, 8);
    }

    #[test]
    fn mark_paths_stay_inside_the_view_box() {
        for d in [MARK_JUICE, MARK_GLASS, MARK_STRAW, MARK_BOLT] {
            for s in svgpath::parse(d).unwrap() {
                if let svgpath::Seg::Move(x, y) | svgpath::Seg::Line(x, y) = s {
                    assert!((0.0..=MARK_VIEWBOX).contains(&x) && (0.0..=MARK_VIEWBOX).contains(&y));
                }
            }
        }
    }
}
