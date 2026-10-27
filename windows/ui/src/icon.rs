// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The tray mark: a juice glass with a straw, drawn with Direct2D from the SVG paths of
//! `spec/design/mark/`. Monochrome in the taskbar's foreground; red juice for a warning, a bolt
//! cut out of the juice while charging.

use std::path::Path;

use windows::Win32::Graphics::Direct2D::{
    D2D1_COMBINE_MODE_EXCLUDE, ID2D1Geometry, ID2D1PathGeometry,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows::core::{Error, Result};

use crate::gfx::{Canvas, Color, Gfx, Pixels, render_offscreen, scale_translate, write_png};
use crate::glyphs::{MARK_BOLT, MARK_GLASS, MARK_JUICE, MARK_STRAW, MARK_STROKE, MARK_VIEWBOX};
use crate::model::IconState;

/// White on a dark taskbar, near-black on a light one.
pub fn foreground(dark_taskbar: bool) -> Color {
    if dark_taskbar {
        Color::hex(0xffffff)
    } else {
        Color::hex(0x1b1b1b)
    }
}

/// The warning juice, `#ff3b30` in `juicebar-mark-warning.svg`.
pub const WARNING_RED: Color = Color::hex(0xff3b30);

/// The taskbar's own background, for the PNG seam only.
pub fn taskbar_background(dark_taskbar: bool) -> Color {
    if dark_taskbar {
        Color::hex(0x202020)
    } else {
        Color::hex(0xf3f3f3)
    }
}

/// Draws the mark filling a `px` x `px` canvas whose transform is the identity.
pub fn draw_mark(c: &Canvas, state: IconState, dark_taskbar: bool, px: f32) -> Result<()> {
    let fg = foreground(dark_taskbar);
    c.set_transform(scale_translate(px / MARK_VIEWBOX, 0.0, 0.0));
    let result = (|| -> Result<()> {
        match state {
            IconState::Normal => c.fill_geometry(&c.path(MARK_JUICE, true)?.into(), fg),
            IconState::Warning => c.fill_geometry(&c.path(MARK_JUICE, true)?.into(), WARNING_RED),
            IconState::Charging => {
                let juice: ID2D1Geometry = c.path(MARK_JUICE, true)?.into();
                let bolt: ID2D1Geometry = c.path(MARK_BOLT, true)?.into();
                let cut = juice_with_bolt(c, &juice, &bolt)?;
                c.fill_geometry(&cut.into(), fg);
            }
        }
        c.stroke_geometry(&c.path(MARK_GLASS, false)?.into(), MARK_STROKE, fg, None);
        c.stroke_geometry(&c.path(MARK_STRAW, false)?.into(), MARK_STROKE, fg, None);
        Ok(())
    })();
    c.set_transform(crate::gfx::IDENTITY);
    result
}

/// The juice minus the bolt (the SVG's mask).
fn juice_with_bolt(
    c: &Canvas,
    juice: &ID2D1Geometry,
    bolt: &ID2D1Geometry,
) -> Result<ID2D1PathGeometry> {
    // SAFETY: plain factory call.
    let out = unsafe { c.gfx.d2d.CreatePathGeometry()? };
    // SAFETY: `out` is valid.
    let sink = unsafe { out.Open()? };
    // SAFETY: both geometries and the sink are valid for the call; the sink is closed right after.
    unsafe {
        juice.CombineWithGeometry(bolt, D2D1_COMBINE_MODE_EXCLUDE, None, 0.01, &sink)?;
        sink.Close()?;
    }
    Ok(out)
}

/// The mark as premultiplied BGRA pixels on a transparent background.
pub fn mark_pixels(state: IconState, dark_taskbar: bool, px: u32) -> Result<Pixels> {
    let gfx = Gfx::new()?;
    render_offscreen(&gfx, px, px, 1.0, |c| {
        c.clear(Color::rgba(0, 0.0));
        draw_mark(c, state, dark_taskbar, px as f32)
    })
}

/// The mark as an `HICON` for `Shell_NotifyIcon`. The caller owns it (`DestroyIcon`).
pub fn render_mark(state: IconState, dark_taskbar: bool, px: u32) -> Result<HICON> {
    icon_from_pixels(&mark_pixels(state, dark_taskbar, px)?)
}

/// Writes the mark on the taskbar's own color as a PNG (tests and the demo).
pub fn render_mark_png(state: IconState, dark_taskbar: bool, px: u32, path: &Path) -> Result<()> {
    let px_data = mark_on_background(state, dark_taskbar, px)?;
    write_png(path, &px_data)
}

/// Mark pixels composited over the taskbar color (opaque result).
pub fn mark_on_background(state: IconState, dark_taskbar: bool, px: u32) -> Result<Pixels> {
    let mut m = mark_pixels(state, dark_taskbar, px)?;
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
    Ok(m)
}

/// A contact sheet of every state at 16, 24 and 32 px on both taskbar colors, each magnified 8x
/// without smoothing so the pixels can be judged.
pub fn render_mark_sheet_png(path: &Path) -> Result<()> {
    const CELL: u32 = 32 * 8;
    let states = [IconState::Normal, IconState::Warning, IconState::Charging];
    let sizes = [16u32, 24, 32];
    let cols = (sizes.len() * 2) as u32;
    let (w, h) = (CELL * cols, CELL * states.len() as u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (row, st) in states.iter().enumerate() {
        for (ci, dark) in [true, false].into_iter().enumerate() {
            for (si, px) in sizes.iter().enumerate() {
                let col = (ci * sizes.len() + si) as u32;
                let m = mark_on_background(*st, dark, *px)?;
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
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx};

    fn alpha_at(p: &Pixels, x: u32, y: u32) -> u8 {
        p.data[((y * p.width + x) * 4 + 3) as usize]
    }

    fn rgb_at(p: &Pixels, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * p.width + x) * 4) as usize;
        [p.data[i + 2], p.data[i + 1], p.data[i]]
    }

    #[test]
    fn mark_has_ink_and_transparency_at_every_size() {
        for px in [16u32, 20, 24, 32] {
            for st in [IconState::Normal, IconState::Warning, IconState::Charging] {
                let m = mark_pixels(st, true, px).unwrap();
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
                assert!(ink > (px * px / 12) as usize, "{st:?} {px}: ink {ink}");
                assert!(clear > (px * px / 4) as usize, "{st:?} {px}: clear {clear}");
            }
        }
    }

    #[test]
    fn mark_uses_the_taskbar_foreground() {
        let dark = mark_pixels(IconState::Normal, true, 32).unwrap();
        let light = mark_pixels(IconState::Normal, false, 32).unwrap();
        // Inside the juice (view box 8,12 -> pixels 16,24).
        assert_eq!(rgb_at(&dark, 16, 24), [255, 255, 255]);
        let l = rgb_at(&light, 16, 24);
        assert!(l.iter().all(|&c| (c as i32 - 0x1b).abs() <= 2), "{l:?}");
    }

    #[test]
    fn warning_juice_is_red_and_the_glass_keeps_the_foreground() {
        let m = mark_pixels(IconState::Warning, true, 32).unwrap();
        assert_eq!(rgb_at(&m, 16, 24), [0xff, 0x3b, 0x30]);
        // The straw's top bar (view box 11, 2) is the foreground, not red.
        assert_eq!(rgb_at(&m, 22, 4), [255, 255, 255]);
    }

    #[test]
    fn charging_cuts_the_bolt_out_of_the_juice() {
        let normal = mark_pixels(IconState::Normal, true, 32).unwrap();
        let chg = mark_pixels(IconState::Charging, true, 32).unwrap();
        // View box (7.6, 12.2) is inside the bolt: filled in Normal, a hole in Charging.
        assert!(alpha_at(&normal, 15, 24) > 250);
        assert!(
            alpha_at(&chg, 15, 24) < 40,
            "alpha {}",
            alpha_at(&chg, 15, 24)
        );
        let ink = |m: &Pixels| {
            m.data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 128)
                .count()
        };
        assert!(ink(&chg) < ink(&normal));
    }

    #[test]
    fn pixels_are_premultiplied() {
        let m = mark_pixels(IconState::Normal, true, 24).unwrap();
        // White ink: every channel equals alpha.
        for p in m.data.as_chunks::<4>().0 {
            assert!(p[..3].iter().all(|&c| c as u32 <= p[3] as u32 + 1));
        }
    }

    /// Draws the HICON over mid-gray and checks an antialiased edge: this settles whether
    /// `CreateIconIndirect` wants straight or premultiplied alpha.
    #[test]
    fn hicon_composites_like_the_source_pixels() {
        let px = 32u32;
        let src = mark_pixels(IconState::Normal, true, px).unwrap();
        let icon = render_mark(IconState::Normal, true, px).unwrap();
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
