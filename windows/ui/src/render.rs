// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The render seam: the flyout drawn offscreen (WIC bitmap) exactly as the window draws it,
//! for tests and for `cargo run -p juicebar-ui --example flyout_demo -- --render <folder>`.

use std::path::Path;

use windows::core::Result;

use crate::draw::{PaintOptions, measure_metrics, paint_flyout};
use crate::gfx::{Color, Gfx, Pixels, render_offscreen, write_png};
use crate::layout::{FlyoutHit, Layout, size_px};
use crate::model::{Theme, UiState};

/// The "wallpaper" behind the panel in a PNG, so the rounded corners and edges can be judged.
pub fn backdrop(theme: Theme) -> Color {
    match theme {
        Theme::Light => Color::hex(0xcfd8f2),
        Theme::Dark => Color::hex(0x10142c),
    }
}

/// Pixel size of the flyout for `state` at `scale` (pixels per DIP).
pub fn flyout_size(state: &UiState, scale: f32) -> Result<(u32, u32)> {
    let gfx = Gfx::new()?;
    let layout = Layout::compute(state, measure_metrics(&gfx, state)?);
    let (w, h) = size_px(layout.width, layout.height, scale);
    Ok((w as u32, h as u32))
}

/// The flyout as premultiplied BGRA pixels, with a solid panel (no system backdrop offscreen).
pub fn flyout_pixels(state: &UiState, scale: f32) -> Result<Pixels> {
    let gfx = Gfx::new()?;
    let layout = Layout::compute(state, measure_metrics(&gfx, state)?);
    let (w, h) = size_px(layout.width, layout.height, scale);
    render_offscreen(&gfx, w as u32, h as u32, scale, |c| {
        paint_flyout(
            c,
            state,
            &layout,
            PaintOptions {
                solid_panel: true,
                clear: Some(backdrop(state.theme)),
                hover: FlyoutHit::None,
            },
        )
    })
}

/// Writes the flyout as a PNG in `theme` at `scale`; returns its pixel size.
pub fn render_flyout_png(
    state: &UiState,
    theme: Theme,
    scale: f32,
    path: &Path,
) -> Result<(u32, u32)> {
    let mut st = state.clone();
    st.theme = theme;
    let px = flyout_pixels(&st, scale)?;
    write_png(path, &px)?;
    Ok((px.width, px.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_size(path: &Path) -> (u32, u32) {
        let b = std::fs::read(path).unwrap();
        assert_eq!(
            &b[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
            "PNG signature"
        );
        (
            u32::from_be_bytes(b[16..20].try_into().unwrap()),
            u32::from_be_bytes(b[20..24].try_into().unwrap()),
        )
    }

    #[test]
    fn flyout_pngs_have_the_expected_size_in_both_themes_and_scales() {
        let dir = std::env::temp_dir().join("juicebar-ui-render-test");
        std::fs::create_dir_all(&dir).unwrap();
        let s = UiState::sample(Theme::Light);
        let gfx = Gfx::new().unwrap();
        let layout = Layout::compute(&s, measure_metrics(&gfx, &s).unwrap());
        for (theme, name) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
            for scale in [1.0f32, 1.5, 2.0] {
                let f = dir.join(format!("flyout-{name}-{scale}.png"));
                let (w, h) = render_flyout_png(&s, theme, scale, &f).unwrap();
                let (ew, eh) = size_px(layout.width, layout.height, scale);
                assert_eq!((w, h), (ew as u32, eh as u32));
                assert_eq!(w, (380.0 * scale).ceil() as u32);
                assert!(
                    std::fs::metadata(&f).unwrap().len() > 5_000,
                    "{f:?} too small"
                );
                assert_eq!(png_size(&f), (w, h));
            }
        }
    }

    #[test]
    fn labels_are_drawn_from_the_state() {
        let s = UiState::sample(Theme::Light);
        assert_eq!(s.labels, crate::model::UiLabels::default());
        let pt = flyout_pixels(&s, 1.0).unwrap();
        let mut en = s.clone();
        en.labels.last_7_days = "Last 7 days".into();
        en.labels.rgb_economy = "Power saver".into();
        let px = flyout_pixels(&en, 1.0).unwrap();
        assert_eq!((pt.width, pt.height), (px.width, px.height));
        assert_ne!(pt.data, px.data);
    }

    #[test]
    fn dark_and_light_differ_and_have_ink() {
        let s = UiState::sample(Theme::Light);
        let l = flyout_pixels(&s, 1.0).unwrap();
        let mut sd = s.clone();
        sd.theme = Theme::Dark;
        let d = flyout_pixels(&sd, 1.0).unwrap();
        assert_ne!(l.data, d.data);
        // The panel centre is opaque in both.
        let mid = ((l.height / 2 * l.width + 5) * 4 + 3) as usize;
        assert_eq!(l.data[mid], 255);
        assert_eq!(d.data[mid], 255);
        // The panel is lighter than the text in light and darker in dark.
        let lum = |p: &Pixels| {
            p.data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| c[0] as u64 + c[1] as u64 + c[2] as u64)
                .sum::<u64>()
                / (p.data.len() as u64 / 4)
        };
        assert!(lum(&l) > lum(&d) + 200);
    }
}
