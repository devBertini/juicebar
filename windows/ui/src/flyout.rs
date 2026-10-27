// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The flyout window: a `WS_POPUP` created when shown and destroyed when hidden, drawn with
//! Direct2D into a DirectComposition swap chain so the DWM system backdrop (acrylic) shows
//! through, with rounded corners from the DWM. No timers, no animation: it draws on show, on
//! hover changes and on `update`, and holds no resources while hidden.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{E_FAIL, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, ID2D1DeviceContext, ID2D1RenderTarget,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
    GetWindowLongPtrW, IDC_ARROW, IsWindow, LoadCursorW, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WA_INACTIVE,
    WM_ACTIVATE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE,
    WM_NCDESTROY, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{Error, Interface, PCWSTR, Result, w};

use crate::draw::{PaintOptions, measure_metrics, paint_flyout};
use crate::gfx::{Canvas, Gfx};
use crate::layout::{FlyoutHit, Layout, PxRect, place, scale_from_dpi, size_px};
use crate::model::{Theme, UiState};

/// Callbacks of a shown flyout. Both run on the UI thread, inside the window's message handling;
/// neither may block.
pub struct Handlers {
    /// A click landed on a control (`FlyoutHit::None` for a click on empty space).
    pub on_hit: Box<dyn FnMut(FlyoutHit)>,
    /// The window was destroyed: Esc, deactivation, `hide` or the owner dropped the flyout.
    pub on_close: Box<dyn FnMut()>,
}

/// A shown flyout. Dropping it (or calling `hide`) destroys the window and frees its graphics.
pub struct Flyout {
    inner: Rc<Inner>,
}

struct Inner {
    hwnd: Cell<HWND>,
    state: RefCell<UiState>,
    layout: RefCell<Layout>,
    gfx: Gfx,
    gpu: RefCell<Option<Gpu>>,
    scale: Cell<f32>,
    hover: Cell<FlyoutHit>,
    tracking: Cell<bool>,
    /// Hide when another window takes the focus (the default); tests and captures turn it off.
    hide_on_deactivate: Cell<bool>,
    /// True when the DWM backdrop is on; false paints the panel itself.
    backdrop: Cell<bool>,
    handlers: RefCell<Handlers>,
}

/// The window's swap chain and the Direct2D context that draws into it.
struct Gpu {
    swap: IDXGISwapChain1,
    dc: ID2D1DeviceContext,
    // Kept alive for the window's lifetime.
    _d3d: ID3D11Device,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

const CLASS: PCWSTR = w!("JuiceBarFlyout");
/// `WM_MOUSELEAVE`, which `windows` does not export.
const WM_MOUSELEAVE: u32 = 0x02A3;

fn rect_of(r: RECT) -> PxRect {
    PxRect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

/// The monitor holding `anchor`: its work area and its effective scale.
fn monitor_of(anchor: PxRect) -> (HMONITOR, PxRect, f32) {
    let rc = RECT {
        left: anchor.left,
        top: anchor.top,
        right: anchor.right,
        bottom: anchor.bottom,
    };
    // SAFETY: `rc` is a valid RECT; MONITOR_DEFAULTTONEAREST always returns a monitor.
    let mon = unsafe { MonitorFromRect(&rc, MONITOR_DEFAULTTONEAREST) };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `mi` has its size set and outlives the call.
    let ok = unsafe { GetMonitorInfoW(mon, &mut mi) }.as_bool();
    let work = if ok {
        rect_of(mi.rcWork)
    } else {
        PxRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        }
    };
    let (mut dx, mut dy) = (96u32, 96u32);
    // SAFETY: valid monitor handle and out pointers; on failure the 96 dpi default stays.
    let _ = unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) };
    (mon, work, scale_from_dpi(dx))
}

/// Makes the process per-monitor DPI aware (v2) if nothing else has; the app manifest should
/// do this at startup, this only covers a host that did not.
pub fn ensure_dpi_awareness() {
    // SAFETY: a plain process-level call; it fails harmlessly when awareness is already set.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

impl Flyout {
    /// Creates and shows the flyout above `anchor` (screen pixels, normally the tray icon's rect).
    pub fn show(state: UiState, anchor: PxRect, handlers: Handlers) -> Result<Flyout> {
        ensure_dpi_awareness();
        let gfx = Gfx::new()?;
        let (_, work, scale) = monitor_of(anchor);
        let metrics = measure_metrics(&gfx, &state)?;
        let layout = Layout::compute(&state, metrics);
        let (w, h) = size_px(layout.width, layout.height, scale);
        let (x, y) = place(anchor, w, h, work, scale);
        let inner = Rc::new(Inner {
            hwnd: Cell::new(HWND::default()),
            state: RefCell::new(state),
            layout: RefCell::new(layout),
            gfx,
            gpu: RefCell::new(None),
            scale: Cell::new(scale),
            hover: Cell::new(FlyoutHit::None),
            tracking: Cell::new(false),
            hide_on_deactivate: Cell::new(true),
            backdrop: Cell::new(false),
            handlers: RefCell::new(handlers),
        });
        register_class()?;
        // The window procedure owns one strong reference, released in WM_NCDESTROY.
        let raw = Rc::into_raw(inner.clone());
        // SAFETY: the class is registered; `raw` stays valid until WM_NCDESTROY reclaims it.
        let created = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP,
                CLASS,
                w!("JuiceBar"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                module_instance(),
                Some(raw as *const _),
            )
        };
        let hwnd = match created {
            Ok(h) => h,
            Err(e) => {
                // SAFETY: the window was never created, so nothing else holds `raw`.
                drop(unsafe { Rc::from_raw(raw) });
                return Err(e);
            }
        };
        inner.hwnd.set(hwnd);
        let flyout = Flyout { inner };
        flyout.inner.apply_dwm_attributes();
        flyout.inner.create_gpu(w as u32, h as u32)?;
        flyout.inner.draw()?;
        // SAFETY: `hwnd` is the window just created on this thread.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        Ok(flyout)
    }

    pub fn hwnd(&self) -> HWND {
        self.inner.hwnd.get()
    }

    /// Keeps the flyout open when it loses focus (for captures and tests); Esc still closes it.
    pub fn set_hide_on_deactivate(&self, hide: bool) {
        self.inner.hide_on_deactivate.set(hide);
    }

    /// True while the window exists.
    pub fn is_open(&self) -> bool {
        let h = self.inner.hwnd.get();
        // SAFETY: IsWindow accepts any handle value.
        !h.is_invalid() && unsafe { IsWindow(Some(h)) }.as_bool()
    }

    /// The flyout window's rectangle in screen pixels.
    pub fn rect(&self) -> Option<PxRect> {
        let mut rc = RECT::default();
        // SAFETY: valid out pointer; the handle is checked by the call.
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.inner.hwnd.get(), &mut rc)
        }
        .ok()?;
        Some(rect_of(rc))
    }

    /// Replaces what is shown (a new snapshot, a theme change, a row expanded) and redraws.
    pub fn update(&self, state: UiState) -> Result<()> {
        if !self.is_open() {
            return Ok(());
        }
        let inner = &self.inner;
        let metrics = measure_metrics(&inner.gfx, &state)?;
        let layout = Layout::compute(&state, metrics);
        let (w, h) = size_px(layout.width, layout.height, inner.scale.get());
        let resized = {
            let old = inner.layout.borrow();
            (old.height - layout.height).abs() > 0.01
        };
        let dark_changed = inner.state.borrow().theme != state.theme;
        *inner.state.borrow_mut() = state;
        *inner.layout.borrow_mut() = layout;
        if dark_changed {
            inner.apply_dwm_attributes();
        }
        if resized {
            inner.resize(w, h)?;
        }
        inner.draw()
    }

    /// Destroys the window; the graphics go with it. Safe to call twice.
    pub fn hide(&self) {
        if self.is_open() {
            // SAFETY: the window belongs to this thread; WM_DESTROY frees the graphics.
            unsafe {
                let _ = DestroyWindow(self.inner.hwnd.get());
            }
        }
    }
}

impl Drop for Flyout {
    fn drop(&mut self) {
        self.hide();
    }
}

fn module_instance() -> Option<HINSTANCE> {
    // SAFETY: a null name asks for this process's own module.
    unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|m| HINSTANCE(m.0))
}

fn register_class() -> Result<()> {
    // SAFETY: the standard arrow cursor needs no instance.
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: Default::default(),
        lpfnWndProc: Some(wndproc),
        hInstance: module_instance().unwrap_or_default(),
        hCursor: cursor,
        lpszClassName: CLASS,
        ..Default::default()
    };
    // SAFETY: `wc` is fully initialised and outlives the call. A second registration of the same
    // class fails with ERROR_CLASS_ALREADY_EXISTS, which is fine.
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        let e = Error::from_thread();
        if e.code() != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
            return Err(e);
        }
    }
    Ok(())
}

impl Inner {
    /// Rounded corners and the transient-window (acrylic) backdrop; a build without them gets
    /// a solid panel painted by us.
    fn apply_dwm_attributes(&self) {
        let hwnd = self.hwnd.get();
        let dark: i32 = (self.state.borrow().theme == Theme::Dark) as i32;
        let round = DWMWCP_ROUND.0;
        let backdrop = DWMSBT_TRANSIENTWINDOW.0;
        // SAFETY: each attribute value is a 4-byte integer that outlives its call.
        let (r1, r2, r3) = unsafe {
            (
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &dark as *const i32 as *const _,
                    4,
                ),
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &round as *const i32 as *const _,
                    4,
                ),
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE,
                    &backdrop as *const i32 as *const _,
                    4,
                ),
            )
        };
        let _ = (r1, r2);
        let ok = r3.is_ok();
        if ok {
            let m = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            // SAFETY: `m` outlives the call.
            let _ = unsafe { DwmExtendFrameIntoClientArea(hwnd, &m) };
        }
        self.backdrop.set(ok);
    }

    fn create_gpu(&self, w: u32, h: u32) -> Result<()> {
        let d3d = create_d3d()?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        // SAFETY: plain interface queries on live objects.
        let factory: IDXGIFactory2 = unsafe { dxgi.GetAdapter()?.GetParent()? };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w,
            Height: h,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        // SAFETY: `desc` outlives the call; the device is the one the factory came from.
        let swap = unsafe { factory.CreateSwapChainForComposition(&d3d, &desc, None)? };
        // SAFETY: `dxgi` is a valid DXGI device from the same factory as `gfx.d2d1`.
        let d2d_dev = unsafe { self.gfx.d2d1.CreateDevice(&dxgi)? };
        // SAFETY: plain call on the device.
        let dc = unsafe { d2d_dev.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        // SAFETY: DirectComposition objects for this window; each call returns a COM object.
        let (dcomp, target, visual) = unsafe {
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(self.hwnd.get(), true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;
            (dcomp, target, visual)
        };
        *self.gpu.borrow_mut() = Some(Gpu {
            swap,
            dc,
            _d3d: d3d,
            _dcomp: dcomp,
            _target: target,
            _visual: visual,
        });
        Ok(())
    }

    fn resize(&self, w: i32, h: i32) -> Result<()> {
        if let Some(g) = self.gpu.borrow().as_ref() {
            // SAFETY: the target bitmap is released first, as ResizeBuffers requires.
            unsafe {
                g.dc.SetTarget(None);
                g.swap.ResizeBuffers(
                    0,
                    w as u32,
                    h as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_FLAG(0),
                )?;
                SetWindowPos(
                    self.hwnd.get(),
                    None,
                    0,
                    0,
                    w,
                    h,
                    SWP_NOACTIVATE
                        | SWP_NOZORDER
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE,
                )?;
            }
        }
        Ok(())
    }

    /// Draws the current state into the swap chain and presents it.
    fn draw(&self) -> Result<()> {
        let gpu = self.gpu.borrow();
        let g = gpu.as_ref().ok_or_else(|| Error::from(E_FAIL))?;
        // SAFETY: buffer 0 of a flip-model swap chain is always the current back buffer.
        let surface: IDXGISurface = unsafe { g.swap.GetBuffer(0)? };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        // SAFETY: `props` outlives the call; the surface is the swap chain's buffer.
        let bitmap = unsafe { g.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))? };
        // SAFETY: the bitmap was created for this context.
        unsafe { g.dc.SetTarget(&bitmap) };
        let rt: ID2D1RenderTarget = g.dc.cast()?;
        let canvas = Canvas::new(&self.gfx, rt, self.scale.get())?;
        let state = self.state.borrow();
        let layout = self.layout.borrow();
        canvas.begin();
        let painted = paint_flyout(
            &canvas,
            &state,
            &layout,
            PaintOptions {
                solid_panel: !self.backdrop.get(),
                clear: None,
                hover: self.hover.get(),
            },
        );
        canvas.end()?;
        painted?;
        // SAFETY: release the target before presenting so the buffer can rotate.
        unsafe {
            g.dc.SetTarget(None);
            g.swap.Present(1, DXGI_PRESENT(0)).ok()?;
        }
        Ok(())
    }

    fn redraw_quietly(&self) {
        let _ = self.draw();
    }

    fn hit_at(&self, x_px: i32, y_px: i32) -> FlyoutHit {
        let s = self.scale.get();
        self.layout.borrow().hit(x_px as f32 / s, y_px as f32 / s)
    }
}

fn create_d3d() -> Result<ID3D11Device> {
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut dev = None;
        // SAFETY: `dev` is a valid out pointer; no adapter, no feature level list.
        let r = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                None,
            )
        };
        if let (true, Some(d)) = (r.is_ok(), dev) {
            return Ok(d);
        }
    }
    Err(Error::from(E_FAIL))
}

/// # Safety
/// Called by the system with the arguments of a window message.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: for WM_NCCREATE, lparam points to the CREATESTRUCTW whose lpCreateParams is our Rc pointer.
        let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        // SAFETY: storing a pointer-sized value in the window's user data.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize) };
        // SAFETY: forwarding the message unchanged.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: reading the user data this window procedure stored.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Inner;
    if ptr.is_null() {
        // SAFETY: forwarding the message unchanged.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the pointer came from `Rc::into_raw` and lives until WM_NCDESTROY below.
    let inner = unsafe { &*ptr };
    match msg {
        WM_ERASEBKGND => return LRESULT(1),
        WM_ACTIVATE
            if (wparam.0 & 0xffff) as u32 == WA_INACTIVE && inner.hide_on_deactivate.get() =>
        {
            // SAFETY: destroying our own window from its procedure is allowed.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
            // SAFETY: as above.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
            if !inner.tracking.get() {
                let mut t = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: `t` is initialised and outlives the call.
                if unsafe { TrackMouseEvent(&mut t) }.is_ok() {
                    inner.tracking.set(true);
                }
            }
            let hit = inner.hit_at(x, y);
            if hit != inner.hover.get() {
                inner.hover.set(hit);
                inner.redraw_quietly();
            }
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            inner.tracking.set(false);
            if inner.hover.get() != FlyoutHit::None {
                inner.hover.set(FlyoutHit::None);
                inner.redraw_quietly();
            }
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
            let hit = inner.hit_at(x, y);
            // No borrow is held while the callback runs, so it may call `update` or `hide`.
            let mut cb =
                std::mem::replace(&mut inner.handlers.borrow_mut().on_hit, Box::new(|_| {}));
            cb(hit);
            inner.handlers.borrow_mut().on_hit = cb;
            return LRESULT(0);
        }
        WM_DESTROY => {
            // Release the graphics before the window goes away.
            if let Ok(mut g) = inner.gpu.try_borrow_mut() {
                g.take();
            }
            let mut cb =
                std::mem::replace(&mut inner.handlers.borrow_mut().on_close, Box::new(|| {}));
            cb();
            inner.handlers.borrow_mut().on_close = cb;
            inner.hwnd.set(HWND::default());
        }
        WM_NCDESTROY => {
            // SAFETY: clearing the user data, then giving back the reference taken in `show`.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Rc::from_raw(ptr));
            }
        }
        _ => {}
    }
    // SAFETY: forwarding the message unchanged.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_LBUTTONUP, WM_MOUSEMOVE};

    fn lparam(x: i32, y: i32) -> LPARAM {
        LPARAM(((y as isize & 0xffff) << 16) | (x as isize & 0xffff))
    }

    fn open(hits: Rc<RefCell<Vec<FlyoutHit>>>, closed: Rc<Cell<u32>>) -> Flyout {
        let handlers = Handlers {
            on_hit: Box::new(move |h| hits.borrow_mut().push(h)),
            on_close: Box::new(move || closed.set(closed.get() + 1)),
        };
        let anchor = PxRect {
            left: 1000,
            top: 1050,
            right: 1024,
            bottom: 1080,
        };
        let f = Flyout::show(UiState::sample(Theme::Dark), anchor, handlers).expect("flyout opens");
        f.set_hide_on_deactivate(false);
        f
    }

    #[test]
    fn click_hover_escape_and_drop_follow_the_window_lifecycle() {
        let hits = Rc::new(RefCell::new(Vec::new()));
        let closed = Rc::new(Cell::new(0u32));
        let f = open(hits.clone(), closed.clone());
        assert!(f.is_open());
        let hwnd = f.hwnd();
        let layout = f.inner.layout.borrow().clone();
        let s = f.inner.scale.get();
        let px = |r: crate::layout::Rect| (((r.x + r.w / 2.0) * s) as i32, ((r.cy()) * s) as i32);

        let (gx, gy) = px(layout.gear);
        // SAFETY: sending messages to our own window on this thread.
        unsafe { SendMessageW(hwnd, WM_MOUSEMOVE, Some(WPARAM(0)), Some(lparam(gx, gy))) };
        assert_eq!(f.inner.hover.get(), FlyoutHit::Gear);
        // SAFETY: as above.
        unsafe { SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(lparam(gx, gy))) };
        let (sx, sy) = px(layout.footer.switch);
        // SAFETY: as above.
        unsafe { SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(lparam(sx, sy))) };
        let (rx, ry) = px(layout.rows[2].bounds);
        // SAFETY: as above.
        unsafe { SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(lparam(rx, ry))) };
        // SAFETY: as above.
        unsafe { SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(lparam(2, 2))) };
        assert_eq!(
            *hits.borrow(),
            vec![
                FlyoutHit::Gear,
                FlyoutHit::RgbSwitch,
                FlyoutHit::Row(3),
                FlyoutHit::None
            ]
        );

        // Updating with a collapsed card shrinks the window.
        let before = f.rect().unwrap();
        let mut st = UiState::sample(Theme::Dark);
        st.devices[0].expanded = false;
        f.update(st).unwrap();
        let after = f.rect().unwrap();
        assert!(after.bottom - after.top < before.bottom - before.top - 150);

        // Esc destroys the window and runs on_close once.
        // SAFETY: as above.
        unsafe {
            SendMessageW(
                hwnd,
                WM_KEYDOWN,
                Some(WPARAM(VK_ESCAPE.0 as usize)),
                Some(LPARAM(0)),
            )
        };
        assert!(!f.is_open());
        assert_eq!(closed.get(), 1);
        // SAFETY: IsWindow accepts any handle value.
        assert!(
            !unsafe { IsWindow(Some(hwnd)) }.as_bool(),
            "no hidden window is kept alive"
        );
        assert!(
            f.inner.gpu.borrow().is_none(),
            "graphics are released with the window"
        );
    }

    #[test]
    fn dropping_the_flyout_destroys_the_window_once() {
        let closed = Rc::new(Cell::new(0u32));
        let f = open(Rc::new(RefCell::new(Vec::new())), closed.clone());
        let hwnd = f.hwnd();
        drop(f);
        assert_eq!(closed.get(), 1);
        // SAFETY: IsWindow accepts any handle value.
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    }

    #[test]
    fn the_window_sits_inside_the_work_area_above_the_anchor() {
        let f = open(Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(0)));
        let r = f.rect().unwrap();
        let anchor = PxRect {
            left: 1000,
            top: 1050,
            right: 1024,
            bottom: 1080,
        };
        let (_, work, _) = monitor_of(anchor);
        assert!(
            r.left >= work.left
                && r.right <= work.right
                && r.top >= work.top
                && r.bottom <= work.bottom,
            "{r:?} outside {work:?}"
        );
    }
}
