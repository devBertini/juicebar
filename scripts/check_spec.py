# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
"""Checks that the shared contract in spec/ is well formed, so both apps' cores can read it.

  python3 -B scripts/check_spec.py        (exit 0 when every file passes; one line per problem otherwise)

Checks: devices.tsv columns and values (the short name every shown device needs), every curve it names exists in
curves.tsv, curves sorted and rising, estimator-cases.tsv has its header and the same number of cells on every row, every fixture line is a note or a
whole HID++ report of a known length. Python 3.9 standard library only.
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SPEC = os.path.join(ROOT, "spec")
KINDS = {"mouse", "keyboard", "headset", "earbuds", "gamepad", "other"}
MATCHES = {"receiver", "dongle", "wpid", "usb", "sony", "microsoft"}
BATTERIES = {"-", "hidpp-0x1000", "hidpp-0x1001", "hidpp-0x1004", "hidpp-0x1f20", "hid-ds5", "wgi"}
VENDOR_MATCHES = {"sony", "microsoft"}
PAD_BATTERIES = {"hid-ds5", "wgi"}
REPORT_LENGTHS = {0x10: 7, 0x11: 20, 0x12: 64, 0x20: 15}


def rows(path):
    """The non-comment rows of a TSV as lists of cells, with their line numbers."""
    out = []
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            out.append((n, line.split("\t")))
    return out


def check_curves(problems):
    curves = {}
    data = rows(os.path.join(SPEC, "curves.tsv"))
    if not data or data[0][1] != ["curve", "millivolts", "percent", "source"]:
        problems.append("curves.tsv: header must be curve, millivolts, percent, source")
        return curves
    for n, cells in data[1:]:
        if len(cells) != 4:
            problems.append("curves.tsv:%d: 4 cells expected, %d found" % (n, len(cells)))
            continue
        try:
            mv, pct = int(cells[1]), float(cells[2])
        except ValueError:
            problems.append("curves.tsv:%d: millivolts and percent must be numbers" % n)
            continue
        curves.setdefault(cells[0], []).append((n, mv, pct))
    for name, points in curves.items():
        for (n0, mv0, p0), (n1, mv1, p1) in zip(points, points[1:]):
            if not (mv1 > mv0 and p1 >= p0):
                problems.append("curves.tsv:%d: curve %s must rise with millivolts" % (n1, name))
        if points and not (points[0][2] == 0 and points[-1][2] == 100):
            problems.append("curves.tsv: curve %s must run from 0 to 100" % name)
    return curves


def check_devices(curves, problems):
    data = rows(os.path.join(SPEC, "devices.tsv"))
    header = ["provider", "match", "id", "model", "kind", "battery", "curve", "use_h", "use_h_rgb", "charge_h", "source", "short"]
    if not data or data[0][1] != header:
        problems.append("devices.tsv: header must be " + ", ".join(header))
        return
    seen = set()
    for n, c in data[1:]:
        where = "devices.tsv:%d" % n
        if len(c) != len(header):
            problems.append("%s: %d cells expected, %d found" % (where, len(header), len(c)))
            continue
        if c[1] not in MATCHES:
            problems.append("%s: match %r unknown" % (where, c[1]))
        if not re.fullmatch(r"[0-9A-F]{4}", c[2]):
            problems.append("%s: id must be four upper-case hex digits" % where)
        if (c[1], c[2]) in seen:
            problems.append("%s: %s %s listed twice" % (where, c[1], c[2]))
        seen.add((c[1], c[2]))
        if c[4] not in KINDS:
            problems.append("%s: kind %r unknown" % (where, c[4]))
        if c[5] not in BATTERIES:
            problems.append("%s: battery %r unknown" % (where, c[5]))
        if (c[0] == "game-controllers") != (c[1] in VENDOR_MATCHES):
            problems.append("%s: match sony and microsoft belong to game-controllers, and only to it" % where)
        if (c[0] == "game-controllers") != (c[5] in PAD_BATTERIES):
            problems.append("%s: battery hid-ds5 and wgi belong to game-controllers, and only to it" % where)
        if c[0] == "game-controllers" and c[4] != "gamepad":
            problems.append("%s: a game-controllers row must be kind gamepad" % where)
        if c[5] in ("hidpp-0x1001", "hidpp-0x1f20") and c[6] not in curves:
            problems.append("%s: a voltage device needs a curve in curves.tsv (%r)" % (where, c[6]))
        for col in (7, 8, 9):
            if c[col] != "-" and not re.fullmatch(r"\d+(\.\d+)?", c[col]):
                problems.append("%s: %s must be a number or -" % (where, header[col]))
        short = c[11]
        if c[1] == "receiver":
            if short != "-":
                problems.append("%s: a receiver is never shown, its short name must be -" % where)
        elif not short.strip() or short == "-" or short != short.strip() or len(short) > 16:
            problems.append("%s: short must be a name of 1 to 16 characters, no edge spaces (and never -)" % where)


def check_cases(problems):
    path = os.path.join(SPEC, "estimator-cases.tsv")
    data = rows(path)
    if len(data) < 2:
        problems.append("estimator-cases.tsv: a header and at least one case expected")
        return
    width = len(data[0][1])
    for n, cells in data[1:]:
        if len(cells) != width:
            problems.append("estimator-cases.tsv:%d: %d cells expected, %d found" % (n, width, len(cells)))


def check_fixtures(problems):
    folder = os.path.join(SPEC, "fixtures")
    for name in sorted(os.listdir(folder)):
        if not name.endswith(".hidtrace"):
            continue
        with open(os.path.join(folder, name), encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                m = re.match(r"\s*\d+ ([<>#])\s?(.*)$", line.rstrip("\n"))
                if not m:
                    problems.append("%s:%d: not '<ms> <|>|# ...'" % (name, n))
                    continue
                if m.group(1) == "#":
                    continue
                text = m.group(2).strip()
                if not re.fullmatch(r"(?:[0-9a-fA-F]{2})+", text):
                    problems.append("%s:%d: a frame must be hexadecimal bytes" % (name, n))
                    continue
                frame = bytes.fromhex(text)
                want = REPORT_LENGTHS.get(frame[0])
                if want is not None and len(frame) != want:
                    problems.append("%s:%d: report 0x%02x must be %d bytes, has %d" % (name, n, frame[0], want, len(frame)))


def main():
    problems = []
    curves = check_curves(problems)
    check_devices(curves, problems)
    check_cases(problems)
    check_fixtures(problems)
    for p in problems:
        print(p)
    print("check_spec: %d problem(s)" % len(problems))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
