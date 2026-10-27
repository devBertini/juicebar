# Estimator: forecast, time to full, usage, health and warnings

Both apps compute these the same way from the readings providers report (`spec/PROVIDERS.md`). `spec/reference/estimator.py` (Python standard library) is the executable form of this file and writes `spec/estimator-cases.tsv`; the Rust and Swift cores must reproduce every case within the tolerance the file states. Where this text and the reference disagree, fix one of them before any core changes.

## Inputs, per device
- **Readings:** wall time (seconds since the epoch, UTC), percent (0 to 100), charging state, quality (`exact`, `derived`, `coarse`). A device with `coarse` quality only shows its level and gets no forecast. A reading in state `unknown` is stored but neither observes nor breaks anything.
- **Presence:** online and offline moments; a device is offline until its first presence event, and a reading never makes it online. Forecasts and time to full exist only while it is online. *Use time* is the time the device is online; JuiceBar cannot see whether a connected device is being touched, so "uso" means connected time.
- **Settings that change consumption:** the RGB economy switch, recorded with its moments.
- **Model prior** from `spec/devices.tsv`: rated hours of use and rated hours to charge (Logitech's own figures, `docs/research/hidpp-and-lighting.md`); defaults 20 h and 2.5 h for an unknown model.

## Discharge rate
1. A *segment* is a run of readings while discharging, broken by any charge. A level that rises while discharging (a charge JuiceBar did not see) starts a new anchor.
2. Inside a segment keep an *anchor* (the first reading, then the last one used). When the level is at least 2 points below the anchor and at least 0.25 h of use passed since it, an observation is `r = (anchor % - now %) / use hours between them`; the reading becomes the new anchor. Single-point steps are noise from 1 % quantization.
3. The observed rate is an exponentially weighted mean over use time: `R = R + a (r - R)`, `a = 1 - 2^(-h / 6)`, `h` the observation's use hours (half-life 6 h of use). The first observation sets `R = r`.
4. The prior rate is `100 / rated hours`. The rate used is `(w R + 3 P) / (w + 3)`, `w` = observed use hours, capped at 12. While `w < 3` the forecast is labeled "estimativa inicial".
5. A change of the RGB economy switch starts a new anchor (the rate keeps its value and adapts; the drop across the change is not observed).

## Forecast to empty
- **Hours of use left:** `level / rate`.
- **Clock time:** walk forward hour by hour from now, spending in each hour the expected use `u(hour of week)` (the share of that hour of the week the device was online, mean over the last 28 days; before 7 days of data: 0.8 from 9 h to 19 h on weekdays, 0.1 otherwise) until the hours left are spent; the moment reached is the ETA. The ETA is rounded to 10 minutes and shown as "acaba hoje ~17:40", "acaba amanhã ~10:20", "acaba qua ~20:00" (the weekday from the day after tomorrow); past 7 days, "~<n> dias de uso" alone (`n` = hours left / 24, rounded; "~1 dia de uso").
- **Rounding of hours left:** under 1 h, minutes to the nearest 10 ("~40 min"); under 10 h, the nearest half hour ("~3 h", "~2,5 h"); under 72 h, the nearest hour; then days.

## Time to full
- The charge rate is learned like the discharge rate over charging segments, in % per wall-clock hour; it is the rate below 80 % (an observation above 80 % counts at twice its rate). Its prior is `120 / rated hours to charge`, so that a full charge at the two-phase pace below takes the rated time; the blend is the same.
- Up to 80 % at that rate, from 80 % to 100 % at half of it (lithium cells slow down near full). Shown as "cheio em ~1 h 10 min" (nearest 10 minutes; under 10 minutes "~10 min") and "cheio às 15:42" (to the minute).
- **Full-charge notice:** once per charging segment, when the device reports `full` or 100 % while charging.

## Usage per day
The online time of the device inside each local calendar day ("Hoje: 4 h 10 min de uso"); kept 365 days.

## Battery health
- A *cycle* is the use between two charges that spent at least 50 points; its autonomy is `100 / its mean rate` (hours of use per full battery), stored with the RGB economy setting at its start.
- Health = median autonomy of the last 3 cycles / median autonomy of the first 3 cycles, both taken under the setting of the latest cycle, capped at 100 %. Until 3 cycles exist under one setting: "aprendendo".

## Smart warning
- For an online, discharging device: the expected use left today is the sum of `u(hour)` from now to the end of its typical day (the last hour of today with `u >= 0.2`). When hours left are strictly below that times 1.1, warn once per device per day: "Não chega ao fim do dia: carregue antes das <H>h", `<H>` the hour (no leading zero) when the level would reach 10 %, rounded down.
- Independently, a level below 10 % (a first reading below it included) warns once per segment ("<device> com <n>%"). Percentages are written without a space everywhere ("23%").

## Storage and cost
Per device: readings of the last 30 days (written on change only), presence moments of 30 days, daily usage of 365 days, cycle summaries, the two rates with their weights. Writes are batched, at most once every 5 minutes and on quit. Everything is recomputed only when a reading or a presence event arrives, never on a timer; the ETA text is refreshed when the panel opens.

## Tolerances for the cores
Rates within 1e-6 relative; hours left within 1 minute; ETA within 1 minute; health within 0.1 point; warning decisions identical.
