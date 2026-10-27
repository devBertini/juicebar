// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Direct2D and DirectWrite plumbing: factories, a `Canvas` with safe drawing helpers, text
//! layouts, SVG-path geometry, and the offscreen (WIC) render target with the PNG writer.
//!
//! Direct2D draws on the CPU only (WIC bitmap targets), and `d2d1.dll` is loaded by `Gfx::new`
//! and unloaded when the last `Gfx` is dropped. Drawing makes Direct2D load `d3d11.dll` (and,
//! with a window in the process, the WARP rasterizer); while `d3d11.dll` stays loaded, the next
//! activation of one of our windows costs about 54 MiB of private memory that is never given
//! back (measured on Windows 11 build 26300 with `examples/mem_probe.rs`; an overlay hook such
//! as RTSS reacting to Direct3D in the process is the likely cause, not confirmed). Unloading
//! `d2d1.dll` between uses unloads `d3d11.dll` with it.

use std::path::Path;

use windows::Win32::Foundation::{FreeLibrary, GENERIC_WRITE, HMODULE, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{Error, GUID, HRESULT, Interface, PCWSTR, Result, s, w};

use crate::layout::Rect;
use crate::svgpath::{self, Seg};

/// A straight-alpha color, components 0.0 to 1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn hex(rgb: u32) -> Color {
        Color {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }
    pub const fn rgba(rgb: u32, a: f32) -> Color {
        let c = Color::hex(rgb);
        Color { a, ..c }
    }
    /// The same color with its alpha multiplied by `k`.
    pub fn fade(self, k: f32) -> Color {
        Color {
            a: self.a * k,
            ..self
        }
    }
    /// Relative luminance (WCAG) of the color, ignoring alpha.
    pub fn luminance(&self) -> f32 {
        let lin = |c: f32| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F {
            r: self.r,
            g: self.g,
            b: self.b,
            a: self.a,
        }
    }
}

/// Builds a Direct2D point. The `windows` crate's point type lives in `windows-numerics`, which
/// this crate does not depend on directly, so the value is made from a struct that has one.
macro_rules! v2 {
    ($x:expr, $y:expr) => {{
        let mut p = D2D1_ELLIPSE::default().point;
        p.X = $x;
        p.Y = $y;
        p
    }};
}

fn rf(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x,
        top: r.y,
        right: r.right(),
        bottom: r.bottom(),
    }
}

/// A 2D affine transform (row-vector convention, like Direct2D's `Matrix3x2`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub dx: f32,
    pub dy: f32,
}

pub const IDENTITY: Mat = Mat {
    m11: 1.0,
    m12: 0.0,
    m21: 0.0,
    m22: 1.0,
    dx: 0.0,
    dy: 0.0,
};

/// Matrix product: `a` first, then `b`.
pub fn mul(a: Mat, b: Mat) -> Mat {
    Mat {
        m11: a.m11 * b.m11 + a.m12 * b.m21,
        m12: a.m11 * b.m12 + a.m12 * b.m22,
        m21: a.m21 * b.m11 + a.m22 * b.m21,
        m22: a.m21 * b.m12 + a.m22 * b.m22,
        dx: a.dx * b.m11 + a.dy * b.m21 + b.dx,
        dy: a.dx * b.m12 + a.dy * b.m22 + b.dy,
    }
}

pub fn scale_translate(s: f32, tx: f32, ty: f32) -> Mat {
    Mat {
        m11: s,
        m12: 0.0,
        m21: 0.0,
        m22: s,
        dx: tx,
        dy: ty,
    }
}

pub fn rotate_about(deg: f32, cx: f32, cy: f32) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    let rot = Mat {
        m11: c,
        m12: s,
        m21: -s,
        m22: c,
        dx: 0.0,
        dy: 0.0,
    };
    mul(
        mul(scale_translate(1.0, -cx, -cy), rot),
        scale_translate(1.0, cx, cy),
    )
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Initializes COM on this thread for WIC; a thread already in another mode is fine.
pub fn ensure_com() {
    // SAFETY: CoInitializeEx has no preconditions; a repeat or a mode mismatch only returns a code.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    debug_assert!(hr.is_ok() || hr == RPC_E_CHANGED_MODE);
}

/// The font families chosen once: Segoe UI Variable when this Windows has it, else Segoe UI.
pub struct Fonts {
    pub text: Vec<u16>,
    pub display: Vec<u16>,
}

/// One reference to `d2d1.dll`, loaded at run time (never imported), freed on drop.
struct D2dLibrary(HMODULE);

impl Drop for D2dLibrary {
    fn drop(&mut self) {
        // SAFETY: the reference taken in `D2dLibrary::load`. `Gfx` drops its factory before this
        // field, and the crate keeps every other Direct2D object (render targets, brushes,
        // geometries) inside the drawing call that made it, so none outlives the `Gfx`.
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

type CreateFactoryFn = unsafe extern "system" fn(
    D2D1_FACTORY_TYPE,
    *const GUID,
    *const D2D1_FACTORY_OPTIONS,
    *mut *mut std::ffi::c_void,
) -> HRESULT;

impl D2dLibrary {
    fn load() -> Result<(D2dLibrary, ID2D1Factory)> {
        // SAFETY: loads the system's own d2d1.dll from System32.
        let module = unsafe { LoadLibraryExW(w!("d2d1.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)? };
        let lib = D2dLibrary(module);
        // SAFETY: `module` is loaded; the export has the documented `D2D1CreateFactory` signature.
        let create: CreateFactoryFn = unsafe {
            std::mem::transmute(
                GetProcAddress(module, s!("D2D1CreateFactory")).ok_or_else(Error::from_thread)?,
            )
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: valid IID and out pointer; no options.
        unsafe {
            create(
                D2D1_FACTORY_TYPE_SINGLE_THREADED,
                &ID2D1Factory::IID,
                std::ptr::null(),
                &mut raw,
            )
            .ok()?;
        }
        // SAFETY: on success `raw` is an owned ID2D1Factory reference.
        Ok((lib, unsafe { ID2D1Factory::from_raw(raw) }))
    }
}

/// Direct2D and DirectWrite factories plus the font choice. Make one for a burst of drawing and
/// drop it after: the last one dropped unloads Direct2D (see the module notes), so no Direct2D
/// object made with it may be kept past its drop.
pub struct Gfx {
    /// The factory, for render targets, geometry and stroke styles.
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    pub fonts: Fonts,
    /// Declared last so it is dropped after the factory.
    _lib: D2dLibrary,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        let (lib, d2d) = D2dLibrary::load()?;
        // SAFETY: plain factory creation.
        let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let fonts = Fonts {
            text: pick_family(&dw, "Segoe UI Variable Text"),
            display: pick_family(&dw, "Segoe UI Variable Display"),
        };
        Ok(Gfx {
            d2d,
            dw,
            fonts,
            _lib: lib,
        })
    }

    pub fn text_format(&self, display: bool, weight: i32, size: f32) -> Result<IDWriteTextFormat> {
        let family = if display {
            &self.fonts.display
        } else {
            &self.fonts.text
        };
        // SAFETY: `family` and the locale are NUL-terminated UTF-16 that outlive the call.
        unsafe {
            self.dw.CreateTextFormat(
                PCWSTR(family.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT(weight),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                PCWSTR(wide("pt-BR").as_ptr()),
            )
        }
    }

    pub fn text_layout(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        max_w: f32,
        max_h: f32,
        align: Align,
    ) -> Result<Text> {
        let w16: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: the string slice and format are valid for the call.
        let layout = unsafe { self.dw.CreateTextLayout(&w16, fmt, max_w, max_h)? };
        let ta = match align {
            Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
        };
        // SAFETY: `layout` is valid; the trimming sign is created from `fmt` and used right away.
        unsafe {
            layout.SetTextAlignment(ta)?;
            layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = self.dw.CreateEllipsisTrimmingSign(fmt)?;
            let trim = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            layout.SetTrimming(&trim, &sign)?;
        }
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: `m` is a valid out pointer.
        unsafe { layout.GetMetrics(&mut m)? };
        let mut lines = [DWRITE_LINE_METRICS::default()];
        let mut n = 0u32;
        // SAFETY: `lines` and `n` are valid out parameters.
        unsafe { layout.GetLineMetrics(Some(&mut lines), &mut n)? };
        Ok(Text {
            layout,
            w: m.width,
            h: m.height,
            baseline: lines[0].baseline,
        })
    }

    /// Natural size of `s` in `fmt`.
    pub fn measure(&self, s: &str, fmt: &IDWriteTextFormat) -> Result<(f32, f32)> {
        let t = self.text_layout(s, fmt, 10_000.0, 10_000.0, Align::Left)?;
        Ok((t.w, t.h))
    }
}

/// `wanted` when the system has the family, else "Segoe UI" (NUL-terminated UTF-16).
fn pick_family(dw: &IDWriteFactory, wanted: &str) -> Vec<u16> {
    let name = wide(wanted);
    let mut coll = None;
    // SAFETY: `coll` is a valid out pointer.
    let ok = unsafe { dw.GetSystemFontCollection(&mut coll, false) }.is_ok();
    if let (true, Some(coll)) = (ok, coll) {
        let (mut idx, mut exists) = (0u32, false.into());
        // SAFETY: `name` is NUL-terminated and the out pointers are valid locals.
        if unsafe { coll.FindFamilyName(PCWSTR(name.as_ptr()), &mut idx, &mut exists) }.is_ok()
            && exists.as_bool()
        {
            return name;
        }
    }
    wide("Segoe UI")
}

/// A laid-out piece of text with its measured size.
pub struct Text {
    layout: IDWriteTextLayout,
    pub w: f32,
    pub h: f32,
    /// Distance from the top of the layout to the first baseline.
    pub baseline: f32,
}

/// Horizontal text alignment inside the layout box.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Draws with Direct2D into any render target, in DIPs.
pub struct Canvas<'a> {
    pub gfx: &'a Gfx,
    rt: ID2D1RenderTarget,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    /// Pixels per DIP, to snap hairlines.
    pub scale: f32,
}

impl<'a> Canvas<'a> {
    /// `rt` is already sized; this sets its DPI from `scale` and grayscale text (alpha targets).
    pub fn new(gfx: &'a Gfx, rt: ID2D1RenderTarget, scale: f32) -> Result<Canvas<'a>> {
        // SAFETY: the render target is valid; these only set state on it.
        unsafe {
            rt.SetDpi(96.0 * scale, 96.0 * scale);
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
        let color = Color::hex(0).d2d();
        // SAFETY: `color` outlives the call.
        let brush = unsafe { rt.CreateSolidColorBrush(&color, None)? };
        let props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        // SAFETY: `props` outlives the call.
        let round = unsafe { gfx.d2d.CreateStrokeStyle(&props, None)? };
        Ok(Canvas {
            gfx,
            rt,
            brush,
            round,
            scale,
        })
    }

    pub fn begin(&self) {
        // SAFETY: the render target is valid.
        unsafe { self.rt.BeginDraw() };
    }

    pub fn end(&self) -> Result<()> {
        // SAFETY: matches `begin`.
        unsafe { self.rt.EndDraw(None, None) }
    }

    pub fn clear(&self, c: Color) {
        let c = c.d2d();
        // SAFETY: `c` outlives the call.
        unsafe { self.rt.Clear(Some(&c)) };
    }

    fn paint(&self, c: Color) -> &ID2D1SolidColorBrush {
        // SAFETY: the brush belongs to this canvas.
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    /// Rounds a DIP coordinate to a whole device pixel.
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    /// One device pixel in DIPs: the width of a hairline.
    pub fn hairline(&self) -> f32 {
        1.0 / self.scale
    }

    pub fn set_transform(&self, m: Mat) {
        // The `windows` matrix type comes from `windows-numerics`; borrow one from a struct that holds it.
        let mut dm = D2D1_LAYER_PARAMETERS::default().maskTransform;
        (dm.M11, dm.M12, dm.M21, dm.M22, dm.M31, dm.M32) = (m.m11, m.m12, m.m21, m.m22, m.dx, m.dy);
        // SAFETY: `dm` outlives the call.
        unsafe { self.rt.SetTransform(&dm) };
    }

    pub fn fill_rect(&self, r: Rect, c: Color) {
        let rect = rf(r);
        // SAFETY: `rect` outlives the call.
        unsafe { self.rt.FillRectangle(&rect, self.paint(c)) };
    }

    pub fn fill_rrect(&self, r: Rect, radius: f32, c: Color) {
        let rr = D2D1_ROUNDED_RECT {
            rect: rf(r),
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: `rr` outlives the call.
        unsafe { self.rt.FillRoundedRectangle(&rr, self.paint(c)) };
    }

    /// A stroke drawn inside `r` (the line's outer edge is `r`'s edge).
    pub fn stroke_rrect(&self, r: Rect, radius: f32, width: f32, c: Color) {
        let i = width / 2.0;
        let r = Rect::new(r.x + i, r.y + i, r.w - width, r.h - width);
        let rr = D2D1_ROUNDED_RECT {
            rect: rf(r),
            radiusX: (radius - i).max(0.0),
            radiusY: (radius - i).max(0.0),
        };
        // SAFETY: `rr` outlives the call.
        unsafe {
            self.rt
                .DrawRoundedRectangle(&rr, self.paint(c), width, None::<&ID2D1StrokeStyle>)
        };
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, c: Color) {
        let e = D2D1_ELLIPSE {
            point: v2!(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        // SAFETY: `e` outlives the call.
        unsafe { self.rt.FillEllipse(&e, self.paint(c)) };
    }

    pub fn stroke_circle(&self, cx: f32, cy: f32, r: f32, width: f32, c: Color) {
        let e = D2D1_ELLIPSE {
            point: v2!(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        // SAFETY: `e` outlives the call.
        unsafe { self.rt.DrawEllipse(&e, self.paint(c), width, &self.round) };
    }

    pub fn line(
        &self,
        p0: (f32, f32),
        p1: (f32, f32),
        width: f32,
        c: Color,
        style: Option<&ID2D1StrokeStyle>,
    ) {
        // SAFETY: both points are plain values; the brush and style outlive the call.
        unsafe {
            self.rt.DrawLine(
                v2!(p0.0, p0.1),
                v2!(p1.0, p1.1),
                self.paint(c),
                width,
                style.unwrap_or(&self.round),
            )
        };
    }

    /// A custom dash pattern (in multiples of the stroke width) with round caps.
    pub fn dash_style(&self, dashes: &[f32]) -> Result<ID2D1StrokeStyle> {
        let props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_CUSTOM,
            dashOffset: 0.0,
        };
        // SAFETY: `props` and `dashes` outlive the call.
        unsafe { self.gfx.d2d.CreateStrokeStyle(&props, Some(dashes)) }
    }

    pub fn stroke_geometry(
        &self,
        g: &ID2D1Geometry,
        width: f32,
        c: Color,
        style: Option<&ID2D1StrokeStyle>,
    ) {
        // SAFETY: geometry, brush and style are valid for the call.
        unsafe {
            self.rt
                .DrawGeometry(g, self.paint(c), width, style.unwrap_or(&self.round))
        };
    }

    pub fn fill_geometry(&self, g: &ID2D1Geometry, c: Color) {
        // SAFETY: geometry and brush are valid for the call.
        unsafe { self.rt.FillGeometry(g, self.paint(c), None::<&ID2D1Brush>) };
    }

    /// Fills with a vertical gradient from `y0` (color `c0`) to `y1` (color `c1`).
    pub fn fill_geometry_vgradient(
        &self,
        g: &ID2D1Geometry,
        y0: f32,
        y1: f32,
        c0: Color,
        c1: Color,
    ) -> Result<()> {
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: c0.d2d(),
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: c1.d2d(),
            },
        ];
        // SAFETY: `stops` outlives the call.
        let coll = unsafe {
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?
        };
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: v2!(0.0, y0),
            endPoint: v2!(0.0, y1),
        };
        // SAFETY: `props` outlives the call; `coll` is valid.
        let brush = unsafe { self.rt.CreateLinearGradientBrush(&props, None, &coll)? };
        // SAFETY: geometry and brush are valid for the call.
        unsafe { self.rt.FillGeometry(g, &brush, None::<&ID2D1Brush>) };
        Ok(())
    }

    pub fn push_clip(&self, r: Rect) {
        let rect = rf(r);
        // SAFETY: `rect` outlives the call; balanced by `pop_clip`.
        unsafe {
            self.rt
                .PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_ALIASED)
        };
    }

    pub fn pop_clip(&self) {
        // SAFETY: balanced with `push_clip`.
        unsafe { self.rt.PopAxisAlignedClip() };
    }

    // ---- geometry from SVG paths ----

    /// A path geometry from SVG path data; `filled` makes the figures fillable.
    pub fn path(&self, d: &str, filled: bool) -> Result<ID2D1PathGeometry> {
        let segs =
            svgpath::parse(d).map_err(|_| Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
        self.geometry(&segs, filled)
    }

    pub fn geometry(&self, segs: &[Seg], filled: bool) -> Result<ID2D1PathGeometry> {
        // SAFETY: plain factory call.
        let geo = unsafe { self.gfx.d2d.CreatePathGeometry()? };
        // SAFETY: `geo` is valid.
        let sink = unsafe { geo.Open()? };
        let begin = if filled {
            D2D1_FIGURE_BEGIN_FILLED
        } else {
            D2D1_FIGURE_BEGIN_HOLLOW
        };
        let mut open = false;
        // SAFETY: every call below passes plain values to a sink that stays alive until `Close`.
        unsafe {
            for s in segs {
                match *s {
                    Seg::Move(x, y) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        }
                        sink.BeginFigure(v2!(x, y), begin);
                        open = true;
                    }
                    Seg::Line(x, y) => sink.AddLine(v2!(x, y)),
                    Seg::Cubic(x1, y1, x2, y2, x, y) => sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                        point1: v2!(x1, y1),
                        point2: v2!(x2, y2),
                        point3: v2!(x, y),
                    }),
                    Seg::Arc {
                        rx,
                        ry,
                        rot,
                        large,
                        sweep,
                        x,
                        y,
                    } => sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: v2!(x, y),
                        size: D2D_SIZE_F {
                            width: rx,
                            height: ry,
                        },
                        rotationAngle: rot,
                        sweepDirection: if sweep {
                            D2D1_SWEEP_DIRECTION_CLOCKWISE
                        } else {
                            D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                        },
                        arcSize: if large {
                            D2D1_ARC_SIZE_LARGE
                        } else {
                            D2D1_ARC_SIZE_SMALL
                        },
                    }),
                    Seg::Close => {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close()?;
        }
        Ok(geo)
    }

    /// A polyline through `pts`; closed when `close`.
    pub fn polyline(
        &self,
        pts: &[(f32, f32)],
        close: bool,
        filled: bool,
    ) -> Result<ID2D1PathGeometry> {
        let mut segs: Vec<Seg> = Vec::with_capacity(pts.len() + 1);
        for (i, p) in pts.iter().enumerate() {
            segs.push(if i == 0 {
                Seg::Move(p.0, p.1)
            } else {
                Seg::Line(p.0, p.1)
            });
        }
        if close {
            segs.push(Seg::Close);
        }
        self.geometry(&segs, filled)
    }

    // ---- text ----

    pub fn text_layout(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        max_w: f32,
        max_h: f32,
        align: Align,
    ) -> Result<Text> {
        self.gfx.text_layout(s, fmt, max_w, max_h, align)
    }

    /// Natural size of `s` in `fmt`.
    pub fn measure(&self, s: &str, fmt: &IDWriteTextFormat) -> Result<(f32, f32)> {
        self.gfx.measure(s, fmt)
    }

    pub fn draw_text_at(&self, t: &Text, x: f32, y: f32, c: Color) {
        // SAFETY: layout and brush are valid for the call.
        unsafe {
            self.rt.DrawTextLayout(
                v2!(x, y),
                &t.layout,
                self.paint(c),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            )
        };
    }

    /// Draws `s` in `r`, vertically centered, trimmed with an ellipsis when too wide.
    pub fn text_in(
        &self,
        s: &str,
        fmt: &IDWriteTextFormat,
        r: Rect,
        align: Align,
        c: Color,
    ) -> Result<()> {
        let t = self.text_layout(s, fmt, r.w, r.h, align)?;
        self.draw_text_at(&t, r.x, r.y + (r.h - t.h) / 2.0, c);
        Ok(())
    }
}

// ---- offscreen rendering ----

/// An offscreen BGRA surface: `px` holds premultiplied pixels, `stride` bytes per row.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Runs `draw` against a WIC bitmap render target of `width` x `height` pixels at `scale` and
/// returns the premultiplied BGRA pixels. This is the same drawing the window uses.
pub fn render_offscreen<F>(
    gfx: &Gfx,
    width: u32,
    height: u32,
    scale: f32,
    draw: F,
) -> Result<Pixels>
where
    F: FnOnce(&Canvas) -> Result<()>,
{
    let mut data = vec![0u8; (width * 4 * height) as usize];
    render_offscreen_into(gfx, width, height, scale, &mut data, draw)?;
    Ok(Pixels {
        width,
        height,
        data,
    })
}

/// `render_offscreen` into a caller's buffer of `width * 4 * height` bytes (top-down rows), such
/// as the flyout window's DIB section. Direct2D rasterizes on the CPU here: no Direct3D device,
/// so no GPU driver is loaded into the process.
pub fn render_offscreen_into<F>(
    gfx: &Gfx,
    width: u32,
    height: u32,
    scale: f32,
    out: &mut [u8],
    draw: F,
) -> Result<()>
where
    F: FnOnce(&Canvas) -> Result<()>,
{
    let stride = width * 4;
    if out.len() != (stride * height) as usize {
        return Err(Error::from(windows::Win32::Foundation::E_INVALIDARG));
    }
    ensure_com();
    // SAFETY: standard in-process COM creation.
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
    // SAFETY: valid GUID and sizes.
    let bitmap = unsafe {
        wic.CreateBitmap(
            width,
            height,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnDemand,
        )?
    };
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 0.0,
        dpiY: 0.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    // SAFETY: `props` outlives the call; `bitmap` is valid.
    let rt = unsafe { gfx.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)? };
    let canvas = Canvas::new(gfx, rt, scale)?;
    canvas.begin();
    let drawn = draw(&canvas);
    canvas.end()?;
    drawn?;
    // SAFETY: `out` is exactly stride * height bytes (checked above), the size CopyPixels is told.
    unsafe { bitmap.CopyPixels(std::ptr::null(), stride, out) }
}

/// Writes premultiplied BGRA pixels as a PNG file.
pub fn write_png(path: &Path, px: &Pixels) -> Result<()> {
    ensure_com();
    // SAFETY: standard in-process COM creation.
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
    let name = wide(&path.to_string_lossy());
    // SAFETY: every pointer below is a valid local that outlives its call; `name` is NUL-terminated.
    unsafe {
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(PCWSTR(name.as_ptr()), GENERIC_WRITE.0)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
        frame.Initialize(None)?;
        frame.SetSize(px.width, px.height)?;
        let mut fmt: GUID = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        let straight = unpremultiply(&px.data);
        frame.WritePixels(px.height, px.width * 4, &straight)?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}

/// Converts premultiplied BGRA to straight alpha.
pub fn unpremultiply(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for p in out.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpremultiply_restores_straight_color() {
        // 50 % white premultiplied is (128,128,128,128).
        let out = unpremultiply(&[128, 128, 128, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
        assert_eq!(&out[..4], &[255, 255, 255, 128]);
        assert_eq!(&out[4..8], &[10, 20, 30, 255]);
        assert_eq!(&out[8..], &[0, 0, 0, 0]);
    }

    #[test]
    fn matrix_helpers_compose_in_drawing_order() {
        let m = mul(
            scale_translate(2.0, 0.0, 0.0),
            scale_translate(1.0, 10.0, 5.0),
        );
        // point (1,1) -> scaled (2,2) -> translated (12,7)
        assert_eq!((m.m11 + m.dx, m.m22 + m.dy), (12.0, 7.0));
        let r = rotate_about(90.0, 12.0, 12.0);
        // (12, 2.5) rotated 90 deg clockwise (y down) about (12,12) lands at (21.5, 12).
        let (x, y) = (
            12.0 * r.m11 + 2.5 * r.m21 + r.dx,
            12.0 * r.m12 + 2.5 * r.m22 + r.dy,
        );
        assert!(
            (x - 21.5).abs() < 1e-4 && (y - 12.0).abs() < 1e-4,
            "({x}, {y})"
        );
    }

    #[test]
    fn luminance_orders_light_and_dark() {
        assert!(Color::hex(0xffffff).luminance() > 0.99);
        assert!(Color::hex(0x000000).luminance() < 0.01);
        assert!(Color::hex(0x60cdff).luminance() > 0.4);
        assert!(Color::hex(0x0067c0).luminance() < 0.2);
    }
}
