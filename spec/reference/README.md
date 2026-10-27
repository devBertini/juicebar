# Estimator reference

`estimator.py` is the executable form of `spec/estimator.md` (Python 3.9 standard library only). The Rust core (Windows) and the Swift core (Mac) are tested against `spec/estimator-cases.tsv`, which `make_cases.py` writes from it.

```
python3 -B spec/reference/test_estimator.py     # the reference's own tests
python3 -B spec/reference/make_cases.py          # rewrites spec/estimator-cases.tsv (byte-identical on every run)
```

## Names the cores use

`DeviceHistory`, `Reading`, `Presence`, `discharge_rate`, `charge_rate`, `hours_left`, `eta`, `time_to_full`, `usage_by_day`, `health`, `smart_warning`, `low_warning`, `full_notice`, and the texts `format_hours_left`, `forecast_text`, `forecast_label`, `eta_text`, `full_in_text`, `full_at_text`, `usage_text`, `usage_today_text`, `health_text`.

## The TSV

Tab separated, UTF-8, LF. Lines starting with `#` are the format description (read it in the file itself). The first other line holds the column names; each next line is a scenario. A cell `-` means "none".

Inputs: `name`, `device` (display name), `rated_use` and `rated_charge` (hours, the model prior), `utc_offset` (seconds), `now` (epoch seconds, UTC), `events`.

`events` is a space separated stream applied in order, from an empty history (offline, RGB off):

| Token | Meaning |
|---|---|
| `R<t>/<percent>/<state>[/<quality>]` | reading; percent a number or `-`; state `d` discharging, `c` charging, `f` full, `u` unknown; quality `e` exact (default), `d` derived, `c` coarse |
| `P<t>/<0\|1>` | presence, 1 online |
| `G<t>/<0\|1>` | RGB economy switch, 1 on |

A core parses a row in a few lines: split the line on tab, split `events` on space, then each token on its first letter and on `/`. After the stream it computes the output columns at `now` and compares: texts and `notices` identical, rates within 1e-6 relative, hours within 1 minute (`hours_left`, `full_hours`), `eta` within 60 seconds, `health` within 0.1 point. `notices` is the list the stream produced, one entry per `add_reading` / `add_presence` call that returned notices, formatted `<kind>@<t>[:<text>]` and joined by `|`.

## Arithmetic the ports copy exactly

- Time is integer seconds (UTC). Local time is `t + utc_offset` and day, hour and weekday come from floor division (Euclid: `div_euclid` in Rust, `floorDiv` in Swift). `weekday = (day + 3) % 7`, Monday = 0 (day 0 is Thursday 1970-01-01). Hour of week = `weekday * 24 + hour`.
- Every text rounds with `floor(x + 0.5)`, never banker's rounding. Minutes of a forecast: `floor(hours * 6 + 0.5) * 10`, at least 10. Halves: `floor(hours * 2 + 0.5) / 2`. The ETA clock time: `floor(t / 600 + 0.5) * 600` (the rounded timestamp then gives the day word and the clock). `cheio às`: `floor(t / 60 + 0.5) * 60`.
- Observations compare integer seconds: at least 900 s of online time and a drop of at least 2.0 points. Everything else is `f64`.
- The smart warning compares `hours_left < expected_use_left_today * 1.1` (both `f64`, in that order).
- The profile sums the online share of each complete local hour in chronological order. Walks (`walk_use`) go hour by hour from `now` with `remaining -= u * d / 3600`; the result is `t + remaining / u * 3600`.
- Notices are produced only when a reading or a presence-online event arrives, in the order full, low, smart.
