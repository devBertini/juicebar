// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The tray mark's motion (spec/design/DESIGN.md, "The bar item and the tray icon"): gentle and rare.
//!
//! * A *run* is 2 seconds at about 20 frames per second: one `SetTimer` that exists only while the run
//!   lasts and is killed when it ends, each picture one `NIM_MODIFY` with an icon that is already
//!   drawn. A slosh run (the juice sloshes, the straw sways) plays when the look changes and about
//!   every 15 minutes; while a device charges, a bubble run plays every 30 to 60 seconds.
//! * Between runs nothing runs: no frame timer, no polling. The next run is waited for with one
//!   long one-shot timer that Windows may coalesce with others, killed as it fires.
//! * With the system's animation effects off no timer exists at all (they are looked at again when
//!   the system says the setting changed).
//! * The pictures of a run are drawn once per look, taskbar theme and icon size, kept for the next
//!   runs, and destroyed when the key changes or the app ends.
//!
//! The driver talks to Windows through `Host`, so its logic is tested with a fake.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, KillTimer, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetCoalescableTimer, SetTimer, SystemParametersInfoW,
};
use windows::core::BOOL;

use juicebar_ui::icon::{IconLook, Juice, RUN_FPS, RUN_FRAMES, Run, render_run};

use crate::tray::{IconKey, Tray};

/// The timer of the long wait between runs (one-shot).
pub const TIMER_IDLE: usize = 3;
/// The timer of the frames; it exists only during a run.
pub const TIMER_FRAME: usize = 4;

/// The frame timer's period. Windows' timers tick at about 15.6 ms, so 40 ms fires about every
/// 47 ms; the picture shown is chosen from the time elapsed, so the run always lasts its 2 seconds.
const FRAME_MS: u32 = 40;

/// The slosh run comes about every 15 minutes (some randomness, so it never looks like a clock).
const SLOSH_EVERY_MS: (u64, u64) = (12 * 60_000, 18 * 60_000);
/// While a device charges, the bubbles come every 30 to 60 seconds.
const BUBBLES_EVERY_MS: (u64, u64) = (30_000, 60_000);

/// What the driver needs from Windows.
pub trait Host {
    /// Puts one of the run's pictures on the tray icon; false when the shell refused it.
    fn show(&mut self, icon: HICON) -> bool;
    /// Puts the mark at rest back.
    fn rest(&mut self);
    /// A one-shot timer; `tolerance_ms` lets Windows fire it late to coalesce wake-ups.
    fn set_timer(&mut self, id: usize, ms: u32, tolerance_ms: u32);
    fn kill_timer(&mut self, id: usize);
    /// Whether the system's animation effects are on (`SPI_GETCLIENTAREAANIMATION`).
    fn animations_on(&mut self) -> bool;
}

/// The driver's Windows side: the app's window for the timers and the tray for the pictures.
pub struct TrayHost<'a> {
    pub hwnd: HWND,
    pub tray: &'a mut Tray,
}

impl Host for TrayHost<'_> {
    fn show(&mut self, icon: HICON) -> bool {
        self.tray.show_frame(icon)
    }

    fn rest(&mut self) {
        self.tray.show_rest();
    }

    fn set_timer(&mut self, id: usize, ms: u32, tolerance_ms: u32) {
        // SAFETY: a timer on our own window; the same id replaces the previous one.
        unsafe {
            if tolerance_ms > 0 {
                SetCoalescableTimer(Some(self.hwnd), id, ms, None, tolerance_ms);
            } else {
                SetTimer(Some(self.hwnd), id, ms, None);
            }
        }
    }

    fn kill_timer(&mut self, id: usize) {
        // SAFETY: stopping our own timer; failing because it is not set is harmless.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
    }

    fn animations_on(&mut self) -> bool {
        animations_on()
    }
}

/// Whether the system's animation effects are on (Settings, Accessibility, Visual effects); on when
/// Windows cannot say.
pub fn animations_on() -> bool {
    let mut on = BOOL(1);
    // SAFETY: `on` is a BOOL, which is what SPI_GETCLIENTAREAANIMATION writes.
    let got = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut BOOL as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    got.is_err() || on.as_bool()
}

/// What happened, for the trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Started(Run),
    Finished(Run),
    /// A run was cut short (the shell refused a picture, or the animations were turned off).
    Stopped,
}

/// A small xorshift generator: the schedule only needs scatter, not quality.
pub struct Rng(u64);

impl Rng {
    pub fn seeded() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Rng(nanos ^ (u64::from(std::process::id()) << 32) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A duration between `lo` and `hi` milliseconds, both included.
    fn between(&mut self, (lo, hi): (u64, u64)) -> Duration {
        Duration::from_millis(lo + self.next() % (hi - lo + 1))
    }
}

/// When the next runs are due. `every` (a test flag) replaces the pace of both with a fixed one.
#[derive(Debug)]
pub struct Schedule {
    slosh: Instant,
    bubbles: Option<Instant>,
    every: Option<Duration>,
}

impl Schedule {
    pub fn new(now: Instant, rng: &mut Rng, every: Option<Duration>) -> Schedule {
        let mut s = Schedule {
            slosh: now,
            bubbles: None,
            every,
        };
        s.slosh = now + s.gap(Run::Slosh, rng);
        s
    }

    fn gap(&self, run: Run, rng: &mut Rng) -> Duration {
        self.every.unwrap_or_else(|| {
            rng.between(match run {
                Run::Slosh => SLOSH_EVERY_MS,
                Run::Bubbles => BUBBLES_EVERY_MS,
            })
        })
    }

    /// Bubbles are scheduled only while a device charges.
    pub fn set_charging(&mut self, charging: bool, now: Instant, rng: &mut Rng) {
        match (charging, self.bubbles) {
            (true, None) => self.bubbles = Some(now + self.gap(Run::Bubbles, rng)),
            (false, Some(_)) => self.bubbles = None,
            _ => {}
        }
    }

    /// The run that is due at `now`; the slosh wins when both are.
    pub fn due(&self, now: Instant) -> Option<Run> {
        if now >= self.slosh {
            Some(Run::Slosh)
        } else if self.bubbles.is_some_and(|b| now >= b) {
            Some(Run::Bubbles)
        } else {
            None
        }
    }

    /// A run started at `now`: the next one of its kind is a gap away. A slosh also covers a
    /// bubble run that was due with it.
    pub fn ran(&mut self, run: Run, now: Instant, rng: &mut Rng) {
        match run {
            Run::Slosh => {
                self.slosh = now + self.gap(Run::Slosh, rng);
                if self.bubbles.is_some_and(|b| now >= b) {
                    self.bubbles = Some(now + self.gap(Run::Bubbles, rng));
                }
            }
            Run::Bubbles => self.bubbles = Some(now + self.gap(Run::Bubbles, rng)),
        }
    }

    /// The soonest moment a run is due.
    pub fn next(&self) -> Instant {
        self.bubbles.map_or(self.slosh, |b| b.min(self.slosh))
    }
}

/// The picture of a run to show after `elapsed`.
pub fn frame_at(elapsed: Duration) -> usize {
    (elapsed.as_millis() * u128::from(RUN_FPS) / 1000) as usize
}

/// The drawn pictures of the runs for one look of the juice, theme and size.
struct Frames {
    juice: Juice,
    dark: bool,
    px: u32,
    slosh: Vec<HICON>,
    bubbles: Vec<HICON>,
}

impl Frames {
    fn matches(&self, key: &IconKey) -> bool {
        self.juice == key.look.juice && self.dark == key.dark_taskbar && self.px == key.px
    }

    fn of(&mut self, run: Run) -> &mut Vec<HICON> {
        match run {
            Run::Slosh => &mut self.slosh,
            Run::Bubbles => &mut self.bubbles,
        }
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        for icon in self.slosh.drain(..).chain(self.bubbles.drain(..)) {
            // SAFETY: every picture was made by `render_run` and is owned here; the tray has taken its
            // own copy of the one it shows.
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

struct Active {
    run: Run,
    started: Instant,
    /// The picture on the icon now.
    shown: usize,
}

pub struct Anim {
    every: Option<Duration>,
    rng: Rng,
    key: Option<IconKey>,
    frames: Option<Frames>,
    active: Option<Active>,
    schedule: Option<Schedule>,
    enabled: bool,
}

impl Anim {
    /// `every` is the test flag that sets the pace.
    pub fn new(every: Option<Duration>) -> Anim {
        Anim {
            every,
            rng: Rng::seeded(),
            key: None,
            frames: None,
            active: None,
            schedule: None,
            enabled: true,
        }
    }

    /// The tray has this icon now (called after every tray update). A new juice colour plays the
    /// slosh; a device starting to charge plays the bubbles; a new theme or size only discards the
    /// pictures that no longer fit. The first key plays nothing.
    pub fn on_key(&mut self, host: &mut dyn Host, key: IconKey, now: Instant) -> Option<Event> {
        let Some(prev) = self.key.replace(key) else {
            self.enabled = host.animations_on();
            self.schedule = Some(Schedule::new(now, &mut self.rng, self.every));
            if let Some(s) = &mut self.schedule {
                s.set_charging(key.look.charging, now, &mut self.rng);
            }
            self.rearm(host, now);
            return None;
        };
        if prev == key {
            return None;
        }
        // The tray has already put the new icon at rest, so a run in flight just stops.
        let was_running = self.active.is_some();
        self.stop_run(host, false);
        if (prev.dark_taskbar, prev.px) != (key.dark_taskbar, key.px) {
            self.frames = None;
        }
        if let Some(s) = &mut self.schedule {
            s.set_charging(key.look.charging, now, &mut self.rng);
        }
        let run = if prev.look.juice != key.look.juice {
            Some(Run::Slosh)
        } else if !prev.look.charging && key.look.charging {
            Some(Run::Bubbles)
        } else {
            None
        };
        let event = run.and_then(|r| self.start(host, r, now));
        if event.is_none() {
            self.rearm(host, now);
        }
        event.or(was_running.then_some(Event::Stopped))
    }

    /// The long wait is over: play the run that is due.
    pub fn on_idle(&mut self, host: &mut dyn Host, now: Instant) -> Option<Event> {
        host.kill_timer(TIMER_IDLE);
        self.enabled = host.animations_on();
        let due = self.schedule.as_ref().and_then(|s| s.due(now));
        let event = match due {
            Some(run) if self.enabled && self.active.is_none() => self.start(host, run, now),
            _ => None,
        };
        if event.is_none() {
            self.rearm(host, now);
        }
        event
    }

    /// A frame tick: show the picture the elapsed time asks for, or end the run.
    pub fn on_frame(&mut self, host: &mut dyn Host, now: Instant) -> Option<Event> {
        let Some(active) = &mut self.active else {
            host.kill_timer(TIMER_FRAME);
            return None;
        };
        let (run, index) = (
            active.run,
            frame_at(now.saturating_duration_since(active.started)),
        );
        if index >= RUN_FRAMES {
            self.stop_run(host, true);
            self.rearm(host, now);
            return Some(Event::Finished(run));
        }
        if index == active.shown {
            return None;
        }
        active.shown = index;
        let icon = self
            .frames
            .as_mut()
            .and_then(|f| f.of(run).get(index).copied());
        if icon.is_some_and(|i| host.show(i)) {
            return None;
        }
        self.stop_run(host, true);
        self.rearm(host, now);
        Some(Event::Stopped)
    }

    /// The system's settings changed: the animation effects may have been turned off or on.
    pub fn on_settings(&mut self, host: &mut dyn Host, now: Instant) -> Option<Event> {
        let on = host.animations_on();
        if on == self.enabled {
            return None;
        }
        self.enabled = on;
        if on {
            self.rearm(host, now);
            return None;
        }
        let was_running = self.active.is_some();
        self.stop_run(host, true);
        host.kill_timer(TIMER_IDLE);
        was_running.then_some(Event::Stopped)
    }

    /// The app is ending: no timer is left behind (the pictures go with the driver).
    pub fn stop(&mut self, host: &mut dyn Host) {
        self.active = None;
        host.kill_timer(TIMER_FRAME);
        host.kill_timer(TIMER_IDLE);
    }

    /// Starts a run if the animations are on and its pictures can be drawn.
    fn start(&mut self, host: &mut dyn Host, run: Run, now: Instant) -> Option<Event> {
        self.enabled = host.animations_on();
        let key = self.key?;
        if !self.enabled || !self.draw(key, run) {
            return None;
        }
        if let Some(s) = &mut self.schedule {
            s.ran(run, now, &mut self.rng);
        }
        host.kill_timer(TIMER_IDLE);
        host.set_timer(TIMER_FRAME, FRAME_MS, 0);
        self.active = Some(Active {
            run,
            started: now,
            shown: 0,
        });
        Some(Event::Started(run))
    }

    /// Draws the pictures of `run` for `key` unless they are kept already.
    fn draw(&mut self, key: IconKey, run: Run) -> bool {
        if !self.frames.as_ref().is_some_and(|f| f.matches(&key)) {
            self.frames = Some(Frames {
                juice: key.look.juice,
                dark: key.dark_taskbar,
                px: key.px,
                slosh: Vec::new(),
                bubbles: Vec::new(),
            });
        }
        let Some(frames) = &mut self.frames else {
            return false;
        };
        let slot = frames.of(run);
        if slot.is_empty() {
            let look = IconLook {
                juice: key.look.juice,
                charging: false,
            };
            match render_run(look, run, key.dark_taskbar, key.px) {
                Ok(icons) => *slot = icons,
                Err(_) => return false,
            }
        }
        true
    }

    /// Ends the run in flight: the frame timer goes, and the mark at rest returns when `restore`.
    fn stop_run(&mut self, host: &mut dyn Host, restore: bool) {
        if self.active.take().is_some() {
            host.kill_timer(TIMER_FRAME);
            if restore {
                host.rest();
            }
        }
    }

    /// Waits for the next run with one long timer; none while a run plays or the animations are off.
    fn rearm(&mut self, host: &mut dyn Host, now: Instant) {
        host.kill_timer(TIMER_IDLE);
        if self.active.is_some() || !self.enabled {
            return;
        }
        let Some(s) = &self.schedule else { return };
        let wait = s.next().saturating_duration_since(now).as_millis();
        let ms = (wait as u32).max(50);
        host.set_timer(TIMER_IDLE, ms, (ms / 10).clamp(100, 30_000));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources};

    #[derive(Default)]
    struct Fake {
        shown: Vec<isize>,
        rests: usize,
        timers: Vec<(usize, u32, u32)>,
        killed: Vec<usize>,
        on: bool,
        refuse: bool,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                on: true,
                ..Fake::default()
            }
        }
        /// The timers that are set now.
        fn live(&self) -> Vec<usize> {
            self.timers.iter().map(|t| t.0).collect()
        }
    }

    impl Host for Fake {
        fn show(&mut self, icon: HICON) -> bool {
            self.shown.push(icon.0 as isize);
            !self.refuse
        }
        fn rest(&mut self) {
            self.rests += 1;
        }
        fn set_timer(&mut self, id: usize, ms: u32, tol: u32) {
            self.timers.retain(|t| t.0 != id);
            self.timers.push((id, ms, tol));
        }
        fn kill_timer(&mut self, id: usize) {
            self.timers.retain(|t| t.0 != id);
            self.killed.push(id);
        }
        fn animations_on(&mut self) -> bool {
            self.on
        }
    }

    fn key(juice: Juice, charging: bool) -> IconKey {
        IconKey {
            look: IconLook { juice, charging },
            dark_taskbar: true,
            px: 16,
        }
    }

    fn rng() -> Rng {
        Rng(0x2545_f491_4f6c_dd1d)
    }

    const MIN: Duration = Duration::from_secs(60);

    /// Plays a started run to its end at 20 ms steps and returns the last event.
    fn play(a: &mut Anim, h: &mut Fake, start: Instant) -> Option<Event> {
        let mut last = None;
        for ms in (0..=2200).step_by(20) {
            if let Some(e) = a.on_frame(h, start + Duration::from_millis(ms)) {
                last = Some(e);
                break;
            }
        }
        last
    }

    // ---- the schedule

    #[test]
    fn the_paces_are_about_fifteen_minutes_and_thirty_to_sixty_seconds() {
        let mut r = rng();
        let now = Instant::now();
        for _ in 0..200 {
            let s = Schedule::new(now, &mut r, None);
            let g = s.slosh - now;
            assert!((12 * MIN..=18 * MIN).contains(&g), "{g:?}");
            let b = r.between(BUBBLES_EVERY_MS);
            assert!((Duration::from_secs(30)..=Duration::from_secs(60)).contains(&b));
        }
        // And it is not a clock: the gaps differ.
        let gaps: Vec<_> = (0..20).map(|_| r.between(SLOSH_EVERY_MS)).collect();
        assert!(gaps.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn bubbles_are_scheduled_only_while_charging() {
        let mut r = rng();
        let now = Instant::now();
        let mut s = Schedule::new(now, &mut r, None);
        assert_eq!(s.bubbles, None);
        s.set_charging(true, now, &mut r);
        let b = s.bubbles.unwrap() - now;
        assert!((Duration::from_secs(30)..=Duration::from_secs(60)).contains(&b));
        assert_eq!(s.next(), now + b);
        // Charging again does not push the date out.
        let first = s.bubbles;
        s.set_charging(true, now + Duration::from_secs(5), &mut r);
        assert_eq!(s.bubbles, first);
        s.set_charging(false, now, &mut r);
        assert_eq!(s.bubbles, None);
        assert!(s.next() > now + 11 * MIN);
    }

    #[test]
    fn the_slosh_wins_a_tie_and_covers_the_bubbles_due_with_it() {
        let mut r = rng();
        let t0 = Instant::now();
        let mut s = Schedule::new(t0, &mut r, Some(Duration::from_secs(60)));
        s.set_charging(true, t0, &mut r);
        assert_eq!(s.due(t0 + Duration::from_secs(59)), None);
        let t = t0 + Duration::from_secs(60);
        assert_eq!(s.due(t), Some(Run::Slosh));
        s.ran(Run::Slosh, t, &mut r);
        assert_eq!(
            s.due(t),
            None,
            "the bubbles that were due are pushed out too"
        );
        assert_eq!(s.next(), t + Duration::from_secs(60));
        // A bubble run only moves its own date.
        let mut s = Schedule::new(t0, &mut r, None);
        s.set_charging(true, t0, &mut r);
        let at = s.bubbles.unwrap();
        assert_eq!(s.due(at), Some(Run::Bubbles));
        let slosh = s.slosh;
        s.ran(Run::Bubbles, at, &mut r);
        assert_eq!(s.slosh, slosh);
        assert!(s.bubbles.unwrap() > at);
    }

    #[test]
    fn the_picture_follows_the_clock_and_a_run_has_forty_of_them() {
        assert_eq!(frame_at(Duration::ZERO), 0);
        assert_eq!(frame_at(Duration::from_millis(49)), 0);
        assert_eq!(frame_at(Duration::from_millis(50)), 1);
        assert_eq!(frame_at(Duration::from_millis(1990)), 39);
        assert_eq!(frame_at(Duration::from_millis(2000)), RUN_FRAMES);
        // A late timer skips pictures instead of stretching the run.
        assert_eq!(frame_at(Duration::from_millis(500)), 10);
    }

    // ---- the driver

    #[test]
    fn the_first_key_plays_nothing_and_waits_for_the_first_run_with_one_timer() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        assert_eq!(a.on_key(&mut h, key(Juice::Purple, false), t0), None);
        assert_eq!(h.live(), [TIMER_IDLE], "no frame timer between runs");
        let (_, ms, tol) = h.timers[0];
        assert!((12 * 60_000..=18 * 60_000).contains(&ms), "{ms}");
        assert!(tol >= 100, "the wait may be coalesced");
        assert!(h.shown.is_empty());
    }

    #[test]
    fn a_new_juice_slosh_runs_two_seconds_with_one_frame_timer_and_ends_at_rest() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        let t = t0 + Duration::from_secs(5);
        assert_eq!(
            a.on_key(&mut h, key(Juice::Red, false), t),
            Some(Event::Started(Run::Slosh))
        );
        assert_eq!(
            h.live(),
            [TIMER_FRAME],
            "only the frame timer while it runs"
        );
        assert_eq!(h.timers[0].1, FRAME_MS);
        assert_eq!(play(&mut a, &mut h, t), Some(Event::Finished(Run::Slosh)));
        assert!(
            h.shown.len() >= RUN_FRAMES - 3,
            "{} pictures",
            h.shown.len()
        );
        assert!(
            h.shown.len() < RUN_FRAMES,
            "picture 0 is the icon already there"
        );
        assert_eq!(h.rests, 1, "the mark at rest comes back");
        assert_eq!(
            h.live(),
            [TIMER_IDLE],
            "the frame timer is gone, the long wait is back"
        );
        // The next slosh is a long way off.
        assert!(h.timers[0].1 >= 12 * 60_000 - 3000);
    }

    #[test]
    fn charging_starting_plays_the_bubbles_but_a_color_change_plays_the_slosh() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        assert_eq!(
            a.on_key(&mut h, key(Juice::Purple, true), t0),
            Some(Event::Started(Run::Bubbles))
        );
        play(&mut a, &mut h, t0);
        let t1 = t0 + Duration::from_secs(10);
        assert_eq!(
            a.on_key(&mut h, key(Juice::Green, true), t1),
            Some(Event::Started(Run::Slosh))
        );
        play(&mut a, &mut h, t1);
        // Charging stopping changes nothing to watch.
        assert_eq!(a.on_key(&mut h, key(Juice::Green, false), t1), None);
        assert_eq!(h.live(), [TIMER_IDLE]);
    }

    #[test]
    fn while_charging_the_bubbles_come_every_thirty_to_sixty_seconds() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, true), t0);
        let (_, ms, _) = h.timers[0];
        assert!((30_000..=60_000).contains(&ms), "{ms}");
        let t = t0 + Duration::from_millis(u64::from(ms));
        assert_eq!(a.on_idle(&mut h, t), Some(Event::Started(Run::Bubbles)));
        assert_eq!(play(&mut a, &mut h, t), Some(Event::Finished(Run::Bubbles)));
        let (_, ms, _) = h.timers[0];
        assert!((28_000..=60_000).contains(&ms), "the next bubbles: {ms}");
    }

    #[test]
    fn a_look_change_in_the_middle_of_a_run_stops_it_and_starts_the_new_one() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        a.on_key(&mut h, key(Juice::Red, false), t0);
        a.on_frame(&mut h, t0 + Duration::from_millis(300));
        let t1 = t0 + Duration::from_millis(400);
        assert_eq!(
            a.on_key(&mut h, key(Juice::Green, false), t1),
            Some(Event::Started(Run::Slosh))
        );
        assert_eq!(h.rests, 0, "the tray put the new mark at rest itself");
        assert_eq!(h.live(), [TIMER_FRAME]);
        assert_eq!(play(&mut a, &mut h, t1), Some(Event::Finished(Run::Slosh)));
    }

    #[test]
    fn a_new_theme_or_size_plays_nothing_and_drops_the_old_pictures() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        a.on_key(&mut h, key(Juice::Red, false), t0);
        play(&mut a, &mut h, t0);
        assert!(a.frames.is_some());
        let mut k = key(Juice::Red, false);
        k.dark_taskbar = false;
        assert_eq!(a.on_key(&mut h, k, t0), None);
        assert!(a.frames.is_none());
        assert_eq!(h.live(), [TIMER_IDLE]);
    }

    #[test]
    fn nothing_runs_while_the_system_animations_are_off() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        h.on = false;
        let t0 = Instant::now();
        assert_eq!(a.on_key(&mut h, key(Juice::Purple, false), t0), None);
        assert!(h.timers.is_empty(), "no timer at all");
        assert_eq!(
            a.on_key(&mut h, key(Juice::Red, false), t0),
            None,
            "no run for a look change"
        );
        assert!(h.timers.is_empty() && h.shown.is_empty());
        // Turned on later: the wait begins.
        h.on = true;
        assert_eq!(a.on_settings(&mut h, t0), None);
        assert_eq!(h.live(), [TIMER_IDLE]);
        // Turned off in the middle of a run: it stops, the mark rests, no timer is left.
        a.on_key(&mut h, key(Juice::Green, false), t0);
        assert_eq!(h.live(), [TIMER_FRAME]);
        h.on = false;
        assert_eq!(a.on_settings(&mut h, t0), Some(Event::Stopped));
        assert!(h.timers.is_empty());
        assert_eq!(h.rests, 1);
        // The wait that was armed when it fired is dropped too.
        assert_eq!(a.on_idle(&mut h, t0 + 20 * MIN), None);
        assert!(h.timers.is_empty());
    }

    #[test]
    fn a_refused_picture_ends_the_run_cleanly() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        a.on_key(&mut h, key(Juice::Red, false), t0);
        h.refuse = true;
        let e = a.on_frame(&mut h, t0 + Duration::from_millis(100));
        assert_eq!(e, Some(Event::Stopped));
        assert_eq!(h.live(), [TIMER_IDLE]);
        assert_eq!(h.rests, 1);
    }

    #[test]
    fn a_stray_frame_tick_with_no_run_kills_its_timer() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        assert_eq!(a.on_frame(&mut h, Instant::now()), None);
        assert_eq!(h.killed, [TIMER_FRAME]);
    }

    #[test]
    fn stopping_leaves_no_timer() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        a.on_key(&mut h, key(Juice::Red, false), t0);
        a.stop(&mut h);
        assert!(h.timers.is_empty());
    }

    #[test]
    fn the_pictures_are_drawn_once_per_look_theme_and_size() {
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let t0 = Instant::now();
        a.on_key(&mut h, key(Juice::Purple, false), t0);
        a.on_key(&mut h, key(Juice::Red, false), t0);
        play(&mut a, &mut h, t0);
        let first: Vec<isize> = a
            .frames
            .as_ref()
            .unwrap()
            .slosh
            .iter()
            .map(|i| i.0 as isize)
            .collect();
        // The periodic run of the same look uses the kept pictures.
        let t1 = t0 + 19 * MIN;
        assert_eq!(a.on_idle(&mut h, t1), Some(Event::Started(Run::Slosh)));
        play(&mut a, &mut h, t1);
        let again: Vec<isize> = a
            .frames
            .as_ref()
            .unwrap()
            .slosh
            .iter()
            .map(|i| i.0 as isize)
            .collect();
        assert_eq!(first, again, "no new drawing");
    }

    #[test]
    fn the_system_setting_can_be_read() {
        // Whatever the machine says, the call answers (on when Windows cannot say).
        let _ = animations_on();
    }

    #[test]
    fn the_test_pace_replaces_the_real_one() {
        let (mut a, mut h) = (Anim::new(Some(Duration::from_secs(60))), Fake::new());
        a.on_key(&mut h, key(Juice::Purple, false), Instant::now());
        assert_eq!(h.timers[0].1, 60_000);
    }

    #[test]
    fn a_hundred_runs_leave_no_gdi_object_behind() {
        let count = || unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
        let (mut a, mut h) = (Anim::new(None), Fake::new());
        let mut t = Instant::now();
        // Warm-up: the graphics libraries keep a few GDI objects of their own once used.
        a.on_key(&mut h, key(Juice::Purple, false), t);
        a.on_key(&mut h, key(Juice::Red, false), t);
        play(&mut a, &mut h, t);
        let before = count();
        for i in 0..100 {
            t += Duration::from_secs(5);
            // Alternate the colour so that pictures are drawn and dropped, and play both kinds.
            let juice = if i % 2 == 0 { Juice::Green } else { Juice::Red };
            assert!(a.on_key(&mut h, key(juice, false), t).is_some());
            assert_eq!(play(&mut a, &mut h, t), Some(Event::Finished(Run::Slosh)));
            t += Duration::from_secs(5);
            assert_eq!(
                a.on_key(&mut h, key(juice, true), t),
                Some(Event::Started(Run::Bubbles))
            );
            assert_eq!(play(&mut a, &mut h, t), Some(Event::Finished(Run::Bubbles)));
            a.on_key(&mut h, key(juice, false), t);
        }
        drop(a);
        let after = count();
        // Other tests run beside this one and may hold a few objects; one leaked picture run
        // would be 80 objects, a hundred of them thousands.
        assert!(after <= before + 8, "GDI objects {before} -> {after}");
        assert!(h.timers.is_empty() || h.live() == [TIMER_IDLE]);
    }
}
