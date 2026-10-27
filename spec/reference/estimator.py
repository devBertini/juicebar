# Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
"""JuiceBar estimator: the executable reference of spec/estimator.md.

Python 3.9 standard library only. The Rust core (Windows) and the Swift core (Mac)
reproduce this file; spec/estimator-cases.tsv (written by make_cases.py) is the
shared test data. Read spec/reference/README.md for the data format and for the
exact arithmetic the ports must copy (rounding, local time, evaluation order).

Time model: every time is an integer count of seconds since the epoch (UTC). The
local time zone is a fixed UTC offset in seconds given to DeviceHistory
(America/Sao_Paulo is -10800 today); nothing here reads the machine's zone.
"local" below means `t + utc_offset`, treated as seconds since a local epoch, so
day, weekday and hour come from floor division (Euclid: `div_euclid` in Rust).
"""

import datetime
import math
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

# ---------------------------------------------------------------- constants

HOUR = 3600
DAY = 86400
WEEK_HOURS = 168

DISCHARGING = "discharging"
CHARGING = "charging"
FULL = "full"
UNKNOWN = "unknown"

EXACT = "exact"
DERIVED = "derived"
COARSE = "coarse"

HALF_LIFE_USE_HOURS = 6.0       # half-life of the rate mean, in hours of use
MIN_DROP_POINTS = 2.0           # a step smaller than this is quantization noise
MIN_OBSERVATION_SECONDS = 900   # 0.25 h
PRIOR_WEIGHT = 3.0              # weight of the model prior, in hours
WEIGHT_CAP = 12.0               # cap of the observed weight, in hours
LABEL_WEIGHT = 3.0              # below this observed weight the forecast is "estimativa inicial"
DEFAULT_RATED_USE_HOURS = 20.0
DEFAULT_RATED_CHARGE_HOURS = 2.5
CHARGE_PRIOR_POINTS = 120.0       # 80 points at the rate + 20 points at half of it = 120 rate-hours-points
FAST_CHARGE_LIMIT = 80.0        # above this level the charge runs at half the rate
PROFILE_LEARN_SECONDS = 7 * DAY
PROFILE_WINDOW_SECONDS = 28 * DAY
FORECAST_LIMIT_SECONDS = 7 * DAY
TYPICAL_DAY_MIN_USE = 0.2
SMART_MARGIN = 1.1
LOW_PERCENT = 10.0
CYCLE_MIN_POINTS = 50.0
HEALTH_CYCLES = 3
USAGE_KEEP_DAYS = 365

WEEKDAYS = ["seg", "ter", "qua", "qui", "sex", "sáb", "dom"]
EPOCH_ORDINAL = datetime.date(1970, 1, 1).toordinal()


# ---------------------------------------------------------------- inputs


@dataclass(frozen=True)
class Reading:
    """A battery reading. percent is None for a coarse-only reading."""
    at: int
    percent: Optional[float]
    charging: str = DISCHARGING   # discharging, charging, full, unknown
    quality: str = EXACT          # exact, derived, coarse


@dataclass(frozen=True)
class Presence:
    at: int
    online: bool


@dataclass(frozen=True)
class Notice:
    """A notification decision: kind is "full", "low" or "smart"."""
    kind: str
    at: int
    text: Optional[str]


@dataclass(frozen=True)
class Cycle:
    autonomy: float   # hours of use per full battery
    rgb: bool         # RGB economy setting the cycle ran under


# ---------------------------------------------------------------- time helpers


def round_half_up(x: float) -> int:
    """Rounding used by every text: floor(x + 0.5). Never Python's round()."""
    return int(math.floor(x + 0.5))


def weekday_of_day(day: int) -> int:
    """0 = Monday. Day 0 (1970-01-01) was a Thursday."""
    return (day + 3) % 7


def hour_of_week(local: int) -> int:
    return weekday_of_day(local // DAY) * 24 + (local % DAY) // HOUR


def date_string(day: int) -> str:
    return datetime.date.fromordinal(EPOCH_ORDINAL + day).isoformat()


def clock_string(local: int) -> str:
    return "%02d:%02d" % ((local % DAY) // HOUR, (local % HOUR) // 60)


def default_profile() -> List[float]:
    """u(hour of week) before 7 days of data: 0.8 from 9 h to 19 h on weekdays, 0.1 otherwise."""
    out = []
    for weekday in range(7):
        for hour in range(24):
            out.append(0.8 if weekday < 5 and 9 <= hour < 19 else 0.1)
    return out


# ---------------------------------------------------------------- device history


class DeviceHistory:
    """Everything the estimator knows about one device, updated by events in time order.

    add_reading, add_presence and set_rgb must be called in chronological order
    (the order of the event stream). add_reading and add_presence return the
    notices that event produced (gated: once per charging segment, per
    discharging segment, per local day).
    """

    def __init__(self, name: str = "Device",
                 rated_use_hours: float = DEFAULT_RATED_USE_HOURS,
                 rated_charge_hours: float = DEFAULT_RATED_CHARGE_HOURS,
                 utc_offset: int = -10800):
        self.name = name
        self.rated_use_hours = rated_use_hours
        self.rated_charge_hours = rated_charge_hours
        self.utc_offset = utc_offset

        self.first_at: Optional[int] = None   # first reading or presence event
        self.last: Optional[Reading] = None
        self.presence: List[Presence] = []    # offline until the first event
        self.rgb = False

        # discharge rate: exponentially weighted mean over use time
        self.R: Optional[float] = None
        self.w = 0.0
        self.anchor: Optional[Tuple[float, int]] = None
        # charge rate: same, over wall-clock time
        self.Rc: Optional[float] = None
        self.wc = 0.0
        self.c_anchor: Optional[Tuple[float, int]] = None

        # segments and cycles
        self.segment: Optional[str] = None    # "discharge", "charge" or None
        self.cycle_points = 0.0
        self.cycle_hours = 0.0
        self.cycle_rgb = False
        self.cycles: List[Cycle] = []

        # notice gates
        self.low_sent = False
        self.full_sent = False
        self.smart_day: Optional[int] = None  # local day of the last smart warning
        self.notices: List[Notice] = []

    # -- state

    @property
    def online(self) -> bool:
        return bool(self.presence) and self.presence[-1].online

    @property
    def level(self) -> Optional[float]:
        if self.last is None or self.last.quality == COARSE:
            return None
        return self.last.percent

    # -- events

    def add_presence(self, p: Presence) -> List[Notice]:
        if self.first_at is None:
            self.first_at = p.at
        self.presence.append(p)
        out: List[Notice] = []
        if p.online:
            self._smart(p.at, out)
        return out

    def set_rgb(self, at: int, on: bool) -> None:
        """A change of the RGB economy switch starts a new discharge anchor."""
        if on != self.rgb:
            self.rgb = on
            self.anchor = None

    def add_reading(self, r: Reading) -> List[Notice]:
        if self.first_at is None:
            self.first_at = r.at
        if r.charging in (CHARGING, FULL):
            kind = "charge"
        elif r.charging == DISCHARGING:
            kind = "discharge"
        else:
            kind = None   # unknown: neither breaks nor extends a segment
        measurable = r.percent is not None and r.quality != COARSE

        if kind is not None and kind != self.segment:
            self._end_segment()
            self._start_segment(kind)
        self.last = r

        if measurable and kind == "discharge":
            self._observe_discharge(r)
        elif measurable and kind == "charge":
            self._observe_charge(r)

        out: List[Notice] = []
        if kind == "charge" and not self.full_sent and (
                r.charging == FULL or (r.percent is not None and r.percent >= 100.0)):
            self.full_sent = True
            out.append(Notice("full", r.at, None))
        if kind == "discharge" and measurable and r.percent < LOW_PERCENT and not self.low_sent:
            self.low_sent = True
            out.append(Notice("low", r.at, low_warning(self)))
        self._smart(r.at, out)
        self.notices.extend(out)
        return out

    def _smart(self, now: int, out: List[Notice]) -> None:
        """Smart warning gate: at most once per local calendar day."""
        text = smart_warning(self, now)
        day = (now + self.utc_offset) // DAY
        if text is not None and self.smart_day != day:
            self.smart_day = day
            out.append(Notice("smart", now, text))

    # -- segments and cycles

    def _end_segment(self) -> None:
        if self.segment == "discharge" and self.cycle_points >= CYCLE_MIN_POINTS:
            autonomy = 100.0 * self.cycle_hours / self.cycle_points
            self.cycles.append(Cycle(autonomy, self.cycle_rgb))

    def _start_segment(self, kind: str) -> None:
        self.segment = kind
        if kind == "discharge":
            self.anchor = None
            self.low_sent = False
            self.cycle_points = 0.0
            self.cycle_hours = 0.0
            self.cycle_rgb = self.rgb
        else:
            self.c_anchor = None
            self.full_sent = False

    def _observe_discharge(self, r: Reading) -> None:
        pct = r.percent
        if self.anchor is None:
            self.anchor = (pct, r.at)
            return
        a_pct, a_at = self.anchor
        if pct > a_pct:
            # the level rose with no charge reading: a missed charge, start over
            self.anchor = (pct, r.at)
            return
        drop = a_pct - pct
        secs = self._online_seconds(a_at, r.at)
        if drop >= MIN_DROP_POINTS and secs >= MIN_OBSERVATION_SECONDS:
            hours = secs / 3600.0
            self.R, self.w = _blend(self.R, self.w, drop / hours, hours)
            self.cycle_points += drop
            self.cycle_hours += hours
            self.anchor = (pct, r.at)

    def _observe_charge(self, r: Reading) -> None:
        pct = r.percent
        if self.c_anchor is None:
            self.c_anchor = (pct, r.at)
            return
        a_pct, a_at = self.c_anchor
        if pct < a_pct:
            self.c_anchor = (pct, r.at)
            return
        rise = pct - a_pct
        secs = r.at - a_at
        if rise >= MIN_DROP_POINTS and secs >= MIN_OBSERVATION_SECONDS:
            hours = secs / 3600.0
            # points above 80 % cost twice the time, so they count double: the learned
            # rate is always the fast-phase rate
            eff = (min(pct, FAST_CHARGE_LIMIT) - min(a_pct, FAST_CHARGE_LIMIT)) \
                + 2.0 * (max(pct, FAST_CHARGE_LIMIT) - max(a_pct, FAST_CHARGE_LIMIT))
            self.Rc, self.wc = _blend(self.Rc, self.wc, eff / hours, hours)
            self.c_anchor = (pct, r.at)

    # -- presence queries

    def intervals(self, upto: int) -> List[Tuple[int, int]]:
        """Online intervals (start, end) in UTC seconds, an open one closed at `upto`."""
        out = []
        start = None
        for p in self.presence:
            if p.at > upto:
                break
            if p.online and start is None:
                start = p.at
            elif not p.online and start is not None:
                out.append((start, p.at))
                start = None
        if start is not None:
            out.append((start, upto))
        return out

    def _online_seconds(self, a: int, b: int) -> int:
        total = 0
        for s, e in self.intervals(b):
            total += max(0, min(e, b) - max(s, a))
        return total

    # -- the hour-of-week profile u(how)

    def profile_learned(self, now: int) -> bool:
        return self.first_at is not None and now - self.first_at >= PROFILE_LEARN_SECONDS

    def profile(self, now: int) -> List[float]:
        """u for each of the 168 hours of the week (index weekday * 24 + hour, Monday first).

        Before 7 days of data: the default. After: the mean, over the complete local
        hours of the last 28 days that start after the first data, of the share of
        each hour the device was online; an hour of the week with no complete
        occurrence keeps its default value.
        """
        default = default_profile()
        if not self.profile_learned(now):
            return default
        off = self.utc_offset
        lo = max(now - PROFILE_WINDOW_SECONDS, self.first_at) + off
        first_slot = -((-lo) // HOUR) * HOUR            # ceil to a local hour
        last_end = ((now + off) // HOUR) * HOUR         # floor to a local hour
        online: Dict[int, int] = {}
        for s, e in self.intervals(now):
            t = max(s + off, first_slot)
            end = min(e + off, last_end)
            while t < end:
                slot = (t // HOUR) * HOUR
                stop = min(end, slot + HOUR)
                online[slot] = online.get(slot, 0) + (stop - t)
                t = stop
        sums = [0.0] * WEEK_HOURS
        counts = [0] * WEEK_HOURS
        slot = first_slot
        while slot + HOUR <= last_end:
            how = hour_of_week(slot)
            sums[how] += online.get(slot, 0) / 3600.0
            counts[how] += 1
            slot += HOUR
        return [sums[i] / counts[i] if counts[i] else default[i] for i in range(WEEK_HOURS)]


def _blend(rate: Optional[float], weight: float, r: float, hours: float):
    """One observation r over `hours`: first sets the mean, then R = R + a (r - R)."""
    if rate is None:
        rate = r
    else:
        a = 1.0 - 2.0 ** (-hours / HALF_LIFE_USE_HOURS)
        rate = rate + a * (r - rate)
    return rate, min(WEIGHT_CAP, weight + hours)


# ---------------------------------------------------------------- rates


def discharge_rate(h: DeviceHistory) -> float:
    """Percent per hour of use: (w R + 3 P) / (w + 3), P = 100 / rated hours of use."""
    prior = 100.0 / h.rated_use_hours
    if h.R is None:
        return prior
    return (h.w * h.R + PRIOR_WEIGHT * prior) / (h.w + PRIOR_WEIGHT)


def charge_rate(h: DeviceHistory) -> float:
    """Fast-phase percent per wall-clock hour: the same blend, P = 120 / rated hours to charge
    (a full charge at the two-phase pace then takes the rated time)."""
    prior = CHARGE_PRIOR_POINTS / h.rated_charge_hours
    if h.Rc is None:
        return prior
    return (h.wc * h.Rc + PRIOR_WEIGHT * prior) / (h.wc + PRIOR_WEIGHT)


# ---------------------------------------------------------------- forecast to empty


def _discharge_forecast_ok(h: DeviceHistory) -> bool:
    return (h.online and h.last is not None and h.level is not None
            and h.last.charging not in (CHARGING, FULL))


def hours_left(h: DeviceHistory) -> Optional[float]:
    """Hours of use left: level / rate. None when offline, charging, coarse-only or unread."""
    if not _discharge_forecast_ok(h):
        return None
    return h.level / discharge_rate(h)


def forecast_label(h: DeviceHistory) -> Optional[str]:
    if hours_left(h) is not None and h.w < LABEL_WEIGHT:
        return "estimativa inicial"
    return None


def walk_use(profile: List[float], off: int, start: int, use_hours: float,
             limit_seconds: int = FORECAST_LIMIT_SECONDS) -> Optional[float]:
    """The moment (epoch seconds) at which `use_hours` of expected use have been spent,
    walking forward from `start` one local hour at a time; None past the limit."""
    remaining = use_hours
    t = start
    end = start + limit_seconds
    while True:
        if remaining <= 0.0:
            return float(t)
        if t >= end:
            return None
        local = t + off
        d = min((local // HOUR) * HOUR + HOUR - local, end - t)
        u = profile[hour_of_week(local)]
        avail = u * d / 3600.0
        if u > 0.0 and remaining <= avail:
            return t + remaining / u * 3600.0
        remaining -= avail
        t += d


def eta(h: DeviceHistory, now: int) -> Optional[float]:
    """Epoch seconds at which the device is expected to be empty; None past 7 days."""
    hl = hours_left(h)
    if hl is None:
        return None
    return walk_use(h.profile(now), h.utc_offset, now, hl)


def format_hours_left(hours: float) -> str:
    """"~40 min" (under 1 h, nearest 10), "~2,5 h" (under 10 h, nearest half), "~18 h"
    (under 72 h, nearest hour), then "~3 dias"."""
    if hours < 1.0:
        minutes = max(10, round_half_up(hours * 6.0) * 10)
        if minutes < 60:
            return "~%d min" % minutes
        return "~1 h"
    if hours < 10.0:
        halves = round_half_up(hours * 2.0)
        whole, half = divmod(halves, 2)
        return "~%d,5 h" % whole if half else "~%d h" % whole
    if hours < 72.0:
        return "~%d h" % round_half_up(hours)
    return _days_text(hours)


def _days_text(hours: float) -> str:
    n = max(1, round_half_up(hours / 24.0))
    return "~%d dia" % n if n == 1 else "~%d dias" % n


def forecast_text(h: DeviceHistory) -> Optional[str]:
    hl = hours_left(h)
    if hl is None:
        return None
    return format_hours_left(hl) + " de uso"


def eta_text(h: DeviceHistory, now: int) -> Optional[str]:
    """"acaba hoje ~17:40", "acaba amanhã ~10:20", "acaba qua ~20:00" (the weekday from the
    day after tomorrow; time rounded to 10 minutes), or past 7 days "~<n> dias de uso" alone."""
    hl = hours_left(h)
    if hl is None:
        return None
    t = eta(h, now)
    if t is None:
        return _days_text(hl) + " de uso"
    rounded = round_half_up(t / 600.0) * 600
    local = rounded + h.utc_offset
    day = local // DAY
    ahead = day - (now + h.utc_offset) // DAY
    if ahead == 0:
        return "acaba hoje ~" + clock_string(local)
    if ahead == 1:
        return "acaba amanhã ~" + clock_string(local)
    return "acaba %s ~%s" % (WEEKDAYS[weekday_of_day(day)], clock_string(local))


# ---------------------------------------------------------------- time to full


def time_to_full(h: DeviceHistory) -> Optional[float]:
    """Wall-clock hours to 100 %: up to 80 % at the charge rate, 80 % to 100 % at half of it.
    None unless online and charging with a known level below 100."""
    if not (h.online and h.last is not None and h.last.charging == CHARGING
            and h.level is not None and h.level < 100.0):
        return None
    rc = charge_rate(h)
    level = h.level
    if level < FAST_CHARGE_LIMIT:
        return (FAST_CHARGE_LIMIT - level) / rc + (100.0 - FAST_CHARGE_LIMIT) / (rc / 2.0)
    return (100.0 - level) / (rc / 2.0)


def full_in_text(h: DeviceHistory) -> Optional[str]:
    """"cheio em ~1 h 10 min": nearest 10 minutes."""
    hours = time_to_full(h)
    if hours is None:
        return None
    minutes = max(10, round_half_up(hours * 6.0) * 10)
    if minutes < 60:
        return "cheio em ~%d min" % minutes
    whole, rest = divmod(minutes, 60)
    if rest == 0:
        return "cheio em ~%d h" % whole
    return "cheio em ~%d h %d min" % (whole, rest)


def full_at_text(h: DeviceHistory, now: int) -> Optional[str]:
    """"cheio às 15:42": local clock time, nearest minute."""
    hours = time_to_full(h)
    if hours is None:
        return None
    t = round_half_up((now + hours * 3600.0) / 60.0) * 60
    return "cheio às " + clock_string(t + h.utc_offset)


def full_notice(h: DeviceHistory) -> bool:
    """True when the last reading says full, or 100 % while charging (the history
    sends the notice once per charging segment)."""
    r = h.last
    if r is None or r.charging not in (CHARGING, FULL):
        return False
    return r.charging == FULL or (r.percent is not None and r.percent >= 100.0)


# ---------------------------------------------------------------- usage per day


def usage_by_day(h: DeviceHistory, now: int) -> Dict[str, int]:
    """Online seconds inside each local calendar day, the last 365 days, days with
    no online time left out. Keys are ISO dates, in order."""
    off = h.utc_offset
    totals: Dict[int, int] = {}
    for s, e in h.intervals(now):
        t = s + off
        end = e + off
        while t < end:
            day = t // DAY
            stop = min(end, (day + 1) * DAY)
            totals[day] = totals.get(day, 0) + (stop - t)
            t = stop
    today = (now + off) // DAY
    return {date_string(d): totals[d] for d in sorted(totals)
            if today - d < USAGE_KEEP_DAYS and totals[d] > 0}


def usage_text(seconds: int) -> str:
    """"50 min", "4 h", "4 h 10 min", "6 h 05 min": nearest minute."""
    minutes = round_half_up(seconds / 60.0)
    if minutes < 60:
        return "%d min" % minutes
    whole, rest = divmod(minutes, 60)
    if rest == 0:
        return "%d h" % whole
    return "%d h %02d min" % (whole, rest)


def usage_today_text(h: DeviceHistory, now: int) -> str:
    today = date_string((now + h.utc_offset) // DAY)
    return "Hoje: %s de uso" % usage_text(usage_by_day(h, now).get(today, 0))


# ---------------------------------------------------------------- battery health


def health(h: DeviceHistory) -> Optional[float]:
    """Percent: median autonomy of the last 3 cycles over the median of the first 3
    cycles under the same RGB setting (the setting of the latest cycle), capped at
    100. None ("aprendendo") until that setting has 3 cycles."""
    if not h.cycles:
        return None
    setting = h.cycles[-1].rgb
    same = [c.autonomy for c in h.cycles if c.rgb == setting]
    if len(same) < HEALTH_CYCLES:
        return None
    first = sorted(same[:HEALTH_CYCLES])[HEALTH_CYCLES // 2]
    last = sorted(same[-HEALTH_CYCLES:])[HEALTH_CYCLES // 2]
    return min(100.0, last / first * 100.0)


def health_text(h: DeviceHistory) -> str:
    value = health(h)
    if value is None:
        return "aprendendo"
    return "%d%%" % round_half_up(value)


# ---------------------------------------------------------------- warnings


def expected_use_left_today(h: DeviceHistory, now: int) -> float:
    """Sum of u(hour) from now to the end of the typical day (the end of the last
    local hour of today with u >= 0.2); 0 when there is none or it has passed."""
    profile = h.profile(now)
    local = now + h.utc_offset
    day_start = (local // DAY) * DAY
    last_hour = None
    for hour in range(24):
        if profile[hour_of_week(day_start + hour * HOUR)] >= TYPICAL_DAY_MIN_USE:
            last_hour = hour
    if last_hour is None:
        return 0.0
    end = day_start + (last_hour + 1) * HOUR
    total = 0.0
    t = local
    while t < end:
        stop = min(end, (t // HOUR) * HOUR + HOUR)
        total += profile[hour_of_week(t)] * (stop - t) / 3600.0
        t = stop
    return total


def smart_warning(h: DeviceHistory, now: int) -> Optional[str]:
    """The text of the smart warning if the condition holds at `now` (online,
    discharging, hours left below 1.1 times the expected use left today), else None.
    The once-per-day gate lives in DeviceHistory."""
    hl = hours_left(h)
    if hl is None or not (hl < expected_use_left_today(h, now) * SMART_MARGIN):
        return None
    to_ten = max(0.0, (h.level - LOW_PERCENT) / discharge_rate(h))
    moment = walk_use(h.profile(now), h.utc_offset, now, to_ten)
    if moment is None:
        moment = float(now)
    hour = (round_half_up(moment) + h.utc_offset) % DAY // HOUR
    return "Não chega ao fim do dia: carregue antes das %dh" % hour


def low_warning(h: DeviceHistory) -> Optional[str]:
    """"<device> com <n> %" when the level of a discharging device is below 10 %."""
    if h.last is None or h.segment != "discharge" or h.last.charging != DISCHARGING:
        return None
    if h.level is None or not (h.level < LOW_PERCENT):
        return None
    return "%s com %d%%" % (h.name, round_half_up(h.level))
