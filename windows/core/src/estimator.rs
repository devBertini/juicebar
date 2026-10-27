// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The estimator: a port of `spec/reference/estimator.py` (spec/estimator.md). Same names, same arithmetic order.
//!
//! Time is integer seconds since the epoch (UTC); local time is `t + utc_offset` and day, hour and weekday come from
//! Euclidean division. Every text rounds with `floor(x + 0.5)`. Texts go through `strings`, so the language is an
//! argument; pt-BR output equals the reference's text.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use crate::store::State;
use crate::strings::{self, Lang, id};
use crate::types::{BatteryReading, Charging, Coarse, Quality};

// ---------------------------------------------------------------- constants

pub const HOUR: i64 = 3600;
pub const DAY: i64 = 86400;
pub const WEEK_HOURS: usize = 168;

pub const HALF_LIFE_USE_HOURS: f64 = 6.0;
pub const MIN_DROP_POINTS: f64 = 2.0;
pub const MIN_OBSERVATION_SECONDS: i64 = 900;
pub const PRIOR_WEIGHT: f64 = 3.0;
pub const WEIGHT_CAP: f64 = 12.0;
pub const LABEL_WEIGHT: f64 = 3.0;
pub const DEFAULT_RATED_USE_HOURS: f64 = 20.0;
pub const DEFAULT_RATED_CHARGE_HOURS: f64 = 2.5;
pub const CHARGE_PRIOR_POINTS: f64 = 120.0;
pub const FAST_CHARGE_LIMIT: f64 = 80.0;
pub const PROFILE_LEARN_SECONDS: i64 = 7 * DAY;
pub const PROFILE_WINDOW_SECONDS: i64 = 28 * DAY;
pub const FORECAST_LIMIT_SECONDS: i64 = 7 * DAY;
pub const TYPICAL_DAY_MIN_USE: f64 = 0.2;
pub const SMART_MARGIN: f64 = 1.1;
pub const LOW_PERCENT: f64 = 10.0;
pub const CYCLE_MIN_POINTS: f64 = 50.0;
pub const HEALTH_CYCLES: usize = 3;
pub const USAGE_KEEP_DAYS: i64 = 365;

// ---------------------------------------------------------------- inputs

/// A battery reading. `percent` is `None` for a coarse-only reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub at: i64,
    pub percent: Option<f64>,
    pub charging: Charging,
    pub quality: Quality,
    /// The coarse level a coarse-only device reports (shown, never forecast).
    pub coarse: Option<Coarse>,
}

impl Reading {
    pub fn new(at: i64, percent: Option<f64>, charging: Charging, quality: Quality) -> Reading {
        Reading {
            at,
            percent,
            charging,
            quality,
            coarse: None,
        }
    }

    /// From what a provider reports; a derived percent is clamped to 0..=100.
    pub fn from_battery(b: &BatteryReading) -> Reading {
        Reading {
            at: b.at.floor() as i64,
            percent: b.percent.map(|p| p.clamp(0.0, 100.0)),
            charging: b.charging,
            quality: b.quality,
            coarse: b.coarse,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presence {
    pub at: i64,
    pub online: bool,
}

impl From<crate::types::Presence> for Presence {
    fn from(p: crate::types::Presence) -> Presence {
        Presence {
            at: p.at.floor() as i64,
            online: p.online,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Full,
    Low,
    Smart,
}

impl NoticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NoticeKind::Full => "full",
            NoticeKind::Low => "low",
            NoticeKind::Smart => "smart",
        }
    }
}

/// A notification decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub at: i64,
    pub text: Option<String>,
}

/// A finished discharge cycle: hours of use per full battery, and the RGB economy setting it ran under.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cycle {
    pub autonomy: f64,
    pub rgb: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    Discharge,
    Charge,
}

// ---------------------------------------------------------------- model priors (spec/devices.tsv)

const DEVICES: &str = include_str!("../../../spec/devices.tsv");

/// The rated figures of one catalog row (Logitech's own numbers); `None` where the catalog has `-`.
#[derive(Clone, Debug, PartialEq)]
pub struct Priors {
    pub provider: &'static str,
    pub match_kind: &'static str,
    pub id: u16,
    pub model: &'static str,
    pub use_h: Option<f64>,
    pub use_h_rgb: Option<f64>,
    pub charge_h: Option<f64>,
}

impl Priors {
    /// Rated hours of use while the RGB economy is on (`use_h`; the other figure when it is the only one),
    /// 20 h when unknown.
    pub fn rated_use_hours(&self) -> f64 {
        self.use_h
            .or(self.use_h_rgb)
            .unwrap_or(DEFAULT_RATED_USE_HOURS)
    }

    /// Rated hours of use while the RGB economy is off (`use_h_rgb`), `None` when the catalog has one figure
    /// at most, which then serves both settings.
    pub fn rated_use_hours_rgb(&self) -> Option<f64> {
        self.use_h_rgb.filter(|_| self.use_h.is_some())
    }

    /// Rated hours to charge, 2.5 h when unknown.
    pub fn rated_charge_hours(&self) -> f64 {
        self.charge_h.unwrap_or(DEFAULT_RATED_CHARGE_HOURS)
    }
}

fn catalog() -> &'static [Priors] {
    static CELL: OnceLock<Vec<Priors>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut rows = Vec::new();
        let mut header_seen = false;
        for line in DEVICES.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            if !header_seen {
                header_seen = true;
                continue;
            }
            let c: Vec<&'static str> = line.split('\t').collect();
            if c.len() < 10 {
                continue;
            }
            let Ok(id) = u16::from_str_radix(c[2], 16) else {
                continue;
            };
            let num = |s: &str| s.parse::<f64>().ok();
            rows.push(Priors {
                provider: c[0],
                match_kind: c[1],
                id,
                model: c[3],
                use_h: num(c[7]),
                use_h_rgb: num(c[8]),
                charge_h: num(c[9]),
            });
        }
        rows
    })
}

/// The catalog row of a provider's `match` kind (`receiver`, `dongle`, `wpid`, `usb`) and id.
pub fn priors(provider: &str, match_kind: &str, id: u16) -> Option<&'static Priors> {
    catalog()
        .iter()
        .find(|p| p.provider == provider && p.match_kind == match_kind && p.id == id)
}

/// The first catalog row whose model name equals `model` (case-insensitive) and that has a rated use.
pub fn priors_for_model(model: &str) -> Option<&'static Priors> {
    catalog()
        .iter()
        .find(|p| p.use_h.is_some() && p.model.eq_ignore_ascii_case(model))
}

// ---------------------------------------------------------------- time helpers

/// Rounding used by every text: `floor(x + 0.5)`, never banker's rounding.
pub fn round_half_up(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// 0 = Monday. Day 0 (1970-01-01) was a Thursday.
pub fn weekday_of_day(day: i64) -> i64 {
    (day + 3).rem_euclid(7)
}

pub fn hour_of_week(local: i64) -> usize {
    (weekday_of_day(local.div_euclid(DAY)) * 24 + local.rem_euclid(DAY) / HOUR) as usize
}

/// `YYYY-MM-DD` of a day count since 1970-01-01.
pub fn date_string(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `HH:MM` of a local time in seconds.
pub fn clock_string(local: i64) -> String {
    format!(
        "{:02}:{:02}",
        local.rem_euclid(DAY) / HOUR,
        local.rem_euclid(HOUR) / 60
    )
}

/// u(hour of week) before 7 days of data: 0.8 from 9 h to 19 h on weekdays, 0.1 otherwise.
pub fn default_profile() -> Vec<f64> {
    let mut out = Vec::with_capacity(WEEK_HOURS);
    for weekday in 0..7 {
        for hour in 0..24 {
            out.push(if weekday < 5 && (9..19).contains(&hour) {
                0.8
            } else {
                0.1
            });
        }
    }
    out
}

// ---------------------------------------------------------------- device history

/// Everything the estimator knows about one device, updated by events in chronological order.
/// Fields are public so the store can persist and restore them; change them only through the methods.
#[derive(Clone, Debug)]
pub struct DeviceHistory {
    pub name: String,
    /// Rated hours of use while the RGB economy is on.
    pub rated_use_hours: f64,
    /// Rated hours of use while the RGB economy is off; `None`: `rated_use_hours` serves both settings.
    pub rated_use_hours_rgb: Option<f64>,
    pub rated_charge_hours: f64,
    pub utc_offset: i64,
    /// The language of the texts of notices (the text functions take their own).
    pub lang: Lang,

    pub first_at: Option<i64>,
    pub last: Option<Reading>,
    pub presence: Vec<Presence>,
    pub rgb: bool,

    pub r: Option<f64>,
    pub w: f64,
    pub anchor: Option<(f64, i64)>,
    pub rc: Option<f64>,
    pub wc: f64,
    pub c_anchor: Option<(f64, i64)>,

    pub segment: Option<Segment>,
    pub cycle_points: f64,
    pub cycle_hours: f64,
    pub cycle_rgb: bool,
    pub cycles: Vec<Cycle>,

    pub low_sent: bool,
    pub full_sent: bool,
    pub smart_day: Option<i64>,
    pub notices: Vec<Notice>,
}

impl DeviceHistory {
    pub fn new(
        name: &str,
        rated_use_hours: f64,
        rated_charge_hours: f64,
        utc_offset: i64,
    ) -> DeviceHistory {
        DeviceHistory {
            name: name.to_string(),
            rated_use_hours,
            rated_use_hours_rgb: None,
            rated_charge_hours,
            utc_offset,
            lang: Lang::En,
            first_at: None,
            last: None,
            presence: Vec::new(),
            rgb: false,
            r: None,
            w: 0.0,
            anchor: None,
            rc: None,
            wc: 0.0,
            c_anchor: None,
            segment: None,
            cycle_points: 0.0,
            cycle_hours: 0.0,
            cycle_rgb: false,
            cycles: Vec::new(),
            low_sent: false,
            full_sent: false,
            smart_day: None,
            notices: Vec::new(),
        }
    }

    /// A history with the catalog's priors for a model (20 h and 2.5 h when it has none).
    pub fn for_model(name: &str, priors: Option<&Priors>, utc_offset: i64) -> DeviceHistory {
        let (u, c) = priors.map_or((DEFAULT_RATED_USE_HOURS, DEFAULT_RATED_CHARGE_HOURS), |p| {
            (p.rated_use_hours(), p.rated_charge_hours())
        });
        let mut h = DeviceHistory::new(name, u, c, utc_offset);
        h.rated_use_hours_rgb = priors.and_then(Priors::rated_use_hours_rgb);
        h
    }

    // -- state

    pub fn online(&self) -> bool {
        self.presence.last().is_some_and(|p| p.online)
    }

    /// The last reading's percent; `None` when there is none or it is coarse-only.
    pub fn level(&self) -> Option<f64> {
        match &self.last {
            Some(l) if l.quality != Quality::Coarse => l.percent,
            _ => None,
        }
    }

    // -- events

    pub fn add_presence(&mut self, p: Presence) -> Vec<Notice> {
        if self.first_at.is_none() {
            self.first_at = Some(p.at);
        }
        self.presence.push(p);
        let mut out = Vec::new();
        if p.online {
            self.smart(p.at, &mut out);
        }
        out
    }

    /// A change of the RGB economy switch starts a new discharge anchor.
    pub fn set_rgb(&mut self, _at: i64, on: bool) {
        if on != self.rgb {
            self.rgb = on;
            self.anchor = None;
        }
    }

    pub fn add_reading(&mut self, r: Reading) -> Vec<Notice> {
        if self.first_at.is_none() {
            self.first_at = Some(r.at);
        }
        let kind = match r.charging {
            Charging::Charging | Charging::Full => Some(Segment::Charge),
            Charging::Discharging => Some(Segment::Discharge),
            Charging::Unknown => None,
        };
        let measurable = r.percent.is_some() && r.quality != Quality::Coarse;

        if let Some(k) = kind
            && self.segment != Some(k)
        {
            self.end_segment();
            self.start_segment(k);
        }
        self.last = Some(r);

        if measurable && kind == Some(Segment::Discharge) {
            self.observe_discharge(&r);
        } else if measurable && kind == Some(Segment::Charge) {
            self.observe_charge(&r);
        }

        let mut out = Vec::new();
        if kind == Some(Segment::Charge)
            && !self.full_sent
            && (r.charging == Charging::Full || r.percent.is_some_and(|p| p >= 100.0))
        {
            self.full_sent = true;
            out.push(Notice {
                kind: NoticeKind::Full,
                at: r.at,
                text: None,
            });
        }
        if kind == Some(Segment::Discharge)
            && measurable
            && r.percent.is_some_and(|p| p < LOW_PERCENT)
            && !self.low_sent
        {
            self.low_sent = true;
            out.push(Notice {
                kind: NoticeKind::Low,
                at: r.at,
                text: low_warning(self, self.lang),
            });
        }
        self.smart(r.at, &mut out);
        self.notices.extend(out.iter().cloned());
        out
    }

    /// Smart warning gate: at most once per local calendar day.
    fn smart(&mut self, now: i64, out: &mut Vec<Notice>) {
        let text = smart_warning(self, now, self.lang);
        let day = (now + self.utc_offset).div_euclid(DAY);
        if text.is_some() && self.smart_day != Some(day) {
            self.smart_day = Some(day);
            out.push(Notice {
                kind: NoticeKind::Smart,
                at: now,
                text,
            });
        }
    }

    // -- segments and cycles

    fn end_segment(&mut self) {
        if self.segment == Some(Segment::Discharge) && self.cycle_points >= CYCLE_MIN_POINTS {
            let autonomy = 100.0 * self.cycle_hours / self.cycle_points;
            self.cycles.push(Cycle {
                autonomy,
                rgb: self.cycle_rgb,
            });
        }
    }

    fn start_segment(&mut self, kind: Segment) {
        self.segment = Some(kind);
        match kind {
            Segment::Discharge => {
                self.anchor = None;
                self.low_sent = false;
                self.cycle_points = 0.0;
                self.cycle_hours = 0.0;
                self.cycle_rgb = self.rgb;
            }
            Segment::Charge => {
                self.c_anchor = None;
                self.full_sent = false;
            }
        }
    }

    fn observe_discharge(&mut self, r: &Reading) {
        let Some(pct) = r.percent else { return };
        let Some((a_pct, a_at)) = self.anchor else {
            self.anchor = Some((pct, r.at));
            return;
        };
        if pct > a_pct {
            // the level rose with no charge reading: a missed charge, start over
            self.anchor = Some((pct, r.at));
            return;
        }
        let drop = a_pct - pct;
        let secs = self.online_seconds(a_at, r.at);
        if drop >= MIN_DROP_POINTS && secs >= MIN_OBSERVATION_SECONDS {
            let hours = secs as f64 / 3600.0;
            let (rate, weight) = blend(self.r, self.w, drop / hours, hours);
            self.r = Some(rate);
            self.w = weight;
            self.cycle_points += drop;
            self.cycle_hours += hours;
            self.anchor = Some((pct, r.at));
        }
    }

    fn observe_charge(&mut self, r: &Reading) {
        let Some(pct) = r.percent else { return };
        let Some((a_pct, a_at)) = self.c_anchor else {
            self.c_anchor = Some((pct, r.at));
            return;
        };
        if pct < a_pct {
            self.c_anchor = Some((pct, r.at));
            return;
        }
        let rise = pct - a_pct;
        let secs = r.at - a_at;
        if rise >= MIN_DROP_POINTS && secs >= MIN_OBSERVATION_SECONDS {
            let hours = secs as f64 / 3600.0;
            // points above 80 % cost twice the time, so they count double: the learned rate is the fast-phase rate
            let eff = (pct.min(FAST_CHARGE_LIMIT) - a_pct.min(FAST_CHARGE_LIMIT))
                + 2.0 * (pct.max(FAST_CHARGE_LIMIT) - a_pct.max(FAST_CHARGE_LIMIT));
            let (rate, weight) = blend(self.rc, self.wc, eff / hours, hours);
            self.rc = Some(rate);
            self.wc = weight;
            self.c_anchor = Some((pct, r.at));
        }
    }

    // -- presence queries

    /// Online intervals `(start, end)` in UTC seconds, an open one closed at `upto`.
    pub fn intervals(&self, upto: i64) -> Vec<(i64, i64)> {
        intervals_of(&self.presence, upto)
    }

    fn online_seconds(&self, a: i64, b: i64) -> i64 {
        self.intervals(b)
            .iter()
            .map(|&(s, e)| (e.min(b) - s.max(a)).max(0))
            .sum()
    }

    // -- the hour-of-week profile u(how)

    pub fn profile_learned(&self, now: i64) -> bool {
        self.first_at
            .is_some_and(|f| now - f >= PROFILE_LEARN_SECONDS)
    }

    /// u for each of the 168 hours of the week (index weekday * 24 + hour, Monday first). Before 7 days of data the
    /// default; after, the mean over the complete local hours of the last 28 days that start after the first data
    /// of the share of each hour the device was online; an hour of the week with no complete occurrence keeps its
    /// default value.
    pub fn profile(&self, now: i64) -> Vec<f64> {
        let default = default_profile();
        let Some(first_at) = self.first_at else {
            return default;
        };
        if !self.profile_learned(now) {
            return default;
        }
        let off = self.utc_offset;
        let lo = (now - PROFILE_WINDOW_SECONDS).max(first_at) + off;
        let first_slot = -((-lo).div_euclid(HOUR)) * HOUR; // ceil to a local hour
        let last_end = (now + off).div_euclid(HOUR) * HOUR; // floor to a local hour
        let mut online: HashMap<i64, i64> = HashMap::new();
        for (s, e) in self.intervals(now) {
            let mut t = (s + off).max(first_slot);
            let end = (e + off).min(last_end);
            while t < end {
                let slot = t.div_euclid(HOUR) * HOUR;
                let stop = end.min(slot + HOUR);
                *online.entry(slot).or_insert(0) += stop - t;
                t = stop;
            }
        }
        let mut sums = [0.0_f64; WEEK_HOURS];
        let mut counts = [0_u32; WEEK_HOURS];
        let mut slot = first_slot;
        while slot + HOUR <= last_end {
            let how = hour_of_week(slot);
            sums[how] += online.get(&slot).copied().unwrap_or(0) as f64 / 3600.0;
            counts[how] += 1;
            slot += HOUR;
        }
        (0..WEEK_HOURS)
            .map(|i| {
                if counts[i] > 0 {
                    sums[i] / f64::from(counts[i])
                } else {
                    default[i]
                }
            })
            .collect()
    }

    // -- persistence (spec/estimator.md, "Storage and cost")

    /// The numbers worth keeping between runs, as the store writes them.
    pub fn state(&self) -> State {
        State {
            first_at: self.first_at,
            r: self.r,
            w: self.w,
            rc: self.rc,
            wc: self.wc,
            segment: self.segment,
            cycle_points: self.cycle_points,
            cycle_hours: self.cycle_hours,
            cycle_rgb: self.cycle_rgb,
            low_sent: self.low_sent,
            full_sent: self.full_sent,
            smart_day: self.smart_day,
        }
    }

    /// Puts a saved state back. Anchors are not kept: the next reading of a segment starts a new one.
    pub fn restore_state(&mut self, s: &State) {
        self.first_at = s.first_at.or(self.first_at);
        self.r = s.r;
        self.w = s.w;
        self.rc = s.rc;
        self.wc = s.wc;
        self.segment = s.segment;
        self.cycle_points = s.cycle_points;
        self.cycle_hours = s.cycle_hours;
        self.cycle_rgb = s.cycle_rgb;
        self.low_sent = s.low_sent;
        self.full_sent = s.full_sent;
        self.smart_day = s.smart_day;
        self.anchor = None;
        self.c_anchor = None;
    }
}

/// Online intervals `(start, end)` of a presence list, an open one closed at `upto`.
pub fn intervals_of(presence: &[Presence], upto: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut start: Option<i64> = None;
    for p in presence {
        if p.at > upto {
            break;
        }
        if p.online && start.is_none() {
            start = Some(p.at);
        } else if !p.online
            && let Some(s) = start.take()
        {
            out.push((s, p.at));
        }
    }
    if let Some(s) = start {
        out.push((s, upto));
    }
    out
}

/// Adds the online seconds of `intervals` to the total of each local calendar day (day number since the epoch).
pub fn add_daily_totals(totals: &mut BTreeMap<i64, i64>, intervals: &[(i64, i64)], off: i64) {
    for &(s, e) in intervals {
        let mut t = s + off;
        let end = e + off;
        while t < end {
            let day = t.div_euclid(DAY);
            let stop = end.min((day + 1) * DAY);
            *totals.entry(day).or_insert(0) += stop - t;
            t = stop;
        }
    }
}

/// One observation `r` over `hours`: the first sets the mean, then `R = R + a (r - R)`.
fn blend(rate: Option<f64>, weight: f64, r: f64, hours: f64) -> (f64, f64) {
    let rate = match rate {
        None => r,
        Some(prev) => {
            let a = 1.0 - 2.0_f64.powf(-hours / HALF_LIFE_USE_HOURS);
            prev + a * (r - prev)
        }
    };
    (rate, WEIGHT_CAP.min(weight + hours))
}

// ---------------------------------------------------------------- rates

/// Percent per hour of use: `(w R + 3 P) / (w + 3)`, `P = 100 / rated hours of use`.
pub fn discharge_rate(h: &DeviceHistory) -> f64 {
    let rated = match h.rated_use_hours_rgb {
        Some(rgb_off) if !h.rgb => rgb_off,
        _ => h.rated_use_hours,
    };
    let prior = 100.0 / rated;
    match h.r {
        None => prior,
        Some(r) => (h.w * r + PRIOR_WEIGHT * prior) / (h.w + PRIOR_WEIGHT),
    }
}

/// Fast-phase percent per wall-clock hour: the same blend, `P = 120 / rated hours to charge`.
pub fn charge_rate(h: &DeviceHistory) -> f64 {
    let prior = CHARGE_PRIOR_POINTS / h.rated_charge_hours;
    match h.rc {
        None => prior,
        Some(rc) => (h.wc * rc + PRIOR_WEIGHT * prior) / (h.wc + PRIOR_WEIGHT),
    }
}

// ---------------------------------------------------------------- forecast to empty

fn discharge_forecast_ok(h: &DeviceHistory) -> bool {
    h.online()
        && h.level().is_some()
        && h.last
            .is_some_and(|l| !matches!(l.charging, Charging::Charging | Charging::Full))
}

/// Hours of use left: `level / rate`. `None` when offline, charging, coarse-only or unread.
pub fn hours_left(h: &DeviceHistory) -> Option<f64> {
    if !discharge_forecast_ok(h) {
        return None;
    }
    Some(h.level()? / discharge_rate(h))
}

pub fn forecast_label(h: &DeviceHistory, lang: Lang) -> Option<String> {
    if hours_left(h).is_some() && h.w < LABEL_WEIGHT {
        return Some(strings::get(lang, id::LABEL_INITIAL).to_string());
    }
    None
}

/// The moment (epoch seconds) at which `use_hours` of expected use have been spent, walking forward from `start`
/// one local hour at a time; `None` past the limit.
pub fn walk_use(
    profile: &[f64],
    off: i64,
    start: i64,
    use_hours: f64,
    limit_seconds: i64,
) -> Option<f64> {
    let mut remaining = use_hours;
    let mut t = start;
    let end = start + limit_seconds;
    loop {
        if remaining <= 0.0 {
            return Some(t as f64);
        }
        if t >= end {
            return None;
        }
        let local = t + off;
        let d = (local.div_euclid(HOUR) * HOUR + HOUR - local).min(end - t);
        let u = profile[hour_of_week(local)];
        let avail = u * d as f64 / 3600.0;
        if u > 0.0 && remaining <= avail {
            return Some(t as f64 + remaining / u * 3600.0);
        }
        remaining -= avail;
        t += d;
    }
}

/// Epoch seconds at which the device is expected to be empty; `None` past 7 days.
pub fn eta(h: &DeviceHistory, now: i64) -> Option<f64> {
    let hl = hours_left(h)?;
    walk_use(
        &h.profile(now),
        h.utc_offset,
        now,
        hl,
        FORECAST_LIMIT_SECONDS,
    )
}

/// "~40 min" (under 1 h, nearest 10), "~2,5 h" (under 10 h, nearest half), "~18 h" (under 72 h, nearest hour),
/// then "~3 dias".
pub fn format_hours_left(hours: f64, lang: Lang) -> String {
    if hours < 1.0 {
        let minutes = 10.max(round_half_up(hours * 6.0) * 10);
        if minutes < 60 {
            return strings::fmt_n(lang, id::HOURS_MIN, &[minutes]);
        }
        return strings::get(lang, id::HOURS_ONE_HOUR).to_string();
    }
    if hours < 10.0 {
        let halves = round_half_up(hours * 2.0);
        let (whole, half) = (halves.div_euclid(2), halves.rem_euclid(2));
        return strings::fmt_n(
            lang,
            if half == 1 {
                id::HOURS_HALF
            } else {
                id::HOURS_WHOLE
            },
            &[whole],
        );
    }
    if hours < 72.0 {
        return strings::fmt_n(lang, id::HOURS_WHOLE, &[round_half_up(hours)]);
    }
    days_text(hours, lang)
}

fn days_text(hours: f64, lang: Lang) -> String {
    let n = 1.max(round_half_up(hours / 24.0));
    if n == 1 {
        strings::get(lang, id::DAYS_ONE).to_string()
    } else {
        strings::fmt_n(lang, id::DAYS_MANY, &[n])
    }
}

fn of_use(text: &str, lang: Lang) -> String {
    strings::fmt(lang, id::OF_USE, &[text])
}

pub fn forecast_text(h: &DeviceHistory, lang: Lang) -> Option<String> {
    let hl = hours_left(h)?;
    Some(of_use(&format_hours_left(hl, lang), lang))
}

/// "acaba hoje ~17:40", "acaba amanhã ~10:20", "acaba qua ~20:00" (the weekday from the day after tomorrow; time
/// rounded to 10 minutes), or past 7 days "~<n> dias de uso" alone.
pub fn eta_text(h: &DeviceHistory, now: i64, lang: Lang) -> Option<String> {
    let hl = hours_left(h)?;
    let Some(t) = eta(h, now) else {
        return Some(of_use(&days_text(hl, lang), lang));
    };
    let rounded = round_half_up(t / 600.0) * 600;
    let local = rounded + h.utc_offset;
    let day = local.div_euclid(DAY);
    let ahead = day - (now + h.utc_offset).div_euclid(DAY);
    let clock = clock_string(local);
    Some(match ahead {
        0 => strings::fmt(lang, id::ETA_TODAY, &[&clock]),
        1 => strings::fmt(lang, id::ETA_TOMORROW, &[&clock]),
        _ => {
            let wd = format!("wd_{}", weekday_of_day(day));
            strings::fmt(lang, id::ETA_WEEKDAY, &[strings::get(lang, &wd), &clock])
        }
    })
}

// ---------------------------------------------------------------- time to full

/// Wall-clock hours to 100 %: up to 80 % at the charge rate, 80 % to 100 % at half of it. `None` unless online and
/// charging with a known level below 100.
pub fn time_to_full(h: &DeviceHistory) -> Option<f64> {
    let last = h.last.as_ref()?;
    let level = h.level()?;
    if !(h.online() && last.charging == Charging::Charging && level < 100.0) {
        return None;
    }
    let rc = charge_rate(h);
    Some(if level < FAST_CHARGE_LIMIT {
        (FAST_CHARGE_LIMIT - level) / rc + (100.0 - FAST_CHARGE_LIMIT) / (rc / 2.0)
    } else {
        (100.0 - level) / (rc / 2.0)
    })
}

/// "cheio em ~1 h 10 min": nearest 10 minutes.
pub fn full_in_text(h: &DeviceHistory, lang: Lang) -> Option<String> {
    let hours = time_to_full(h)?;
    let minutes = 10.max(round_half_up(hours * 6.0) * 10);
    if minutes < 60 {
        return Some(strings::fmt_n(lang, id::FULL_IN_MIN, &[minutes]));
    }
    let (whole, rest) = (minutes.div_euclid(60), minutes.rem_euclid(60));
    if rest == 0 {
        return Some(strings::fmt_n(lang, id::FULL_IN_H, &[whole]));
    }
    Some(strings::fmt_n(lang, id::FULL_IN_HMIN, &[whole, rest]))
}

/// "cheio às 15:42": local clock time, nearest minute.
pub fn full_at_text(h: &DeviceHistory, now: i64, lang: Lang) -> Option<String> {
    let hours = time_to_full(h)?;
    let t = round_half_up((now as f64 + hours * 3600.0) / 60.0) * 60;
    Some(strings::fmt(
        lang,
        id::FULL_AT,
        &[&clock_string(t + h.utc_offset)],
    ))
}

/// True when the last reading says full, or 100 % while charging (the history sends the notice once per segment).
pub fn full_notice(h: &DeviceHistory) -> bool {
    match &h.last {
        Some(r) if matches!(r.charging, Charging::Charging | Charging::Full) => {
            r.charging == Charging::Full || r.percent.is_some_and(|p| p >= 100.0)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- usage per day

/// Online seconds inside each local calendar day (day number since the epoch), the last 365 days, days with no
/// online time left out, in order.
pub fn usage_by_day(h: &DeviceHistory, now: i64) -> Vec<(i64, i64)> {
    let off = h.utc_offset;
    let mut totals: BTreeMap<i64, i64> = BTreeMap::new();
    add_daily_totals(&mut totals, &h.intervals(now), off);
    let today = (now + off).div_euclid(DAY);
    totals
        .into_iter()
        .filter(|&(d, secs)| today - d < USAGE_KEEP_DAYS && secs > 0)
        .collect()
}

/// "50 min", "4 h", "4 h 10 min", "6 h 05 min": nearest minute.
pub fn usage_text(seconds: i64, lang: Lang) -> String {
    let minutes = round_half_up(seconds as f64 / 60.0);
    if minutes < 60 {
        return strings::fmt_n(lang, id::USAGE_MIN, &[minutes]);
    }
    let (whole, rest) = (minutes.div_euclid(60), minutes.rem_euclid(60));
    if rest == 0 {
        return strings::fmt_n(lang, id::USAGE_H, &[whole]);
    }
    strings::fmt(
        lang,
        id::USAGE_HMIN,
        &[&whole.to_string(), &format!("{rest:02}")],
    )
}

/// Seconds of use today (local day of `now`).
pub fn usage_today_seconds(h: &DeviceHistory, now: i64) -> i64 {
    let today = (now + h.utc_offset).div_euclid(DAY);
    usage_by_day(h, now)
        .into_iter()
        .find(|&(d, _)| d == today)
        .map_or(0, |(_, s)| s)
}

pub fn usage_today_text(h: &DeviceHistory, now: i64, lang: Lang) -> String {
    strings::fmt(
        lang,
        id::USAGE_TODAY,
        &[&usage_text(usage_today_seconds(h, now), lang)],
    )
}

// ---------------------------------------------------------------- battery health

/// Percent: median autonomy of the last 3 cycles over the median of the first 3 cycles under the same RGB setting
/// (the setting of the latest cycle), capped at 100. `None` ("aprendendo") until that setting has 3 cycles.
pub fn health(h: &DeviceHistory) -> Option<f64> {
    health_of(&h.cycles)
}

/// `health` over a list of cycles (the store keeps all of them).
pub fn health_of(cycles: &[Cycle]) -> Option<f64> {
    let setting = cycles.last()?.rgb;
    let same: Vec<f64> = cycles
        .iter()
        .filter(|c| c.rgb == setting)
        .map(|c| c.autonomy)
        .collect();
    if same.len() < HEALTH_CYCLES {
        return None;
    }
    let median = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        s[HEALTH_CYCLES / 2]
    };
    let first = median(&same[..HEALTH_CYCLES]);
    let last = median(&same[same.len() - HEALTH_CYCLES..]);
    Some((last / first * 100.0).min(100.0))
}

pub fn health_text(h: &DeviceHistory, lang: Lang) -> String {
    match health(h) {
        None => strings::get(lang, id::HEALTH_LEARNING).to_string(),
        Some(v) => strings::fmt_n(lang, id::HEALTH_PERCENT, &[round_half_up(v)]),
    }
}

// ---------------------------------------------------------------- warnings

/// Sum of u(hour) from now to the end of the typical day (the end of the last local hour of today with
/// `u >= 0.2`); 0 when there is none or it has passed.
pub fn expected_use_left_today(h: &DeviceHistory, now: i64) -> f64 {
    let profile = h.profile(now);
    let local = now + h.utc_offset;
    let day_start = local.div_euclid(DAY) * DAY;
    let mut last_hour = None;
    for hour in 0..24 {
        if profile[hour_of_week(day_start + hour * HOUR)] >= TYPICAL_DAY_MIN_USE {
            last_hour = Some(hour);
        }
    }
    let Some(last_hour) = last_hour else {
        return 0.0;
    };
    let end = day_start + (last_hour + 1) * HOUR;
    let mut total = 0.0;
    let mut t = local;
    while t < end {
        let stop = end.min(t.div_euclid(HOUR) * HOUR + HOUR);
        total += profile[hour_of_week(t)] * (stop - t) as f64 / 3600.0;
        t = stop;
    }
    total
}

/// The text of the smart warning if the condition holds at `now` (online, discharging, hours left below 1.1 times
/// the expected use left today), else `None`. The once-per-day gate lives in `DeviceHistory`.
pub fn smart_warning(h: &DeviceHistory, now: i64, lang: Lang) -> Option<String> {
    let hl = hours_left(h)?;
    if hl >= expected_use_left_today(h, now) * SMART_MARGIN {
        return None;
    }
    let to_ten = ((h.level()? - LOW_PERCENT) / discharge_rate(h)).max(0.0);
    let moment = walk_use(
        &h.profile(now),
        h.utc_offset,
        now,
        to_ten,
        FORECAST_LIMIT_SECONDS,
    )
    .unwrap_or(now as f64);
    let hour = (round_half_up(moment) + h.utc_offset).rem_euclid(DAY) / HOUR;
    Some(strings::fmt_n(lang, id::SMART_WARNING, &[hour]))
}

/// "<device> com <n>%" when the level of a discharging device is below 10 %.
pub fn low_warning(h: &DeviceHistory, lang: Lang) -> Option<String> {
    let last = h.last.as_ref()?;
    if h.segment != Some(Segment::Discharge) || last.charging != Charging::Discharging {
        return None;
    }
    let level = h.level()?;
    if level >= LOW_PERCENT {
        return None;
    }
    Some(strings::fmt(
        lang,
        id::LOW_WARNING,
        &[&h.name, &round_half_up(level).to_string()],
    ))
}
