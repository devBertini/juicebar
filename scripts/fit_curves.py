# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
"""Fits JuiceBar's own voltage-to-percent curves (plan stage K1) from the calibration the app wrote.

  python3 -B scripts/fit_curves.py [--dir DIR] [--curves FILE]          print the fitted rows, change nothing
  python3 -B scripts/fit_curves.py [--dir DIR] [--curves FILE] --write  replace those rows in spec/curves.tsv

The app appends one line `<unix seconds>\\t<model>\\t<millivolts>\\t<ghub percent>\\t<charging 0/1>` per device per ten
minutes to `%LOCALAPPDATA%\\JuiceBar\\calibration\\<model>.tsv` (the default --dir) for two weeks. For each model this
script drops the charging rows, fits a rising curve of percent over millivolts by isotonic regression (pool adjacent
violators), reduces it to at most 12 knots, and prints the rows with the measured range and the largest error against
the points. The full-charge end (4200 mV = 100) is added only when no measurement is above it; when the lowest
measurement is above 0 %, the curve's provisional empty point already in curves.tsv is kept below it.
`--write` refuses a model with fewer than 15 points, and a curve that would not run from 0 to 100 (scripts/check_spec.py
would fail on it). Python 3.9 standard library only.
"""
import argparse
import datetime
import os
import sys

MAX_KNOTS = 12
MIN_POINTS = 15
FULL_MV = 4200
# A knot is added while the line through the knots so far misses a fitted point by more than this many points.
TOLERANCE = 0.5
HEADER = "curve\tmillivolts\tpercent\tsource"
EMPTY_NOTE = "provisional empty point"
FULL_NOTE = "full charge of a lithium cell"


def read_points(path):
    """The usable lines of one calibration file as (unix, millivolts, percent, charging); other lines are skipped."""
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            cells = line.rstrip("\r\n").split("\t")
            if len(cells) != 5:
                continue
            try:
                at, mv, pct = int(cells[0]), int(cells[2]), float(cells[3])
                charging = cells[4].strip() == "1"
            except ValueError:
                continue
            if not (2500 <= mv <= 4500 and 0.0 <= pct <= 100.0):
                continue
            out.append((at, mv, pct, charging))
    return out


def read_dir(directory):
    """{model: points} for every <model>.tsv in the folder."""
    models = {}
    for name in sorted(os.listdir(directory)):
        if name.endswith(".tsv"):
            models[name[:-4]] = read_points(os.path.join(directory, name))
    return models


def isotonic(points):
    """Pool-adjacent-violators on (millivolts, percent): the non-decreasing percent per distinct millivolts that
    is closest to the points in least squares. Returns [(millivolts, fitted percent)] sorted by millivolts."""
    groups = {}
    for mv, pct in points:
        total, count = groups.get(mv, (0.0, 0))
        groups[mv] = (total + pct, count + 1)
    blocks = []  # each: [sum, weight, [millivolts...]]
    for mv in sorted(groups):
        total, count = groups[mv]
        blocks.append([total, count, [mv]])
        while len(blocks) > 1 and blocks[-2][0] / blocks[-2][1] > blocks[-1][0] / blocks[-1][1]:
            last = blocks.pop()
            blocks[-1][0] += last[0]
            blocks[-1][1] += last[1]
            blocks[-1][2].extend(last[2])
    out = []
    for total, weight, mvs in blocks:
        for mv in mvs:
            out.append((mv, total / weight))
    return out


def interpolate(knots, mv):
    """Percent at `mv` on the piecewise-linear curve through the knots, clamped to the end values."""
    if mv <= knots[0][0]:
        return knots[0][1]
    if mv >= knots[-1][0]:
        return knots[-1][1]
    for (x0, y0), (x1, y1) in zip(knots, knots[1:]):
        if x0 <= mv <= x1:
            return y0 + (y1 - y0) * (mv - x0) / (x1 - x0)
    return knots[-1][1]


def reduce_knots(curve, max_knots=MAX_KNOTS):
    """At most `max_knots` of the curve's points, the two ends always kept, chosen by adding the point the line
    through the chosen ones misses the most (Douglas-Peucker order). A subset of a rising curve rises."""
    if len(curve) <= 2:
        return list(curve)
    chosen = [0, len(curve) - 1]
    while len(chosen) < max_knots:
        knots = [curve[i] for i in sorted(chosen)]
        worst, at = TOLERANCE, None
        for i, (mv, pct) in enumerate(curve):
            if i in chosen:
                continue
            miss = abs(interpolate(knots, mv) - pct)
            if miss > worst:
                worst, at = miss, i
        if at is None:
            break
        chosen.append(at)
    return [curve[i] for i in sorted(chosen)]


def _pretty(value):
    text = "%.1f" % value
    return text[:-2] if text.endswith(".0") else text


def _day(unix):
    return datetime.datetime.fromtimestamp(unix, datetime.timezone.utc).strftime("%Y-%m-%d")


class Fit(object):
    """The result for one model: `rows` are (millivolts, percent, source) ready for curves.tsv."""

    def __init__(self, model, used, dropped, rows, mv_range, pct_range, max_error, problems):
        self.model = model
        self.used = used
        self.dropped = dropped
        self.rows = rows
        self.mv_range = mv_range
        self.pct_range = pct_range
        self.max_error = max_error
        self.problems = problems


def fit_model(model, points, existing=None, max_knots=MAX_KNOTS):
    """Fits one model. `points` are the calibration tuples; `existing` the (millivolts, percent) pairs now in
    curves.tsv for it, whose empty point (0 %) is kept when the measurements do not reach 0 %."""
    kept = [p for p in points if not p[3]]
    dropped = len(points) - len(kept)
    problems = []
    if not kept:
        return Fit(model, 0, dropped, [], None, None, None, ["no point left after dropping the charging rows"])
    data = [(p[1], p[2]) for p in kept]
    curve = isotonic(data)
    lo_mv, hi_mv = curve[0][0], curve[-1][0]
    lo_pct, hi_pct = min(p[1] for p in data), max(p[1] for p in data)
    source = "fitted from %d G HUB readings, %s to %s" % (
        len(kept), _day(min(p[0] for p in kept)), _day(max(p[0] for p in kept)))
    anchors_low = []
    if curve[0][1] > 0:
        for mv, pct in sorted(existing or []):
            if pct == 0 and mv < lo_mv:
                anchors_low = [(mv, 0.0)]
        if not anchors_low:
            problems.append("the lowest measurement is %s %% and curves.tsv has no empty point below %d mV"
                            % (_pretty(curve[0][1]), lo_mv))
    anchors_high = []
    if hi_mv < FULL_MV:
        anchors_high = [(FULL_MV, 100.0)]
    elif curve[-1][1] < 100:
        problems.append("the highest measurement is %s %%, not 100" % _pretty(curve[-1][1]))
    full = anchors_low + curve + anchors_high
    knots = reduce_knots(full, max_knots)
    knots = [(mv, round(pct, 1)) for mv, pct in knots]
    rows = []
    for mv, pct in knots:
        if (mv, pct) in anchors_low:
            note = EMPTY_NOTE
        elif (mv, pct) in anchors_high:
            note = FULL_NOTE
        else:
            note = source
        rows.append((mv, pct, note))
    max_error = max(abs(interpolate(knots, mv) - pct) for mv, pct in data)
    return Fit(model, len(kept), dropped, rows, (lo_mv, hi_mv), (lo_pct, hi_pct), max_error, problems)


def valid(rows):
    """Why a curve cannot go into curves.tsv (the rules of scripts/check_spec.py), or None."""
    if not rows:
        return "no rows"
    if rows[0][1] != 0 or rows[-1][1] != 100:
        return "the curve must run from 0 to 100"
    for (m0, p0, _), (m1, p1, _) in zip(rows, rows[1:]):
        if not (m1 > m0 and p1 >= p0):
            return "the curve must rise with millivolts"
    return None


def report(fit, curve_id=None):
    """The text printed for one model."""
    if not fit.rows:
        return "%s: nothing to fit (%s)" % (fit.model, "; ".join(fit.problems))
    lines = [
        "%s: %d points (%d charging rows dropped), measured %d to %d mV and %s to %s %%, %d knots, "
        "largest error against the points %.1f percentage points"
        % (fit.model, fit.used, fit.dropped, fit.mv_range[0], fit.mv_range[1],
           _pretty(fit.pct_range[0]), _pretty(fit.pct_range[1]), len(fit.rows), fit.max_error)
    ]
    for problem in fit.problems:
        lines.append("  warning: " + problem)
    lines.append(HEADER)
    for mv, pct, note in fit.rows:
        lines.append("%s\t%d\t%s\t%s" % (curve_id or fit.model, mv, _pretty(pct), note))
    return "\n".join(lines)


def existing_curves(text):
    """{curve: [(millivolts, percent)]} from the text of curves.tsv."""
    out = {}
    for line in text.splitlines():
        if not line or line.startswith("#") or line.startswith("curve\t"):
            continue
        cells = line.split("\t")
        if len(cells) < 3:
            continue
        try:
            out.setdefault(cells[0], []).append((int(cells[1]), float(cells[2])))
        except ValueError:
            continue
    return out


def replace_rows(text, fits):
    """The text of curves.tsv with each fitted model's rows in place of its old ones (appended when it had none).
    The comment lines are kept as they are."""
    lines = text.split("\n")
    for model, rows in fits:
        new = ["%s\t%d\t%s\t%s" % (model, mv, _pretty(pct), note) for mv, pct, note in rows]
        at = [i for i, l in enumerate(lines) if l.startswith(model + "\t")]
        if at:
            first = at[0]
            lines = [l for i, l in enumerate(lines) if i not in set(at)]
            lines[first:first] = new
        else:
            end = len(lines)
            while end > 0 and lines[end - 1] == "":
                end -= 1
            lines[end:end] = new
    return "\n".join(lines)


def run(directory, curves_path, write, out=sys.stdout):
    """Returns the exit status: 0, or 1 when --write refused a model or the folder holds nothing."""
    if not os.path.isdir(directory):
        out.write("no calibration folder at %s\n" % directory)
        return 1
    models = read_dir(directory)
    if not models:
        out.write("no calibration file in %s\n" % directory)
        return 1
    with open(curves_path, encoding="utf-8", newline="") as f:
        text = f.read()
    current = existing_curves(text)
    status = 0
    accepted = []
    for model in sorted(models):
        fit = fit_model(model, models[model], current.get(model))
        out.write(report(fit) + "\n")
        if not write:
            continue
        if fit.used < MIN_POINTS:
            out.write("refused %s: %d points, %d needed\n" % (model, fit.used, MIN_POINTS))
            status = 1
            continue
        why = valid(fit.rows) or ("; ".join(fit.problems) if fit.problems else None)
        if why:
            out.write("refused %s: %s\n" % (model, why))
            status = 1
            continue
        accepted.append((model, fit.rows))
    if write and accepted:
        eol = "\r\n" if "\r\n" in text else "\n"
        new = replace_rows(text.replace("\r\n", "\n"), accepted).replace("\n", eol)
        with open(curves_path, "w", encoding="utf-8", newline="") as f:
            f.write(new)
        out.write("wrote %s: %s\n" % (curves_path, ", ".join(m for m, _ in accepted)))
        out.write("update the INTERIM comment lines at the top of curves.tsv, then run scripts/check_spec.py\n")
    return status


def main(argv=None):
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    base = os.environ.get("LOCALAPPDATA")
    default_dir = os.path.join(base, "JuiceBar", "calibration") if base else None
    ap = argparse.ArgumentParser(description="Fit JuiceBar's voltage curves from the calibration files.")
    ap.add_argument("--dir", default=default_dir, help="folder of <model>.tsv files (default: the app's calibration folder)")
    ap.add_argument("--curves", default=os.path.join(here, "spec", "curves.tsv"))
    ap.add_argument("--write", action="store_true", help="replace the fitted models' rows in curves.tsv")
    args = ap.parse_args(argv)
    if not args.dir:
        ap.error("--dir is needed (LOCALAPPDATA is not set)")
    return run(args.dir, args.curves, args.write)


if __name__ == "__main__":
    sys.exit(main())
