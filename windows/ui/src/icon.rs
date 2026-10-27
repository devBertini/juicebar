// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The tray mark: a juice glass with a straw, drawn with Direct2D from the SVG paths of
//! `spec/design/mark/`. The glass outline and the straw take the taskbar's foreground; the juice
//! takes the state's colour (green, grape purple, red). A `Frame` holds the animation phase: the
//! juice surface tilts and ripples (a gentle slosh that dies away), the straw sways a degree or two
//! around its foot, and while a device charges a few bubbles rise inside the juice. `Frame::STILL`
//! is the mark at rest, exactly the design.
//!
//! The mark fills the icon's height like the system's own tray icons (whose ink spans the whole
//! 16 px slot at 100 %), and is fitted to the pixel grid at every size: the stroke is a whole
//! number of pixels, and the straw's top bar, the glass bottom and the juice surface sit on pixel
//! edges, so the horizontal lines stay sharp at rest.

use std::f32::consts::{PI, TAU};
use std::path::Path;

use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};
use windows::core::{Error, Result};

use crate::gfx::{Canvas, Color, Gfx, Pixels, render_offscreen, write_png};
use crate::glyphs::{MARK_GLASS, MARK_STROKE};
use crate::model::IconState;
use crate::svgpath::{self, Seg};

/// The colour of the juice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Juice {
    Green,
    /// Grape juice, the resting colour.
    #[default]
    Purple,
    Red,
}

/// What the mark shows: the juice colour, and whether bubbles may rise (a device charges).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IconLook {
    pub juice: Juice,
    pub charging: bool,
}

impl From<IconState> for IconLook {
    /// The look of the flyout's alert state (the flyout's own drawing).
    fn from(s: IconState) -> IconLook {
        match s {
            IconState::Normal => IconLook::default(),
            IconState::Warning => IconLook {
                juice: Juice::Red,
                charging: false,
            },
            IconState::Charging => IconLook {
                juice: Juice::Purple,
                charging: true,
            },
        }
    }
}

/// White on a dark taskbar, near-black on a light one.
pub fn foreground(dark_taskbar: bool) -> Color {
    if dark_taskbar {
        Color::hex(0xffffff)
    } else {
        Color::hex(0x1b1b1b)
    }
}

/// The juice colour on a taskbar. Each is at least 3:1 against the taskbar's own colour
/// (`taskbar_background`), the contrast WCAG asks of graphics; the tests measure them.
pub fn juice_color(juice: Juice, dark_taskbar: bool) -> Color {
    Color::hex(match (juice, dark_taskbar) {
        (Juice::Purple, true) => 0xb36be0,
        (Juice::Purple, false) => 0x8e44ad,
        (Juice::Green, true) => 0x4ade80,
        (Juice::Green, false) => 0x159347,
        (Juice::Red, true) => 0xff6369,
        (Juice::Red, false) => 0xe5484d,
    })
}

/// The bubbles: pale on the dark juice of a light taskbar, dark on the pale juice of a dark one, so
/// they read on both.
fn bubble_color(dark_taskbar: bool) -> Color {
    if dark_taskbar {
        Color::hex(0x202020)
    } else {
        Color::hex(0xffffff)
    }
}

/// The taskbar's own background, for the PNG seam and the contrast measurements.
pub fn taskbar_background(dark_taskbar: bool) -> Color {
    if dark_taskbar {
        Color::hex(0x202020)
    } else {
        Color::hex(0xf3f3f3)
    }
}

/// WCAG contrast ratio between two colours (1.0 to 21.0).
pub fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

// ---- the animation phase

/// Frames per second of a run.
pub const RUN_FPS: u32 = 20;
/// Frames in a run: 2 seconds.
pub const RUN_FRAMES: usize = 40;

/// The surface tilts at most this many degrees.
const TILT_DEG: f32 = 6.0;
/// The straw sways at most this many degrees around its foot.
const SWAY_DEG: f32 = 2.0;
/// The surface ripple, in view-box units.
const RIPPLE: f32 = 0.22;

/// One bubble in view-box units; `alpha` is 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bubble {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub alpha: f32,
}

/// The animation phase of one picture of the mark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Tilt of the juice surface in degrees (positive lowers the right side).
    pub tilt: f32,
    /// Height of the ripple on the surface, in view-box units.
    pub ripple: f32,
    /// Phase of the ripple in radians.
    pub ripple_phase: f32,
    /// Rotation of the straw about its foot in degrees (positive leans it to the right).
    pub sway: f32,
    /// The bubbles inside the juice that are rising now.
    pub bubbles: [Option<Bubble>; 3],
}

impl Frame {
    /// The mark at rest.
    pub const STILL: Frame = Frame {
        tilt: 0.0,
        ripple: 0.0,
        ripple_phase: 0.0,
        sway: 0.0,
        bubbles: [None; 3],
    };
}

/// A run of the animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Run {
    /// The liquid sloshes and the straw sways, then both settle.
    Slosh,
    /// Three bubbles rise, one after the other.
    Bubbles,
}

/// Where each bubble rises (x), its size, when it starts (seconds) and how long it takes.
const BUBBLES: [(f32, f32, f32, f32); 3] = [
    (6.4, 0.62, 0.10, 1.00),
    (9.9, 0.55, 0.50, 0.95),
    (8.5, 0.50, 0.85, 1.00),
];
/// A bubble starts near the glass bottom and pops just under the surface.
const BUBBLE_FROM: f32 = 13.3;
const BUBBLE_TO: f32 = 10.4;

/// Picture `i` (0 to `RUN_FRAMES - 1`) of a run. Picture 0 is the mark at rest and the motion has
/// died away before the last one, so the run ends where it began.
pub fn run_frame(run: Run, i: usize) -> Frame {
    let t = i as f32 / RUN_FPS as f32;
    match run {
        Run::Slosh => {
            let decay = (-t / 0.75).exp();
            let swell = 1.0 - (-t / 0.12).exp();
            Frame {
                tilt: TILT_DEG * decay * (TAU * t / 0.9).sin(),
                ripple: RIPPLE * decay * swell,
                ripple_phase: -TAU * t / 0.7,
                sway: SWAY_DEG * (-t / 0.9).exp() * (TAU * t / 1.25).sin(),
                bubbles: [None; 3],
            }
        }
        Run::Bubbles => {
            let mut bubbles = [None; 3];
            for (slot, &(x, r, start, dur)) in bubbles.iter_mut().zip(&BUBBLES) {
                let p = (t - start) / dur;
                if p > 0.0 && p < 1.0 {
                    *slot = Some(Bubble {
                        x: x + 0.25 * (TAU * p * 1.5).sin(),
                        y: BUBBLE_FROM + (BUBBLE_TO - BUBBLE_FROM) * p,
                        r,
                        alpha: (PI * p).sin().powf(0.6),
                    });
                }
            }
            Frame {
                bubbles,
                ..Frame::STILL
            }
        }
    }
}

// ---- the fit to the pixel grid

// Lines of the mark's view box the fit puts on pixel edges.
/// Centre line of the straw's top bar.
const TOP: f32 = 2.0;
/// The juice surface.
const JUICE: f32 = 9.5;
/// Centre line of the glass bottom.
const BOTTOM: f32 = 14.25;
/// Middle between the glass's left rim and the straw's end: the mark's horizontal centre.
const CENTRE_X: f32 = (3.5 + 13.0) / 2.0;

/// How the mark's 16-unit view box lands on a `px` x `px` icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkFit {
    /// Pixels per view-box unit.
    pub k: f32,
    /// Stroke width in pixels (whole).
    pub stroke: f32,
    ox: f32,
    top: f32,
    juice: f32,
    bottom: f32,
}

impl MarkFit {
    pub fn new(px: f32) -> MarkFit {
        // The design's ink is 13.75 units tall (stroke included) and should span `px`.
        let stroke = (MARK_STROKE * px / (BOTTOM - TOP + MARK_STROKE))
            .round()
            .max(1.0);
        let k = (px - stroke) / (BOTTOM - TOP);
        let top = stroke / 2.0;
        MarkFit {
            k,
            stroke,
            ox: px / 2.0 - CENTRE_X * k,
            top,
            juice: (top + (JUICE - TOP) * k).round(),
            bottom: px - stroke / 2.0,
        }
    }

    /// A view-box point in pixels.
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        let py = if y <= JUICE {
            self.top + (y - TOP) * (self.juice - self.top) / (JUICE - TOP)
        } else {
            self.juice + (y - JUICE) * (self.bottom - self.juice) / (BOTTOM - JUICE)
        };
        (self.ox + x * self.k, py)
    }

    /// SVG path data in view-box units as segments in pixels.
    fn segments(&self, d: &str) -> Result<Vec<Seg>> {
        let segs =
            svgpath::parse(d).map_err(|_| Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
        let p = |x, y| self.point(x, y);
        Ok(segs
            .into_iter()
            .map(|s| match s {
                Seg::Move(x, y) => {
                    let (x, y) = p(x, y);
                    Seg::Move(x, y)
                }
                Seg::Line(x, y) => {
                    let (x, y) = p(x, y);
                    Seg::Line(x, y)
                }
                Seg::Cubic(x1, y1, x2, y2, x, y) => {
                    let ((x1, y1), (x2, y2), (x, y)) = (p(x1, y1), p(x2, y2), p(x, y));
                    Seg::Cubic(x1, y1, x2, y2, x, y)
                }
                Seg::Arc {
                    rx,
                    ry,
                    rot,
                    large,
                    sweep,
                    x,
                    y,
                } => {
                    let (x, y) = p(x, y);
                    Seg::Arc {
                        rx: rx * self.k,
                        ry: ry * self.k,
                        rot,
                        large,
                        sweep,
                        x,
                        y,
                    }
                }
                Seg::Close => Seg::Close,
            })
            .collect())
    }

    /// The straw in pixels (foot, bend, end), leaning `sway` degrees about its foot. It is nudged
    /// down just enough to keep its top bar inside the icon.
    pub fn straw_points(&self, sway: f32) -> [(f32, f32); 3] {
        let mut pts = STRAW.map(|(x, y)| self.point(x, y));
        if sway != 0.0 {
            let (s, c) = sway.to_radians().sin_cos();
            let (fx, fy) = pts[0];
            for p in &mut pts {
                let (dx, dy) = (p.0 - fx, p.1 - fy);
                *p = (fx + dx * c - dy * s, fy + dx * s + dy * c);
            }
            let top = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min) - self.stroke / 2.0;
            if top < 0.0 {
                for p in &mut pts {
                    p.1 -= top;
                }
            }
        }
        pts
    }

    /// The juice polygon of a frame in pixels.
    pub fn juice_points(&self, frame: &Frame) -> Vec<(f32, f32)> {
        juice_outline(frame)
            .into_iter()
            .map(|(x, y)| self.point(x, y))
            .collect()
    }
}

/// The juice surface's middle, in view-box units.
const SURFACE_CX: f32 = (4.2 + 11.55) / 2.0;
/// The inner glass wall slants 0.8 units over 4.5 (from the juice polygon of the design).
const WALL_SLANT: f32 = 0.8 / 4.5;
/// Samples along the surface.
const SURFACE_STEPS: usize = 12;
/// The straw: foot, bend and end, in view-box units (`MARK_STRAW`).
const STRAW: [(f32, f32); 3] = [(7.25, 9.5), (9.0, 2.0), (13.0, 2.0)];

/// Height of the juice surface at `x`.
fn surface_y(f: &Frame, x: f32) -> f32 {
    JUICE
        + f.tilt.to_radians().tan() * (x - SURFACE_CX)
        + f.ripple * ((x - 4.2) / (11.55 - 4.2) * TAU + f.ripple_phase).sin()
}

/// Where the glass wall is at height `y`.
fn wall_x(y: f32, left: bool) -> f32 {
    let d = (y - JUICE) * WALL_SLANT;
    if left { 4.2 + d } else { 11.55 - d }
}

/// The juice as a polygon in view-box units: the surface (tilted and rippled) between the glass
/// walls, then the bottom. At rest it is the design's `MARK_JUICE`.
fn juice_outline(f: &Frame) -> Vec<(f32, f32)> {
    let (mut xl, mut xr) = (4.2, 11.55);
    for _ in 0..3 {
        xl = wall_x(surface_y(f, xl), true);
        xr = wall_x(surface_y(f, xr), false);
    }
    let mut pts: Vec<(f32, f32)> = (0..=SURFACE_STEPS)
        .map(|i| {
            let x = xl + (xr - xl) * i as f32 / SURFACE_STEPS as f32;
            (x, surface_y(f, x))
        })
        .collect();
    pts.push((10.75, 14.0));
    pts.push((5.0, 14.0));
    pts
}

// ---- drawing

/// Draws the mark filling a `px` x `px` canvas whose transform is the identity.
pub fn draw_mark(
    c: &Canvas,
    look: IconLook,
    frame: &Frame,
    dark_taskbar: bool,
    px: f32,
) -> Result<()> {
    let fg = foreground(dark_taskbar);
    let fit = MarkFit::new(px);
    let juice = c.polyline(&fit.juice_points(frame), true, true)?;
    c.fill_geometry(&juice.into(), juice_color(look.juice, dark_taskbar));
    for b in frame.bubbles.iter().flatten() {
        let (x, y) = fit.point(b.x, b.y);
        // Under a pixel a bubble would vanish: keep it a visible dot.
        let r = (b.r * fit.k).max(0.6);
        c.fill_circle(x, y, r, bubble_color(dark_taskbar).fade(0.9 * b.alpha));
    }
    let glass = c.geometry(&fit.segments(MARK_GLASS)?, false)?;
    c.stroke_geometry(&glass.into(), fit.stroke, fg, None);
    let straw = c.polyline(&fit.straw_points(frame.sway), false, false)?;
    c.stroke_geometry(&straw.into(), fit.stroke, fg, None);
    Ok(())
}

/// The mark as premultiplied BGRA pixels on a transparent background.
pub fn mark_pixels(look: IconLook, frame: &Frame, dark_taskbar: bool, px: u32) -> Result<Pixels> {
    mark_pixels_with(&Gfx::new()?, look, frame, dark_taskbar, px)
}

/// `mark_pixels` with a factory the caller keeps (a whole run is drawn with one).
pub fn mark_pixels_with(
    gfx: &Gfx,
    look: IconLook,
    frame: &Frame,
    dark_taskbar: bool,
    px: u32,
) -> Result<Pixels> {
    render_offscreen(gfx, px, px, 1.0, |c| {
        c.clear(Color::rgba(0, 0.0));
        draw_mark(c, look, frame, dark_taskbar, px as f32)
    })
}

/// The mark as an `HICON` for `Shell_NotifyIcon`. The caller owns it (`DestroyIcon`).
pub fn render_mark(look: IconLook, frame: &Frame, dark_taskbar: bool, px: u32) -> Result<HICON> {
    icon_from_pixels(&mark_pixels(look, frame, dark_taskbar, px)?)
}

/// Every picture of a run as an `HICON`, drawn with one factory. The caller owns them all
/// (`DestroyIcon`); on an error none is left behind.
pub fn render_run(look: IconLook, run: Run, dark_taskbar: bool, px: u32) -> Result<Vec<HICON>> {
    let gfx = Gfx::new()?;
    let mut icons = Vec::with_capacity(RUN_FRAMES);
    for i in 0..RUN_FRAMES {
        let made = mark_pixels_with(&gfx, look, &run_frame(run, i), dark_taskbar, px)
            .and_then(|p| icon_from_pixels(&p));
        match made {
            Ok(icon) => icons.push(icon),
            Err(e) => {
                for icon in icons {
                    // SAFETY: each icon was made above and handed to nobody.
                    unsafe {
                        let _ = DestroyIcon(icon);
                    }
                }
                return Err(e);
            }
        }
    }
    Ok(icons)
}

/// Writes the mark at rest on the taskbar's own color as a PNG (tests and the demo).
pub fn render_mark_png(
    look: impl Into<IconLook>,
    dark_taskbar: bool,
    px: u32,
    path: &Path,
) -> Result<()> {
    let px_data = mark_on_background(look.into(), &Frame::STILL, dark_taskbar, px)?;
    write_png(path, &px_data)
}

/// Mark pixels composited over the taskbar color (opaque result).
pub fn mark_on_background(
    look: IconLook,
    frame: &Frame,
    dark_taskbar: bool,
    px: u32,
) -> Result<Pixels> {
    let m = mark_pixels(look, frame, dark_taskbar, px)?;
    Ok(over_background(m, dark_taskbar))
}

/// `m` composited over the taskbar color.
pub fn over_background(mut m: Pixels, dark_taskbar: bool) -> Pixels {
    let bg = taskbar_background(dark_taskbar);
    let bgp = [
        (bg.b * 255.0).round() as u32,
        (bg.g * 255.0).round() as u32,
        (bg.r * 255.0).round() as u32,
    ];
    for p in m.data.as_chunks_mut::<4>().0 {
        let inv = 255 - p[3] as u32;
        for (c, b) in p[..3].iter_mut().zip(bgp) {
            *c = (*c as u32 + (b * inv + 127) / 255).min(255) as u8;
        }
        p[3] = 255;
    }
    m
}

/// A contact sheet of every juice at 16, 24 and 32 px on both taskbar colors, each magnified 8x
/// without smoothing so the pixels can be judged.
pub fn render_mark_sheet_png(path: &Path) -> Result<()> {
    const CELL: u32 = 32 * 8;
    let juices = [Juice::Purple, Juice::Green, Juice::Red];
    let sizes = [16u32, 24, 32];
    let cols = (sizes.len() * 2) as u32;
    let (w, h) = (CELL * cols, CELL * juices.len() as u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (row, juice) in juices.iter().enumerate() {
        let look = IconLook {
            juice: *juice,
            charging: false,
        };
        for (ci, dark) in [true, false].into_iter().enumerate() {
            for (si, px) in sizes.iter().enumerate() {
                let col = (ci * sizes.len() + si) as u32;
                let m = mark_on_background(look, &Frame::STILL, dark, *px)?;
                let bg = taskbar_background(dark);
                let (x0, y0) = (col * CELL, row as u32 * CELL);
                for y in 0..CELL {
                    for x in 0..CELL {
                        let (sx, sy) = (
                            x as i32 / 8 - (32 - *px as i32) / 2,
                            y as i32 / 8 - (32 - *px as i32) / 2,
                        );
                        let o = (((y0 + y) * w + x0 + x) * 4) as usize;
                        // The mark is centered in its cell; outside it the taskbar color shows.
                        let src = if (0..*px as i32).contains(&sx) && (0..*px as i32).contains(&sy)
                        {
                            let i = ((sy as u32 * px + sx as u32) * 4) as usize;
                            [m.data[i], m.data[i + 1], m.data[i + 2]]
                        } else {
                            [
                                (bg.b * 255.0) as u8,
                                (bg.g * 255.0) as u8,
                                (bg.r * 255.0) as u8,
                            ]
                        };
                        data[o..o + 3].copy_from_slice(&src);
                        data[o + 3] = 255;
                    }
                }
            }
        }
    }
    write_png(
        path,
        &Pixels {
            width: w,
            height: h,
            data,
        },
    )
}

/// Builds an `HICON` from premultiplied BGRA pixels.
fn icon_from_pixels(px: &Pixels) -> Result<HICON> {
    let (w, h) = (px.width as i32, px.height as i32);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `bmi` describes a 32-bit top-down DIB and `bits` is a valid out pointer; no DC is needed for DIB_RGB_COLORS.
    let color = unsafe { CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)? };
    if bits.is_null() {
        // SAFETY: `color` was just created and is not selected anywhere.
        unsafe {
            let _ = DeleteObject(color.into());
        }
        return Err(Error::from(windows::Win32::Foundation::E_OUTOFMEMORY));
    }
    // Icons take straight (non-premultiplied) alpha in the color bitmap.
    let straight = crate::gfx::unpremultiply(&px.data);
    // SAFETY: the DIB holds exactly w * h * 4 bytes and `straight` has that length.
    unsafe { std::ptr::copy_nonoverlapping(straight.as_ptr(), bits as *mut u8, straight.len()) };
    // SAFETY: a 1-bpp mask of the same size; all zero means every pixel is drawn (alpha rules).
    let mask = unsafe { CreateBitmap(w, h, 1, 1, None) };
    let info = ICONINFO {
        fIcon: true.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    // SAFETY: `info` holds two valid bitmaps; the icon copies them.
    let icon = unsafe { CreateIconIndirect(&info) };
    // SAFETY: the bitmaps are ours and no longer needed once the icon exists.
    unsafe {
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx};

    const LOOKS: [Juice; 3] = [Juice::Green, Juice::Purple, Juice::Red];
    const SIZES: [u32; 4] = [16, 20, 24, 32];

    fn look(juice: Juice) -> IconLook {
        IconLook {
            juice,
            charging: false,
        }
    }

    fn alpha_at(p: &Pixels, x: u32, y: u32) -> u8 {
        p.data[((y * p.width + x) * 4 + 3) as usize]
    }

    fn rgb_at(p: &Pixels, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * p.width + x) * 4) as usize;
        [p.data[i + 2], p.data[i + 1], p.data[i]]
    }

    fn rgb_of(c: Color) -> [u8; 3] {
        [
            (c.r * 255.0).round() as u8,
            (c.g * 255.0).round() as u8,
            (c.b * 255.0).round() as u8,
        ]
    }

    fn still(l: IconLook, dark: bool, px: u32) -> Pixels {
        mark_pixels(l, &Frame::STILL, dark, px).unwrap()
    }

    #[test]
    fn mark_has_ink_and_transparency_at_every_size() {
        for px in SIZES {
            for juice in LOOKS {
                let m = still(look(juice), true, px);
                assert_eq!((m.width, m.height), (px, px));
                let ink = m
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[3] > 200)
                    .count();
                let clear = m
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[3] == 0)
                    .count();
                assert!(ink > (px * px / 12) as usize, "{juice:?} {px}: ink {ink}");
                assert!(
                    clear > (px * px / 4) as usize,
                    "{juice:?} {px}: clear {clear}"
                );
            }
        }
    }

    /// The pixel holding view-box point (x, y) of the mark at `px`.
    fn at(px: u32, x: f32, y: f32) -> (u32, u32) {
        let (fx, fy) = MarkFit::new(px as f32).point(x, y);
        (fx as u32, fy as u32)
    }

    #[test]
    fn glass_and_straw_take_the_taskbar_foreground_and_the_juice_its_colour() {
        for dark in [true, false] {
            for juice in LOOKS {
                let m = still(look(juice), dark, 32);
                // Inside the juice (view box 8, 12).
                let (x, y) = at(32, 8.0, 12.0);
                assert_eq!(
                    rgb_at(&m, x, y),
                    rgb_of(juice_color(juice, dark)),
                    "{juice:?} dark={dark}"
                );
                // The straw's top bar (view box 11, 2) is the foreground, never the juice.
                let (x, y) = at(32, 11.0, 2.0);
                assert_eq!(rgb_at(&m, x, y), rgb_of(foreground(dark)));
            }
        }
    }

    /// Like the system's tray icons, the ink spans the icon's full height at every size; the
    /// straw's top bar and the glass bottom are whole rows of solid ink (sharp, not smeared).
    #[test]
    fn mark_fills_the_height_on_the_pixel_grid() {
        for px in SIZES {
            let m = still(look(Juice::Purple), true, px);
            let fit = MarkFit::new(px as f32);
            let sw = fit.stroke as u32;
            let row_ink = |y: u32| (0..px).filter(|&x| alpha_at(&m, x, y) > 0).count();
            assert!(
                row_ink(0) > 0 && row_ink(px - 1) > 0,
                "{px}: ink from top to bottom"
            );
            // The straw's bar between view box x 10 and 12: solid for `sw` rows, clear below.
            let (x0, _) = at(px, 10.0, 2.0);
            let (x1, _) = at(px, 12.0, 2.0);
            for x in x0 + 1..x1 {
                for y in 0..sw {
                    assert_eq!(alpha_at(&m, x, y), 255, "{px}: bar at ({x}, {y})");
                }
                assert_eq!(alpha_at(&m, x, sw), 0, "{px}: under the bar at {x}");
            }
            // The glass bottom between view box x 6 and 10: solid in its last `sw` rows.
            let (b0, _) = at(px, 6.0, 14.25);
            let (b1, _) = at(px, 10.0, 14.25);
            for x in b0..=b1 {
                for y in px - sw..px {
                    assert_eq!(alpha_at(&m, x, y), 255, "{px}: bottom at ({x}, {y})");
                }
            }
            // Wide and centred.
            let cols: Vec<u32> = (0..px)
                .filter(|&x| (0..px).any(|y| alpha_at(&m, x, y) > 128))
                .collect();
            let (l, r) = (cols[0], *cols.last().unwrap());
            assert!(r - l + 1 >= px * 3 / 4, "{px}: ink {l}..{r}");
            assert!(
                (l as i32 - (px - 1 - r) as i32).abs() <= 1,
                "{px}: off centre {l}..{r}"
            );
        }
    }

    #[test]
    fn at_rest_the_juice_is_the_designs_polygon() {
        let pts = juice_outline(&Frame::STILL);
        assert_eq!(pts.len(), SURFACE_STEPS + 3);
        assert_eq!(pts[0], (4.2, 9.5));
        let last = pts[SURFACE_STEPS];
        assert!((last.0 - 11.55).abs() < 1e-4 && last.1 == 9.5, "{last:?}");
        assert_eq!(&pts[SURFACE_STEPS + 1..], &[(10.75, 14.0), (5.0, 14.0)]);
        assert!(pts[..=SURFACE_STEPS].iter().all(|p| p.1 == 9.5));
    }

    // ---- colours

    #[test]
    fn juice_reads_against_both_taskbars() {
        for dark in [true, false] {
            let bg = taskbar_background(dark);
            for juice in LOOKS {
                let ratio = contrast(juice_color(juice, dark), bg);
                assert!(ratio >= 3.0, "{juice:?} dark={dark}: {ratio:.2}:1");
            }
        }
    }

    #[test]
    fn the_three_juices_are_clearly_different_colours() {
        for dark in [true, false] {
            for (i, a) in LOOKS.iter().enumerate() {
                for b in &LOOKS[i + 1..] {
                    let (ca, cb) = (juice_color(*a, dark), juice_color(*b, dark));
                    let d = (ca.r - cb.r).abs() + (ca.g - cb.g).abs() + (ca.b - cb.b).abs();
                    assert!(d > 0.7, "{a:?} vs {b:?} dark={dark}: {d:.2}");
                }
            }
        }
    }

    #[test]
    fn bubbles_read_against_the_juice() {
        for dark in [true, false] {
            for juice in LOOKS {
                let ratio = contrast(bubble_color(dark), juice_color(juice, dark));
                assert!(ratio >= 2.0, "{juice:?} dark={dark}: {ratio:.2}:1");
            }
        }
    }

    // ---- the phase of the animation

    #[test]
    fn every_run_starts_at_rest_and_ends_at_rest() {
        for run in [Run::Slosh, Run::Bubbles] {
            assert_eq!(run_frame(run, 0), Frame::STILL, "{run:?} starts still");
            let end = run_frame(run, RUN_FRAMES - 1);
            assert!(
                end.tilt.abs() < 0.5 && end.sway.abs() < 0.2,
                "{run:?}: {end:?}"
            );
            assert!(end.bubbles.iter().all(Option::is_none));
        }
        assert_eq!(RUN_FRAMES as u32, 2 * RUN_FPS, "a run lasts 2 seconds");
    }

    #[test]
    fn the_slosh_is_gentle_and_dies_away() {
        let frames: Vec<Frame> = (0..RUN_FRAMES).map(|i| run_frame(Run::Slosh, i)).collect();
        let peak = |a: usize, b: usize, f: fn(&Frame) -> f32| {
            frames[a..b].iter().map(|x| f(x).abs()).fold(0.0, f32::max)
        };
        let (tilt, sway) = (|f: &Frame| f.tilt, |f: &Frame| f.sway);
        assert!(
            (3.0..=TILT_DEG + 0.01).contains(&peak(0, 40, tilt)),
            "a few degrees"
        );
        assert!(peak(0, 40, sway) <= SWAY_DEG + 0.01);
        assert!(peak(0, 40, sway) >= 0.8, "the straw does move");
        assert!(peak(20, 40, tilt) < peak(0, 20, tilt) / 2.0, "decays");
        assert!(peak(20, 40, sway) < peak(0, 20, sway) / 2.0, "decays");
        // It really swings both ways.
        assert!(frames.iter().any(|f| f.tilt > 2.0) && frames.iter().any(|f| f.tilt < -2.0));
        // No jump between two pictures (20 fps): the tilt moves under 3 degrees a frame.
        for w in frames.windows(2) {
            assert!((w[1].tilt - w[0].tilt).abs() < 3.0, "{w:?}");
            assert!((w[1].sway - w[0].sway).abs() < 0.6, "{w:?}");
        }
        assert!(frames.iter().all(|f| f.bubbles.iter().all(Option::is_none)));
    }

    #[test]
    fn two_or_three_bubbles_rise_inside_the_juice() {
        let frames: Vec<Frame> = (0..RUN_FRAMES)
            .map(|i| run_frame(Run::Bubbles, i))
            .collect();
        let most = frames
            .iter()
            .map(|f| f.bubbles.iter().flatten().count())
            .max()
            .unwrap();
        assert!((2..=3).contains(&most), "{most} bubbles at once");
        for f in &frames {
            assert_eq!((f.tilt, f.sway, f.ripple), (0.0, 0.0, 0.0));
            for b in f.bubbles.iter().flatten() {
                // Between the surface and the bottom, and between the glass walls.
                assert!((9.5 + b.r..=14.0 - b.r).contains(&b.y), "{b:?}");
                assert!(
                    b.x - b.r > wall_x(b.y, true) && b.x + b.r < wall_x(b.y, false),
                    "{b:?}"
                );
                assert!((0.0..=1.0).contains(&b.alpha));
            }
        }
        // Each one rises: its y only goes up from picture to picture.
        for slot in 0..3 {
            let ys: Vec<f32> = frames
                .iter()
                .filter_map(|f| f.bubbles[slot].map(|b| b.y))
                .collect();
            assert!(ys.len() >= 15, "bubble {slot} shows {} pictures", ys.len());
            assert!(ys.windows(2).all(|w| w[1] < w[0]), "bubble {slot} rises");
        }
    }

    /// No picture of any run reaches outside the icon: the straw (swayed), the juice and the
    /// bubbles stay on the canvas at every size, and the juice stays between the glass walls.
    #[test]
    fn no_frame_clips() {
        for px in SIZES {
            let fit = MarkFit::new(px as f32);
            let half = fit.stroke / 2.0;
            let size = px as f32;
            for run in [Run::Slosh, Run::Bubbles] {
                for i in 0..RUN_FRAMES {
                    let f = run_frame(run, i);
                    for p in fit.straw_points(f.sway) {
                        assert!(
                            p.0 - half >= -1e-3 && p.0 + half <= size + 1e-3,
                            "{px} {run:?} {i}: straw x {p:?}"
                        );
                        assert!(
                            p.1 - half >= -1e-3 && p.1 + half <= size + 1e-3,
                            "{px} {run:?} {i}: straw y {p:?}"
                        );
                    }
                    for p in fit.juice_points(&f) {
                        assert!((0.0..=size).contains(&p.0) && (0.0..=size).contains(&p.1));
                    }
                    // The surface never rises to the straw's bend nor sinks to the glass bottom.
                    for (_, y) in juice_outline(&f).iter().take(SURFACE_STEPS + 1) {
                        assert!((8.0..=11.0).contains(y), "{px} {run:?} {i}: surface {y}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_straw_pivots_on_its_foot_and_stays_at_rest_when_not_swaying() {
        for px in SIZES {
            let fit = MarkFit::new(px as f32);
            let rest = fit.straw_points(0.0);
            assert_eq!(rest[0], fit.point(7.25, 9.5));
            // Leaning right moves the top to the right and keeps the foot where it was.
            let lean = fit.straw_points(2.0);
            assert!(lean[2].0 > rest[2].0);
            assert!((lean[0].0 - rest[0].0).abs() < 1e-4 && (lean[0].1 - rest[0].1).abs() < 1e-4);
            // The length is kept.
            let len = |p: &[(f32, f32); 3]| {
                ((p[2].0 - p[0].0).powi(2) + (p[2].1 - p[0].1).powi(2)).sqrt()
            };
            assert!((len(&lean) - len(&rest)).abs() < 1e-3);
            // Leaning left would lift the bar out of the icon: it is held at the edge.
            let back = fit.straw_points(-2.0);
            let top = back.iter().map(|p| p.1).fold(f32::MAX, f32::min) - fit.stroke / 2.0;
            assert!(top >= -1e-4, "{px}: {top}");
        }
    }

    #[test]
    fn motion_changes_pixels_and_keeps_the_colours() {
        for px in [16u32, 24] {
            let rest = still(look(Juice::Purple), true, px);
            let mid =
                mark_pixels(look(Juice::Purple), &run_frame(Run::Slosh, 5), true, px).unwrap();
            assert_ne!(rest.data, mid.data, "{px}: the slosh shows");
            let bub =
                mark_pixels(look(Juice::Purple), &run_frame(Run::Bubbles, 10), true, px).unwrap();
            assert_ne!(rest.data, bub.data, "{px}: the bubbles show");
            // Same frame, same pixels.
            let again =
                mark_pixels(look(Juice::Purple), &run_frame(Run::Slosh, 5), true, px).unwrap();
            assert_eq!(mid.data, again.data);
        }
    }

    #[test]
    fn a_bubble_is_a_visible_spot_inside_the_juice() {
        // 24 px, a bubble well inside the juice: the pixel under it differs from plain juice.
        let l = look(Juice::Purple);
        let f = Frame {
            bubbles: [
                Some(Bubble {
                    x: 6.4,
                    y: 12.0,
                    r: 0.62,
                    alpha: 1.0,
                }),
                None,
                None,
            ],
            ..Frame::STILL
        };
        for dark in [true, false] {
            let with = mark_pixels(l, &f, dark, 24).unwrap();
            let (x, y) = at(24, 6.4, 12.0);
            let juice = rgb_of(juice_color(Juice::Purple, dark));
            let got = rgb_at(&with, x, y);
            let d: i32 = got
                .iter()
                .zip(juice)
                .map(|(a, b)| (*a as i32 - b as i32).abs())
                .sum();
            assert!(d > 40, "dark={dark}: {got:?} vs {juice:?}");
            assert_eq!(alpha_at(&with, x, y), 255);
        }
    }

    #[test]
    fn rendered_runs_become_icons_and_leave_no_gdi_object_behind() {
        use windows::Win32::System::Threading::{
            GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
        };
        let count = || unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        // The graphics libraries keep a few GDI objects of their own once they are used.
        for i in render_run(look(Juice::Green), Run::Slosh, false, 16).unwrap() {
            // SAFETY: made just above, owned here.
            unsafe {
                DestroyIcon(i).unwrap();
            }
        }
        // The count is the whole process's and other tests run beside this one, so a pass is any
        // round that leaves it as it found it; a leak (80 objects a run) never would.
        let mut balanced = false;
        for _ in 0..8 {
            let before = count();
            for run in [Run::Slosh, Run::Bubbles] {
                let icons = render_run(look(Juice::Green), run, false, 16).unwrap();
                assert_eq!(icons.len(), RUN_FRAMES);
                for i in icons {
                    // SAFETY: made just above, owned here.
                    unsafe {
                        DestroyIcon(i).unwrap();
                    }
                }
            }
            if count() == before {
                balanced = true;
                break;
            }
        }
        assert!(balanced, "GDI objects after a run");
    }

    // ---- icons

    /// Draws the HICON over mid-gray and checks an antialiased edge: this settles whether
    /// `CreateIconIndirect` wants straight or premultiplied alpha.
    #[test]
    fn hicon_composites_like_the_source_pixels() {
        let px = 32u32;
        let l = look(Juice::Purple);
        let src = still(l, true, px);
        let icon = render_mark(l, &Frame::STILL, true, px).unwrap();
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: px as i32,
                biHeight: -(px as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: test-only GDI calls on locals that are released at the end.
        let (dc, dib) = unsafe {
            let dc = CreateCompatibleDC(None);
            let dib = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            SelectObject(dc, dib.into());
            (dc, dib)
        };
        // SAFETY: the DIB holds px * px * 4 bytes.
        let out = unsafe {
            std::slice::from_raw_parts_mut(bits as *mut u8, (px * px * 4) as usize).fill(128);
            DrawIconEx(dc, 0, 0, icon, px as i32, px as i32, 0, None, DI_NORMAL).unwrap();
            std::slice::from_raw_parts(bits as *const u8, (px * px * 4) as usize).to_vec()
        };
        // SAFETY: release what the test made.
        unsafe {
            let _ = DeleteDC(dc);
            let _ = DeleteObject(dib.into());
            let _ = DestroyIcon(icon);
        }
        let mut checked = 0;
        for (i, s) in src.data.as_chunks::<4>().0.iter().enumerate() {
            let a = s[3] as u32;
            if (60..200).contains(&a) {
                // Expected = premultiplied source over gray.
                let expect = s[0] as u32 + 128 * (255 - a) / 255;
                let got = out[i * 4] as u32;
                assert!(
                    (expect as i32 - got as i32).abs() <= 4,
                    "pixel {i}: alpha {a} expect {expect} got {got}"
                );
                checked += 1;
            }
        }
        assert!(checked > 10, "no antialiased pixels to check");
    }

    #[test]
    fn the_flyouts_alert_state_has_a_look() {
        assert_eq!(IconLook::from(IconState::Normal), IconLook::default());
        assert_eq!(IconLook::from(IconState::Warning).juice, Juice::Red);
        assert!(IconLook::from(IconState::Charging).charging);
    }

    #[test]
    fn sheet_and_pngs_are_written() {
        let dir = std::env::temp_dir().join("juicebar-ui-icon-test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mark.png");
        render_mark_png(IconState::Charging, false, 24, &f).unwrap();
        assert!(std::fs::metadata(&f).unwrap().len() > 80);
        let sheet = dir.join("sheet.png");
        render_mark_sheet_png(&sheet).unwrap();
        assert!(std::fs::metadata(&sheet).unwrap().len() > 1000);
    }
}
