// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
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
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::Shell::{NIN_SELECT, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GWLP_USERDATA,
    GetMessageW, GetWindowLongPtrW, GetWindowRect, KillTimer, MSG, PBT_APMRESUMEAUTOMATIC,
    PBT_APMSUSPEND, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW,
    SW_SHOWNORMAL, SetTimer, SetWindowLongPtrW, TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_HOTKEY, WM_NCCREATE,
    WM_NCDESTROY, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use juicebar_core::snapshot::Snapshot;
use juicebar_core::strings::Lang;
use juicebar_core::types::{Action, DeviceKey};
use juicebar_hid::Transport;
use juicebar_ui::flyout::{Flyout, Handlers};
use juicebar_ui::icon::IconLook;
use juicebar_ui::layout::{FlyoutHit, PxRect};
use juicebar_ui::model::UiState;
use juicebar_ui::settings::{Chord, EditKey, RowId, UiSettings};
use juicebar_ui::system;

use crate::anim::{self, Anim, TrayHost};
use crate::args::Args;
use crate::calib::{self, Calibrator};
use crate::controllers::Controllers;
use crate::devices::{self, Edited, Rename};
use crate::files::DataDir;
use crate::hotkey::{self, Hotkey, Registrar};
use crate::live::{DeviceChoice, LightChange, Live, Prefs, StartConfig};
use crate::menu::{self, Command};
use crate::notify::{Notice, Tracker};
use crate::runkey;
use crate::settings::{LangChoice, Settings};
use crate::settings_page::{self, Change, DeviceOp, HotkeyState, PageInput, row};
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
/// The one-shot timer the source asks for (`wake_at`, and the live source's `Wake`).
const TIMER_WAKE: usize = 1;
/// `--seconds N`: quit by itself (tests and captures only).
const TIMER_QUIT: usize = 2;
/// The game-controller reads: armed only while a pad is present, for the earliest read due.
const TIMER_PADS: usize = 5;

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
        FlyoutHit::Info => usize::MAX - 3,
        FlyoutHit::Start => usize::MAX - 4,
        FlyoutHit::Back => usize::MAX - 5,
        FlyoutHit::Setting { id, part } => {
            SETTING_BASE + ((usize::from(id) << 8) | usize::from(part))
        }
        FlyoutHit::Edit(key) => match key {
            EditKey::Backspace => EDIT_BASE,
            EditKey::Enter => EDIT_BASE + 1,
            EditKey::Escape => EDIT_BASE + 2,
            EditKey::Char(c) => EDIT_CHAR_BASE + c as usize,
        },
        FlyoutHit::Chord(c) => CHORD_BASE + ((usize::from(c.mods) << 16) | usize::from(c.vk)),
    }
}

/// Hits of the Settings page ride above every device index: `SETTING_BASE + (row id << 8 | part)`.
const SETTING_BASE: usize = 1 << 32;
/// Keys typed into a text field: `EDIT_BASE` + 0 Backspace, 1 Enter, 2 Escape, and `EDIT_CHAR_BASE + code point`.
const EDIT_BASE: usize = 1 << 33;
const EDIT_CHAR_BASE: usize = 1 << 34;
/// A key combination: `CHORD_BASE + (modifiers << 16 | virtual key)`.
const CHORD_BASE: usize = 1 << 35;

pub fn decode_hit(v: usize) -> FlyoutHit {
    match v {
        usize::MAX => FlyoutHit::Gear,
        x if x == usize::MAX - 1 => FlyoutHit::RgbSwitch,
        x if x == usize::MAX - 2 => FlyoutHit::None,
        x if x == usize::MAX - 3 => FlyoutHit::Info,
        x if x == usize::MAX - 4 => FlyoutHit::Start,
        x if x == usize::MAX - 5 => FlyoutHit::Back,
        x if x >= CHORD_BASE => FlyoutHit::Chord(Chord {
            vk: ((x - CHORD_BASE) & 0xffff) as u16,
            mods: (((x - CHORD_BASE) >> 16) & 0xff) as u8,
        }),
        x if x >= EDIT_CHAR_BASE => char::from_u32((x - EDIT_CHAR_BASE) as u32)
            .map_or(FlyoutHit::None, |c| FlyoutHit::Edit(EditKey::Char(c))),
        x if x >= EDIT_BASE => match x - EDIT_BASE {
            0 => FlyoutHit::Edit(EditKey::Backspace),
            1 => FlyoutHit::Edit(EditKey::Enter),
            2 => FlyoutHit::Edit(EditKey::Escape),
            _ => FlyoutHit::None,
        },
        x if x >= SETTING_BASE => FlyoutHit::Setting {
            id: ((x - SETTING_BASE) >> 8) as u16,
            part: ((x - SETTING_BASE) & 0xff) as u8,
        },
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

/// The shortcut's registration with the system, on the shell's window. A test instance (`--no-hotkey`) registers
/// nothing and takes every shortcut as free, so it never fights the owner's own JuiceBar.
struct HotkeyReg {
    hwnd: HWND,
    live: bool,
}

impl Registrar for HotkeyReg {
    fn register(&mut self, hotkey: Hotkey) -> bool {
        if !self.live {
            return true;
        }
        // SAFETY: registering a hot key for our own window; failure (taken by another app) is the answer.
        unsafe {
            RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_ID,
                HOT_KEY_MODIFIERS(u32::from(hotkey.mods)) | MOD_NOREPEAT,
                u32::from(hotkey.vk),
            )
        }
        .is_ok()
    }

    fn unregister(&mut self) {
        if self.live {
            // SAFETY: our own hot key; it fails harmlessly when not registered.
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ID);
            }
        }
    }
}

pub struct App {
    hwnd: Cell<HWND>,
    args: Args,
    exe: PathBuf,
    /// The language of the texts: the user's choice, else Windows' (or the command line's).
    lang: Cell<Lang>,
    msg_open: u32,
    msg_taskbar: u32,
    /// Posted by the transport's waker: events are waiting in `drain`.
    msg_hid: u32,
    /// Posted by the game-controller provider: arrivals, removals and finished reads are waiting.
    msg_pads: u32,
    start: Instant,
    data: Option<DataDir>,
    settings: RefCell<Settings>,
    transport: RefCell<Option<Rc<Transport>>>,
    pads: RefCell<Option<Controllers>>,
    /// The live source has flushed and said good-bye to its devices (quit runs once).
    live_closed: Cell<bool>,
    /// The first snapshot always reaches the tray; later ones only when they differ.
    applied_once: Cell<bool>,
    tray: RefCell<Tray>,
    /// The tray mark's motion: no timer between its runs (`anim`).
    anim: RefCell<Anim>,
    flyout: RefCell<Option<Flyout>>,
    source: RefCell<Source>,
    snapshot: RefCell<Snapshot>,
    tracker: RefCell<Tracker>,
    /// The power saver switch (stored in settings.txt, handed to every tracker; it does not yet change the
    /// devices' lighting).
    rgb_economy: Cell<bool>,
    expanded: RefCell<Option<DeviceKey>>,
    /// The (i) next to the power saver switch was clicked: its text stays open until the next click or the flyout
    /// closes.
    info_open: Cell<bool>,
    /// The open flyout shows the first-open welcome page instead of the list.
    welcome: Cell<bool>,
    /// The open flyout shows the Settings page instead of the list.
    page_open: Cell<bool>,
    /// The limit that the last change of the other one moved (its row says so until the next change).
    adjusted: Cell<Option<crate::settings::Adjusted>>,
    hotkey_ok: Cell<bool>,
    /// How many window-procedure calls are inside `dispatch` right now; the App must never be
    /// freed while it is above zero.
    in_dispatch: Cell<u32>,
    /// The shortcut's row: idle, waiting for a key combination, or told that the last one is taken.
    hotkey_state: Cell<HotkeyState>,
    /// The device being renamed on the Settings page, if any.
    rename: RefCell<Option<Rename>>,
    /// The device behind each device row's id (`row::DEVICE_BASE` + index), kept for the whole run so a click always
    /// reaches the device it was drawn for.
    device_slots: RefCell<Vec<DeviceKey>>,
}

impl App {
    fn new(args: Args, user: &str) -> App {
        let source = if args.demo {
            Source::Demo {
                phase: DemoPhase::Scene,
                cycle: args.demo_cycle,
            }
        } else {
            // The live source needs the window; `init` replaces this one.
            Source::Empty
        };
        // A run with its own data folder keeps its settings there in any mode; the demo and the empty state otherwise
        // touch no file.
        let data = match &args.data {
            Some(p) => Some(DataDir::at(p.clone())),
            None if args.is_live() => DataDir::from_env(),
            None => None,
        };
        let mut settings = data
            .as_ref()
            .map(|d| Settings::parse(&d.read_settings()))
            .unwrap_or_default();
        // The two weeks of battery calibration open at the first plain start of the version that has them, once.
        if args.is_plain()
            && let Some(d) = &data
        {
            let (until, fresh) =
                calib::window_at_start(settings.calibrate_until, Clock::now().wall);
            settings.calibrate_until = Some(until);
            if fresh {
                let _ = d.write_settings(&settings.encode());
            }
        }
        let lang = settings.language.resolve(detect_lang(args.lang.as_deref()));
        let snapshot = source.snapshot(&Clock::now(), lang, &settings);
        let mut tracker = Tracker::default();
        tracker.set_alerts(settings.smart_warning, settings.full_notice);
        let mut anim = Anim::new(args.anim_every.map(|s| Duration::from_secs(u64::from(s))));
        anim.set_user_on(settings.icon_animation);
        // SAFETY: registering messages by name.
        let (msg_taskbar, msg_hid) = unsafe {
            (
                RegisterWindowMessageW(w!("TaskbarCreated")),
                RegisterWindowMessageW(w!("JuiceBar.HidEvents")),
            )
        };
        // SAFETY: registering a message by name.
        let msg_pads = unsafe { RegisterWindowMessageW(w!("JuiceBar.PadEvents")) };
        App {
            hwnd: Cell::new(HWND::default()),
            exe: std::env::current_exe().unwrap_or_default(),
            lang: Cell::new(lang),
            msg_open: single::open_message(user),
            msg_taskbar,
            msg_hid,
            msg_pads,
            start: Instant::now(),
            data,
            rgb_economy: Cell::new(settings.rgb_economy),
            settings: RefCell::new(settings),
            transport: RefCell::new(None),
            pads: RefCell::new(None),
            live_closed: Cell::new(false),
            applied_once: Cell::new(false),
            tray: RefCell::new(Tray::new(HWND::default(), WM_TRAY)),
            anim: RefCell::new(anim),
            flyout: RefCell::new(None),
            source: RefCell::new(source),
            snapshot: RefCell::new(snapshot),
            tracker: RefCell::new(tracker),
            expanded: RefCell::new(None),
            info_open: Cell::new(false),
            welcome: Cell::new(false),
            page_open: Cell::new(false),
            adjusted: Cell::new(None),
            hotkey_ok: Cell::new(false),
            in_dispatch: Cell::new(0),
            hotkey_state: Cell::new(HotkeyState::Idle),
            rename: RefCell::new(None),
            device_slots: RefCell::new(Vec::new()),
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
        let ok = !self.args.no_hotkey && self.registrar().register(self.settings.borrow().hotkey);
        self.hotkey_ok.set(ok);
        self.trace(&format!(
            "hotkey {}",
            if ok { "registered" } else { "unavailable" }
        ));
        if self.args.is_plain() {
            runkey::refresh(runkey::VALUE, &self.exe);
        }
        if self.args.is_live() {
            self.start_live();
        }
        let snap = self.fresh_snapshot();
        self.apply(snap);
        trim_memory();
        if self.args.open_settings {
            self.open_flyout_at(None, true);
        }
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

    fn registrar(&self) -> HotkeyReg {
        HotkeyReg {
            hwnd: self.hwnd.get(),
            live: !self.args.no_hotkey,
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
            let snap = self.fresh_snapshot();
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
        )
        .with_calibrator(self.calibrator());
        let histories = self
            .data
            .as_ref()
            .map(DataDir::read_histories)
            .unwrap_or_default();
        let cfg = {
            let settings = self.settings.borrow();
            StartConfig {
                lang: self.lang.get(),
                prefs: Self::prefs_of(&settings),
                rgb_economy: settings.rgb_economy,
                hidden: settings.hidden.clone(),
                arrangement: settings.arrangement.clone(),
                lights: settings.lights.clone(),
            }
        };
        let live = Live::start(host, histories, cfg, winhost::core_now(self.start));
        *self.source.borrow_mut() = Source::Live(Box::new(live));
        *self.pads.borrow_mut() = Some(Controllers::start(hwnd, self.msg_pads));
    }

    /// The battery calibration, while its two weeks run (plain launches only; it writes under the data folder).
    fn calibrator(&self) -> Option<Calibrator> {
        let until = self.settings.borrow().calibrate_until;
        let data = self.data.as_ref().filter(|_| self.args.is_plain())?;
        calib::is_on(until, Clock::now().wall)
            .then(|| Calibrator::new(data.calibration_dir(), until.unwrap_or_default()))
    }

    /// The game-controller provider has something: its events reach the live source like the Logitech provider's.
    fn on_pads(&self) {
        let now = winhost::core_now(self.start);
        let events = match self.pads.borrow_mut().as_mut() {
            Some(pads) => pads.run(now.mono, now.wall),
            None => return,
        };
        if !events.is_empty() {
            if let Some(live) = self.source.borrow_mut().live_mut() {
                live.execute(events.into_iter().map(Action::Emit).collect(), now);
            }
            self.after_live(false);
        }
        self.arm_pads();
    }

    /// One timer for the earliest pad read; none while no read is waiting.
    fn arm_pads(&self) {
        let due = self.pads.borrow().as_ref().and_then(Controllers::next_due);
        // SAFETY: a timer on our own window; the same id replaces the previous one.
        unsafe {
            match due {
                Some(at) => {
                    let ms = ((at - winhost::core_now(self.start).mono) * 1000.0).ceil();
                    SetTimer(
                        Some(self.hwnd.get()),
                        TIMER_PADS,
                        ms.clamp(1.0, 2_000_000_000.0) as u32,
                        None,
                    );
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd.get()), TIMER_PADS);
                }
            }
        }
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

    fn icon_key(&self, look: juicebar_core::snapshot::IconLook) -> IconKey {
        IconKey {
            look: view::icon_look(look),
            dark_taskbar: system::taskbar_is_dark(),
            px: tray::small_icon_px(tray::taskbar_dpi()),
        }
    }

    /// Takes a snapshot of a source that has no estimators of its own (the demo): the notices come from
    /// comparing it with the last one.
    fn apply(&self, snap: Snapshot) {
        let notices = self.tracker.borrow_mut().observe(&snap, self.lang.get());
        self.apply_with(snap, notices);
    }

    /// Takes a new snapshot with the notices to show: the icon and tooltip and the open flyout are touched only
    /// when it differs from the last one (the first one always reaches the tray).
    fn apply_with(&self, snap: Snapshot, notices: Vec<Notice>) {
        let notices = notices_to_show(self.settings.borrow().do_not_disturb, notices);
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
            self.anim_key(&mut t, key);
            let dpi = tray::taskbar_dpi();
            let dark_toast = toast_is_dark(system::taskbar_is_dark());
            for n in &notices {
                self.trace(&format!("notice: {} | {}", n.title, n.body));
                let shown = t.balloon(n, IconLook::default(), dark_toast, tray::large_icon_px(dpi));
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
            self.anim_key(&mut t, key);
        }
    }

    // ---- the tray mark's motion

    /// The tray shows `key` now: the driver plays what a change of it asks for.
    fn anim_key(&self, tray: &mut Tray, key: IconKey) {
        let mut host = TrayHost {
            hwnd: self.hwnd.get(),
            tray,
        };
        let event = self
            .anim
            .borrow_mut()
            .on_key(&mut host, key, Instant::now());
        self.trace_anim(event);
    }

    /// One of the driver's timers fired.
    fn on_anim_timer(&self, id: usize) {
        let Ok(mut tray) = self.tray.try_borrow_mut() else {
            return;
        };
        let mut host = TrayHost {
            hwnd: self.hwnd.get(),
            tray: &mut tray,
        };
        let now = Instant::now();
        let event = if id == anim::TIMER_FRAME {
            self.anim.borrow_mut().on_frame(&mut host, now)
        } else {
            self.anim.borrow_mut().on_idle(&mut host, now)
        };
        self.trace_anim(event);
    }

    /// The system's settings changed: the animation effects may have been switched.
    fn on_anim_settings(&self) {
        let Ok(mut tray) = self.tray.try_borrow_mut() else {
            return;
        };
        let mut host = TrayHost {
            hwnd: self.hwnd.get(),
            tray: &mut tray,
        };
        let event = self
            .anim
            .borrow_mut()
            .on_settings(&mut host, Instant::now());
        self.trace_anim(event);
    }

    fn trace_anim(&self, event: Option<anim::Event>) {
        if let Some(e) = event {
            self.trace(&format!("anim {e:?}"));
        }
    }

    /// The core's alert settings for the user's choices.
    fn prefs_of(s: &Settings) -> Prefs {
        Prefs {
            limits: s.limits(),
            low_percent: s.warn_below,
            smart: s.smart_warning,
            full: s.full_notice,
        }
    }

    /// The source's snapshot now, with the user's limits and language.
    fn fresh_snapshot(&self) -> Snapshot {
        self.source
            .borrow()
            .snapshot(&Clock::now(), self.lang.get(), &self.settings.borrow())
    }

    /// The Dispositivos rows: every known device with the id of its row.
    fn device_rows(&self) -> Vec<(RowId, DeviceChoice)> {
        let list =
            self.source
                .borrow()
                .devices(&Clock::now(), self.lang.get(), &self.settings.borrow());
        let mut slots = self.device_slots.borrow_mut();
        list.into_iter()
            .filter_map(|d| {
                let at = match slots.iter().position(|k| *k == d.key) {
                    Some(at) => at,
                    None => {
                        slots.push(d.key.clone());
                        slots.len() - 1
                    }
                };
                let id = row::DEVICE_BASE.checked_add(u16::try_from(at).ok()?)?;
                Some((id, d))
            })
            .collect()
    }

    /// The device a device row stands for.
    fn device_of(&self, id: RowId) -> Option<DeviceKey> {
        let at = usize::from(id.checked_sub(row::DEVICE_BASE)?);
        self.device_slots.borrow().get(at).cloned()
    }

    /// The Settings page for the user's settings.
    fn settings_page(&self) -> UiSettings {
        let devices = self.device_rows();
        let settings = self.settings.borrow();
        let rename = self.rename.borrow();
        settings_page::build(&PageInput {
            settings: &settings,
            lang: self.lang.get(),
            autostart: runkey::is_enabled(runkey::VALUE),
            autostart_locked: !self.args.is_plain(),
            transparency: system::transparency_enabled(),
            hotkey: settings.hotkey,
            hotkey_ok: self.hotkey_ok.get() || self.args.no_hotkey,
            hotkey_state: self.hotkey_state.get(),
            devices: &devices,
            rename: rename.as_ref(),
            adjusted: self.adjusted.get(),
            calibration_ends: self.calibration_ends(),
        })
    }

    /// `dd/mm` of the day the calibration ends, while it runs.
    fn calibration_ends(&self) -> Option<String> {
        let until = self.settings.borrow().calibrate_until;
        let clock = Clock::now();
        if !self.args.is_plain() || !calib::is_on(until, clock.wall) {
            return None;
        }
        Some(calib::day_month(until?, clock.utc_offset))
    }

    fn ui_state(&self) -> UiState {
        // The flyout's colors are the user's choice; the tray mark keeps following the taskbar.
        let theme = self.settings.borrow().theme.resolve(system::app_theme());
        let accent = system::accent_for(theme, system::accent_color());
        let clock = Clock::now();
        let snap = self.snapshot.borrow();
        let expanded = self.expanded.borrow();
        let lang = self.lang.get();
        let empty = menu::texts(lang).empty;
        // While G HUB is putting the lighting back, the footer says so on its own line, instead of fighting it.
        let conflict = self.rgb_economy.get()
            && self
                .source
                .borrow()
                .live()
                .is_some_and(|live| live.lighting_conflict());
        let ghub_notice = conflict.then(|| view::ghub_notice(lang));
        let hotkey_text = self.settings.borrow().hotkey.text();
        view::ui_state(&ViewInput {
            snapshot: &snap,
            theme,
            accent,
            lang,
            rgb_economy: self.rgb_economy.get(),
            info_open: self.info_open.get(),
            hotkey_text: &hotkey_text,
            ghub_notice,
            welcome: self.welcome.get(),
            settings: self.page_open.get().then(|| self.settings_page()),
            low_threshold: self.settings.borrow().warn_below,
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
        self.open_flyout_at(point, false);
    }

    /// Opens the flyout on the device list, or on the Settings page when `page`; nothing when it is open.
    pub fn open_flyout_at(&self, point: Option<POINT>, page: bool) {
        if self.flyout_is_open() {
            return;
        }
        *self.flyout.borrow_mut() = None;
        // The forecast texts are as of now, not as of the last event.
        self.after_live(true);
        *self.expanded.borrow_mut() = view::default_expanded(&self.snapshot.borrow());
        self.info_open.set(false);
        self.page_open.set(page);
        self.adjusted.set(None);
        self.end_text_entry();
        // The first open of a new install shows the welcome page (unless Settings was asked for); it is marked seen
        // at once, so it never nags.
        let first = !self.settings.borrow().welcome_seen;
        self.welcome.set(first && !page);
        if first {
            self.settings.borrow_mut().welcome_seen = true;
            self.save_settings();
        }
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
        match menu::show(self.hwnd.get(), at, self.lang.get()) {
            Some(Command::Open) => self.open_flyout(None),
            Some(Command::Settings) => self.open_settings(),
            Some(Command::Quit) => {
                self.request_quit();
            }
            None => {}
        }
    }

    /// The flyout on the Settings page: the open one switches to it, a closed one opens on it.
    fn open_settings(&self) {
        if self.flyout_is_open() {
            self.welcome.set(false);
            self.page_open.set(true);
            self.adjusted.set(None);
            self.end_text_entry();
            self.refresh_flyout();
        } else {
            self.open_flyout_at(None, true);
        }
    }

    // ---- the Settings page

    /// A click or key on a row of the page.
    fn on_setting(&self, id: u16, part: u8) {
        let page = self.settings_page();
        let action = settings_page::action(&page, id, part);
        // A click on anything but the field being renamed ends the rename (the name is not kept; Enter keeps it).
        let renaming = self.rename.borrow().as_ref().and_then(|r| {
            self.device_slots
                .borrow()
                .iter()
                .position(|k| *k == r.key)
                .map(|at| row::DEVICE_BASE + at as u16)
        });
        if renaming.is_some_and(|rid| rid != id) {
            *self.rename.borrow_mut() = None;
        }
        // The shortcut's capture ends on any other click too.
        if self.hotkey_state.get().waits() && action != Some(Change::HotkeyCapture) {
            self.hotkey_state.set(HotkeyState::Idle);
        }
        match action {
            Some(action) => self.apply_action(action),
            None => {
                self.trace(&format!("setting {id}/{part}: nothing to do"));
                self.refresh_flyout();
            }
        }
    }

    /// A rename and the wait for a key combination end together (the page closes, opens again, goes back).
    fn end_text_entry(&self) {
        *self.rename.borrow_mut() = None;
        self.hotkey_state.set(HotkeyState::Idle);
    }

    /// Carries out what a row asked: the new value reaches the settings file, the source and the screen at once.
    pub fn apply_action(&self, action: Change) {
        self.trace(&format!("setting {action:?}"));
        let mut adjusted = None;
        match action {
            Change::Theme(t) => self.settings.borrow_mut().theme = t,
            Change::IconAnimation(on) => {
                self.settings.borrow_mut().icon_animation = on;
                self.anim_user(on);
            }
            Change::Green(v) => adjusted = self.settings.borrow_mut().set_green(v),
            Change::Red(v) => adjusted = self.settings.borrow_mut().set_red(v),
            Change::Warn(v) => self.settings.borrow_mut().set_warn(v),
            Change::Smart(on) => self.settings.borrow_mut().smart_warning = on,
            Change::Full(on) => self.settings.borrow_mut().full_notice = on,
            Change::DoNotDisturb(on) => self.settings.borrow_mut().do_not_disturb = on,
            // A test or development launch never touches the owner's Run value.
            Change::Autostart(_) if !self.args.is_plain() => {}
            Change::Autostart(on) => {
                if let Err(e) = runkey::set_enabled(runkey::VALUE, &self.exe, on) {
                    self.trace(&format!("run key failed: {e}"));
                }
            }
            Change::Language(l) => self.settings.borrow_mut().language = l,
            Change::Device(rid, op) => {
                self.device_op(rid, op);
                return;
            }
            Change::HotkeyCapture => {
                let next = if self.hotkey_state.get().waits() {
                    HotkeyState::Idle
                } else {
                    HotkeyState::Capturing
                };
                self.hotkey_state.set(next);
                *self.rename.borrow_mut() = None;
                self.refresh_flyout();
                return;
            }
            Change::HotkeyReset => {
                self.change_hotkey(Hotkey::DEFAULT);
                return;
            }
            Change::Open(url) => {
                self.open_url(url);
                return;
            }
        }
        self.adjusted.set(adjusted);
        self.save_settings();
        match action {
            Change::Green(_)
            | Change::Red(_)
            | Change::Warn(_)
            | Change::Smart(_)
            | Change::Full(_) => self.prefs_changed(),
            Change::Language(l) => self.language_changed(l),
            _ => self.refresh_flyout(),
        }
    }

    /// A control of a device row.
    fn device_op(&self, rid: RowId, op: DeviceOp) {
        let Some(key) = self.device_of(rid) else {
            return;
        };
        match op {
            DeviceOp::Show(on) => {
                devices::set_shown(&mut self.settings.borrow_mut(), &key, on);
                if let Some(live) = self.source.borrow_mut().live_mut() {
                    live.set_shown(&key, on);
                }
            }
            DeviceOp::Rename => {
                let list = self.device_rows();
                let Some((_, d)) = list.into_iter().find(|(_, d)| d.key == key) else {
                    return;
                };
                self.hotkey_state.set(HotkeyState::Idle);
                *self.rename.borrow_mut() = Some(Rename::new(key, &d.name, &d.default_name));
                self.refresh_flyout();
                return;
            }
            DeviceOp::Up | DeviceOp::Down => {
                let keys: Vec<DeviceKey> =
                    self.device_rows().into_iter().map(|(_, d)| d.key).collect();
                let delta = if op == DeviceOp::Up { -1 } else { 1 };
                if !self
                    .settings
                    .borrow_mut()
                    .arrangement
                    .move_in(&keys, &key, delta)
                {
                    return;
                }
                self.arrangement_changed();
                return;
            }
        }
        self.save_settings();
        self.resnapshot();
    }

    /// The user's names or order changed: they reach the source, the settings file, the tray and the screen at once.
    fn arrangement_changed(&self) {
        let arrangement = self.settings.borrow().arrangement.clone();
        if let Some(live) = self.source.borrow_mut().live_mut() {
            live.set_arrangement(arrangement);
        }
        self.save_settings();
        self.resnapshot();
    }

    /// A key typed into the field being renamed.
    fn on_edit(&self, key: EditKey) {
        let outcome = self
            .rename
            .borrow_mut()
            .as_mut()
            .map(|r| (r.key(key), r.key.clone()));
        match outcome {
            Some((Edited::Editing, _)) => self.refresh_flyout(),
            Some((Edited::Cancel, _)) => {
                *self.rename.borrow_mut() = None;
                self.refresh_flyout();
            }
            Some((Edited::Save(name), device)) => {
                *self.rename.borrow_mut() = None;
                self.settings
                    .borrow_mut()
                    .arrangement
                    .set_name(&device, &name);
                self.arrangement_changed();
            }
            None if key == EditKey::Escape && self.hotkey_state.get().waits() => {
                self.hotkey_state.set(HotkeyState::Idle);
                self.refresh_flyout();
            }
            None => {}
        }
    }

    /// A key combination typed while the shortcut's row waits for one. One that cannot be a shortcut is ignored and
    /// the wait goes on.
    fn on_chord(&self, chord: Chord) {
        if !self.hotkey_state.get().waits() {
            return;
        }
        match Hotkey::from_chord(chord) {
            Some(hk) => self.change_hotkey(hk),
            None => {
                self.trace(&format!("chord {chord:?}: not a shortcut"));
                self.hotkey_state.set(HotkeyState::Refused);
                self.refresh_flyout();
            }
        }
    }

    /// Registers `wanted` in place of the shortcut in use; when another program holds it, the old one stays and the
    /// row says so.
    fn change_hotkey(&self, wanted: Hotkey) {
        let current = self.settings.borrow().hotkey;
        let done = hotkey::switch(&mut self.registrar(), current, self.hotkey_ok.get(), wanted);
        self.hotkey_ok.set(done.registered);
        self.trace(&format!(
            "hotkey {} -> {}: {}",
            current.text(),
            wanted.text(),
            if done.changed { "changed" } else { "in use" }
        ));
        if done.changed {
            self.settings.borrow_mut().hotkey = wanted;
            self.save_settings();
            self.hotkey_state.set(HotkeyState::Idle);
        } else {
            self.hotkey_state.set(HotkeyState::InUse);
        }
        self.refresh_flyout();
    }

    /// The limits or the alerts changed: the source and the tray follow at once.
    fn prefs_changed(&self) {
        let prefs = Self::prefs_of(&self.settings.borrow());
        if let Some(live) = self.source.borrow_mut().live_mut() {
            live.set_prefs(prefs);
        }
        self.tracker
            .borrow_mut()
            .set_alerts(prefs.smart, prefs.full);
        self.resnapshot();
    }

    /// The language changed: every text, the tray tooltip included, is taken again.
    fn language_changed(&self, choice: LangChoice) {
        let lang = choice.resolve(detect_lang(self.args.lang.as_deref()));
        if lang == self.lang.get() {
            self.refresh_flyout();
            return;
        }
        self.lang.set(lang);
        if let Some(live) = self.source.borrow_mut().live_mut() {
            live.set_lang(lang);
        }
        self.resnapshot();
    }

    /// Takes a new snapshot (the live source when there is one) and redraws the open flyout.
    fn resnapshot(&self) {
        let live = self.source.borrow().live().is_some();
        if live {
            self.after_live(true);
        } else {
            let snap = self.fresh_snapshot();
            self.apply(snap);
        }
        self.refresh_flyout();
    }

    /// The icon animation was switched: a run in flight stops, and no timer waits while it is off.
    fn anim_user(&self, on: bool) {
        let Ok(mut tray) = self.tray.try_borrow_mut() else {
            return;
        };
        let mut host = TrayHost {
            hwnd: self.hwnd.get(),
            tray: &mut tray,
        };
        let event = self
            .anim
            .borrow_mut()
            .set_user_enabled(&mut host, on, Instant::now());
        self.trace_anim(event);
    }

    /// Opens a web page (or a Windows settings page) in its default app.
    fn open_url(&self, url: &str) {
        self.trace(&format!("open {url}"));
        let target = HSTRING::from(url);
        // SAFETY: both strings live through the call; the shell starts the default handler on its own.
        let _ = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &target,
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
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
            FlyoutHit::Info => {
                self.info_open.set(!self.info_open.get());
                self.refresh_flyout();
            }
            FlyoutHit::Start => {
                self.welcome.set(false);
                self.refresh_flyout();
            }
            FlyoutHit::Gear => self.open_settings(),
            FlyoutHit::Back => {
                self.page_open.set(false);
                self.adjusted.set(None);
                self.end_text_entry();
                self.refresh_flyout();
            }
            FlyoutHit::Setting { id, part } => self.on_setting(id, part),
            FlyoutHit::Edit(key) => self.on_edit(key),
            FlyoutHit::Chord(chord) => self.on_chord(chord),
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

    /// Sleep and wake: the devices are off while the PC sleeps, and asked again when it wakes.
    fn on_power(&self, event: PowerEvent) {
        let now = winhost::core_now(self.start);
        self.trace(&format!("power {event:?}"));
        let touched = self.source.borrow_mut().live_mut().map(|live| match event {
            PowerEvent::Suspend => live.suspend(now),
            PowerEvent::Resume => live.resume(now),
        });
        if touched.is_some() {
            self.after_live(true);
        }
    }

    /// Asks the window to close. Posted, never a direct `DestroyWindow`: the destroy messages
    /// then arrive from the loop, not from inside the message that asked, and the App is freed
    /// only after the loop ends.
    fn request_quit(&self) {
        // SAFETY: posting a message to our own window; the default handler of WM_CLOSE destroys it.
        unsafe {
            let _ = PostMessageW(Some(self.hwnd.get()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    /// Takes the icon off the area and lets go of the hotkey. Safe to call twice.
    fn shutdown(&self) {
        let hwnd = self.hwnd.get();
        // SAFETY: our own timers and hot key; each fails harmlessly when already gone.
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_WAKE);
            let _ = KillTimer(Some(hwnd), TIMER_QUIT);
            let _ = KillTimer(Some(hwnd), TIMER_PADS);
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
        }
        if let Ok(mut t) = self.tray.try_borrow_mut() {
            let mut host = TrayHost { hwnd, tray: &mut t };
            self.anim.borrow_mut().stop(&mut host);
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
        if let Some(mut pads) = self.pads.borrow_mut().take() {
            pads.stop();
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
        if msg == self.msg_pads {
            self.on_pads();
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
            // While the shortcut's row waits for a combination, the one in use is just another key to capture.
            WM_HOTKEY if self.hotkey_state.get().waits() => {}
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
                TIMER_PADS => self.on_pads(),
                anim::TIMER_IDLE | anim::TIMER_FRAME => self.on_anim_timer(wparam.0),
                TIMER_QUIT => {
                    self.request_quit();
                }
                _ => {}
            },
            WM_SETTINGCHANGE => {
                if setting_name(lparam).as_deref() == Some("ImmersiveColorSet") {
                    self.refresh_flyout();
                }
                self.refresh_tray();
                self.on_anim_settings();
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE => self.refresh_tray(),
            WM_ENDSESSION if wparam.0 != 0 => self.shutdown(),
            WM_POWERBROADCAST => {
                if let Some(event) = power_event(wparam.0) {
                    self.on_power(event);
                }
                // TRUE: the broadcast is granted whatever it was.
                return Some(LRESULT(1));
            }
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

impl Drop for App {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.in_dispatch.get(),
            0,
            "the App was freed while a message was being dispatched"
        );
    }
}

/// Windows draws a toast in the system mode ("choose your default Windows mode"), the same setting the taskbar
/// follows, not in the apps' mode the flyout follows. The mark in the toast is drawn for that background: white
/// on a dark one, near-black on a light one (`icon::foreground`).
fn toast_is_dark(system_mode_is_dark: bool) -> bool {
    system_mode_is_dark
}

/// What a `WM_POWERBROADCAST` means for the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PowerEvent {
    Suspend,
    Resume,
}

/// `PBT_APMSUSPEND` and `PBT_APMRESUMEAUTOMATIC` (sent on every wake; `PBT_APMRESUMESUSPEND` follows only after
/// a user action, so the automatic one is the single wake signal); everything else is ignored.
fn power_event(wparam: usize) -> Option<PowerEvent> {
    match wparam as u32 {
        PBT_APMSUSPEND => Some(PowerEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC => Some(PowerEvent::Resume),
        _ => None,
    }
}

/// The balloons to show: none while do-not-disturb is on (the icon still changes).
pub fn notices_to_show(do_not_disturb: bool, notices: Vec<Notice>) -> Vec<Notice> {
    if do_not_disturb { Vec::new() } else { notices }
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
        // SAFETY: clearing the user data so later messages go to the default handler. The App
        // itself is freed by `run` after the loop ends, never here, where a caller up the stack
        // may still hold a reference to it.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    // SAFETY: the pointer lives until `run` frees it after the message loop.
    let app = unsafe { &*ptr };
    app.in_dispatch.set(app.in_dispatch.get() + 1);
    let handled = app.dispatch(msg, wparam, lparam);
    // The App may be gone here only if a nested destroy freed it, which `in_dispatch` forbids.
    app.in_dispatch.set(app.in_dispatch.get() - 1);
    match handled {
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
    // SAFETY: the class is registered; `app` stays valid until `run` reclaims it after the loop.
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
    // SAFETY: the loop is over and the window is destroyed, so nothing holds the pointer.
    drop(unsafe { Box::from_raw(app) });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_timer_has_its_own_id() {
        // Two timers sharing an id on one window replace each other and one of them never fires.
        let ids = [
            TIMER_WAKE,
            TIMER_QUIT,
            TIMER_PADS,
            anim::TIMER_IDLE,
            anim::TIMER_FRAME,
        ];
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "two timers share id {a}");
            }
        }
    }

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
            FlyoutHit::Info,
            FlyoutHit::Start,
            FlyoutHit::Back,
            FlyoutHit::None,
            FlyoutHit::Setting { id: 1, part: 0 },
            FlyoutHit::Setting { id: 16, part: 5 },
            FlyoutHit::Setting {
                id: u16::MAX,
                part: u8::MAX,
            },
            FlyoutHit::Edit(EditKey::Backspace),
            FlyoutHit::Edit(EditKey::Enter),
            FlyoutHit::Edit(EditKey::Escape),
            FlyoutHit::Edit(EditKey::Char('a')),
            FlyoutHit::Edit(EditKey::Char('ç')),
            FlyoutHit::Edit(EditKey::Char('\u{10ffff}')),
            FlyoutHit::Chord(Chord {
                vk: b'B' as u16,
                mods: Chord::CONTROL | Chord::ALT,
            }),
            FlyoutHit::Chord(Chord {
                vk: u16::MAX,
                mods: u8::MAX,
            }),
            FlyoutHit::Chord(Chord { vk: 0, mods: 0 }),
        ] {
            assert_eq!(decode_hit(encode_hit(hit)), hit, "{hit:?}");
        }
        // No device index is ever mistaken for a settings hit.
        assert_eq!(decode_hit(7), FlyoutHit::Row(7));
        assert_eq!(decode_hit((1 << 32) - 1), FlyoutHit::Row((1 << 32) - 1));
    }

    #[test]
    fn do_not_disturb_drops_every_balloon_and_otherwise_none() {
        let n = |t: &str| Notice {
            title: t.into(),
            body: String::new(),
        };
        let list = vec![n("a"), n("b")];
        assert!(notices_to_show(true, list.clone()).is_empty());
        assert_eq!(notices_to_show(false, list.clone()), list);
        assert!(notices_to_show(true, Vec::new()).is_empty());
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

    #[test]
    fn a_timed_run_quits_without_freeing_the_app_inside_its_own_message() {
        let dir = std::env::temp_dir().join(format!("juicebar-test-quit-{}", std::process::id()));
        let args = Args {
            demo: true,
            no_hotkey: true,
            seconds: Some(1),
            data: Some(dir.clone()),
            ..Args::default()
        };
        run(args, "tester").expect("the run ends by itself");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_sleep_and_the_automatic_wake_reach_the_devices() {
        assert_eq!(
            power_event(PBT_APMSUSPEND as usize),
            Some(PowerEvent::Suspend)
        );
        assert_eq!(
            power_event(PBT_APMRESUMEAUTOMATIC as usize),
            Some(PowerEvent::Resume)
        );
        // PBT_APMRESUMESUSPEND (7), PBT_APMPOWERSTATUSCHANGE (10) and the rest.
        for other in [7usize, 10, 0x8013] {
            assert_eq!(power_event(other), None, "{other}");
        }
    }

    #[test]
    fn the_toast_mark_is_white_on_a_dark_system_and_dark_on_a_light_one() {
        use juicebar_ui::icon::foreground;
        let white = foreground(true);
        let ink = foreground(false);
        assert_eq!(foreground(toast_is_dark(true)).r, white.r);
        assert_eq!(foreground(toast_is_dark(false)).r, ink.r);
        assert!(white.r > ink.r);
    }
}
