// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The flyout window: a `WS_POPUP` created when shown and destroyed when hidden. Direct2D draws it
//! on the CPU (the render seam's WIC path, so a PNG is what the user sees) into a 32-bit
//! premultiplied DIB section, and `WM_PAINT` copies that to the window with GDI. The DWM frame is
//! extended over the whole client area, so transparent pixels show the system backdrop (acrylic),
//! and the DWM rounds the corners. No Direct3D device is ever made: a GPU device loads the graphics
//! driver into the process (about 50 MiB of private memory while it lives). No timers, no
//! animation: it draws on show, on hover changes and on `update`, and holds nothing while hidden.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{E_FAIL, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_TRANSIENTWINDOW, DWMWA_CLOAK, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ,
    HMONITOR, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, PAINTSTRUCT,
    SRCCOPY, ScreenToClient, SelectObject, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_BACK, VK_DOWN, VK_ESCAPE, VK_LEFT,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetCursorPos,
    GetWindowLongPtrW, IDC_ARROW, IsWindow, LoadCursorW, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, WA_INACTIVE,
    WM_ACTIVATE, WM_CHAR, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Error, PCWSTR, Result, w};

use crate::draw::{PaintOptions, measure_metrics, paint_flyout};
use crate::gfx::{Gfx, render_offscreen_into};
use crate::layout::{
    EDGE_MARGIN, FlyoutHit, Layout, Metrics, PxRect, place, scale_from_dpi, size_px,
};
use crate::model::{Theme, UiState};
use crate::settings::{EditKey, NavKey};
use crate::settings_layout::WHEEL_STEP;

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
    /// The pixels `WM_PAINT` copies to the window; dropped with the window.
    surface: RefCell<Option<Surface>>,
    scale: Cell<f32>,
    /// Where the flyout was anchored (the tray icon), to place it again when its height changes.
    anchor: PxRect,
    hover: Cell<FlyoutHit>,
    /// How far the Settings page is scrolled, in DIPs.
    scroll: Cell<f32>,
    tracking: Cell<bool>,
    /// Hide when another window takes the focus (the default); tests and captures turn it off.
    hide_on_deactivate: Cell<bool>,
    /// True when the DWM backdrop is on and shows the desktop (acrylic); false paints the panel itself.
    backdrop: Cell<bool>,
    handlers: RefCell<Handlers>,
}

/// A 32-bit top-down DIB section selected into a memory DC: what the window shows.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    width: u32,
    height: u32,
}

impl Surface {
    fn new(width: u32, height: u32) -> Result<Surface> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: a memory DC compatible with the screen and a DIB section described by `bmi`;
        // every handle is owned by the returned value and released in `Drop`, or here on failure.
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err(Error::from_thread());
            }
            let bitmap = match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(b) if !bits.is_null() => b,
                Ok(b) => {
                    let _ = DeleteObject(b.into());
                    let _ = DeleteDC(dc);
                    return Err(Error::from(E_FAIL));
                }
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let old = SelectObject(dc, bitmap.into());
            Ok(Surface {
                dc,
                bitmap,
                old,
                bits: bits as *mut u8,
                width,
                height,
            })
        }
    }

    fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: the DIB section holds width * 4 * height bytes for as long as `self` lives,
        // and `&mut self` makes this the only view of them.
        unsafe {
            std::slice::from_raw_parts_mut(self.bits, (self.width * 4 * self.height) as usize)
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the handles were made in `new` and are released once, the bitmap deselected first.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
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
        let metrics = metrics_for(&gfx, &state, work, scale)?;
        let layout = Layout::compute(&state, metrics);
        let (w, h) = size_px(layout.width, layout.height, scale);
        let (x, y) = place(anchor, w, h, work, scale);
        let inner = Rc::new(Inner {
            hwnd: Cell::new(HWND::default()),
            state: RefCell::new(state),
            layout: RefCell::new(layout),
            gfx,
            surface: RefCell::new(None),
            scale: Cell::new(scale),
            anchor,
            hover: Cell::new(FlyoutHit::None),
            scroll: Cell::new(0.0),
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
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
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
        // The window is activated before the first draw, cloaked so nothing half-drawn shows:
        // an activation while `d3d11.dll` is in the process costs about 54 MiB of private
        // memory that stays after the window is gone (the `gfx` module notes), and Direct2D
        // loads `d3d11.dll` only when it draws.
        set_cloak(hwnd, true);
        // SAFETY: `hwnd` is the window just created on this thread.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        let drawn = flyout.inner.draw();
        // SAFETY: as above; UpdateWindow paints the surface now, before the window is uncloaked.
        unsafe {
            let _ = UpdateWindow(hwnd);
        }
        set_cloak(hwnd, false);
        drawn?;
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
        let mut state = state;
        let (_, work, _) = monitor_of(inner.anchor);
        let metrics = metrics_for(&inner.gfx, &state, work, inner.scale.get())?;
        // The window keeps the keyboard focus and the scroll of the Settings page across the app's updates.
        match (&inner.state.borrow().settings, &mut state.settings) {
            (Some(old), Some(new)) => {
                if new.focus.is_none() {
                    new.focus = old.focus.filter(|id| new.focusable().contains(id));
                }
            }
            _ => inner.scroll.set(0.0),
        }
        let layout = Layout::compute(&state, metrics);
        match &layout.settings {
            Some(g) => inner.scroll.set(g.clamp_scroll(inner.scroll.get())),
            None => inner.scroll.set(0.0),
        }
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

/// Hides (`true`) or shows the window on screen without changing its state or activation.
fn set_cloak(hwnd: HWND, cloak: bool) {
    let v: i32 = cloak as i32;
    // SAFETY: a 4-byte BOOL that outlives the call; failure only leaves the window uncloaked.
    let _ = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &v as *const i32 as *const _, 4) };
}

/// The measured texts for `state`, with the Settings page capped to the work area (minus the margins).
fn metrics_for(gfx: &Gfx, state: &UiState, work: PxRect, scale: f32) -> Result<Metrics> {
    let mut m = measure_metrics(gfx, state)?;
    m.max_height = (work.bottom - work.top) as f32 / scale - 2.0 * EDGE_MARGIN;
    Ok(m)
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
        // With "Transparency effects" off in Windows the system paints every acrylic backdrop as one flat
        // solid color, so the glow would sit on a gray slab: paint our own panel instead.
        self.backdrop
            .set(ok && crate::system::transparency_enabled());
    }

    /// Resizes the window and places it again against the anchor: a flyout that grows (a card opens, the
    /// welcome page gives way to the list) keeps its bottom edge above the taskbar instead of running past
    /// the work area.
    fn resize(&self, w: i32, h: i32) -> Result<()> {
        let (_, work, scale) = monitor_of(self.anchor);
        let (x, y) = place(self.anchor, w, h, work, scale);
        // SAFETY: our own window, moved and resized in place.
        unsafe {
            SetWindowPos(
                self.hwnd.get(),
                None,
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
        }
    }

    /// Draws the current state into the surface (made or remade at the layout's pixel size) and
    /// asks for a repaint.
    fn draw(&self) -> Result<()> {
        let scale = self.scale.get();
        let state = self.state.borrow();
        let layout = self.layout.borrow();
        let (w, h) = size_px(layout.width, layout.height, scale);
        let (w, h) = (w as u32, h as u32);
        let mut slot = self.surface.borrow_mut();
        if slot.as_ref().is_none_or(|s| (s.width, s.height) != (w, h)) {
            // The old DIB goes before the new one is made, so the two never coexist.
            *slot = None;
            *slot = Some(Surface::new(w, h)?);
        }
        let surface = slot.as_mut().ok_or_else(|| Error::from(E_FAIL))?;
        render_offscreen_into(&self.gfx, w, h, scale, surface.pixels(), |c| {
            paint_flyout(
                c,
                &state,
                &layout,
                PaintOptions {
                    solid_panel: !self.backdrop.get(),
                    clear: None,
                    hover: self.hover.get(),
                    scroll: self.scroll.get(),
                },
            )
        })?;
        // SAFETY: our own window; the whole client area is repainted from the surface.
        let _ = unsafe { InvalidateRect(Some(self.hwnd.get()), None, false) };
        Ok(())
    }

    /// `WM_PAINT`: copies the surface, alpha included, to the window.
    fn paint(&self, hwnd: HWND) {
        let mut ps = PAINTSTRUCT::default();
        // SAFETY: a Begin/EndPaint pair on our own window; the surface's DC is valid while borrowed.
        unsafe {
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Ok(slot) = self.surface.try_borrow()
                && let Some(s) = slot.as_ref()
            {
                let _ = BitBlt(
                    hdc,
                    0,
                    0,
                    s.width as i32,
                    s.height as i32,
                    Some(s.dc),
                    0,
                    0,
                    SRCCOPY,
                );
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }

    fn redraw_quietly(&self) {
        let _ = self.draw();
    }

    fn hit_at(&self, x_px: i32, y_px: i32) -> FlyoutHit {
        let s = self.scale.get();
        self.layout
            .borrow()
            .hit_scrolled(x_px as f32 / s, y_px as f32 / s, self.scroll.get())
    }

    /// Runs the app's click callback; no borrow is held, so it may call `update` or `hide`.
    fn emit(&self, hit: FlyoutHit) {
        let mut cb = std::mem::replace(&mut self.handlers.borrow_mut().on_hit, Box::new(|_| {}));
        cb(hit);
        self.handlers.borrow_mut().on_hit = cb;
    }

    /// The hover under the pointer after the page moved beneath it.
    fn rehover(&self, hwnd: HWND) {
        let mut p = windows::Win32::Foundation::POINT::default();
        // SAFETY: valid out pointers; the window is ours.
        let ok = unsafe { GetCursorPos(&mut p).is_ok() && ScreenToClient(hwnd, &mut p).as_bool() };
        let hit = if ok {
            self.hit_at(p.x, p.y)
        } else {
            FlyoutHit::None
        };
        self.hover.set(hit);
    }

    /// Scrolls the Settings page by `delta` DIPs; true when it moved.
    fn scroll_by(&self, delta: f32) -> bool {
        let max = self
            .layout
            .borrow()
            .settings
            .as_ref()
            .map_or(0.0, |g| g.max_scroll());
        let now = self.scroll.get();
        let next = (now + delta).clamp(0.0, max);
        if (next - now).abs() < 0.01 {
            return false;
        }
        self.scroll.set(next);
        true
    }

    /// A key pressed while the flyout has the focus.
    fn key_down(&self, vk: u16) -> Key {
        let (page, editing) = {
            let st = self.state.borrow();
            (
                st.settings.is_some(),
                st.settings.as_ref().is_some_and(|s| s.editing()),
            )
        };
        if vk == VK_ESCAPE.0 {
            return if editing {
                Key::Hit(FlyoutHit::Edit(EditKey::Escape))
            } else if page {
                Key::Hit(FlyoutHit::Back)
            } else {
                Key::Close
            };
        }
        if !page {
            return Key::Pass;
        }
        if editing {
            return match vk {
                v if v == VK_BACK.0 => Key::Hit(FlyoutHit::Edit(EditKey::Backspace)),
                v if v == VK_RETURN.0 => Key::Hit(FlyoutHit::Edit(EditKey::Enter)),
                _ => Key::Handled,
            };
        }
        // SAFETY: a plain key-state query.
        let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
        let nav = match vk {
            v if v == VK_TAB.0 && shift => NavKey::ShiftTab,
            v if v == VK_TAB.0 => NavKey::Tab,
            v if v == VK_UP.0 => NavKey::Up,
            v if v == VK_DOWN.0 => NavKey::Down,
            v if v == VK_LEFT.0 => NavKey::Left,
            v if v == VK_RIGHT.0 => NavKey::Right,
            v if v == VK_SPACE.0 => NavKey::Space,
            v if v == VK_RETURN.0 => NavKey::Enter,
            _ => return Key::Pass,
        };
        self.nav(nav)
    }

    /// A navigation key on the Settings page: moves the focus (and the scroll, to keep it in view) or acts.
    fn nav(&self, key: NavKey) -> Key {
        match key {
            NavKey::Tab | NavKey::Down | NavKey::ShiftTab | NavKey::Up => {
                let forward = matches!(key, NavKey::Tab | NavKey::Down);
                let focus = self
                    .state
                    .borrow_mut()
                    .settings
                    .as_mut()
                    .and_then(|s| s.move_focus(forward));
                if let Some(id) = focus {
                    let reveal = self
                        .layout
                        .borrow()
                        .settings
                        .as_ref()
                        .map(|g| g.scroll_to_reveal(id, self.scroll.get()));
                    if let Some(s) = reveal {
                        self.scroll.set(s);
                    }
                    self.rehover_quietly();
                    self.redraw_quietly();
                }
                Key::Handled
            }
            _ => {
                let act = self
                    .state
                    .borrow()
                    .settings
                    .as_ref()
                    .and_then(|s| s.activate(key));
                match act {
                    Some((id, part)) => Key::Hit(FlyoutHit::Setting { id, part }),
                    None => Key::Handled,
                }
            }
        }
    }

    fn rehover_quietly(&self) {
        let hwnd = self.hwnd.get();
        if !hwnd.is_invalid() {
            self.rehover(hwnd);
        }
    }
}

/// What a key press does to the flyout.
enum Key {
    /// Destroy the window (Esc on the list).
    Close,
    /// Tell the app, as a click would.
    Hit(FlyoutHit),
    /// Used here (the focus moved, or nothing to do).
    Handled,
    /// Not ours: the system's default handling.
    Pass,
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
        WM_PAINT => {
            inner.paint(hwnd);
            return LRESULT(0);
        }
        WM_ACTIVATE
            if (wparam.0 & 0xffff) as u32 == WA_INACTIVE && inner.hide_on_deactivate.get() =>
        {
            // SAFETY: destroying our own window from its procedure is allowed.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_KEYDOWN => match inner.key_down(wparam.0 as u16) {
            Key::Close => {
                // SAFETY: as above.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return LRESULT(0);
            }
            Key::Hit(hit) => {
                inner.emit(hit);
                return LRESULT(0);
            }
            Key::Handled => return LRESULT(0),
            Key::Pass => {}
        },
        WM_CHAR => {
            let editing = inner
                .state
                .borrow()
                .settings
                .as_ref()
                .is_some_and(|s| s.editing());
            if editing && let Some(ch) = char::from_u32(wparam.0 as u32).filter(|c| !c.is_control())
            {
                inner.emit(FlyoutHit::Edit(EditKey::Char(ch)));
            }
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let notches = f32::from(((wparam.0 >> 16) & 0xffff) as u16 as i16) / 120.0;
            if inner.scroll_by(-notches * WHEEL_STEP) {
                inner.rehover(hwnd);
                inner.redraw_quietly();
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
            // A click ends the keyboard's focus ring.
            if let Some(ui) = inner.state.borrow_mut().settings.as_mut() {
                ui.focus = None;
            }
            inner.emit(hit);
            return LRESULT(0);
        }
        WM_DESTROY => {
            // Release the pixels before the window goes away.
            if let Ok(mut g) = inner.surface.try_borrow_mut() {
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
            f.inner.surface.borrow().is_none(),
            "graphics are released with the window"
        );
    }

    #[test]
    fn growing_keeps_the_flyout_inside_the_work_area_with_its_bottom_edge_in_place() {
        let f = open(Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(0)));
        f.update(UiState::sample_collapsed(Theme::Dark)).unwrap();
        let small = f.rect().unwrap();
        // The welcome page, then the whole list with an open card: each taller than the one before.
        f.update(UiState::sample(Theme::Dark)).unwrap();
        let tall = f.rect().unwrap();
        assert!(tall.bottom - tall.top > small.bottom - small.top + 150);
        let anchor = PxRect {
            left: 1000,
            top: 1050,
            right: 1024,
            bottom: 1080,
        };
        let (_, work, _) = monitor_of(anchor);
        assert!(
            tall.bottom <= work.bottom && tall.top >= work.top,
            "{tall:?} outside {work:?}"
        );
        assert_eq!(tall.bottom, small.bottom, "grows upward, bottom edge stays");
        f.update(UiState::sample_welcome(Theme::Dark)).unwrap();
        let page = f.rect().unwrap();
        assert!(page.bottom <= work.bottom && page.top >= work.top);
    }

    fn open_page(state: UiState, hits: Rc<RefCell<Vec<FlyoutHit>>>) -> Flyout {
        let handlers = Handlers {
            on_hit: Box::new(move |h| hits.borrow_mut().push(h)),
            on_close: Box::new(|| {}),
        };
        let anchor = PxRect {
            left: 1000,
            top: 1050,
            right: 1024,
            bottom: 1080,
        };
        let f = Flyout::show(state, anchor, handlers).expect("flyout opens");
        f.set_hide_on_deactivate(false);
        f
    }

    fn key(hwnd: HWND, vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) {
        // SAFETY: sending a message to our own window on this thread.
        unsafe {
            SendMessageW(
                hwnd,
                WM_KEYDOWN,
                Some(WPARAM(vk.0 as usize)),
                Some(LPARAM(0)),
            )
        };
    }

    #[test]
    fn the_settings_page_takes_the_keys_the_wheel_and_the_clicks() {
        use crate::settings::{PART_MINUS, PART_PLUS, PART_ROW};
        use windows::Win32::UI::WindowsAndMessaging::WM_MOUSEWHEEL;
        let hits = Rc::new(RefCell::new(Vec::new()));
        let f = open_page(UiState::sample_settings(Theme::Dark), hits.clone());
        let hwnd = f.hwnd();
        let focus = |f: &Flyout| f.inner.state.borrow().settings.as_ref().unwrap().focus;
        assert_eq!(focus(&f), None);

        // Tab walks the controls; Down is the same; the focus ring is drawn from the state.
        key(hwnd, VK_TAB);
        assert_eq!(focus(&f), Some(1));
        key(hwnd, VK_DOWN);
        assert_eq!(focus(&f), Some(2));
        assert!(
            hits.borrow().is_empty(),
            "moving the focus asks the app nothing"
        );

        // Space flips the toggle under the focus; Left turns an "on" toggle off; Right does nothing.
        key(hwnd, VK_SPACE);
        key(hwnd, VK_LEFT);
        key(hwnd, VK_RIGHT);
        let row_hit = FlyoutHit::Setting {
            id: 2,
            part: PART_ROW,
        };
        assert_eq!(*hits.borrow(), vec![row_hit, row_hit]);
        hits.borrow_mut().clear();

        // Two more Tabs reach the first stepper; Right presses its plus, Left its minus.
        key(hwnd, VK_TAB);
        key(hwnd, VK_TAB);
        assert_eq!(focus(&f), Some(4));
        key(hwnd, VK_RIGHT);
        key(hwnd, VK_LEFT);
        assert_eq!(
            *hits.borrow(),
            vec![
                FlyoutHit::Setting {
                    id: 4,
                    part: PART_PLUS
                },
                FlyoutHit::Setting {
                    id: 4,
                    part: PART_MINUS
                }
            ]
        );
        hits.borrow_mut().clear();

        // The app's update keeps the focus the window owns.
        f.update(UiState::sample_settings(Theme::Dark)).unwrap();
        assert_eq!(focus(&f), Some(4));
        // A click ends the focus ring and is reported where it landed.
        let layout = f.inner.layout.borrow().clone();
        let s = f.inner.scale.get();
        let row = layout.settings.as_ref().unwrap().row(4).unwrap().clone();
        let (px, py) = (
            ((row.plus.x + row.plus.w / 2.0) * s) as i32,
            (row.plus.cy() * s) as i32,
        );
        // SAFETY: sending a message to our own window on this thread.
        unsafe { SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(lparam(px, py))) };
        assert_eq!(
            *hits.borrow(),
            vec![FlyoutHit::Setting {
                id: 4,
                part: PART_PLUS
            }]
        );
        assert_eq!(focus(&f), None);
        hits.borrow_mut().clear();

        // The wheel scrolls a page taller than the window, never past either end; the window stays within the work area.
        let g = layout.settings.clone().unwrap();
        let (_, work, _) = monitor_of(f.inner.anchor);
        let r = f.rect().unwrap();
        assert!(r.bottom - r.top <= work.bottom - work.top);
        let down = ((-120i16) as u16 as usize) << 16;
        let up = 120usize << 16;
        let wheel = |w: usize| {
            // SAFETY: sending a message to our own window on this thread.
            unsafe { SendMessageW(hwnd, WM_MOUSEWHEEL, Some(WPARAM(w)), Some(LPARAM(0))) };
        };
        wheel(down);
        assert_eq!(f.inner.scroll.get(), 48.0_f32.min(g.max_scroll()));
        for _ in 0..200 {
            wheel(down);
        }
        assert_eq!(f.inner.scroll.get(), g.max_scroll());
        for _ in 0..400 {
            wheel(up);
        }
        assert_eq!(f.inner.scroll.get(), 0.0);

        // Tabbing to a row below the fold scrolls it into view.
        let last = layout.settings.as_ref().unwrap().row(15).unwrap().clone();
        for _ in 0..30 {
            key(hwnd, VK_TAB);
            if focus(&f) == Some(15) {
                break;
            }
        }
        assert_eq!(focus(&f), Some(15));
        let sy = last.bounds.bottom() - f.inner.scroll.get();
        assert!(
            sy <= g.height,
            "the focused row is on screen: {sy} of {}",
            g.height
        );

        // Esc leaves the page (the app answers by showing the list); on the list it closes the window.
        key(hwnd, VK_ESCAPE);
        assert_eq!(*hits.borrow(), vec![FlyoutHit::Back]);
        assert!(f.is_open());
        f.update(UiState::sample_collapsed(Theme::Dark)).unwrap();
        assert_eq!(f.inner.scroll.get(), 0.0, "the list does not scroll");
        key(hwnd, VK_ESCAPE);
        assert!(!f.is_open());
    }

    #[test]
    fn a_text_field_being_edited_gets_the_characters_and_escape_only_cancels_the_edit() {
        use crate::settings::{EditKey, SettingsRow, SettingsSection, UiSettings};
        use windows::Win32::UI::WindowsAndMessaging::WM_CHAR;
        let mut state = UiState::sample_settings(Theme::Light);
        state.settings = Some(UiSettings::new(
            "Configurações",
            vec![SettingsSection::new(
                "Nome",
                vec![SettingsRow::text(1, "Nome", "G7", true)],
            )],
        ));
        let hits = Rc::new(RefCell::new(Vec::new()));
        let f = open_page(state, hits.clone());
        let hwnd = f.hwnd();
        // SAFETY: sending messages to our own window on this thread.
        unsafe {
            SendMessageW(hwnd, WM_CHAR, Some(WPARAM(0x30)), Some(LPARAM(0)));
            SendMessageW(hwnd, WM_CHAR, Some(WPARAM(0x08)), Some(LPARAM(0)));
        }
        key(hwnd, VK_BACK);
        key(hwnd, VK_RETURN);
        key(hwnd, VK_TAB);
        key(hwnd, VK_ESCAPE);
        assert_eq!(
            *hits.borrow(),
            vec![
                FlyoutHit::Edit(EditKey::Char('0')),
                FlyoutHit::Edit(EditKey::Backspace),
                FlyoutHit::Edit(EditKey::Enter),
                FlyoutHit::Edit(EditKey::Escape),
            ],
            "the backspace control character is not text, Tab does nothing, Escape is the edit's"
        );
        assert!(f.is_open());
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
