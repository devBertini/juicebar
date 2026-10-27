// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The shell: one hidden top-level window that owns the tray icon, the hotkey, the timers and the
//! flyout, and one message loop. Nothing here polls; the process sleeps in `GetMessageW`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW,
    GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowRect, KillTimer, MSG,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer,
    SetWindowLongPtrW, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_ENDSESSION, WM_HOTKEY, WM_NCCREATE, WM_NCDESTROY, WM_SETTINGCHANGE, WM_TIMER,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use juicebar_core::snapshot::Snapshot;
use juicebar_core::strings::Lang;
use juicebar_core::types::DeviceKey;
use juicebar_hid::Transport;
use juicebar_ui::flyout::{Flyout, Handlers};
use juicebar_ui::layout::{FlyoutHit, PxRect};
use juicebar_ui::model::{IconState, UiState};
use juicebar_ui::system;

use crate::args::Args;
use crate::files::DataDir;
use crate::live::{LightChange, Live, StartConfig};
use crate::menu::{self, Command};
use crate::notify::{Notice, Tracker};
use crate::runkey;
use crate::settings::Settings;
use crate::single::{self, WINDOW_CLASS};
use crate::source::{Clock, DemoPhase, Source};
use crate::tray::{self, IconKey, Tray};
use crate::view::{self, ViewInput};
use crate::winhost::{self, WinHost};

/// `NINF_KEY | NIN_SELECT`: the icon was activated from the keyboard (Enter or Space).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
/// The tray icon's callback message.
const WM_TRAY: u32 = WM_APP + 1;
/// Posted by the flyout: a click landed on a control (`wparam` is the encoded hit).
const WM_FLYOUT_HIT: u32 = WM_APP + 2;
/// Posted by the flyout: its window is gone.
const WM_FLYOUT_CLOSED: u32 = WM_APP + 3;

const HOTKEY_ID: i32 = 1;
const HOTKEY_TEXT: &str = "Ctrl+Alt+B";
/// The one-shot timer the source asks for (`wake_at`, and the live source's `Wake`).
const TIMER_WAKE: usize = 1;
/// `--seconds N`: quit by itself (tests and captures only).
const TIMER_QUIT: usize = 2;

/// A tray click that follows the flyout's closing by less than this is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(500);

thread_local! {
    /// When the flyout last went away, set by its close handler.
    static LAST_CLOSE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Gives the pages the process is not using back to the system. Called once at start and each time
/// the flyout closes, so a tray app that sits idle shows a small working set; what is needed again is
/// faulted back in.
fn trim_memory() {
    // SAFETY: the pseudo-handle of the current process needs no closing.
    unsafe {
        let _ = K32EmptyWorkingSet(GetCurrentProcess());
    }
}

/// What a tray callback event asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Toggle,
    Menu,
    Ignore,
}

/// With `NOTIFYICON_VERSION_4` a left click arrives as `NIN_SELECT` (and `NIN_KEYSELECT` from the
/// keyboard) and a right click as `WM_CONTEXTMENU`; the raw mouse messages are ignored so one click
/// does one thing.
pub fn classify(event: u32) -> TrayEvent {
    match event {
        NIN_SELECT | NIN_KEYSELECT => TrayEvent::Toggle,
        WM_CONTEXTMENU => TrayEvent::Menu,
        _ => TrayEvent::Ignore,
    }
}

/// True when a tray click comes right after the flyout closed itself (it lost focus to that click).
pub fn reopen_blocked(since_close: Option<Duration>) -> bool {
    since_close.is_some_and(|d| d < REOPEN_GUARD)
}

pub fn encode_hit(hit: FlyoutHit) -> usize {
    match hit {
        FlyoutHit::Row(i) => i,
        FlyoutHit::Gear => usize::MAX,
        FlyoutHit::RgbSwitch => usize::MAX - 1,
        FlyoutHit::None => usize::MAX - 2,
    }
}

pub fn decode_hit(v: usize) -> FlyoutHit {
    match v {
        usize::MAX => FlyoutHit::Gear,
        x if x == usize::MAX - 1 => FlyoutHit::RgbSwitch,
        x if x == usize::MAX - 2 => FlyoutHit::None,
        i => FlyoutHit::Row(i),
    }
}

/// The signed x and y packed in a `wparam` or `lparam`.
fn point_of(v: usize) -> POINT {
    POINT {
        x: (v & 0xffff) as u16 as i16 as i32,
        y: ((v >> 16) & 0xffff) as u16 as i16 as i32,
    }
}

/// Portuguese in any variant is Brazilian Portuguese (the only one the app ships); every other language is English.
pub fn lang_from_langid(langid: u16) -> Lang {
    /// LANG_PORTUGUESE, the primary language id (the low ten bits of a LANGID).
    const LANG_PORTUGUESE: u16 = 0x16;
    if langid & 0x3ff == LANG_PORTUGUESE {
        Lang::PtBr
    } else {
        Lang::En
    }
}

/// The display language of the user's Windows, or the one forced on the command line.
pub fn detect_lang(forced: Option<&str>) -> Lang {
    if let Some(tag) = forced {
        return Lang::from_tag(tag);
    }
    // SAFETY: a plain query with no arguments.
    lang_from_langid(unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() })
}

pub struct App {
    hwnd: Cell<HWND>,
    args: Args,
    exe: PathBuf,
    lang: Lang,
    msg_open: u32,
    msg_taskbar: u32,
    /// Posted by the transport's waker: events are waiting in `drain`.
    msg_hid: u32,
    start: Instant,
    data: Option<DataDir>,
    settings: RefCell<Settings>,
    transport: RefCell<Option<Rc<Transport>>>,
    /// The live source has flushed and said good-bye to its devices (quit runs once).
    live_closed: Cell<bool>,
    /// The first snapshot always reaches the tray; later ones only when they differ.
    applied_once: Cell<bool>,
    tray: RefCell<Tray>,
    flyout: RefCell<Option<Flyout>>,
    source: RefCell<Source>,
    snapshot: RefCell<Snapshot>,
    tracker: RefCell<Tracker>,
    /// The "RGB saver" switch (stored in settings.txt, handed to every tracker; it does not yet change the
    /// devices' lighting).
    rgb_economy: Cell<bool>,
    expanded: RefCell<Option<DeviceKey>>,
    hotkey_ok: Cell<bool>,
}

impl App {
    fn new(args: Args, user: &str) -> App {
        let lang = detect_lang(args.lang.as_deref());
        let source = if args.demo {
            Source::Demo {
                phase: DemoPhase::Scene,
                cycle: args.demo_cycle,
            }
        } else {
            // The live source needs the window; `init` replaces this one.
            Source::Empty
        };
        let data = if args.is_live() {
            args.data
                .clone()
                .map(DataDir::at)
                .or_else(DataDir::from_env)
        } else {
            None
        };
        let settings = data
            .as_ref()
            .map(|d| Settings::parse(&d.read_settings()))
            .unwrap_or_default();
        let snapshot = source.snapshot(&Clock::now(), lang);
        // SAFETY: registering messages by name.
        let (msg_taskbar, msg_hid) = unsafe {
            (
                RegisterWindowMessageW(w!("TaskbarCreated")),
                RegisterWindowMessageW(w!("JuiceBar.HidEvents")),
            )
        };
        App {
            hwnd: Cell::new(HWND::default()),
            exe: std::env::current_exe().unwrap_or_default(),
            lang,
            msg_open: single::open_message(user),
            msg_taskbar,
            msg_hid,
            start: Instant::now(),
            data,
            rgb_economy: Cell::new(settings.rgb_economy),
            settings: RefCell::new(settings),
            transport: RefCell::new(None),
            live_closed: Cell::new(false),
            applied_once: Cell::new(false),
            tray: RefCell::new(Tray::new(HWND::default(), WM_TRAY)),
            flyout: RefCell::new(None),
            source: RefCell::new(source),
            snapshot: RefCell::new(snapshot),
            tracker: RefCell::new(Tracker::default()),
            expanded: RefCell::new(None),
            hotkey_ok: Cell::new(false),
            args,
        }
    }

    fn trace(&self, line: &str) {
        if let Some(p) = &self.args.trace {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    /// Runs once the window exists: hotkey, start-with-Windows self-heal, the tray icon, timers.
    fn init(&self) {
        let hwnd = self.hwnd.get();
        self.tray.borrow_mut().bind(hwnd);
        // SAFETY: registering a hot key for our own window; failure (taken by another app) is handled.
        let ok = unsafe {
            RegisterHotKey(
                Some(hwnd),
                HOTKEY_ID,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                u32::from(b'B'),
            )
        }
        .is_ok();
        self.hotkey_ok.set(ok);
        self.trace(&format!(
            "hotkey {}",
            if ok { "registered" } else { "unavailable" }
        ));
        runkey::refresh(runkey::VALUE, &self.exe);
        if self.args.is_live() {
            self.start_live();
        }
        let snap = self.source.borrow().snapshot(&Clock::now(), self.lang);
        self.apply(snap);
        trim_memory();
        if let Some(ms) = self.source.borrow().next_wake_ms() {
            self.wake_at(ms);
        }
        if let Some(s) = self.args.seconds {
            // SAFETY: a timer on our own window.
            unsafe {
                SetTimer(Some(hwnd), TIMER_QUIT, s.saturating_mul(1000), None);
            }
        }
    }

    /// The core's only way to ask for time: one one-shot timer, replaced on every call.
    pub fn wake_at(&self, ms: u32) {
        // SAFETY: a timer on our own window; the same id replaces the previous one.
        unsafe {
            SetTimer(Some(self.hwnd.get()), TIMER_WAKE, ms, None);
        }
    }

    fn on_wake(&self) {
        // SAFETY: stopping our own timer; it fires once.
        unsafe {
            let _ = KillTimer(Some(self.hwnd.get()), TIMER_WAKE);
        }
        let ticked = self.source.borrow_mut().live_mut().map(|live| {
            live.on_tick(winhost::core_now(self.start));
        });
        if ticked.is_some() {
            self.after_live(false);
            return;
        }
        let changed = self.source.borrow_mut().advance();
        if changed {
            let snap = self.source.borrow().snapshot(&Clock::now(), self.lang);
            self.apply(snap);
        }
        if let Some(ms) = self.source.borrow().next_wake_ms() {
            self.wake_at(ms);
        }
    }

    // ---- the live source

    /// Starts the real devices: the history files are read and every known device is set offline before the
    /// transport's first event is looked at (events only reach the loop through the HID message, which is
    /// handled after this returns).
    fn start_live(&self) {
        let hwnd = self.hwnd.get();
        let transport = match winhost::start_transport(hwnd, self.msg_hid) {
            Ok(t) => Some(t),
            Err(e) => {
                self.trace(&format!("transport failed: {e}"));
                None
            }
        };
        *self.transport.borrow_mut() = transport.clone();
        let host = WinHost::new(
            hwnd,
            TIMER_WAKE,
            transport,
            self.start,
            self.data.clone(),
            self.args.log.clone(),
        );
        let histories = self
            .data
            .as_ref()
            .map(DataDir::read_histories)
            .unwrap_or_default();
        let cfg = {
            let settings = self.settings.borrow();
            StartConfig {
                lang: self.lang,
                rgb_economy: settings.rgb_economy,
                hidden: settings.hidden.clone(),
                lights: settings.lights.clone(),
            }
        };
        let live = Live::start(host, histories, cfg, winhost::core_now(self.start));
        *self.source.borrow_mut() = Source::Live(Box::new(live));
    }

    /// The transport has events waiting: hand each to the provider.
    fn on_hid(&self) {
        let Some(transport) = self.transport.borrow().clone() else {
            return;
        };
        let events = transport.drain();
        {
            let mut source = self.source.borrow_mut();
            let Some(live) = source.live_mut() else {
                return;
            };
            for event in events {
                live.on_event(winhost::to_core(event), winhost::core_now(self.start));
            }
        }
        self.after_live(false);
    }

    /// Takes what the live source changed: the new snapshot (when something shown may differ, or `force`) with
    /// the balloons the estimators decided on.
    fn after_live(&self, force: bool) {
        let now = winhost::core_now(self.start);
        let (changes, taken) = {
            let mut source = self.source.borrow_mut();
            let Some(live) = source.live_mut() else {
                return;
            };
            let changes = live.take_light_changes();
            let taken = if live.take_dirty() || force {
                Some((live.snapshot(&now), live.take_notices(&now)))
            } else {
                None
            };
            (changes, taken)
        };
        if !changes.is_empty() {
            self.keep_lights(changes);
        }
        if let Some((snap, notices)) = taken {
            self.apply_with(snap, notices);
        }
    }

    /// The RGB economy's kept levels reach `settings.txt` as they change, so a run that dies with the lighting down
    /// still knows what to give back.
    fn keep_lights(&self, changes: Vec<LightChange>) {
        {
            let mut settings = self.settings.borrow_mut();
            for change in changes {
                match change {
                    LightChange::Keep(key, text) => settings.keep_light(&key, text),
                    LightChange::Drop(key) => settings.drop_light(&key),
                }
            }
        }
        self.save_settings();
    }

    fn save_settings(&self) {
        if let Some(d) = &self.data
            && let Err(e) = d.write_settings(&self.settings.borrow().encode())
        {
            self.trace(&format!("settings not saved: {e}"));
        }
    }

    /// Writes the histories and takes the devices offline; runs once, before the tray icon goes.
    fn flush_live(&self) {
        if self.live_closed.replace(true) {
            return;
        }
        let now = winhost::core_now(self.start);
        if let Ok(mut source) = self.source.try_borrow_mut()
            && let Some(live) = source.live_mut()
        {
            live.quit(now);
        }
    }

    fn icon_key(&self, state: juicebar_core::snapshot::IconState) -> IconKey {
        IconKey {
            state: view::icon_state(state),
            dark_taskbar: system::taskbar_is_dark(),
            px: tray::small_icon_px(tray::taskbar_dpi()),
        }
    }

    /// Takes a snapshot of a source that has no estimators of its own (the demo): the notices come from
    /// comparing it with the last one.
    fn apply(&self, snap: Snapshot) {
        let notices = self.tracker.borrow_mut().observe(&snap, self.lang);
        self.apply_with(snap, notices);
    }

    /// Takes a new snapshot with the notices to show: the icon and tooltip and the open flyout are touched only
    /// when it differs from the last one (the first one always reaches the tray).
    fn apply_with(&self, snap: Snapshot, notices: Vec<Notice>) {
        let changed = *self.snapshot.borrow() != snap;
        if !changed && notices.is_empty() && self.applied_once.get() {
            return;
        }
        let key = self.icon_key(snap.icon);
        let tip = snap.tooltip.clone();
        *self.snapshot.borrow_mut() = snap;
        if let Ok(mut t) = self.tray.try_borrow_mut() {
            if let Err(e) = t.update(key, &tip) {
                self.trace(&format!("tray update failed: {e}"));
            }
            let dpi = tray::taskbar_dpi();
            let dark_ui = system::app_theme() == juicebar_ui::model::Theme::Dark;
            for n in &notices {
                self.trace(&format!("notice: {} | {}", n.title, n.body));
                let shown = t.balloon(n, IconState::Normal, !dark_ui, tray::large_icon_px(dpi));
                self.trace(&format!("balloon ok: {}", shown.is_ok()));
            }
        }
        self.applied_once.set(true);
        if changed {
            self.refresh_flyout();
        }
    }

    /// The tray icon re-rendered if the theme or the taskbar's DPI changed.
    fn refresh_tray(&self) {
        let (key, tip) = {
            let s = self.snapshot.borrow();
            (self.icon_key(s.icon), s.tooltip.clone())
        };
        if let Ok(mut t) = self.tray.try_borrow_mut() {
            let _ = t.update(key, &tip);
        }
    }

    fn ui_state(&self) -> UiState {
        let theme = system::app_theme();
        let accent = system::accent_for(theme, system::accent_color());
        let clock = Clock::now();
        let snap = self.snapshot.borrow();
        let expanded = self.expanded.borrow();
        let empty = menu::texts(self.lang, HOTKEY_TEXT).empty;
        // The footer's right-hand text is the hotkey, except while G HUB is putting the lighting back: then it says
        // so, instead of fighting it.
        let conflict = self.rgb_economy.get()
            && self
                .source
                .borrow()
                .live()
                .is_some_and(|live| live.lighting_conflict());
        let footer_text = if conflict {
            view::ghub_notice(self.lang).to_string()
        } else {
            HOTKEY_TEXT.to_string()
        };
        view::ui_state(&ViewInput {
            snapshot: &snap,
            theme,
            accent,
            lang: self.lang,
            rgb_economy: self.rgb_economy.get(),
            hotkey_text: &footer_text,
            expanded: expanded.as_ref(),
            empty_text: &empty,
            now: clock.wall,
            utc_offset: clock.utc_offset,
        })
    }

    fn refresh_flyout(&self) {
        if self.flyout_is_open() {
            let state = self.ui_state();
            if let Some(f) = self.flyout.borrow().as_ref() {
                let _ = f.update(state);
            }
        }
    }

    fn flyout_is_open(&self) -> bool {
        self.flyout.borrow().as_ref().is_some_and(Flyout::is_open)
    }

    /// Where the flyout anchors: the tray icon, else the right end of the taskbar.
    fn anchor(&self, point: Option<POINT>) -> PxRect {
        if let Some(r) = self.tray.borrow().rect() {
            return r;
        }
        if let Some(p) = point {
            return PxRect {
                left: p.x - 12,
                top: p.y - 12,
                right: p.x + 12,
                bottom: p.y + 12,
            };
        }
        taskbar_corner()
    }

    pub fn open_flyout(&self, point: Option<POINT>) {
        if self.flyout_is_open() {
            return;
        }
        *self.flyout.borrow_mut() = None;
        // The forecast texts are as of now, not as of the last event.
        self.after_live(true);
        *self.expanded.borrow_mut() = view::default_expanded(&self.snapshot.borrow());
        let hwnd = self.hwnd.get();
        let handlers = Handlers {
            on_hit: Box::new(move |hit| post(hwnd, WM_FLYOUT_HIT, encode_hit(hit))),
            on_close: Box::new(move || {
                LAST_CLOSE.with(|c| c.set(Some(Instant::now())));
                post(hwnd, WM_FLYOUT_CLOSED, 0);
            }),
        };
        match Flyout::show(self.ui_state(), self.anchor(point), handlers) {
            Ok(f) => {
                self.trace(&format!("flyout opened at {:?}", f.rect()));
                *self.flyout.borrow_mut() = Some(f);
            }
            Err(e) => self.trace(&format!("flyout failed: {e}")),
        }
    }

    fn close_flyout(&self) {
        if let Some(f) = self.flyout.borrow().as_ref() {
            f.hide();
        }
    }

    /// A tray click or the hotkey. `from_tray` adds the "this click just closed it" guard.
    fn toggle_flyout(&self, point: Option<POINT>, from_tray: bool) {
        if self.flyout_is_open() {
            self.close_flyout();
            return;
        }
        if from_tray {
            let since = LAST_CLOSE.with(Cell::get).map(|t| t.elapsed());
            if reopen_blocked(since) {
                self.trace("tray click ignored: it closed the flyout");
                return;
            }
        }
        self.open_flyout(point);
    }

    fn show_menu(&self, at: POINT) {
        menu::style_menus(system::app_theme() == juicebar_ui::model::Theme::Dark);
        let choices = self
            .source
            .borrow_mut()
            .live_mut()
            .map(|l| l.devices())
            .unwrap_or_default();
        let items: Vec<(String, bool)> =
            choices.iter().map(|c| (c.name.clone(), c.shown)).collect();
        let command = menu::show(
            self.hwnd.get(),
            at,
            self.lang,
            HOTKEY_TEXT,
            runkey::is_enabled(runkey::VALUE),
            self.hotkey_ok.get(),
            &items,
        );
        match command {
            Some(Command::Open) => self.open_flyout(None),
            Some(Command::Autostart) => {
                let on = !runkey::is_enabled(runkey::VALUE);
                if let Err(e) = runkey::set_enabled(runkey::VALUE, &self.exe, on) {
                    self.trace(&format!("run key failed: {e}"));
                }
            }
            Some(Command::Device(i)) => {
                if let Some(choice) = choices.get(i) {
                    let hidden = self.source.borrow_mut().live_mut().map(|live| {
                        live.set_shown(&choice.key, !choice.shown);
                        live.hidden()
                    });
                    if let Some(hidden) = hidden {
                        self.settings.borrow_mut().hidden = hidden;
                        self.save_settings();
                        self.after_live(false);
                    }
                }
            }
            Some(Command::Quit) => {
                // SAFETY: destroying our own window ends the loop through WM_DESTROY.
                unsafe {
                    let _ = DestroyWindow(self.hwnd.get());
                }
            }
            None => {}
        }
    }

    fn on_flyout_hit(&self, hit: FlyoutHit) {
        match hit {
            FlyoutHit::Row(i) => {
                let key = self.snapshot.borrow().devices.get(i).map(|d| d.key.clone());
                {
                    let mut exp = self.expanded.borrow_mut();
                    *exp = if *exp == key { None } else { key };
                }
                self.refresh_flyout();
            }
            FlyoutHit::RgbSwitch => {
                let on = !self.rgb_economy.get();
                self.rgb_economy.set(on);
                if let Some(live) = self.source.borrow_mut().live_mut() {
                    live.set_rgb_economy(on, winhost::core_now(self.start));
                }
                self.settings.borrow_mut().rgb_economy = on;
                self.save_settings();
                self.after_live(false);
                self.refresh_flyout();
            }
            FlyoutHit::Gear => {
                let mut p = POINT::default();
                // SAFETY: a valid out pointer.
                let _ = unsafe { GetCursorPos(&mut p) };
                self.show_menu(p);
            }
            FlyoutHit::None => {}
        }
    }

    fn on_tray(&self, wparam: WPARAM, lparam: LPARAM) {
        let event = (lparam.0 & 0xffff) as u32;
        self.trace(&format!("tray event {event:#x}"));
        let at = point_of(wparam.0);
        match classify(event) {
            TrayEvent::Toggle => self.toggle_flyout(Some(at), true),
            TrayEvent::Menu => self.show_menu(at),
            TrayEvent::Ignore => {}
        }
    }

    /// Takes the icon off the area and lets go of the hotkey. Safe to call twice.
    fn shutdown(&self) {
        let hwnd = self.hwnd.get();
        // SAFETY: our own timers and hot key; each fails harmlessly when already gone.
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_WAKE);
            let _ = KillTimer(Some(hwnd), TIMER_QUIT);
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
        }
        *self.flyout.borrow_mut() = None;
        // The histories reach their files before the icon goes.
        self.flush_live();
        if let Ok(mut t) = self.tray.try_borrow_mut() {
            t.remove();
        }
        if let Some(t) = self.transport.borrow_mut().take() {
            t.stop();
        }
    }

    fn dispatch(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if msg == self.msg_open {
            self.open_flyout(None);
            return Some(LRESULT(0));
        }
        if msg == self.msg_hid {
            self.on_hid();
            return Some(LRESULT(0));
        }
        if msg == self.msg_taskbar {
            let (key, tip) = {
                let s = self.snapshot.borrow();
                (self.icon_key(s.icon), s.tooltip.clone())
            };
            if let Ok(mut t) = self.tray.try_borrow_mut() {
                let _ = t.readd(key, &tip);
            }
            return Some(LRESULT(0));
        }
        match msg {
            WM_TRAY => self.on_tray(wparam, lparam),
            WM_HOTKEY => self.toggle_flyout(None, false),
            WM_FLYOUT_HIT => self.on_flyout_hit(decode_hit(wparam.0)),
            WM_FLYOUT_CLOSED => {
                let mut f = self.flyout.borrow_mut();
                if f.as_ref().is_some_and(|f| !f.is_open()) {
                    *f = None;
                    drop(f);
                    trim_memory();
                }
            }
            WM_TIMER => match wparam.0 {
                TIMER_WAKE => self.on_wake(),
                TIMER_QUIT => {
                    // SAFETY: destroying our own window ends the loop through WM_DESTROY.
                    unsafe {
                        let _ = DestroyWindow(self.hwnd.get());
                    }
                }
                _ => {}
            },
            WM_SETTINGCHANGE => {
                if setting_name(lparam).as_deref() == Some("ImmersiveColorSet") {
                    self.refresh_flyout();
                }
                self.refresh_tray();
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => self.refresh_tray(),
            WM_ENDSESSION if wparam.0 != 0 => self.shutdown(),
            WM_DESTROY => {
                self.shutdown();
                // SAFETY: ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

fn post(hwnd: HWND, msg: u32, wparam: usize) {
    // SAFETY: posting to our own window; a failure (window gone) is harmless.
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(wparam), LPARAM(0));
    }
}

/// The text a `WM_SETTINGCHANGE` carries in `lparam` ("ImmersiveColorSet", "Environment", ...).
fn setting_name(lparam: LPARAM) -> Option<String> {
    if lparam.0 == 0 {
        return None;
    }
    // SAFETY: for WM_SETTINGCHANGE a non-zero lparam points to a NUL-terminated wide string.
    unsafe { PCWSTR(lparam.0 as *const u16).to_string().ok() }
}

/// The right end of the primary taskbar, where the notification area is, as a small rectangle.
fn taskbar_corner() -> PxRect {
    // SAFETY: a lookup of the shell's window and a read of its rectangle.
    unsafe {
        if let Ok(h) = FindWindowW(w!("Shell_TrayWnd"), None) {
            let mut r = RECT::default();
            if GetWindowRect(h, &mut r).is_ok() {
                return PxRect {
                    left: r.right - 170,
                    top: r.top,
                    right: r.right - 146,
                    bottom: r.bottom,
                };
            }
        }
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut mi);
        PxRect {
            left: mi.rcMonitor.right - 170,
            top: mi.rcWork.bottom,
            right: mi.rcMonitor.right - 146,
            bottom: mi.rcMonitor.bottom,
        }
    }
}

/// # Safety
/// Called by the system with the arguments of a window message.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: for WM_NCCREATE, lparam points to the CREATESTRUCTW whose lpCreateParams is our App pointer.
        let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        // SAFETY: storing a pointer-sized value in the window's user data.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize) };
        // SAFETY: the pointer is the live App created by `run`.
        unsafe { (*(cs.lpCreateParams as *const App)).hwnd.set(hwnd) };
        // SAFETY: forwarding the message unchanged.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: reading the user data this window procedure stored.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    if ptr.is_null() {
        // SAFETY: forwarding the message unchanged.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    if msg == WM_NCDESTROY {
        // SAFETY: clearing the user data, then taking back the Box leaked in `run`; nothing uses it after.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    // SAFETY: the pointer lives until WM_NCDESTROY above.
    let app = unsafe { &*ptr };
    match app.dispatch(msg, wparam, lparam) {
        Some(r) => r,
        // SAFETY: forwarding the message unchanged.
        None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Creates the hidden window and runs the message loop until the app quits.
pub fn run(args: Args, user: &str) -> Result<()> {
    // SAFETY: a null name asks for this process's own module.
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised and outlives the call.
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread());
    }
    let app = Box::into_raw(Box::new(App::new(args, user)));
    // SAFETY: the class is registered; `app` stays valid until WM_NCDESTROY reclaims it.
    let created = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            WINDOW_CLASS,
            w!("JuiceBar"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            Some(app as *const _),
        )
    };
    if let Err(e) = created {
        // SAFETY: the window was never created, so nothing else holds the pointer.
        drop(unsafe { Box::from_raw(app) });
        return Err(e);
    }
    // SAFETY: the window (and with it the App) lives until the loop below ends.
    unsafe { (*app).init() };
    let mut msg = MSG::default();
    // SAFETY: the standard loop; GetMessageW returns false on WM_QUIT.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_click_toggles_and_right_click_opens_the_menu() {
        assert_eq!(classify(NIN_SELECT), TrayEvent::Toggle);
        assert_eq!(classify(NIN_KEYSELECT), TrayEvent::Toggle);
        assert_eq!(classify(WM_CONTEXTMENU), TrayEvent::Menu);
        assert_eq!(classify(0x0202), TrayEvent::Ignore, "WM_LBUTTONUP");
        assert_eq!(classify(0x0200), TrayEvent::Ignore, "WM_MOUSEMOVE");
    }

    #[test]
    fn a_click_right_after_a_close_does_not_reopen() {
        assert!(reopen_blocked(Some(Duration::from_millis(80))));
        assert!(!reopen_blocked(Some(Duration::from_millis(900))));
        assert!(!reopen_blocked(None));
    }

    #[test]
    fn hits_survive_the_message_round_trip() {
        for hit in [
            FlyoutHit::Row(0),
            FlyoutHit::Row(7),
            FlyoutHit::Gear,
            FlyoutHit::RgbSwitch,
            FlyoutHit::None,
        ] {
            assert_eq!(decode_hit(encode_hit(hit)), hit);
        }
    }

    #[test]
    fn packed_points_keep_their_sign() {
        let p = point_of(((200u32 << 16) | 0xFFF6) as usize);
        assert_eq!((p.x, p.y), (-10, 200));
    }

    #[test]
    fn the_language_follows_the_tag() {
        assert_eq!(detect_lang(Some("pt-BR")), Lang::PtBr);
        assert_eq!(detect_lang(Some("en-US")), Lang::En);
        let _ = detect_lang(None);
    }

    #[test]
    fn the_windows_display_language_picks_portuguese_for_any_pt() {
        assert_eq!(lang_from_langid(0x0416), Lang::PtBr, "pt-BR");
        assert_eq!(lang_from_langid(0x0816), Lang::PtBr, "pt-PT");
        assert_eq!(lang_from_langid(0x0409), Lang::En, "en-US");
        assert_eq!(lang_from_langid(0x0C0A), Lang::En, "es");
        assert_eq!(lang_from_langid(0x0000), Lang::En);
    }
}
