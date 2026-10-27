// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! A device's history as text lines (spec/estimator.md, "Storage and cost"; spec/ARCHITECTURE.md, module `store`).
//!
//! One event per line, a versioned header first. The shell does the file I/O; the core produces and reads the lines:
//!
//! ```text
//! juicebar-history 1
//! P <at> <0|1>                                  presence (online or offline)
//! G <at> <0|1>                                  RGB economy switch
//! R <at> <percent|-> <d|c|f|u> <e|d|c> <-|f|g|l|c>   reading: state, quality, coarse level
//! U <day> <seconds>                             online seconds of a local day (days older than the presence window)
//! C <autonomy> <0|1>                            a finished discharge cycle: hours per full battery, RGB setting
//! S ...                                         the estimator's state (the last one wins)
//! ```
//!
//! A line that does not parse is skipped and counted (a write cut short at the end of the file must not lose the
//! rest). A different header version is an error: the shell then keeps the file and starts a fresh history.

use std::collections::BTreeMap;

use crate::estimator::{
    self, Cycle, DAY, DeviceHistory, Notice, Presence, Priors, Reading, Segment, add_daily_totals,
    intervals_of,
};
use crate::strings::Lang;
use crate::types::{Charging, Coarse, Quality};

pub const FORMAT_VERSION: u32 = 1;
pub const HEADER: &str = "juicebar-history 1";

/// Readings and presence moments are kept this long.
pub const KEEP_READINGS_DAYS: i64 = 30;
/// Daily usage is kept this long (`estimator::USAGE_KEEP_DAYS`).
pub const KEEP_USAGE_DAYS: i64 = estimator::USAGE_KEEP_DAYS;
/// Writes are batched: at most one every this many seconds, and on quit.
pub const BATCH_SECONDS: f64 = 300.0;

// ---------------------------------------------------------------- the lines

/// The estimator's numbers kept between runs (anchors are not kept: the next reading starts a new one).
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub first_at: Option<i64>,
    pub r: Option<f64>,
    pub w: f64,
    pub rc: Option<f64>,
    pub wc: f64,
    pub segment: Option<Segment>,
    pub cycle_points: f64,
    pub cycle_hours: f64,
    pub cycle_rgb: bool,
    pub low_sent: bool,
    pub full_sent: bool,
    pub smart_day: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Reading(Reading),
    Presence(Presence),
    Rgb { at: i64, on: bool },
    Usage { day: i64, seconds: i64 },
    Cycle(Cycle),
    State(State),
}

fn bit(b: bool) -> char {
    if b { '1' } else { '0' }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_string(), |x| x.to_string())
}

impl Line {
    pub fn encode(&self) -> String {
        match self {
            Line::Reading(r) => {
                let state = match r.charging {
                    Charging::Discharging => 'd',
                    Charging::Charging => 'c',
                    Charging::Full => 'f',
                    Charging::Unknown => 'u',
                };
                let quality = match r.quality {
                    Quality::Exact => 'e',
                    Quality::Derived => 'd',
                    Quality::Coarse => 'c',
                };
                let coarse = match r.coarse {
                    None => '-',
                    Some(Coarse::Full) => 'f',
                    Some(Coarse::Good) => 'g',
                    Some(Coarse::Low) => 'l',
                    Some(Coarse::Critical) => 'c',
                };
                format!("R {} {} {state} {quality} {coarse}", r.at, opt(r.percent))
            }
            Line::Presence(p) => format!("P {} {}", p.at, bit(p.online)),
            Line::Rgb { at, on } => format!("G {at} {}", bit(*on)),
            Line::Usage { day, seconds } => format!("U {day} {seconds}"),
            Line::Cycle(c) => format!("C {} {}", c.autonomy, bit(c.rgb)),
            Line::State(s) => {
                let seg = match s.segment {
                    Some(Segment::Discharge) => 'd',
                    Some(Segment::Charge) => 'c',
                    None => '-',
                };
                format!(
                    "S {} {} {} {} {} {seg} {} {} {} {} {} {}",
                    opt(s.first_at),
                    opt(s.r),
                    s.w,
                    opt(s.rc),
                    s.wc,
                    s.cycle_points,
                    s.cycle_hours,
                    bit(s.cycle_rgb),
                    bit(s.low_sent),
                    bit(s.full_sent),
                    opt(s.smart_day),
                )
            }
        }
    }

    /// `None` for a line that is not one of the formats above.
    pub fn decode(line: &str) -> Option<Line> {
        let mut t = line.split_whitespace();
        let tag = t.next()?;
        let cells: Vec<&str> = t.collect();
        let int = |s: &str| s.parse::<i64>().ok();
        let flag = |s: &str| match s {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        let num = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite());
        let opt_int = |s: &str| {
            if s == "-" {
                Some(None)
            } else {
                int(s).map(Some)
            }
        };
        let opt_num = |s: &str| {
            if s == "-" {
                Some(None)
            } else {
                num(s).map(Some)
            }
        };
        match (tag, cells.as_slice()) {
            ("R", [at, pct, state, quality, coarse]) => Some(Line::Reading(Reading {
                at: int(at)?,
                percent: opt_num(pct)?,
                charging: match *state {
                    "d" => Charging::Discharging,
                    "c" => Charging::Charging,
                    "f" => Charging::Full,
                    "u" => Charging::Unknown,
                    _ => return None,
                },
                quality: match *quality {
                    "e" => Quality::Exact,
                    "d" => Quality::Derived,
                    "c" => Quality::Coarse,
                    _ => return None,
                },
                coarse: match *coarse {
                    "-" => None,
                    "f" => Some(Coarse::Full),
                    "g" => Some(Coarse::Good),
                    "l" => Some(Coarse::Low),
                    "c" => Some(Coarse::Critical),
                    _ => return None,
                },
            })),
            ("P", [at, on]) => Some(Line::Presence(Presence {
                at: int(at)?,
                online: flag(on)?,
            })),
            ("G", [at, on]) => Some(Line::Rgb {
                at: int(at)?,
                on: flag(on)?,
            }),
            ("U", [day, secs]) => Some(Line::Usage {
                day: int(day)?,
                seconds: int(secs)?,
            }),
            ("C", [autonomy, rgb]) => Some(Line::Cycle(Cycle {
                autonomy: num(autonomy)?,
                rgb: flag(rgb)?,
            })),
            ("S", [first_at, r, w, rc, wc, seg, cp, ch, crgb, low, full, smart]) => {
                Some(Line::State(State {
                    first_at: opt_int(first_at)?,
                    r: opt_num(r)?,
                    w: num(w)?,
                    rc: opt_num(rc)?,
                    wc: num(wc)?,
                    segment: match *seg {
                        "d" => Some(Segment::Discharge),
                        "c" => Some(Segment::Charge),
                        "-" => None,
                        _ => return None,
                    },
                    cycle_points: num(cp)?,
                    cycle_hours: num(ch)?,
                    cycle_rgb: flag(crgb)?,
                    low_sent: flag(low)?,
                    full_sent: flag(full)?,
                    smart_day: opt_int(smart)?,
                }))
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- the record

/// Why a history file could not be read at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub reason: &'static str,
}

/// A device's persisted history, in memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceRecord {
    /// Readings that changed (written on change only), oldest first.
    pub readings: Vec<Reading>,
    pub presence: Vec<Presence>,
    /// The RGB economy switch changes.
    pub rgb: Vec<(i64, bool)>,
    /// Online seconds per local day for the days older than the presence window.
    pub usage: BTreeMap<i64, i64>,
    pub cycles: Vec<Cycle>,
    pub state: Option<State>,
    /// Lines `decode` could not read (not encoded).
    pub skipped: usize,
}

impl DeviceRecord {
    pub fn new() -> DeviceRecord {
        DeviceRecord::default()
    }

    /// Reads a history file. An empty text is an empty record; a first line that is not the current header is an
    /// error; a line that does not parse is skipped and counted.
    pub fn decode(text: &str) -> Result<DeviceRecord, DecodeError> {
        let mut record = DeviceRecord::new();
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let Some(header) = lines.next() else {
            return Ok(record);
        };
        if header != HEADER {
            return Err(DecodeError {
                reason: if header.starts_with("juicebar-history ") {
                    "unsupported version"
                } else {
                    "not a history file"
                },
            });
        }
        for line in lines {
            match Line::decode(line) {
                Some(l) => record.apply(l),
                None => record.skipped += 1,
            }
        }
        Ok(record)
    }

    /// Applies one line (as read from the file, or just produced).
    pub fn apply(&mut self, line: Line) {
        match line {
            Line::Reading(r) => self.readings.push(r),
            Line::Presence(p) => self.presence.push(p),
            Line::Rgb { at, on } => self.rgb.push((at, on)),
            Line::Usage { day, seconds } => {
                self.usage.insert(day, seconds);
            }
            Line::Cycle(c) => self.cycles.push(c),
            Line::State(s) => self.state = Some(s),
        }
    }

    /// The events in time order (presence, then switch, then reading at the same moment), then usage, cycles and
    /// state.
    pub fn lines(&self) -> Vec<Line> {
        let mut out =
            Vec::with_capacity(self.readings.len() + self.presence.len() + self.rgb.len() + 8);
        let (mut i, mut j, mut k) = (0, 0, 0);
        loop {
            let tp = self.presence.get(i).map(|p| p.at);
            let tg = self.rgb.get(j).map(|g| g.0);
            let tr = self.readings.get(k).map(|r| r.at);
            let Some(min) = [tp, tg, tr].into_iter().flatten().min() else {
                break;
            };
            if tp == Some(min) {
                out.push(Line::Presence(self.presence[i]));
                i += 1;
            } else if tg == Some(min) {
                out.push(Line::Rgb {
                    at: self.rgb[j].0,
                    on: self.rgb[j].1,
                });
                j += 1;
            } else {
                out.push(Line::Reading(self.readings[k]));
                k += 1;
            }
        }
        out.extend(
            self.usage
                .iter()
                .map(|(&day, &seconds)| Line::Usage { day, seconds }),
        );
        out.extend(self.cycles.iter().copied().map(Line::Cycle));
        if let Some(s) = &self.state {
            out.push(Line::State(s.clone()));
        }
        out
    }

    /// The whole file: the header, then `lines()`.
    pub fn encode(&self) -> Vec<String> {
        let mut out = vec![HEADER.to_string()];
        out.extend(self.lines().iter().map(Line::encode));
        out
    }

    /// The whole file as one text, one line per event, a final newline.
    pub fn encode_text(&self) -> String {
        let mut text = self.encode().join("\n");
        text.push('\n');
        text
    }

    /// Whether a reading differs from the last stored one (readings are written on change only).
    pub fn reading_changed(&self, r: &Reading) -> bool {
        match self.readings.last() {
            None => true,
            Some(l) => {
                l.percent != r.percent
                    || l.charging != r.charging
                    || l.quality != r.quality
                    || l.coarse != r.coarse
            }
        }
    }

    /// The RGB economy setting now (off before any change).
    pub fn rgb_now(&self) -> bool {
        self.rgb.last().is_some_and(|g| g.1)
    }

    /// Cuts the record to what spec/estimator.md keeps: readings and presence of the last 30 days, daily usage of
    /// the last 365, every cycle. The online time of the days that leave the presence window is folded into
    /// `usage` first; the last reading, the last switch change and an open online interval are carried across the
    /// cut, so the state at the cut is not lost.
    pub fn compact(&mut self, now: i64, utc_offset: i64) {
        // Presence is cut at the start of the local day that holds the cutoff, so every kept day is complete.
        let cutoff = now - KEEP_READINGS_DAYS * DAY;
        let day_cut = (cutoff + utc_offset).div_euclid(DAY);
        let start_cut = day_cut * DAY - utc_offset;

        let mut totals = BTreeMap::new();
        add_daily_totals(
            &mut totals,
            &intervals_of(&self.presence, start_cut),
            utc_offset,
        );
        for (day, secs) in totals {
            if day < day_cut && secs > 0 {
                self.usage.insert(day, secs);
            }
        }
        let was_online_at_cut = self
            .presence
            .iter()
            .rev()
            .find(|p| p.at < start_cut)
            .is_some_and(|p| p.online);
        self.presence.retain(|p| p.at >= start_cut);
        if was_online_at_cut {
            self.presence.insert(
                0,
                Presence {
                    at: start_cut,
                    online: true,
                },
            );
        }

        let last_reading = self.readings.last().copied();
        self.readings.retain(|r| r.at >= cutoff);
        if self.readings.is_empty() {
            self.readings.extend(last_reading);
        }

        if let Some(idx) = self.rgb.iter().rposition(|g| g.0 < cutoff) {
            self.rgb.drain(..idx);
        }

        let today = (now + utc_offset).div_euclid(DAY);
        self.usage.retain(|&day, _| today - day < KEEP_USAGE_DAYS);
    }

    /// Online seconds per local day, merged: stored days older than the presence window, derived days from presence
    /// (a derived day wins), the last 365 days, days with no time left out, in order.
    pub fn usage_days(&self, now: i64, utc_offset: i64) -> Vec<(i64, i64)> {
        let mut totals = self.usage.clone();
        let mut derived = BTreeMap::new();
        add_daily_totals(&mut derived, &intervals_of(&self.presence, now), utc_offset);
        totals.extend(derived);
        let today = (now + utc_offset).div_euclid(DAY);
        totals
            .into_iter()
            .filter(|&(d, s)| today - d < KEEP_USAGE_DAYS && s > 0)
            .collect()
    }

    /// A history rebuilt from this record: the saved state when there is one, else a replay of the events.
    pub fn history(
        &self,
        name: &str,
        rated_use: f64,
        rated_charge: f64,
        utc_offset: i64,
        lang: Lang,
    ) -> DeviceHistory {
        let mut h = DeviceHistory::new(name, rated_use, rated_charge, utc_offset);
        h.lang = lang;
        match &self.state {
            Some(state) => {
                h.presence = self.presence.clone();
                h.rgb = self.rgb_now();
                h.last = self.readings.last().copied();
                h.cycles = self.cycles.clone();
                h.first_at = [
                    state.first_at,
                    self.presence.first().map(|p| p.at),
                    self.readings.first().map(|r| r.at),
                ]
                .into_iter()
                .flatten()
                .min();
                h.restore_state(state);
            }
            None => {
                for line in self.lines() {
                    match line {
                        Line::Presence(p) => {
                            h.add_presence(p);
                        }
                        Line::Rgb { at, on } => h.set_rgb(at, on),
                        Line::Reading(r) => {
                            h.add_reading(r);
                        }
                        _ => {}
                    }
                }
                h.cycles = self.cycles.clone();
                h.notices.clear();
            }
        }
        h
    }
}

// ---------------------------------------------------------------- batching

/// Collects lines and releases them at most once every `BATCH_SECONDS` of monotonic time, and on demand (quit).
#[derive(Clone, Debug)]
pub struct Batcher {
    pending: Vec<String>,
    last_flush: Option<f64>,
}

impl Default for Batcher {
    fn default() -> Batcher {
        Batcher::new()
    }
}

impl Batcher {
    pub fn new() -> Batcher {
        Batcher {
            pending: Vec::new(),
            last_flush: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Adds lines at monotonic time `mono`; returns the lines to persist when 300 s have passed since the last
    /// release (the first lines of a run are released at once).
    pub fn push<I: IntoIterator<Item = String>>(
        &mut self,
        lines: I,
        mono: f64,
    ) -> Option<Vec<String>> {
        self.pending.extend(lines);
        self.tick(mono)
    }

    /// Releases the pending lines if their time has come.
    pub fn tick(&mut self, mono: f64) -> Option<Vec<String>> {
        if self.pending.is_empty() {
            return None;
        }
        match self.last_flush {
            Some(last) if mono - last < BATCH_SECONDS => None,
            _ => self.flush(mono),
        }
    }

    /// Releases the pending lines now (on quit).
    pub fn flush(&mut self, mono: f64) -> Option<Vec<String>> {
        if self.pending.is_empty() {
            return None;
        }
        self.last_flush = Some(mono);
        Some(std::mem::take(&mut self.pending))
    }

    /// The monotonic moment the shell must call `tick` at (a `Wake`), or `None` with nothing pending.
    pub fn due(&self) -> Option<f64> {
        if self.pending.is_empty() {
            return None;
        }
        Some(self.last_flush.map_or(0.0, |last| last + BATCH_SECONDS))
    }
}

// ---------------------------------------------------------------- the tracker

/// One device's live history, its persisted record and its write batching, kept in step: the registry feeds it the
/// provider events and gets back notices and the lines to persist.
#[derive(Clone, Debug)]
pub struct Tracker {
    pub history: DeviceHistory,
    pub record: DeviceRecord,
    batcher: Batcher,
}

impl Tracker {
    pub fn new(history: DeviceHistory) -> Tracker {
        Tracker {
            history,
            record: DeviceRecord::new(),
            batcher: Batcher::new(),
        }
    }

    /// A tracker for a device with no stored history, with the catalog's priors.
    pub fn for_model(name: &str, priors: Option<&Priors>, utc_offset: i64, lang: Lang) -> Tracker {
        let mut history = DeviceHistory::for_model(name, priors, utc_offset);
        history.lang = lang;
        Tracker::new(history)
    }

    /// Reads a stored history file, compacts it and rebuilds the estimator from it. The shell writes
    /// `record.encode_text()` back to replace the file with its compacted form.
    pub fn restore(
        text: &str,
        name: &str,
        priors: Option<&Priors>,
        now: i64,
        utc_offset: i64,
        lang: Lang,
    ) -> Result<Tracker, DecodeError> {
        let mut record = DeviceRecord::decode(text)?;
        record.compact(now, utc_offset);
        let (u, c) = priors.map_or(
            (
                estimator::DEFAULT_RATED_USE_HOURS,
                estimator::DEFAULT_RATED_CHARGE_HOURS,
            ),
            |p| (p.rated_use_hours(), p.rated_charge_hours()),
        );
        let mut history = record.history(name, u, c, utc_offset, lang);
        // The prior follows the RGB economy setting (spec/estimator.md, "Model prior").
        history.rated_use_hours_rgb = priors.and_then(Priors::rated_use_hours_rgb);
        Ok(Tracker {
            history,
            record,
            batcher: Batcher::new(),
        })
    }

    pub fn on_reading(&mut self, r: Reading, mono: f64) -> (Vec<Notice>, Option<Vec<String>>) {
        let cycles_before = self.history.cycles.len();
        let notices = self.history.add_reading(r);
        let mut lines = Vec::new();
        if self.record.reading_changed(&r) {
            self.record.readings.push(r);
            lines.push(Line::Reading(r).encode());
        }
        for c in &self.history.cycles[cycles_before..] {
            self.record.cycles.push(*c);
            lines.push(Line::Cycle(*c).encode());
        }
        (notices, self.release(lines, mono, false))
    }

    pub fn on_presence(&mut self, p: Presence, mono: f64) -> (Vec<Notice>, Option<Vec<String>>) {
        if !self.history.presence.is_empty() && self.history.online() == p.online {
            return (Vec::new(), self.release(Vec::new(), mono, false));
        }
        let notices = self.history.add_presence(p);
        self.record.presence.push(p);
        (
            notices,
            self.release(vec![Line::Presence(p).encode()], mono, false),
        )
    }

    pub fn on_rgb(&mut self, at: i64, on: bool, mono: f64) -> Option<Vec<String>> {
        if self.record.rgb_now() == on {
            return self.release(Vec::new(), mono, false);
        }
        self.history.set_rgb(at, on);
        self.record.rgb.push((at, on));
        self.release(vec![Line::Rgb { at, on }.encode()], mono, false)
    }

    /// The lines to persist when the batch is due (called from `tick`).
    pub fn tick(&mut self, mono: f64) -> Option<Vec<String>> {
        self.release(Vec::new(), mono, false)
    }

    /// The lines to persist now, on quit.
    pub fn flush(&mut self, mono: f64) -> Option<Vec<String>> {
        self.release(Vec::new(), mono, true)
    }

    /// The monotonic moment a `Wake` must ask for, or `None` with nothing pending.
    pub fn due(&self) -> Option<f64> {
        self.batcher.due()
    }

    fn release(&mut self, lines: Vec<String>, mono: f64, force: bool) -> Option<Vec<String>> {
        let released = if force {
            self.batcher.pending.extend(lines);
            self.batcher.flush(mono)
        } else {
            self.batcher.push(lines, mono)
        };
        released.map(|mut out| {
            let state = self.history.state();
            out.push(Line::State(state.clone()).encode());
            self.record.state = Some(state);
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(at: i64, pct: f64, charging: Charging) -> Reading {
        Reading::new(at, Some(pct), charging, Quality::Exact)
    }

    fn sample_record() -> DeviceRecord {
        let mut r = DeviceRecord::new();
        r.presence.push(Presence {
            at: 1000,
            online: true,
        });
        r.rgb.push((1500, true));
        r.readings.push(reading(1000, 80.0, Charging::Discharging));
        r.readings.push(Reading {
            coarse: Some(Coarse::Low),
            ..Reading::new(2000, None, Charging::Unknown, Quality::Coarse)
        });
        r.readings.push(reading(3000, 97.5, Charging::Charging));
        r.presence.push(Presence {
            at: 4000,
            online: false,
        });
        r.usage.insert(20_000, 12_345);
        r.cycles.push(Cycle {
            autonomy: 27.333333333333332,
            rgb: false,
        });
        r.state = Some(State {
            first_at: Some(1000),
            r: Some(3.712_345_678_901_234_5),
            w: 7.5,
            rc: None,
            wc: 0.0,
            segment: Some(Segment::Charge),
            cycle_points: 12.5,
            cycle_hours: 3.25,
            cycle_rgb: true,
            low_sent: false,
            full_sent: true,
            smart_day: Some(20_000),
        });
        r
    }

    #[test]
    fn round_trip_is_exact() {
        let record = sample_record();
        let text = record.encode_text();
        assert!(text.starts_with("juicebar-history 1\n"));
        let back = DeviceRecord::decode(&text).expect("decodes");
        assert_eq!(back, record);
        assert_eq!(back.encode_text(), text);
    }

    #[test]
    fn one_event_per_line_in_time_order() {
        let text = sample_record().encode_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], HEADER);
        assert_eq!(lines[1], "P 1000 1");
        assert_eq!(lines[2], "R 1000 80 d e -");
        assert_eq!(lines[3], "G 1500 1");
        assert_eq!(lines[4], "R 2000 - u c l");
        assert_eq!(lines[5], "R 3000 97.5 c e -");
        assert_eq!(lines[6], "P 4000 0");
        assert!(lines[7].starts_with("U 20000 12345"));
        assert!(lines[8].starts_with("C 27.33"));
        assert!(lines[9].starts_with("S 1000 "));
    }

    #[test]
    fn bad_lines_are_skipped_and_a_bad_header_is_refused() {
        let text = "juicebar-history 1\nP 10 1\nR 11 oops d e -\nP 12\nR 13 50 d e -\n\nR 14 49";
        let record = DeviceRecord::decode(text).expect("decodes");
        assert_eq!(record.presence.len(), 1);
        assert_eq!(record.readings.len(), 1);
        assert_eq!(record.skipped, 3);
        assert_eq!(
            DeviceRecord::decode("juicebar-history 2\nP 1 1")
                .unwrap_err()
                .reason,
            "unsupported version"
        );
        assert_eq!(
            DeviceRecord::decode("hello").unwrap_err().reason,
            "not a history file"
        );
        assert_eq!(DeviceRecord::decode("").unwrap(), DeviceRecord::new());
    }

    #[test]
    fn compaction_keeps_the_windows_and_folds_usage() {
        let off = -10_800;
        let now = 1_793_017_800_i64; // 2026-10-26 12:30 local
        let mut r = DeviceRecord::new();
        // online from 40 days ago for 6 h, then again from 35 days ago and never offline (open interval)
        let d40 = now - 40 * DAY;
        r.presence.push(Presence {
            at: d40,
            online: true,
        });
        r.presence.push(Presence {
            at: d40 + 6 * 3600,
            online: false,
        });
        let d35 = now - 35 * DAY;
        r.presence.push(Presence {
            at: d35,
            online: true,
        });
        r.presence.push(Presence {
            at: now - 10 * DAY,
            online: false,
        });
        r.presence.push(Presence {
            at: now - 5 * DAY,
            online: true,
        });
        r.readings
            .push(reading(now - 100 * DAY, 90.0, Charging::Discharging));
        r.readings
            .push(reading(now - 31 * DAY, 80.0, Charging::Discharging));
        r.readings
            .push(reading(now - 29 * DAY, 70.0, Charging::Discharging));
        r.readings
            .push(reading(now - 1000, 60.0, Charging::Discharging));
        r.rgb.push((now - 200 * DAY, true));
        r.rgb.push((now - 100 * DAY, false));
        r.rgb.push((now - 3 * DAY, true));
        r.usage.insert((now + off) / DAY - 400, 999); // older than 365 days
        r.usage.insert((now + off) / DAY - 300, 888);
        for i in 0..5 {
            r.cycles.push(Cycle {
                autonomy: 20.0 + f64::from(i),
                rgb: false,
            });
        }
        let before_total: i64 = r.usage_days(now, off).iter().map(|d| d.1).sum();
        r.compact(now, off);
        // readings: the 29-day one and the recent one survive
        assert_eq!(
            r.readings.iter().map(|x| x.percent).collect::<Vec<_>>(),
            vec![Some(70.0), Some(60.0)]
        );
        // presence: nothing before the start of the cut day, the open interval at the cut carried as online
        let cutoff = now - 30 * DAY;
        let start_cut = ((cutoff + off).div_euclid(DAY)) * DAY - off;
        assert!(r.presence.iter().all(|p| p.at >= start_cut));
        assert_eq!(
            r.presence[0],
            Presence {
                at: start_cut,
                online: true
            }
        );
        // usage: the 6 h of 40 days ago and the stretch from 35 days ago to the cut are folded in; 400-day entry gone
        assert!(!r.usage.contains_key(&((now + off) / DAY - 400)));
        assert_eq!(r.usage.get(&((now + off) / DAY - 300)), Some(&888));
        assert_eq!(r.usage.get(&((d40 + off) / DAY)), Some(&(6 * 3600)));
        // total online time is preserved by the fold (the dropped 400-day entry aside)
        let after_total: i64 = r.usage_days(now, off).iter().map(|d| d.1).sum();
        assert_eq!(
            before_total, after_total,
            "usage total changed by compaction"
        );
        // the switch: the last change before the window is kept, older ones are not
        assert_eq!(r.rgb, vec![(now - 100 * DAY, false), (now - 3 * DAY, true)]);
        assert_eq!(r.cycles.len(), 5);
        // idempotent
        let again = {
            let mut c = r.clone();
            c.compact(now, off);
            c
        };
        assert_eq!(again, r);
    }

    #[test]
    fn compaction_keeps_the_last_reading_of_a_long_silent_device() {
        let mut r = DeviceRecord::new();
        r.readings.push(reading(100, 40.0, Charging::Discharging));
        r.compact(100 + 90 * DAY, -10_800);
        assert_eq!(r.readings.len(), 1);
    }

    #[test]
    fn batcher_releases_at_most_every_300_seconds_and_on_quit() {
        let mut b = Batcher::new();
        assert_eq!(
            b.push(vec!["a".to_string()], 10.0),
            Some(vec!["a".to_string()])
        );
        assert_eq!(b.push(vec!["b".to_string()], 20.0), None);
        assert_eq!(b.due(), Some(310.0));
        assert_eq!(b.tick(309.0), None);
        assert_eq!(
            b.push(vec!["c".to_string()], 310.0),
            Some(vec!["b".to_string(), "c".to_string()])
        );
        assert_eq!(b.due(), None);
        assert_eq!(b.push(vec!["d".to_string()], 400.0), None);
        assert_eq!(b.flush(401.0), Some(vec!["d".to_string()]));
        assert_eq!(b.flush(402.0), None);
        assert_eq!(b.tick(5000.0), None);
    }

    #[test]
    fn tracker_writes_changes_only_and_batches() {
        let mut t = Tracker::for_model("G703", None, -10_800, Lang::PtBr);
        let (_, out) = t.on_presence(
            Presence {
                at: 1000,
                online: true,
            },
            0.0,
        );
        let out = out.expect("first release is immediate");
        assert_eq!(out[0], "P 1000 1");
        assert!(out.last().unwrap().starts_with("S "));
        let (_, out) = t.on_reading(reading(1000, 80.0, Charging::Discharging), 1.0);
        assert!(out.is_none());
        // the same level again adds no line
        let (_, out) = t.on_reading(reading(1600, 80.0, Charging::Discharging), 600.0);
        let out = out.expect("300 s passed");
        assert_eq!(out.iter().filter(|l| l.starts_with("R ")).count(), 1);
        assert_eq!(t.record.readings.len(), 1);
        // a repeated presence is ignored
        let (_, out) = t.on_presence(
            Presence {
                at: 1700,
                online: true,
            },
            601.0,
        );
        assert!(out.is_none());
        assert_eq!(t.record.presence.len(), 1);
        // quit flushes
        let (_, _) = t.on_reading(reading(1800, 75.0, Charging::Discharging), 610.0);
        let out = t.flush(611.0).expect("pending lines");
        assert!(out.iter().any(|l| l == "R 1800 75 d e -"));
    }

    #[test]
    fn restore_keeps_the_rated_use_with_lighting() {
        let off = -10_800;
        let priors = estimator::priors_for_model("G703 LIGHTSPEED");
        assert!(priors.is_some(), "the catalog has the G703");
        let t = Tracker::for_model("G703", priors, off, Lang::PtBr);
        let text = t.record.encode_text();
        let back = Tracker::restore(&text, "G703", priors, 1_793_016_000, off, Lang::PtBr)
            .expect("restores");
        assert_eq!(
            back.history.rated_use_hours_rgb,
            t.history.rated_use_hours_rgb
        );
        assert_eq!(back.history.rated_use_hours_rgb, Some(22.0));
    }

    #[test]
    fn restore_resumes_the_estimator() {
        let off = -10_800;
        let mut t = Tracker::for_model("G703", None, off, Lang::PtBr);
        let t0 = 1_793_016_000_i64;
        let _ = t.on_presence(
            Presence {
                at: t0,
                online: true,
            },
            0.0,
        );
        for (i, pct) in [100.0, 96.0, 92.0, 88.0].into_iter().enumerate() {
            let _ = t.on_reading(
                reading(t0 + i as i64 * 3600, pct, Charging::Discharging),
                1.0 + i as f64,
            );
        }
        let _ = t.flush(500.0);
        let text = t.record.encode_text();
        let now = t0 + 3 * 3600 + 60;
        let back = Tracker::restore(&text, "G703", None, now, off, Lang::PtBr).expect("restores");
        assert_eq!(back.history.r, t.history.r);
        assert_eq!(back.history.w, t.history.w);
        assert_eq!(back.history.level(), Some(88.0));
        assert!(back.history.online());
        assert_eq!(
            estimator::discharge_rate(&back.history),
            estimator::discharge_rate(&t.history)
        );
    }

    #[test]
    fn restore_without_a_state_line_replays_the_events() {
        let off = -10_800;
        let t0 = 1_793_016_000_i64;
        let mut r = DeviceRecord::new();
        r.presence.push(Presence {
            at: t0,
            online: true,
        });
        for (i, pct) in [100.0, 96.0, 92.0].into_iter().enumerate() {
            r.readings
                .push(reading(t0 + i as i64 * 3600, pct, Charging::Discharging));
        }
        let h = r.history("G703", 20.0, 2.5, off, Lang::En);
        assert_eq!(h.r, Some(4.0));
        assert!((h.w - 2.0).abs() < 1e-9);
        assert!(h.notices.is_empty());
    }
}
