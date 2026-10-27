# Copyright (c) 2026 Claudio Bertini (devBertini)
# SPDX-License-Identifier: GPL-3.0-or-later
"""Tests of scripts/fit_curves.py with synthetic data.  python3 -B scripts/test_fit_curves.py"""
import io
import os
import random
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fit_curves as fc  # noqa: E402

T0 = 1793035800  # 2026-10-26


def synthetic(n, lo=3600, hi=4150, seed=1, noise=1.5):
    """n discharge readings: percent follows a smooth rising curve of millivolts, with a little noise."""
    rnd = random.Random(seed)
    out = []
    for i in range(n):
        mv = rnd.randint(lo, hi)
        base = 100.0 * ((mv - 3500) / 700.0) ** 1.6
        pct = min(100.0, max(0.0, round(base + rnd.uniform(-noise, noise))))
        out.append((T0 + 600 * i, mv, pct, False))
    return out


def rising(rows):
    return all(m1 > m0 and p1 >= p0 for (m0, p0, _), (m1, p1, _) in zip(rows, rows[1:]))


class Reading(unittest.TestCase):
    def test_lines_are_read_and_bad_ones_skipped(self):
        d = tempfile.mkdtemp()
        try:
            with open(os.path.join(d, "g703.tsv"), "w", encoding="utf-8") as f:
                f.write("1793035800\tg703\t3968\t77\t0\n")
                f.write("1793036400\tg703\t3960\t76.5\t1\n")
                f.write("bad line\n")
                f.write("1\tg703\t9999\t50\t0\n")  # implausible voltage
                f.write("1\tg703\t3900\t140\t0\n")  # implausible percent
                f.write("1\tg703\t3900\t50\n")  # four fields
            points = fc.read_dir(d)["g703"]
            self.assertEqual(points, [(1793035800, 3968, 77.0, False), (1793036400, 3960, 76.5, True)])
        finally:
            shutil.rmtree(d)


class Isotonic(unittest.TestCase):
    def test_violators_are_pooled_and_the_result_never_falls(self):
        fitted = fc.isotonic([(3700, 10), (3800, 30), (3900, 20), (4000, 60), (3900, 40)])
        self.assertEqual([mv for mv, _ in fitted], [3700, 3800, 3900, 4000])
        self.assertEqual(fitted[0][1], 10)
        self.assertAlmostEqual(fitted[1][1], 30)
        self.assertAlmostEqual(fitted[2][1], 30)  # the two readings at 3900 average (20 + 40) / 2
        self.assertEqual(fitted[3][1], 60)
        fitted = fc.isotonic([(3700, 50), (3800, 20), (3900, 80)])
        self.assertAlmostEqual(fitted[0][1], 35)  # 50 above 20: pooled
        self.assertAlmostEqual(fitted[1][1], 35)
        values = [p for _, p in fc.isotonic([(m, random.Random(m).uniform(0, 100)) for m in range(3600, 4100, 7)])]
        self.assertTrue(all(b >= a for a, b in zip(values, values[1:])))


class Fitting(unittest.TestCase):
    def test_output_rises_stays_within_twelve_knots_and_reports_its_error(self):
        points = synthetic(400)
        fit = fc.fit_model("g703", points, [(3500, 0.0)])
        self.assertTrue(rising(fit.rows), fit.rows)
        self.assertLessEqual(len(fit.rows), 12)
        self.assertGreaterEqual(len(fit.rows), 4)
        self.assertEqual(fit.rows[0][1], 0)
        self.assertEqual(fit.rows[-1][:2], (4200, 100.0))
        self.assertEqual(fit.problems, [])
        self.assertIsNone(fc.valid(fit.rows))
        # The error is what it says: the largest miss of the knots' line against the raw points.
        knots = [(m, p) for m, p, _ in fit.rows]
        miss = max(abs(fc.interpolate(knots, p[1]) - p[2]) for p in points)
        self.assertAlmostEqual(fit.max_error, miss)
        self.assertLess(fit.max_error, 4.0)
        self.assertEqual(fit.mv_range[0], min(p[1] for p in points))
        self.assertEqual(fit.mv_range[1], max(p[1] for p in points))
        text = fc.report(fit)
        self.assertIn("400 points", text)
        self.assertIn("largest error", text)
        self.assertIn("measured %d to %d mV" % fit.mv_range, text)

    def test_the_knot_limit_holds_for_a_ragged_curve(self):
        # A staircase with 40 steps needs far more than 12 knots to follow exactly.
        stairs = [(T0 + i, 3600 + 10 * i, float(i * 2 + (i % 2) * 3), False) for i in range(40)]
        fit = fc.fit_model("g733", stairs, [(3400, 0.0)])
        self.assertEqual(len(fit.rows), 12)
        self.assertTrue(rising(fit.rows))
        few = fc.fit_model("g733", stairs, [(3400, 0.0)], max_knots=5)
        self.assertEqual(len(few.rows), 5)
        self.assertLess(fit.max_error, 3.0)
        # A straight line needs two knots (plus the anchors the end rules add).
        line = [(T0 + i, 3600 + 10 * i, float(i), False) for i in range(1, 40)]
        self.assertLess(len(fc.fit_model("x", line, [(3400, 0.0)]).rows), 6)

    def test_charging_rows_are_dropped(self):
        points = synthetic(100)
        wild = [(T0 + 99999 + i, 3700 + i, 100.0, True) for i in range(30)]
        clean = fc.fit_model("g703", points, [(3500, 0.0)])
        mixed = fc.fit_model("g703", points + wild, [(3500, 0.0)])
        self.assertEqual(mixed.dropped, 30)
        self.assertEqual(mixed.used, 100)
        self.assertEqual(mixed.rows, clean.rows)
        self.assertIn("30 charging rows dropped", fc.report(mixed))
        only_charging = fc.fit_model("g703", wild, None)
        self.assertEqual(only_charging.rows, [])
        self.assertIn("nothing to fit", fc.report(only_charging))

    def test_the_full_end_is_added_only_when_nothing_is_measured_above_it(self):
        below = fc.fit_model("a", synthetic(200, hi=4150), [(3500, 0.0)])
        self.assertEqual(below.rows[-1][0], 4200)
        self.assertEqual(below.rows[-1][2], fc.FULL_NOTE)
        above = [(T0 + i, 3600 + 3 * i, min(100.0, i * 0.45), False) for i in range(250)]  # reaches 4347 mV
        high = fc.fit_model("a", above, [(3500, 0.0)])
        self.assertGreater(max(p[1] for p in above), 4200)
        self.assertFalse(any(note == fc.FULL_NOTE for _, _, note in high.rows))
        self.assertEqual(high.rows[-1][0], max(p[1] for p in above))
        self.assertEqual(high.rows[-1][1], 100.0)
        self.assertIsNone(fc.valid(high.rows))

    def test_the_empty_point_comes_from_curves_tsv_or_the_fit_says_it_is_missing(self):
        points = synthetic(100, lo=3750)  # nothing near empty
        kept = fc.fit_model("g703", points, [(3500, 0.0), (3968, 77.0), (4200, 100.0)])
        self.assertEqual(kept.rows[0], (3500, 0.0, fc.EMPTY_NOTE))
        self.assertEqual(kept.problems, [])
        missing = fc.fit_model("g703", points, None)
        self.assertEqual(len(missing.problems), 1)
        self.assertIsNotNone(fc.valid(missing.rows))
        # An empty point that is not below the measurements is not used.
        too_high = fc.fit_model("g703", points, [(3800, 0.0)])
        self.assertEqual(len(too_high.problems), 1)

    def test_equal_voltages_with_different_percents_are_averaged_and_counted_as_error(self):
        points = [(T0, 3700, 40.0, False), (T0 + 1, 3700, 50.0, False)]
        points += [(T0 + 2 + i, 3800 + 20 * i, 55.0 + 5 * i, False) for i in range(8)]
        fit = fc.fit_model("a", points, [(3500, 0.0)])
        knots = [(m, p) for m, p, _ in fit.rows]
        self.assertAlmostEqual(fc.interpolate(knots, 3700), 45.0, places=0)
        self.assertGreaterEqual(fit.max_error, 5.0 - 0.1)


class Writing(unittest.TestCase):
    CURVES = (
        "# header comment\n"
        "curve\tmillivolts\tpercent\tsource\n"
        "g703\t3500\t0\tprovisional empty point\n"
        "g703\t3968\t77\tmeasured\n"
        "g703\t4200\t100\tfull charge of a lithium cell\n"
        "g733\t3400\t0\tprovisional empty point\n"
        "g733\t3753\t45\tmeasured\n"
        "g733\t4200\t100\tfull charge of a lithium cell\n"
    )

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.curves = os.path.join(self.dir, "curves.tsv")
        self.cal = os.path.join(self.dir, "cal")
        os.mkdir(self.cal)
        with open(self.curves, "w", encoding="utf-8", newline="") as f:
            f.write(self.CURVES)

    def tearDown(self):
        shutil.rmtree(self.dir)

    def put(self, model, points):
        with open(os.path.join(self.cal, model + ".tsv"), "w", encoding="utf-8") as f:
            for at, mv, pct, charging in points:
                f.write("%d\t%s\t%d\t%s\t%d\n" % (at, model, mv, fc._pretty(pct), 1 if charging else 0))

    def read(self):
        with open(self.curves, encoding="utf-8", newline="") as f:
            return f.read()

    def test_print_only_changes_nothing(self):
        self.put("g703", synthetic(60))
        out = io.StringIO()
        self.assertEqual(fc.run(self.cal, self.curves, False, out), 0)
        self.assertEqual(self.read(), self.CURVES)
        self.assertIn("g703\t", out.getvalue())
        self.assertIn("curve\tmillivolts\tpercent\tsource", out.getvalue())

    def test_write_refuses_a_model_under_15_points_and_replaces_the_others(self):
        self.put("g703", synthetic(200))
        self.put("g733", synthetic(14, seed=3))
        out = io.StringIO()
        status = fc.run(self.cal, self.curves, True, out)
        self.assertEqual(status, 1)
        self.assertIn("refused g733: 14 points, 15 needed", out.getvalue())
        text = self.read()
        # g733 keeps its old rows, g703 has new ones, the comment and the header stay.
        self.assertTrue(text.startswith("# header comment\ncurve\tmillivolts\tpercent\tsource\n"))
        self.assertIn("g733\t3753\t45\tmeasured\n", text)
        self.assertNotIn("g703\t3968\t77\tmeasured", text)
        rows = [l.split("\t") for l in text.splitlines() if l.startswith("g703\t")]
        self.assertGreaterEqual(len(rows), 4)
        self.assertLessEqual(len(rows), 12)
        mvs = [int(r[1]) for r in rows]
        pcts = [float(r[2]) for r in rows]
        self.assertEqual(mvs, sorted(set(mvs)))
        self.assertEqual(pcts, sorted(pcts))
        self.assertEqual((pcts[0], pcts[-1]), (0.0, 100.0))
        # The old rows of g703 were replaced where they stood: g703 still comes before g733.
        self.assertLess(text.index("g703\t"), text.index("g733\t"))

    def test_exactly_15_points_are_enough_and_14_after_dropping_charging_rows_are_not(self):
        points = synthetic(15, seed=5)
        self.put("g703", points)
        self.assertEqual(fc.run(self.cal, self.curves, True, io.StringIO()), 0)
        self.assertNotIn("g703\t3968\t77\tmeasured", self.read())
        with open(self.curves, "w", encoding="utf-8", newline="") as f:
            f.write(self.CURVES)
        self.put("g703", points + [(T0 + 1, 3800, 90.0, True)] * 5)
        self.put("g703", points[:14] + [(T0 + 1, 3800, 90.0, True)] * 5)
        out = io.StringIO()
        self.assertEqual(fc.run(self.cal, self.curves, True, out), 1)
        self.assertIn("refused g703: 14 points", out.getvalue())
        self.assertEqual(self.read(), self.CURVES)

    def test_a_curve_that_cannot_run_from_0_to_100_is_refused(self):
        # Only mid-range readings and no empty point to keep: the fit has no 0 % end.
        self.put("g999", synthetic(40, lo=3800, hi=4000))
        out = io.StringIO()
        self.assertEqual(fc.run(self.cal, self.curves, True, out), 1)
        self.assertIn("refused g999", out.getvalue())
        self.assertEqual(self.read(), self.CURVES)

    def test_a_new_model_is_appended_and_crlf_files_keep_their_line_ends(self):
        with open(self.curves, "w", encoding="utf-8", newline="") as f:
            f.write(self.CURVES.replace("\n", "\r\n"))
        self.put("g999", synthetic(80, lo=3500))
        self.assertEqual(fc.run(self.cal, self.curves, True, io.StringIO()), 0)
        text = self.read()
        self.assertNotIn("\n", text.replace("\r\n", ""))
        self.assertIn("g733\t3753\t45\tmeasured\r\n", text)
        self.assertIn("g703\t3968\t77\tmeasured\r\n", text)
        self.assertTrue(text.rstrip("\r\n").splitlines()[-1].startswith("g999\t"))

    def test_a_missing_folder_or_an_empty_one_is_reported(self):
        out = io.StringIO()
        self.assertEqual(fc.run(os.path.join(self.dir, "nope"), self.curves, False, out), 1)
        self.assertIn("no calibration folder", out.getvalue())
        self.assertEqual(fc.run(self.cal, self.curves, False, out), 1)
        self.assertIn("no calibration file", out.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=1)
