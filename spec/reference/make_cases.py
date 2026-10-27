# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
"""Writes spec/estimator-cases.tsv: named scenarios (inputs as compact event strings,
expected outputs as columns) computed by estimator.py. Both cores parse the file and
must reproduce every expected column within the tolerances of spec/estimator.md.

    python3 -B spec/reference/make_cases.py

The output is deterministic: running it twice gives a byte-identical file.
The format is described in the header comment block of the TSV and in README.md.
"""

import math
import os
import sys
from typing import Dict, List, NamedTuple, Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import estimator as est  # noqa: E402

OUT_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "estimator-cases.tsv")
BRT = -10800   # America/Sao_Paulo, UTC-3, no DST

STATE_CODE = {"d": est.DISCHARGING, "c": est.CHARGING, "f": est.FULL, "u": est.UNKNOWN}
QUALITY_CODE = {"e": est.EXACT, "d": est.DERIVED, "c": est.COARSE}
STATE_LETTER = {v: k for k, v in STATE_CODE.items()}
QUALITY_LETTER = {v: k for k, v in QUALITY_CODE.items()}

INPUT_COLUMNS = ["name", "device", "rated_use", "rated_use_rgb", "rated_charge", "utc_offset", "now", "events"]
OUTPUT_COLUMNS = [
    "level", "online", "rate", "observed_hours", "label", "hours_left", "hours_left_text",
    "forecast_text", "eta", "eta_text", "charge_rate", "full_hours", "full_in_text",
    "full_at_text", "usage_today_text", "usage_by_day", "health", "smart_warning",
    "notices", "profile_learned", "u_now", "note",
]
COLUMNS = INPUT_COLUMNS + OUTPUT_COLUMNS

HEADER = """\
# estimator-cases.tsv: written by spec/reference/make_cases.py from spec/reference/estimator.py. Do not edit by hand.
# One scenario per line, tab separated. Lines starting with '#' are comments. The first other line holds the column names.
# A cell '-' means "none" (the function returns no value). No cell contains a tab or a newline.
#
# Inputs
#   name          scenario id.
#   device        display name (used in the low-battery text "<device> com <n> %").
#   rated_use     rated hours of use (model prior: percent per hour = 100 / rated_use).
#   rated_use_rgb rated hours of use while the RGB economy is off ('-': none, rated_use serves both settings).
#                 The prior follows the setting in force at `now`: rated_use_rgb while off, rated_use while on.
#   rated_charge  rated hours to charge (prior charge rate = 120 / rated_charge, percent per wall-clock hour).
#   utc_offset    local UTC offset in seconds (-10800 = America/Sao_Paulo). Local time = UTC + offset.
#   now           the moment every expected value is computed at, seconds since the epoch (UTC).
#   events        the event stream, space separated tokens, in chronological order, applied one by one:
#                   R<t>/<percent>/<state>[/<quality>]   a reading. percent is a number or '-' (none).
#                                                         state: d discharging, c charging, f full, u unknown.
#                                                         quality: e exact (default when omitted), d derived, c coarse.
#                   P<t>/<0|1>                            presence: 1 online, 0 offline (a device is offline until its first P).
#                   G<t>/<0|1>                            the RGB economy switch: 1 on, 0 off (off at the start).
#                 <t> is seconds since the epoch (UTC). '-' alone means no events.
# Expected outputs, all computed at `now` after the whole stream was applied (functions of spec/estimator.md)
#   level            last reading's percent ('-' when none or coarse).
#   online           1 or 0 (the last presence event).
#   rate             discharge_rate: blended percent per hour of use (9 significant digits).
#   observed_hours   w, the observed use weight in hours, capped at 12.
#   label            'estimativa inicial' or '-'.
#   hours_left       hours of use left (9 significant digits) or '-'.
#   hours_left_text  format_hours_left, e.g. '~2,5 h', or '-'.
#   forecast_text    hours_left_text + ' de uso', or '-'.
#   eta              epoch seconds of the forecast moment, rounded to a whole second, or '-' (offline, charging, coarse, or past 7 days).
#   eta_text         'acaba hoje ~17:40', 'acaba amanhã ~10:20', 'acaba qua ~20:00' (weekday from the day after tomorrow), or past 7 days '~<n> dias de uso', or '-'.
#   charge_rate      charge_rate: blended fast-phase percent per wall-clock hour.
#   full_hours       time to full in wall-clock hours, or '-'.
#   full_in_text     'cheio em ~1 h 10 min' or '-'.
#   full_at_text     'cheio às 15:42' or '-'.
#   usage_today_text 'Hoje: 4 h 10 min de uso'.
#   usage_by_day     online seconds per local day, 'YYYY-MM-DD:seconds' joined by ',', or '-'.
#   health           percent (4 decimals) or 'aprendendo'.
#   smart_warning    the stateless smart_warning(now) text or '-'.
#   notices          notices produced by the events, gated, in order: '<kind>@<t>[:<text>]' joined by '|'
#                    (kind: full, low, smart; the text is everything after the first ':'), or '-'.
#   profile_learned  1 when the hour-of-week profile is learned (7 days or more of data), else 0.
#   u_now            u(hour of week of `now`), 6 decimals.
#   note             human text only.
# Tolerances (spec/estimator.md): rates 1e-6 relative, hours 1 minute, eta 60 seconds, health 0.1 point,
# texts and notices identical (a notice's time identical).
"""


class Case(NamedTuple):
    name: str
    device: str
    rated_use: float
    rated_use_rgb: Optional[float]
    rated_charge: float
    utc_offset: int
    now: int
    events: List[tuple]   # ("R", t, percent, state, quality) | ("P", t, online) | ("G", t, on)
    note: str


# ---------------------------------------------------------------- events: text form


def _num(x: float) -> str:
    return repr(int(x)) if float(x).is_integer() else repr(float(x))


def format_events(events: List[tuple]) -> str:
    out = []
    for ev in events:
        if ev[0] == "R":
            _, t, pct, state, quality = ev
            tok = "R%d/%s/%s" % (t, "-" if pct is None else _num(pct), STATE_LETTER[state])
            if quality != est.EXACT:
                tok += "/" + QUALITY_LETTER[quality]
            out.append(tok)
        else:
            out.append("%s%d/%d" % (ev[0], ev[1], 1 if ev[2] else 0))
    return " ".join(out) if out else "-"


def parse_events(text: str) -> List[tuple]:
    if text == "-":
        return []
    out = []
    for tok in text.split(" "):
        kind, rest = tok[0], tok[1:].split("/")
        if kind == "R":
            pct = None if rest[1] == "-" else float(rest[1])
            quality = QUALITY_CODE[rest[3]] if len(rest) > 3 else est.EXACT
            out.append(("R", int(rest[0]), pct, STATE_CODE[rest[2]], quality))
        elif kind in ("P", "G"):
            out.append((kind, int(rest[0]), rest[1] == "1"))
        else:
            raise ValueError("bad event token " + tok)
    return out


def replay(case: Case):
    """Applies the case's events; returns (history, notices)."""
    h = est.DeviceHistory(case.device, case.rated_use, case.rated_charge, case.utc_offset,
                          rated_use_hours_rgb=case.rated_use_rgb)
    notices = []
    for ev in case.events:
        if ev[0] == "R":
            notices += h.add_reading(est.Reading(ev[1], ev[2], ev[3], ev[4]))
        elif ev[0] == "P":
            notices += h.add_presence(est.Presence(ev[1], ev[2]))
        else:
            h.set_rgb(ev[1], ev[2])
    return h, notices


# ---------------------------------------------------------------- expected columns


def _g(x: Optional[float]) -> str:
    return "-" if x is None else "%.9g" % x


def _t(x: Optional[str]) -> str:
    return "-" if x is None else x


def evaluate(case: Case) -> Dict[str, str]:
    h, notices = replay(case)
    now = case.now
    hl = est.hours_left(h)
    eta = est.eta(h, now)
    usage = est.usage_by_day(h, now)
    hv = est.health(h)
    profile = h.profile(now)
    how = est.hour_of_week(now + case.utc_offset)
    return {
        "level": _g(h.level),
        "online": "1" if h.online else "0",
        "rate": _g(est.discharge_rate(h)),
        "observed_hours": _g(h.w),
        "label": _t(est.forecast_label(h)),
        "hours_left": _g(hl),
        "hours_left_text": "-" if hl is None else est.format_hours_left(hl),
        "forecast_text": _t(est.forecast_text(h)),
        "eta": "-" if eta is None else "%d" % est.round_half_up(eta),
        "eta_text": _t(est.eta_text(h, now)),
        "charge_rate": _g(est.charge_rate(h)),
        "full_hours": _g(est.time_to_full(h)),
        "full_in_text": _t(est.full_in_text(h)),
        "full_at_text": _t(est.full_at_text(h, now)),
        "usage_today_text": est.usage_today_text(h, now),
        "usage_by_day": ",".join("%s:%d" % kv for kv in usage.items()) or "-",
        "health": "aprendendo" if hv is None else "%.4f" % hv,
        "smart_warning": _t(est.smart_warning(h, now)),
        "notices": "|".join(
            "%s@%d%s" % (n.kind, n.at, "" if n.text is None else ":" + n.text) for n in notices) or "-",
        "profile_learned": "1" if h.profile_learned(now) else "0",
        "u_now": "%.6f" % profile[how],
        "note": case.note,
    }


def row(case: Case) -> List[str]:
    cells = {
        "name": case.name, "device": case.device,
        "rated_use": _num(case.rated_use),
        "rated_use_rgb": "-" if case.rated_use_rgb is None else _num(case.rated_use_rgb),
        "rated_charge": _num(case.rated_charge),
        "utc_offset": str(case.utc_offset), "now": str(case.now),
        "events": format_events(case.events),
    }
    cells.update(evaluate(case))
    return [cells[c] for c in COLUMNS]


def parse_row(line: str) -> Dict[str, str]:
    return dict(zip(COLUMNS, line.split("\t")))


def case_from_cells(cells: Dict[str, str]) -> Case:
    return Case(cells["name"], cells["device"], float(cells["rated_use"]),
                None if cells["rated_use_rgb"] == "-" else float(cells["rated_use_rgb"]), float(cells["rated_charge"]),
                int(cells["utc_offset"]), int(cells["now"]), parse_events(cells["events"]), cells["note"])


# ---------------------------------------------------------------- scenario builders


def epoch(year: int, month: int, day: int, hour: int = 0, minute: int = 0, offset: int = BRT) -> int:
    import datetime
    days = datetime.date(year, month, day).toordinal() - est.EPOCH_ORDINAL
    return days * est.DAY + hour * 3600 + minute * 60 - offset


MON = epoch(2026, 10, 26)   # Monday 26/10/2026 00:00 local (UTC-3)
HR = 3600


def R(t, pct, state="d", quality=est.EXACT):
    return ("R", t, pct, STATE_CODE[state], quality)


def P(t, online=True):
    return ("P", t, online)


def G(t, on=True):
    return ("G", t, on)


def fall(t0, pct0, hours, drop_per_hour):
    """Discharging readings, one per hour for `hours` hours, falling `drop_per_hour` points each."""
    return [R(t0 + i * HR, pct0 - drop_per_hour * i) for i in range(hours + 1)]


def cycle(t0, hours, drop_per_hour):
    """A discharging cycle (hours of use, integer points per hour: autonomy 100 / drop),
    then a charge. Returns (events, end time)."""
    evs = fall(t0, 100, hours, drop_per_hour)
    t_end = t0 + hours * HR
    evs.append(R(t_end + 600, 100 - drop_per_hour * hours, "c"))
    evs.append(R(t_end + 3 * HR, 100, "f"))
    return evs, t_end + 3 * HR


def cycles_from(t, specs):
    evs = []
    for hours, drop in specs:
        e_, t = cycle(t, hours, drop)
        evs += e_
    return evs, t


def t_of(evs):
    """Time of the last event of a list."""
    return evs[-1][1]


def mk(name, events, now, note, device="G703", use=20.0, charge=2.5, off=BRT, use_rgb=None):
    return Case(name, device, use, use_rgb, charge, off, now, events, note)


# ---------------------------------------------------------------- the sample devices of the mockups


def _sample_points(anchors):
    """Port of build() in docs/design/harness/sample-state.js: (hour offset, level, state)."""
    hs = set(a[0] for a in anchors)
    for hh in range(int(math.ceil(anchors[0][0])), int(anchors[-1][0]) + 1):
        hs.add(hh)

    def weight(hh):
        hod = ((14.5 + hh) % 24 + 24) % 24
        return 1.0 if 9 <= hod < 19 else 0.15

    def level(hh):
        for i in range(len(anchors) - 1):
            a, b = anchors[i], anchors[i + 1]
            if a[0] <= hh <= b[0]:
                if b[1] >= a[1]:
                    return a[1] + (b[1] - a[1]) * (hh - a[0]) / (b[0] - a[0])
                total = part = 0.0
                t = a[0]
                while t < b[0]:
                    w = weight(t)
                    total += w
                    if t < hh:
                        part += w
                    t += 0.25
                return a[1] - (a[1] - b[1]) * part / total
        return anchors[-1][1]

    def state(hh, lvl):
        for i in range(len(anchors) - 1):
            a, b = anchors[i], anchors[i + 1]
            if a[0] <= hh <= b[0]:
                if b[1] > a[1]:
                    return "f" if lvl >= 100 else "c"
                return "d"
        return "d"

    out = []
    for hh in sorted(hs):
        lvl = math.floor(level(hh) * 10 + 0.5) / 10
        out.append((hh, lvl, state(hh, lvl)))
    return out


def sample_case(name, anchors, rated_use, rated_charge, device, last_offline_at=None, note=""):
    now = epoch(2026, 10, 26, 14, 32)
    evs = [P(now + int(anchors[0][0] * HR), True)]
    prev = None
    for hh, lvl, st in _sample_points(anchors):
        key = (lvl, st)
        if key != prev:
            evs.append(R(now + int(round(hh * HR)), lvl, st))
            prev = key
    if last_offline_at is not None:
        evs.append(P(now + int(last_offline_at * HR), False))
    return mk(name, evs, now, note, device=device, use=rated_use, charge=rated_charge)


# ---------------------------------------------------------------- scenarios


def scenarios() -> List[Case]:
    c: List[Case] = []
    at = lambda d, h, m=0: MON + d * est.DAY + h * HR + m * 60   # day offset from Monday, local hour

    # first observation and noise
    c.append(mk("first_reading_prior_only",
                [P(at(0, 9)), R(at(0, 9), 80)], at(0, 9, 30),
                "one reading: no observation, rate is the prior 100/20, label initial"))
    c.append(mk("first_observation_sets_rate",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 10), 76)], at(0, 10),
                "4 points in 1 h: R=4, w=1, rate=(4+15)/4"))
    c.append(mk("one_point_steps_ignored",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 10), 79), R(at(0, 11), 79), R(at(0, 12), 78)], at(0, 12),
                "drops of 1 point never observe; the anchor stays until 2 points: r=2/3 over 3 h"))
    c.append(mk("min_quarter_hour_not_reached",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 9, 14), 78)], at(0, 9, 14),
                "2 points in 14 min: no observation"))
    c.append(mk("min_quarter_hour_reached",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 9, 15), 78)], at(0, 9, 15),
                "2 points in exactly 15 min: r=8"))
    c.append(mk("anchor_kept_until_quarter_hour",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 9, 14), 78), R(at(0, 9, 30), 77)], at(0, 9, 30),
                "the 14 min step does not move the anchor: 3 points in 0.5 h, r=6"))
    c.append(mk("ewma_two_observations",
                [P(at(0, 9)), R(at(0, 9), 100), R(at(0, 10), 96), R(at(0, 16), 84)], at(0, 16),
                "r=4 over 1 h then r=2 over 6 h: a=0.5, R=3, w=7"))
    c.append(mk("label_leaves_at_three_hours",
                [P(at(0, 9)), R(at(0, 9), 100), R(at(0, 11), 96), R(at(0, 12), 94)], at(0, 12),
                "w=3 exactly: no 'estimativa inicial'"))
    c.append(mk("weight_capped_at_twelve",
                [P(at(0, 0)), R(at(0, 0), 100)] + [R(at(0, 0) + i * 10 * HR, 100 - 8 * i) for i in range(1, 6)],
                at(2, 2), "5 observations of 10 h: w stays 12"))

    # segments
    c.append(mk("charge_breaks_segment",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 10), 78), R(at(0, 11), 100, "c"),
                 R(at(0, 12), 95), R(at(0, 14), 91)], at(0, 14),
                "no observation spans the charge; the first reading after it is the anchor: r=2 then 2 h"))
    c.append(mk("rgb_switch_new_anchor",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 11), 76), G(at(0, 11)), R(at(0, 12), 74),
                 R(at(0, 14), 70)], at(0, 14),
                "the 76->74 step is lost: the 74 reading is the new anchor; w=2+2"))
    c.append(mk("rgb_switch_prior_follows_setting",
                [P(at(0, 9)), R(at(0, 9), 80), G(at(0, 9, 30))], at(0, 9, 30),
                "prior 100/22 while off, switched on at 09:30 to 100/30: the rate follows the setting in force",
                device="G703", use=30.0, use_rgb=22.0))
    c.append(mk("rise_without_charge_reanchors",
                [P(at(0, 9)), R(at(0, 9), 50), R(at(0, 10), 90), R(at(0, 12), 86)], at(0, 12),
                "level rose with state discharging: a missed charge, new anchor (reading of the spec gap)"))
    c.append(mk("unknown_state_keeps_segment",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 10), 79, "u"), R(at(0, 11), 76)], at(0, 11),
                "an unknown-state reading neither breaks nor extends the segment: r=4/2 h"))

    # forecast and profile
    c.append(mk("eta_default_profile_today",
                [P(at(0, 9)), R(at(0, 9), 80), R(at(0, 14), 23)], at(0, 14, 32),
                "weekday afternoon, learned over 5 h: forecast ends today",
                use=20.0))
    c.append(mk("eta_across_midnight",
                [P(at(0, 9)), R(at(0, 18), 10)], at(0, 18),
                "10 % at 5 %/h = 2 h of use: 18-19 h at 0.8, then 0.1/h through the night, ends Tuesday morning"))
    c.append(mk("eta_across_days",
                [P(at(0, 9)), R(at(0, 18), 64)], at(0, 18),
                "64 % at 5 %/h = 12.8 h of use: ends on Wednesday"))
    c.append(mk("eta_beyond_seven_days",
                [P(at(0, 9)), R(at(0, 9), 100)], at(0, 9),
                "rated 100 h of use: 100 h of use do not fit in 7 days of default profile", use=100.0))
    for nm, level, use in [("band_minutes", 3.3, 20.0), ("band_half_hours", 12.5, 20.0),
                           ("band_hours", 70, 20.0), ("band_days", 100, 80.0)]:
        c.append(mk(nm, [P(at(0, 9)), R(at(0, 9), level)], at(0, 9),
                    "rounding band of hours left (%s %% at rated %g h)" % (level, use), use=use))

    # learned profile: online 09-12 every day for 10 days, Monday 10 days later at 08:00
    learned = [P(at(0, 9)), R(at(0, 9), 100), P(at(0, 12), False)]
    for d in range(1, 10):
        learned += [P(at(d, 9)), P(at(d, 12), False)]
    learned.append(P(at(10, 9)))
    c.append(mk("profile_learned_after_seven_days", learned, at(10, 10),
                "10 days of 09-12 h online: u is 1 from 9 to 12 h and 0 elsewhere (learned); 20 h of use left span 7 days"))
    c.append(mk("profile_default_before_seven_days",
                [P(at(0, 9)), R(at(0, 9), 60), P(at(0, 12), False), P(at(1, 9)), P(at(1, 12), False),
                 P(at(5, 9))], at(6, 8),
                "6 days of data: default profile"))

    # charge
    c.append(mk("charge_below_80",
                [P(at(0, 14)), R(at(0, 14, 32), 40, "c")], at(0, 14, 32),
                "prior 120/2.5 = 48 %/h: 40/48 + 20/24 = 100 min", device="G915 X"))
    c.append(mk("charge_above_80",
                [P(at(0, 14)), R(at(0, 14, 32), 92, "c")], at(0, 14, 32),
                "8 points at half of 48 %/h: 20 min", device="G915 X"))
    c.append(mk("charge_rate_learned_two_phase",
                [P(at(0, 14)), R(at(0, 14), 70, "c"), R(at(0, 15), 90, "c")], at(0, 15),
                "70->90 in 1 h counts 10 + 2*10 = 30 effective points: Rc blends to (30 + 3*48)/4 = 43.5"))
    c.append(mk("full_notice_once_per_segment",
                [P(at(0, 14)), R(at(0, 14), 90, "c"), R(at(0, 15), 100, "f"), R(at(0, 16), 100, "f"),
                 R(at(0, 17), 99), R(at(0, 18), 90), R(at(0, 19), 98, "c"), R(at(0, 20), 100, "c"),
                 R(at(0, 21), 100, "c")], at(0, 21),
                "one notice per charging segment; 100 % while charging counts", device="G915 X"))

    # usage
    c.append(mk("usage_split_across_midnight",
                [P(at(0, 23)), R(at(0, 23), 80), P(at(1, 1, 30), False), P(at(1, 22)), P(at(1, 23, 59), False)],
                at(2, 6),
                "23:00-01:30 splits 1 h / 1.5 h across the local midnight"))
    c.append(mk("usage_open_interval_counts_to_now",
                [P(at(0, 9)), R(at(0, 9), 80)], at(0, 13, 10),
                "still online: 4 h 10 min today"))
    c.append(mk("usage_kept_365_days",
                [P(at(0, 9)), P(at(0, 10), False), P(at(60, 9)), P(at(60, 11), False),
                 P(at(430, 9)), P(at(430, 9, 30), False)], at(430, 12),
                "the day 430 days later keeps; day 0 (430 days ago) is gone; day 60 (370 days ago) is gone"))
    c.append(mk("timezone_half_hour_offset",
                [P(epoch(2026, 10, 26, 23, 0, 19800)), P(epoch(2026, 10, 27, 1, 0, 19800), False)],
                epoch(2026, 10, 27, 12, 0, 19800),
                "UTC+5:30: the local midnight splits the interval 1 h / 1 h", off=19800))

    # health (cycles of integer points per hour, so the 50-point threshold is exact)
    evs, t = cycles_from(at(0, 0), [(10, 5)])
    c.append(mk("health_one_cycle_learning", [P(at(0, 0))] + evs, t + HR, "one cycle (fewer than 3): aprendendo"))
    evs, t = cycles_from(at(0, 0), [(10, 5)] * 5)
    c.append(mk("health_stable_five_cycles", [P(at(0, 0))] + evs, t + HR, "autonomy 20 h five times: 100 %"))
    evs, t = cycles_from(at(0, 0), [(15, 4)] * 3 + [(12, 5)] * 2)
    c.append(mk("health_wearing", [P(at(0, 0))] + evs, t + HR,
                "autonomy 25, 25, 25, 20, 20: last-3 median 20 over 25 = 80 %"))
    e1, t = cycles_from(at(0, 0), [(12, 5)] * 3)
    e2, t = cycles_from(t + 1200, [(6, 10)] * 2)
    rgb_on = [G(t_of(e1) + 600)]
    c.append(mk("health_two_rgb_settings_learning", [P(at(0, 0))] + e1 + rgb_on + e2, t + HR,
                "3 cycles with RGB off then 2 with RGB on: the latest setting has 2 cycles, aprendendo"))
    e3, t = cycles_from(t + 1200, [(7, 8)])
    c.append(mk("health_two_rgb_settings_compared_apart", [P(at(0, 0))] + e1 + rgb_on + e2 + e3, t + HR,
                "RGB on cycles 10, 10, 12.5 h autonomy compared among themselves, not with the RGB off ones: 100 %"))

    # warnings
    c.append(mk("smart_warning_fires",
                [P(at(0, 13, 50)), R(at(0, 14), 20)], at(0, 14),
                "4 h left < 1.1 x 4 h of expected use until 19 h: warns, 10 % reached at 16 h"))
    c.append(mk("smart_warning_margin_not_reached",
                [P(at(0, 13, 50)), R(at(0, 14), 22.1)], at(0, 14),
                "4.42 h left is above 1.1 x 4.0 = 4.4: no warning"))
    c.append(mk("smart_warning_once_per_day",
                [P(at(0, 13, 50)), R(at(0, 14), 20), R(at(0, 14, 5), 19.5), R(at(0, 15), 18),
                 R(at(1, 14), 17), R(at(1, 15), 16.5)], at(1, 15),
                "one warning on Monday, one on Tuesday"))
    c.append(mk("smart_warning_weekend_has_no_typical_day",
                [P(at(5, 13, 50)), R(at(5, 14), 10)], at(5, 14),
                "Saturday: u is 0.1 all day, no hour reaches 0.2: nothing expected, no warning"))
    c.append(mk("low_crossing_once_per_segment",
                [P(at(6, 20)), R(at(6, 20), 14), R(at(6, 21), 11), R(at(6, 22), 9), R(at(6, 23), 8),
                 R(at(7, 0), 60, "c"), R(at(7, 1), 40), R(at(7, 2), 9)], at(7, 2),
                "Sunday night (no smart warning): '<device> com 9 %' once per discharge segment"))

    # devices that give nothing
    c.append(mk("coarse_only_device",
                [P(at(0, 9)), R(at(0, 9), None, "d", est.COARSE), R(at(0, 12), None, "d", est.COARSE)], at(0, 12),
                "level only, no forecast, no notices", device="Headset"))
    c.append(mk("offline_whole_time",
                [P(at(0, 9), False), R(at(0, 9), 55), R(at(0, 12), 50)], at(0, 13),
                "never online: no use time, no observation, no forecast, no usage"))
    c.append(mk("no_events", [], at(0, 12), "nothing known"))

    # mockup sample devices (docs/design/harness/sample-state.js): its history, levels and states;
    # the text of the mockup is illustrative, the expected columns are the estimator's.
    c.append(sample_case("sample_g703", [(-168, 70), (-120, 14), (-117, 100), (-52, 18), (-49, 100), (0, 23)],
                         20.0, 2.5, "G703", note="mockup device G703, 7 days of history, 23 % discharging"))
    c.append(sample_case("sample_g915x", [(-168, 81), (-95, 22), (-92, 100), (-26, 33), (-1, 5), (0, 46)],
                         20.0, 2.5, "G915 X", note="mockup device G915 X, 46 % charging"))
    c.append(sample_case("sample_g733", [(-168, 60), (-120, 8), (-117, 100), (-50, 20), (-47, 100), (-0.5, 71)],
                         20.0, 2.5, "G733", last_offline_at=-0.5, note="mockup device G733, offline since 14:02 at 71 %"))
    c.append(sample_case("sample_xbox", [(-168, 30), (-140, 100), (-30, 75), (0, 64)],
                         20.0, 2.5, "Controle Xbox", note="mockup Controle Xbox, 64 % discharging"))
    c.append(sample_case("sample_airpods", [(-48, 100), (-46, 85), (-45, 100), (-1, 100), (0, 80)],
                         20.0, 2.5, "AirPods Pro", note="mockup AirPods Pro, 48 h of history: default profile"))
    return c


def render() -> str:
    lines = [HEADER.rstrip("\n"), "\t".join(COLUMNS)]
    names = set()
    for case in scenarios():
        assert case.name not in names, case.name
        names.add(case.name)
        cells = row(case)
        assert all("\t" not in x and "\n" not in x for x in cells), case.name
        lines.append("\t".join(cells))
    return "\n".join(lines) + "\n"


def main() -> None:
    data = render().encode("utf-8")
    with open(OUT_PATH, "wb") as f:
        f.write(data)
    print("wrote %s (%d bytes, %d scenarios)" % (os.path.normpath(OUT_PATH), len(data), len(scenarios())))


if __name__ == "__main__":
    main()
