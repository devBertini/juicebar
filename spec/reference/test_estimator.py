# Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
"""Tests of the reference estimator: every rule of spec/estimator.md and its edges.

    python3 -B spec/reference/test_estimator.py
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import estimator as e  # noqa: E402
import make_cases  # noqa: E402

HR = 3600
OFF = -10800
# Monday 2026-10-26 00:00 local (UTC-3)
MON = make_cases.epoch(2026, 10, 26)


def at(day, hour=0, minute=0):
    """Local time, `day` days after Monday 26/10/2026."""
    return MON + day * e.DAY + hour * HR + minute * 60


def hist(**kw):
    kw.setdefault("name", "G703")
    return e.DeviceHistory(**kw)


def rd(h, t, pct, state=e.DISCHARGING, quality=e.EXACT):
    return h.add_reading(e.Reading(t, pct, state, quality))


def online(h, t, on=True):
    return h.add_presence(e.Presence(t, on))


def started(t0, pct=None, **kw):
    """A history online since t0, optionally with a first reading."""
    h = hist(**kw)
    online(h, t0)
    if pct is not None:
        rd(h, t0, pct)
    return h


class Calendar(unittest.TestCase):
    def test_monday_is_weekday_zero(self):
        local = at(0, 12) + OFF
        self.assertEqual(e.weekday_of_day(local // e.DAY), 0)
        self.assertEqual(e.hour_of_week(local), 12)
        self.assertEqual(e.hour_of_week(at(6, 23) + OFF), 6 * 24 + 23)

    def test_round_half_up(self):
        self.assertEqual([e.round_half_up(x) for x in (0.5, 1.5, 2.5, 2.49)], [1, 2, 3, 2])


class DischargeRate(unittest.TestCase):
    def test_first_reading_gives_only_the_prior(self):
        h = started(at(0, 9), 80)
        self.assertIsNone(h.R)
        self.assertEqual(h.w, 0.0)
        self.assertAlmostEqual(e.discharge_rate(h), 5.0)

    def test_first_observation_sets_rate_and_blends_with_prior(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 10), 76)
        self.assertAlmostEqual(h.R, 4.0)
        self.assertAlmostEqual(h.w, 1.0)
        self.assertAlmostEqual(e.discharge_rate(h), (1 * 4.0 + 3 * 5.0) / 4)

    def test_unknown_model_defaults_to_20_hours(self):
        self.assertAlmostEqual(e.discharge_rate(hist()), 5.0)
        self.assertAlmostEqual(e.charge_rate(hist()), 48.0)    # 120 / 2.5

    def test_model_prior_comes_from_rated_hours(self):
        self.assertAlmostEqual(e.discharge_rate(hist(rated_use_hours=40.0)), 2.5)
        self.assertAlmostEqual(e.charge_rate(hist(rated_charge_hours=2.0)), 60.0)   # 120 / 2

    def test_one_point_steps_are_ignored(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 10), 79)
        self.assertIsNone(h.R)
        rd(h, at(0, 11), 79)
        self.assertIsNone(h.R)
        rd(h, at(0, 12), 78)   # 2 points below the anchor, over 3 h
        self.assertAlmostEqual(h.R, 2.0 / 3.0)
        self.assertAlmostEqual(h.w, 3.0)

    def test_minimum_quarter_hour(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 9, 14), 78)   # 14 min: no observation
        self.assertIsNone(h.R)
        rd(h, at(0, 9, 30), 77)   # the anchor stayed: 3 points in 0.5 h
        self.assertAlmostEqual(h.R, 6.0)
        h = started(at(0, 9), 80)
        rd(h, at(0, 9, 15), 78)   # exactly 15 min
        self.assertAlmostEqual(h.R, 8.0)
        self.assertAlmostEqual(h.w, 0.25)

    def test_exponentially_weighted_mean_over_use_time(self):
        h = started(at(0, 9), 100)
        rd(h, at(0, 10), 96)      # r = 4 over 1 h
        rd(h, at(0, 16), 84)      # r = 2 over 6 h, a = 1 - 2^-1 = 0.5
        self.assertAlmostEqual(h.R, 3.0)
        self.assertAlmostEqual(h.w, 7.0)
        self.assertAlmostEqual(e.discharge_rate(h), (7 * 3.0 + 15.0) / 10)

    def test_use_time_excludes_offline_time(self):
        h = started(at(0, 9), 100)
        online(h, at(0, 10), False)
        online(h, at(0, 18))
        rd(h, at(0, 20), 90)      # 3 h online between the anchor and now
        self.assertAlmostEqual(h.R, 10.0 / 3.0)

    def test_no_use_time_means_no_observation(self):
        h = hist()
        online(h, at(0, 9), False)
        rd(h, at(0, 9), 80)
        rd(h, at(0, 20), 50)
        self.assertIsNone(h.R)
        self.assertAlmostEqual(e.discharge_rate(h), 5.0)

    def test_weight_is_capped_at_12(self):
        h = started(at(0, 0), 100)
        for i in range(1, 6):
            rd(h, at(0, 0) + i * 10 * HR, 100 - 8 * i)
        self.assertEqual(h.w, 12.0)
        self.assertAlmostEqual(h.R, 0.8)
        self.assertAlmostEqual(e.discharge_rate(h), (12 * 0.8 + 15.0) / 15)

    def test_label_until_three_hours_of_use(self):
        h = started(at(0, 9), 100)
        online_ok = e.forecast_label(h)
        self.assertEqual(online_ok, "estimativa inicial")
        rd(h, at(0, 11), 96)
        self.assertEqual(e.forecast_label(h), "estimativa inicial")   # w = 2
        rd(h, at(0, 12), 94)
        self.assertEqual(h.w, 3.0)
        self.assertIsNone(e.forecast_label(h))                        # w = 3: leaves

    def test_charge_breaks_the_segment(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 10), 78)                       # r = 2
        rd(h, at(0, 11), 100, e.CHARGING)
        rd(h, at(0, 12), 95)                       # new anchor, no observation against 100 or 78
        self.assertAlmostEqual(h.w, 1.0)
        rd(h, at(0, 14), 91)                       # r = 2
        self.assertAlmostEqual(h.w, 3.0)
        self.assertAlmostEqual(h.R, 2.0)

    def test_rgb_switch_starts_a_new_anchor(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 11), 76)                       # r = 2, w = 2
        h.set_rgb(at(0, 11), True)
        rd(h, at(0, 12), 74)                       # becomes the anchor, no observation
        self.assertAlmostEqual(h.w, 2.0)
        rd(h, at(0, 14), 70)                       # r = 2 over 2 h, rate kept and adapted
        self.assertAlmostEqual(h.w, 4.0)
        self.assertAlmostEqual(h.R, 2.0)

    def test_rgb_set_to_the_same_value_changes_nothing(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 10), 78)
        h.set_rgb(at(0, 10), False)
        rd(h, at(0, 11), 76)                       # anchor kept: observation
        self.assertAlmostEqual(h.w, 2.0)

    def test_level_rise_without_charge_reanchors(self):
        h = started(at(0, 9), 50)
        rd(h, at(0, 10), 90)
        self.assertIsNone(h.R)
        rd(h, at(0, 12), 86)
        self.assertAlmostEqual(h.R, 2.0)

    def test_unknown_state_neither_breaks_nor_observes(self):
        h = started(at(0, 9), 80)
        rd(h, at(0, 10), 70, e.UNKNOWN)
        self.assertIsNone(h.R)
        rd(h, at(0, 11), 76)
        self.assertAlmostEqual(h.R, 2.0)           # 4 points over 2 h from the 80 anchor
        self.assertAlmostEqual(h.w, 2.0)


class ForecastTexts(unittest.TestCase):
    def test_every_rounding_band(self):
        f = e.format_hours_left
        cases = [
            (0.01, "~10 min"), (0.1, "~10 min"), (0.5, "~30 min"), (40 / 60, "~40 min"),
            (0.9, "~50 min"), (0.95, "~1 h"),
            (1.0, "~1 h"), (1.2, "~1 h"), (1.25, "~1,5 h"), (2.5, "~2,5 h"), (3.0, "~3 h"),
            (3.2, "~3 h"), (3.3, "~3,5 h"), (9.7, "~9,5 h"), (9.75, "~10 h"),
            (10.0, "~10 h"), (17.49, "~17 h"), (17.5, "~18 h"), (71.4, "~71 h"), (71.5, "~72 h"),
            (72.0, "~3 dias"), (83.9, "~3 dias"), (84.0, "~4 dias"), (240.0, "~10 dias"),
        ]
        for hours, text in cases:
            self.assertEqual(f(hours), text, hours)

    def test_hours_left_is_level_over_rate(self):
        h = started(at(0, 9), 50)
        self.assertAlmostEqual(e.hours_left(h), 10.0)
        self.assertEqual(e.forecast_text(h), "~10 h de uso")

    def test_no_forecast_when_charging_offline_or_coarse(self):
        h = started(at(0, 9), 50)
        online(h, at(0, 9, 30), False)
        self.assertIsNone(e.hours_left(h))
        self.assertIsNone(e.eta_text(h, at(0, 10)))
        self.assertIsNone(e.forecast_label(h))
        h = started(at(0, 9))
        rd(h, at(0, 9), 50, e.CHARGING)
        self.assertIsNone(e.hours_left(h))
        h = started(at(0, 9))
        rd(h, at(0, 9), None, e.DISCHARGING, e.COARSE)
        self.assertIsNone(e.hours_left(h))
        self.assertIsNone(e.hours_left(hist()))

    def test_derived_quality_is_forecast_like_exact(self):
        h = started(at(0, 9))
        rd(h, at(0, 9), 40, e.DISCHARGING, e.DERIVED)
        self.assertAlmostEqual(e.hours_left(h), 8.0)


class Profile(unittest.TestCase):
    def test_default_before_seven_days(self):
        h = started(at(0, 9), 80)
        p = h.profile(at(6, 20))                   # 6 d 11 h of data
        self.assertFalse(h.profile_learned(at(6, 20)))
        self.assertEqual(p[0 * 24 + 9], 0.8)       # Monday 9 h
        self.assertEqual(p[0 * 24 + 18], 0.8)      # Monday 18 h (until 19 h)
        self.assertEqual(p[0 * 24 + 19], 0.1)
        self.assertEqual(p[0 * 24 + 8], 0.1)
        self.assertEqual(p[4 * 24 + 12], 0.8)      # Friday
        self.assertEqual(p[5 * 24 + 12], 0.1)      # Saturday
        self.assertEqual(p[6 * 24 + 12], 0.1)      # Sunday

    def test_learned_from_exactly_seven_days(self):
        h = started(at(0, 9), 80)
        self.assertFalse(h.profile_learned(at(7, 9) - 1))
        self.assertTrue(h.profile_learned(at(7, 9)))

    def test_learned_profile_is_the_online_share_per_hour_of_week(self):
        h = hist()
        for d in range(10):                        # online 09:00-09:30 every day
            online(h, at(d, 9))
            online(h, at(d, 9, 30), False)
        now = at(10, 8)
        self.assertTrue(h.profile_learned(now))
        p = h.profile(now)
        self.assertAlmostEqual(p[2 * 24 + 9], 0.5)   # Wednesday 9 h
        self.assertAlmostEqual(p[2 * 24 + 10], 0.0)
        self.assertAlmostEqual(p[0 * 24 + 15], 0.0)  # Monday 15 h: was 0.8 by default

    def test_learned_profile_averages_the_occurrences(self):
        h = hist()
        online(h, at(0, 0), False)
        online(h, at(7, 9))                        # online Monday 9-10 only on the second Monday
        online(h, at(7, 10), False)
        online(h, at(13, 23), False)
        p = h.profile(at(14, 0))
        self.assertAlmostEqual(p[0 * 24 + 9], 0.5)  # two Mondays 9 h: 0 and 1

    def test_hour_with_no_complete_occurrence_keeps_the_default(self):
        h = hist()
        online(h, at(0, 12, 30), False)            # data starts mid-hour
        now = at(7, 12, 30)                        # exactly 7 days: Monday 12 h never completed
        self.assertTrue(h.profile_learned(now))
        p = h.profile(now)
        self.assertEqual(p[0 * 24 + 12], 0.8)      # default
        self.assertEqual(p[0 * 24 + 13], 0.0)      # learned (offline)

    def test_only_the_last_28_days_count(self):
        h = hist()
        online(h, at(0, 9))
        online(h, at(0, 10), False)                # Monday 26/10 9 h online once
        now = at(35, 12)                           # 35 days later: that Monday is out of the window
        p = h.profile(now)
        self.assertAlmostEqual(p[0 * 24 + 9], 0.0)

    def test_profile_uses_the_local_zone(self):
        h = hist(utc_offset=19800)                 # UTC+5:30
        t = make_cases.epoch(2026, 10, 26, 9, 0, 19800)
        for d in range(9):
            online(h, t + d * e.DAY)
            online(h, t + d * e.DAY + HR, False)
        p = h.profile(t + 9 * e.DAY + 12 * HR)
        self.assertAlmostEqual(p[0 * 24 + 9], 1.0)
        self.assertAlmostEqual(p[0 * 24 + 8], 0.0)


class EtaWalk(unittest.TestCase):
    def test_eta_inside_the_afternoon(self):
        h = started(at(0, 13, 50), 20)             # 4 h of use at 0.8/h up to 19 h
        now = at(0, 14)
        # 14:00 -> 19:00 offers 5 * 0.8 = 4.0 h of use: the 4.0 h run out at 19:00
        self.assertAlmostEqual(e.eta(h, now), at(0, 19), delta=1)
        self.assertEqual(e.eta_text(h, now), "acaba hoje ~19:00")

    def test_eta_partial_first_hour(self):
        h = started(at(0, 9), 4)                   # 0.8 h of use
        now = at(0, 9, 30)                         # 30 min left in the hour offer 0.4 h
        self.assertAlmostEqual(e.eta(h, now), at(0, 10, 30), delta=1)

    def test_eta_across_midnight(self):
        h = started(at(0, 9), 10)                  # 2 h of use
        now = at(0, 18)
        # 18-19 h: 0.8 spent, 1.2 left; 19-24 h: 0.5, 0.7 left; 0.7 / 0.1 h = 7 h after midnight
        self.assertAlmostEqual(e.eta(h, now), at(1, 7), delta=1)
        self.assertEqual(e.eta_text(h, now), "acaba amanhã ~07:00")

    def test_eta_across_days_shows_the_weekday(self):
        h = started(at(0, 9), 64)                  # 12.8 h of use
        now = at(0, 18)
        self.assertEqual(e.eta_text(h, now), "acaba qua ~10:30")

    def test_eta_time_rounds_to_ten_minutes_and_can_cross_midnight(self):
        # Friday 23:57, 0.1 h of use: 3 min offer 0.005 h at u 0.1, the rest 0.95 h of wall time
        # into Saturday (u 0.1): 00:57 rounds to 01:00
        h = started(at(4, 9))
        rd(h, at(4, 23, 57), 0.5)
        self.assertEqual(e.eta_text(h, at(4, 23, 57)), "acaba amanhã ~01:00")

    def test_eta_day_words(self):
        h = started(at(0, 9), 10)                  # 2 h of use
        self.assertEqual(e.eta_text(h, at(0, 14)), "acaba hoje ~16:30")        # 2 h at 0.8: 2.5 h of wall time
        self.assertEqual(e.eta_text(h, at(0, 18)), "acaba amanhã ~07:00")
        h = started(at(0, 9), 64)                  # 12.8 h of use: the day after tomorrow
        self.assertEqual(e.eta_text(h, at(0, 18)), "acaba qua ~10:30")

    def test_eta_with_learned_profile(self):
        h = hist()
        online(h, at(0, 9))
        rd(h, at(0, 9), 100)
        online(h, at(0, 12), False)
        for d in range(1, 10):
            online(h, at(d, 9))
            online(h, at(d, 12), False)
        online(h, at(10, 9))
        now = at(10, 10)                           # Thursday 10:00
        # 20 h of use; u is 1 from 9 to 12 h and 0 elsewhere: 2 h today, 3 h on each of
        # Fri, Sat, Sun, Mon, Tue (17), the last 3 h on Wednesday 9-12 h
        self.assertTrue(h.profile_learned(now))
        self.assertAlmostEqual(e.eta(h, now), at(16, 12), delta=1)
        self.assertEqual(e.eta_text(h, now), "acaba qua ~12:00")

    def test_eta_none_beyond_seven_days_and_days_text(self):
        h = started(at(0, 9), 100, rated_use_hours=100.0)
        now = at(0, 9)
        self.assertIsNone(e.eta(h, now))
        self.assertEqual(e.eta_text(h, now), "~4 dias de uso")   # 100 h / 24 = 4.17 days
        self.assertEqual(e.forecast_text(h), "~4 dias de uso")

    def test_seven_day_limit_is_on_wall_time(self):
        # default week offers 5 * (10 * 0.8 + 14 * 0.1) + 2 * 24 * 0.1 = 51.8 h of use
        h = started(at(0, 0), 51.5, rated_use_hours=100.0)   # 51.5 / 1 %/h = 51.5 h
        now = at(0, 0)
        self.assertIsNotNone(e.eta(h, now))
        h = started(at(0, 0), 52.0, rated_use_hours=100.0)
        self.assertIsNone(e.eta(h, now))

    def test_beyond_seven_days_with_a_learned_sparse_profile_is_singular(self):
        h = hist(rated_use_hours=100.0)
        online(h, at(0, 9))
        online(h, at(0, 9, 1), False)               # one minute online in 8 days
        online(h, at(8, 9))
        rd(h, at(8, 9), 30)                         # 30 h of use at 1 %/h
        now = at(8, 10)
        self.assertTrue(h.profile_learned(now))
        self.assertIsNone(e.eta(h, now))
        self.assertEqual(e.eta_text(h, now), "~1 dia de uso")


class TimeToFull(unittest.TestCase):
    def charging(self, pct, **kw):
        h = started(at(0, 14, 32), **kw)
        rd(h, at(0, 14, 32), pct, e.CHARGING)
        return h

    def test_two_phase_below_80(self):
        h = self.charging(40)                      # prior 120 / 2.5 = 48 %/h: 40/48 + 20/24 = 100 min
        self.assertAlmostEqual(e.time_to_full(h), 100.0 / 60.0)
        self.assertEqual(e.full_in_text(h), "cheio em ~1 h 40 min")
        self.assertEqual(e.full_at_text(h, at(0, 14, 32)), "cheio às 16:12")

    def test_a_full_charge_from_zero_takes_the_rated_time(self):
        for rated in (1.0, 2.0, 2.5, 7.0 / 6.0):
            self.assertAlmostEqual(e.time_to_full(self.charging(0, rated_charge_hours=rated)), rated)

    def test_above_80_at_half_rate(self):
        h = self.charging(92)                      # 8 points at 24 %/h = 20 min
        self.assertAlmostEqual(e.time_to_full(h), 1.0 / 3.0)
        self.assertEqual(e.full_in_text(h), "cheio em ~20 min")
        self.assertEqual(e.full_at_text(h, at(0, 14, 32)), "cheio às 14:52")
        self.assertAlmostEqual(e.time_to_full(self.charging(80)), 20.0 / 24.0)
        self.assertAlmostEqual(e.time_to_full(self.charging(79)), 1.0 / 48.0 + 40.0 / 48.0)

    def test_full_in_text_rounding(self):
        self.assertEqual(e.full_in_text(self.charging(99)), "cheio em ~10 min")          # 2.5 min: floor of 10 min
        self.assertEqual(e.full_in_text(self.charging(60, rated_charge_hours=1.0)), "cheio em ~30 min")
        self.assertEqual(e.full_in_text(self.charging(0, rated_charge_hours=1.0)), "cheio em ~1 h")
        self.assertEqual(e.full_in_text(self.charging(0, rated_charge_hours=7.0 / 6.0)), "cheio em ~1 h 10 min")
        self.assertEqual(e.full_in_text(self.charging(0, rated_charge_hours=1.45)), "cheio em ~1 h 30 min")  # 87 min
        self.assertEqual(e.full_in_text(self.charging(0, rated_charge_hours=6.0)), "cheio em ~6 h")

    def test_charge_rate_is_learned_in_wall_clock_hours_and_fast_phase_equivalent(self):
        h = started(at(0, 14))
        rd(h, at(0, 14), 70, e.CHARGING)
        rd(h, at(0, 15), 90, e.CHARGING)           # 10 points fast + 10 points at half rate = 30 effective
        self.assertAlmostEqual(h.Rc, 30.0)
        self.assertAlmostEqual(h.wc, 1.0)
        self.assertAlmostEqual(e.charge_rate(h), (30.0 + 3 * 48.0) / 4)

    def test_charge_observation_below_80_is_plain(self):
        h = started(at(0, 14))
        rd(h, at(0, 14), 20, e.CHARGING)
        rd(h, at(0, 14, 30), 60, e.CHARGING)
        self.assertAlmostEqual(h.Rc, 80.0)
        rd(h, at(0, 14, 40), 61, e.CHARGING)       # 1 point: ignored
        self.assertAlmostEqual(h.wc, 0.5)

    def test_charge_needs_a_quarter_hour(self):
        h = started(at(0, 14))
        rd(h, at(0, 14), 20, e.CHARGING)
        rd(h, at(0, 14, 14), 60, e.CHARGING)
        self.assertIsNone(h.Rc)

    def test_charge_time_counts_even_if_offline_in_between(self):
        h = started(at(0, 14))
        rd(h, at(0, 14), 20, e.CHARGING)
        online(h, at(0, 14, 10), False)
        online(h, at(0, 14, 50))
        rd(h, at(0, 15), 60, e.CHARGING)           # wall time 1 h, not the 20 min online
        self.assertAlmostEqual(h.Rc, 40.0)

    def test_no_time_to_full_when_not_charging_or_full(self):
        h = started(at(0, 14), 50)
        self.assertIsNone(e.time_to_full(h))
        h = started(at(0, 14))
        rd(h, at(0, 14), 100, e.FULL)
        self.assertIsNone(e.time_to_full(h))
        self.assertIsNone(e.full_in_text(h))
        h = started(at(0, 14))
        rd(h, at(0, 14), 100, e.CHARGING)
        self.assertIsNone(e.time_to_full(h))
        h = self.charging(50)
        online(h, at(0, 15), False)
        self.assertIsNone(e.time_to_full(h))


class FullNotice(unittest.TestCase):
    def test_once_per_charging_segment(self):
        h = started(at(0, 14))
        self.assertEqual(rd(h, at(0, 14), 90, e.CHARGING), [])
        n = rd(h, at(0, 15), 100, e.FULL)
        self.assertEqual([(x.kind, x.at) for x in n], [("full", at(0, 15))])
        self.assertTrue(e.full_notice(h))
        self.assertEqual(rd(h, at(0, 16), 100, e.FULL), [])
        self.assertEqual(rd(h, at(0, 17), 100, e.CHARGING), [])
        rd(h, at(0, 18), 90)                       # unplugged
        self.assertFalse(e.full_notice(h))
        rd(h, at(0, 19), 98, e.CHARGING)
        n = rd(h, at(0, 20), 100, e.CHARGING)      # 100 % while charging counts
        self.assertEqual([x.kind for x in n], ["full"])

    def test_not_below_100_while_charging(self):
        h = started(at(0, 14))
        self.assertEqual(rd(h, at(0, 14), 99, e.CHARGING), [])
        self.assertFalse(e.full_notice(h))


class Usage(unittest.TestCase):
    def test_split_across_local_midnight(self):
        h = hist()
        online(h, at(0, 23))
        online(h, at(1, 1, 30), False)
        self.assertEqual(e.usage_by_day(h, at(1, 12)), {"2026-10-26": 3600, "2026-10-27": 5400})

    def test_open_interval_counts_until_now(self):
        h = started(at(0, 9))
        self.assertEqual(e.usage_by_day(h, at(0, 13, 10)), {"2026-10-26": 15000})
        self.assertEqual(e.usage_today_text(h, at(0, 13, 10)), "Hoje: 4 h 10 min de uso")

    def test_days_without_use_are_absent_and_offline_only_device_is_empty(self):
        h = hist()
        online(h, at(0, 9), False)
        self.assertEqual(e.usage_by_day(h, at(3, 9)), {})
        self.assertEqual(e.usage_today_text(h, at(3, 9)), "Hoje: 0 min de uso")

    def test_kept_365_days(self):
        h = hist()
        for day in (0, 60, 430):
            online(h, at(day, 9))
            online(h, at(day, 9, 30), False)
        got = e.usage_by_day(h, at(430, 12))
        self.assertEqual(list(got), ["2027-12-30"])
        # the 365th day back is kept, the 366th is not
        h = hist()
        online(h, at(0, 9))
        online(h, at(0, 10), False)
        self.assertIn("2026-10-26", e.usage_by_day(h, at(364, 12)))
        self.assertNotIn("2026-10-26", e.usage_by_day(h, at(365, 12)))

    def test_text_format(self):
        cases = [(0, "0 min"), (29, "0 min"), (30, "1 min"), (3000, "50 min"), (3569, "59 min"),
                 (3570, "1 h"), (14400, "4 h"), (15000, "4 h 10 min"), (21900, "6 h 05 min"),
                 (5400, "1 h 30 min")]
        for secs, text in cases:
            self.assertEqual(e.usage_text(secs), text, secs)

    def test_half_hour_zone(self):
        h = hist(utc_offset=19800)
        online(h, make_cases.epoch(2026, 10, 26, 23, 0, 19800))
        online(h, make_cases.epoch(2026, 10, 27, 1, 0, 19800), False)
        self.assertEqual(e.usage_by_day(h, make_cases.epoch(2026, 10, 27, 12, 0, 19800)),
                         {"2026-10-26": 3600, "2026-10-27": 3600})


def run_cycles(h, t, specs):
    """specs: (hours of use, points spent per hour). Each cycle is followed by a charge. Returns the end time."""
    for hours, drop in specs:
        for i in range(hours + 1):
            rd(h, t + i * HR, 100 - drop * i)
        t += hours * HR
        rd(h, t + 600, 100 - drop * hours, e.CHARGING)
        rd(h, t + 3 * HR, 100, e.FULL)
        t += 3 * HR
    return t


class Health(unittest.TestCase):
    def test_fewer_than_three_cycles_is_learning(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(10, 5), (10, 5)])
        self.assertEqual(len(h.cycles), 2)
        self.assertIsNone(e.health(h))
        self.assertEqual(e.health_text(h), "aprendendo")

    def test_a_cycle_needs_50_points(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(7, 7), (7, 7), (7, 7)])   # 49 points each
        self.assertEqual(h.cycles, [])
        self.assertIsNone(e.health(h))
        run_cycles(h, at(5, 0), [(10, 5)])                   # exactly 50
        self.assertEqual(len(h.cycles), 1)

    def test_cycle_autonomy_is_hours_of_use_per_full_battery(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(10, 5)])
        self.assertAlmostEqual(h.cycles[0].autonomy, 20.0)
        self.assertFalse(h.cycles[0].rgb)

    def test_exactly_three_cycles_give_100(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(12, 5), (13, 4), (15, 4)])
        self.assertEqual(len(h.cycles), 3)
        self.assertAlmostEqual(e.health(h), 100.0)

    def test_median_of_last_three_over_median_of_first_three(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(15, 4), (15, 4), (15, 4), (12, 5), (12, 5)])
        # autonomies 25 25 25 20 20: first-3 median 25, last-3 [25, 20, 20] median 20
        self.assertAlmostEqual(e.health(h), 80.0)
        self.assertEqual(e.health_text(h), "80%")

    def test_capped_at_100(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(12, 5), (12, 5), (12, 5), (15, 4), (15, 4)])
        self.assertAlmostEqual(e.health(h), 100.0)         # 125 capped

    def test_health_under_two_rgb_settings_compares_like_with_like(self):
        h = started(at(0, 0))
        t = run_cycles(h, at(0, 0), [(12, 5), (12, 5), (12, 5)])      # RGB off: autonomy 20
        h.set_rgb(t, True)
        t = run_cycles(h, t + HR, [(6, 10), (6, 10)])                  # RGB on: autonomy 10
        self.assertEqual([c.rgb for c in h.cycles], [False] * 3 + [True] * 2)
        self.assertIsNone(e.health(h))                                 # the latest setting has two cycles
        run_cycles(h, t + HR, [(7, 8)])                                # autonomy 12.5
        # RGB-on autonomies 10, 10, 12.5: first median 10, last median 10
        self.assertAlmostEqual(e.health(h), 100.0)
        self.assertTrue(h.cycles[-1].rgb)

    def test_rgb_setting_of_a_cycle_is_the_one_at_its_start(self):
        h = started(at(0, 0))
        run_cycles(h, at(0, 0), [(12, 5)])
        h.set_rgb(at(5, 0), True)
        t = at(5, 1)
        for i in range(13):
            rd(h, t + i * HR, 100 - 5 * i)
            if i == 4:
                h.set_rgb(t + i * HR, False)       # switched off mid-cycle (loses one step)
        rd(h, t + 13 * HR, 40, e.CHARGING)
        self.assertEqual([c.rgb for c in h.cycles], [False, True])


class Warnings(unittest.TestCase):
    def test_expected_use_left_today_on_a_weekday(self):
        h = started(at(0, 9))
        self.assertAlmostEqual(e.expected_use_left_today(h, at(0, 14)), 4.0)       # to 19 h
        self.assertAlmostEqual(e.expected_use_left_today(h, at(0, 14, 30)), 3.6)
        self.assertAlmostEqual(e.expected_use_left_today(h, at(0, 8)), 0.1 + 10 * 0.8)
        self.assertAlmostEqual(e.expected_use_left_today(h, at(0, 19)), 0.0)
        self.assertAlmostEqual(e.expected_use_left_today(h, at(0, 22)), 0.0)

    def test_no_typical_day_on_a_weekend(self):
        h = started(at(5, 9))
        self.assertAlmostEqual(e.expected_use_left_today(h, at(5, 10)), 0.0)

    def test_warns_when_the_forecast_will_not_last(self):
        h = started(at(0, 13, 50))
        n = rd(h, at(0, 14), 20)                   # 4.0 h left, expected 4.0 x 1.1
        self.assertEqual([x.kind for x in n], ["smart"])
        self.assertEqual(n[0].text, "Não chega ao fim do dia: carregue antes das 16h")
        self.assertEqual(e.smart_warning(h, at(0, 14)), n[0].text)

    def test_margin_is_one_point_one_times(self):
        h = started(at(0, 13, 50))
        rd(h, at(0, 14), 21.9)                     # 4.38 h < 4.4
        self.assertIsNotNone(e.smart_warning(h, at(0, 14)))
        h = started(at(0, 13, 50))
        n = rd(h, at(0, 14), 22.1)                 # 4.42 h > 4.4
        self.assertEqual(n, [])
        self.assertIsNone(e.smart_warning(h, at(0, 14)))

    def test_hour_when_ten_percent_is_reached_is_rounded_down(self):
        h = started(at(0, 13, 50))
        rd(h, at(0, 14), 21.9)                     # 11.9 points = 2.38 h of use: 16:22
        self.assertEqual(e.smart_warning(h, at(0, 14)), "Não chega ao fim do dia: carregue antes das 16h")

    def test_hour_has_no_leading_zero(self):
        h = started(at(0, 9))
        rd(h, at(0, 9, 20), 8)
        self.assertEqual(e.smart_warning(h, at(0, 9, 20)), "Não chega ao fim do dia: carregue antes das 9h")

    def test_level_at_or_below_ten_names_the_current_hour(self):
        h = started(at(0, 9))
        rd(h, at(0, 11, 20), 8)
        self.assertEqual(e.smart_warning(h, at(0, 11, 20)), "Não chega ao fim do dia: carregue antes das 11h")

    def test_once_per_device_per_day(self):
        h = started(at(0, 13, 50))
        self.assertEqual(len(rd(h, at(0, 14), 20)), 1)
        self.assertEqual(rd(h, at(0, 14, 5), 19.5), [])
        self.assertEqual(rd(h, at(0, 15), 18), [])
        n = rd(h, at(1, 14), 17)                   # next day: the rate is 2 %/h blended to 4.25
        self.assertEqual([x.kind for x in n], ["smart"])
        self.assertEqual(rd(h, at(1, 15), 16.5), [])

    def test_no_warning_leaves_the_daily_gate_open(self):
        h = started(at(0, 13, 50))
        self.assertEqual(rd(h, at(0, 14), 60), [])
        self.assertEqual([x.kind for x in rd(h, at(0, 16), 12)], ["smart"])

    def test_presence_arrival_can_warn(self):
        h = hist()
        online(h, at(0, 13, 0))
        rd(h, at(0, 13, 0), 18)
        self.assertEqual(len(h.notices), 1)
        h = hist()
        rd(h, at(0, 13, 0), 18)                    # offline: nothing
        self.assertEqual(h.notices, [])
        n = online(h, at(0, 13, 30))
        self.assertEqual([x.kind for x in n], ["smart"])

    def test_no_warning_when_offline_charging_or_after_the_typical_day(self):
        h = started(at(0, 13, 50))
        online(h, at(0, 13, 55), False)
        self.assertEqual(rd(h, at(0, 14), 20), [])
        h = started(at(0, 13, 50))
        self.assertEqual(rd(h, at(0, 14), 20, e.CHARGING), [])
        h = started(at(0, 13, 50))
        self.assertEqual(rd(h, at(0, 20), 20), [])  # after 19 h nothing is expected

    def test_no_warning_on_a_weekend(self):
        h = started(at(5, 13, 50))
        self.assertEqual(rd(h, at(5, 14), 30), [])

    def test_low_crossing_once_per_segment(self):
        h = started(at(6, 20))                     # Sunday: no smart warnings
        self.assertEqual(rd(h, at(6, 20), 14), [])
        self.assertEqual(rd(h, at(6, 21), 10), [])   # 10 is not below 10
        n = rd(h, at(6, 22), 9)
        self.assertEqual([(x.kind, x.text) for x in n], [("low", "G703 com 9%")])
        self.assertEqual(rd(h, at(6, 23), 8), [])
        rd(h, at(7, 0), 60, e.CHARGING)
        rd(h, at(7, 1), 40)
        n = rd(h, at(7, 2), 9)
        self.assertEqual([x.kind for x in n if x.kind == "low"], ["low"])

    def test_low_text_rounds_the_level(self):
        h = started(at(6, 20))
        n = rd(h, at(6, 20), 8.6)
        self.assertEqual(n[0].text, "G703 com 9%")

    def test_low_first_reading_below_ten_warns(self):
        h = started(at(6, 20))
        self.assertEqual([x.kind for x in rd(h, at(6, 20), 4)], ["low"])

    def test_no_low_when_charging_or_coarse(self):
        h = started(at(6, 20))
        self.assertEqual(rd(h, at(6, 20), 4, e.CHARGING), [])
        h = started(at(6, 20))
        self.assertEqual(rd(h, at(6, 20), None, e.DISCHARGING, e.COARSE), [])


class SpecialDevices(unittest.TestCase):
    def test_coarse_only_device(self):
        h = started(at(0, 9))
        for t in (at(0, 9), at(0, 12)):
            self.assertEqual(rd(h, t, None, e.DISCHARGING, e.COARSE), [])
        self.assertIsNone(h.level)
        self.assertIsNone(e.hours_left(h))
        self.assertIsNone(e.eta(h, at(0, 12)))
        self.assertIsNone(e.eta_text(h, at(0, 12)))
        self.assertIsNone(e.forecast_text(h))
        self.assertIsNone(e.forecast_label(h))
        self.assertIsNone(h.R)
        self.assertIsNone(e.health(h))
        self.assertEqual(h.notices, [])
        self.assertEqual(e.usage_by_day(h, at(0, 12)), {"2026-10-26": 3 * HR})

    def test_coarse_charging_flag_still_counts_for_segments(self):
        h = started(at(0, 9))
        rd(h, at(0, 9), None, e.CHARGING, e.COARSE)
        n = rd(h, at(0, 10), None, e.FULL, e.COARSE)
        self.assertEqual([x.kind for x in n], ["full"])

    def test_device_offline_the_whole_time(self):
        h = hist()
        online(h, at(0, 9), False)
        rd(h, at(0, 9), 55)
        rd(h, at(0, 12), 50)
        now = at(0, 13)
        self.assertIsNone(h.R)
        self.assertAlmostEqual(e.discharge_rate(h), 5.0)
        self.assertIsNone(e.hours_left(h))
        self.assertIsNone(e.eta(h, now))
        self.assertEqual(e.usage_by_day(h, now), {})
        self.assertIsNone(e.health(h))
        self.assertEqual(h.notices, [])
        self.assertFalse(h.online)

    def test_device_with_no_events(self):
        h = hist()
        now = at(0, 12)
        self.assertIsNone(e.hours_left(h))
        self.assertEqual(e.usage_by_day(h, now), {})
        self.assertFalse(h.profile_learned(now))


class CasesFile(unittest.TestCase):
    """The scenarios are self-consistent: the TSV parses back and replays to the same cells."""

    def test_round_trip_of_every_scenario(self):
        text = make_cases.render()
        lines = [x for x in text.split("\n") if x and not x.startswith("#")]
        self.assertEqual(lines[0].split("\t"), make_cases.COLUMNS)
        scenarios = make_cases.scenarios()
        self.assertEqual(len(lines) - 1, len(scenarios))
        self.assertGreaterEqual(len(scenarios), 30)
        for line in lines[1:]:
            cells = make_cases.parse_row(line)
            case = make_cases.case_from_cells(cells)
            again = make_cases.evaluate(case)
            for col in make_cases.OUTPUT_COLUMNS:
                self.assertEqual(again[col], cells[col], (cells["name"], col))

    def test_render_is_deterministic(self):
        self.assertEqual(make_cases.render(), make_cases.render())

    def test_event_text_round_trip(self):
        evs = [("P", 100, True), ("R", 200, 80.5, e.DISCHARGING, e.EXACT), ("G", 300, False),
               ("R", 400, None, e.CHARGING, e.COARSE), ("R", 500, 100.0, e.FULL, e.DERIVED)]
        text = make_cases.format_events(evs)
        self.assertEqual(text, "P100/1 R200/80.5/d G300/0 R400/-/c/c R500/100/f/d")
        self.assertEqual(make_cases.parse_events(text), evs)
        self.assertEqual(make_cases.parse_events("-"), [])

    def test_the_sample_devices_are_present(self):
        names = [c.name for c in make_cases.scenarios()]
        for n in ("sample_g703", "sample_g915x", "sample_g733", "sample_xbox", "sample_airpods"):
            self.assertIn(n, names)


if __name__ == "__main__":
    unittest.main(verbosity=1)
