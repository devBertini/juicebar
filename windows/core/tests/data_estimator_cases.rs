// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Reproduces every scenario of `spec/estimator-cases.tsv` within the tolerances of spec/estimator.md, and runs the
//! same scenarios in English to prove no text is missing.

use juicebar_core::estimator::{self as est, DeviceHistory, Notice, Presence, Reading};
use juicebar_core::strings::Lang;
use juicebar_core::types::{Charging, Quality};

const CASES: &str = include_str!("../../../spec/estimator-cases.tsv");

struct Case {
    cells: Vec<String>,
    names: Vec<String>,
}

impl Case {
    fn get(&self, col: &str) -> &str {
        let i = self
            .names
            .iter()
            .position(|n| n == col)
            .unwrap_or_else(|| panic!("no column {col}"));
        &self.cells[i]
    }
    fn name(&self) -> &str {
        self.get("name")
    }
}

fn load() -> Vec<Case> {
    let mut names: Vec<String> = Vec::new();
    let mut cases = Vec::new();
    for line in CASES.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let cells: Vec<String> = line.split('\t').map(str::to_string).collect();
        if names.is_empty() {
            names = cells;
        } else {
            assert_eq!(cells.len(), names.len(), "row width: {}", cells[0]);
            cases.push(Case {
                cells,
                names: names.clone(),
            });
        }
    }
    cases
}

fn replay(case: &Case, lang: Lang) -> (DeviceHistory, Vec<Notice>) {
    let mut h = DeviceHistory::new(
        case.get("device"),
        case.get("rated_use").parse().unwrap(),
        case.get("rated_charge").parse().unwrap(),
        case.get("utc_offset").parse().unwrap(),
    );
    h.lang = lang;
    let mut notices = Vec::new();
    let events = case.get("events");
    if events != "-" {
        for tok in events.split(' ') {
            let (head, rest) = tok.split_at(1);
            let f: Vec<&str> = rest.split('/').collect();
            match head {
                "R" => {
                    let percent = if f[1] == "-" {
                        None
                    } else {
                        Some(f[1].parse::<f64>().unwrap())
                    };
                    let charging = match f[2] {
                        "d" => Charging::Discharging,
                        "c" => Charging::Charging,
                        "f" => Charging::Full,
                        "u" => Charging::Unknown,
                        other => panic!("state {other}"),
                    };
                    let quality = match f.get(3).copied().unwrap_or("e") {
                        "e" => Quality::Exact,
                        "d" => Quality::Derived,
                        "c" => Quality::Coarse,
                        other => panic!("quality {other}"),
                    };
                    notices.extend(h.add_reading(Reading::new(
                        f[0].parse().unwrap(),
                        percent,
                        charging,
                        quality,
                    )));
                }
                "P" => notices.extend(h.add_presence(Presence {
                    at: f[0].parse().unwrap(),
                    online: f[1] == "1",
                })),
                "G" => h.set_rgb(f[0].parse().unwrap(), f[1] == "1"),
                other => panic!("token {other}"),
            }
        }
    }
    (h, notices)
}

fn opt_num(cell: &str) -> Option<f64> {
    if cell == "-" {
        None
    } else {
        Some(cell.parse().unwrap())
    }
}

fn opt_text(v: Option<String>) -> String {
    v.unwrap_or_else(|| "-".to_string())
}

fn close_rel(case: &str, what: &str, got: f64, want: f64) {
    let tol = 1e-6 * want.abs().max(1e-12);
    assert!(
        (got - want).abs() <= tol,
        "{case}: {what}: got {got}, want {want}"
    );
}

fn close_opt(case: &str, what: &str, got: Option<f64>, want: Option<f64>, tol_abs: f64) {
    match (got, want) {
        (None, None) => {}
        (Some(g), Some(w)) => assert!(
            (g - w).abs() <= tol_abs,
            "{case}: {what}: got {g}, want {w}"
        ),
        (g, w) => panic!("{case}: {what}: got {g:?}, want {w:?}"),
    }
}

fn notices_text(notices: &[Notice]) -> String {
    if notices.is_empty() {
        return "-".to_string();
    }
    notices
        .iter()
        .map(|n| match &n.text {
            None => format!("{}@{}", n.kind.as_str(), n.at),
            Some(t) => format!("{}@{}:{}", n.kind.as_str(), n.at, t),
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn usage_text(h: &DeviceHistory, now: i64) -> String {
    let rows = est::usage_by_day(h, now);
    if rows.is_empty() {
        return "-".to_string();
    }
    rows.iter()
        .map(|&(d, s)| format!("{}:{}", est::date_string(d), s))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn reproduces_every_scenario() {
    let cases = load();
    assert_eq!(cases.len(), 49, "the cases file holds 49 scenarios");
    for case in &cases {
        let name = case.name();
        let now: i64 = case.get("now").parse().unwrap();
        let (h, notices) = replay(case, Lang::PtBr);

        close_opt(name, "level", h.level(), opt_num(case.get("level")), 1e-9);
        assert_eq!(
            if h.online() { "1" } else { "0" },
            case.get("online"),
            "{name}: online"
        );
        close_rel(
            name,
            "rate",
            est::discharge_rate(&h),
            case.get("rate").parse().unwrap(),
        );
        close_rel(
            name,
            "observed_hours",
            h.w,
            case.get("observed_hours").parse().unwrap(),
        );
        assert_eq!(
            opt_text(est::forecast_label(&h, Lang::PtBr)),
            case.get("label"),
            "{name}: label"
        );

        let hl = est::hours_left(&h);
        close_opt(
            name,
            "hours_left",
            hl,
            opt_num(case.get("hours_left")),
            1.0 / 60.0,
        );
        assert_eq!(
            opt_text(hl.map(|x| est::format_hours_left(x, Lang::PtBr))),
            case.get("hours_left_text"),
            "{name}: hours_left_text"
        );
        assert_eq!(
            opt_text(est::forecast_text(&h, Lang::PtBr)),
            case.get("forecast_text"),
            "{name}: forecast_text"
        );

        let eta = est::eta(&h, now).map(|t| est::round_half_up(t) as f64);
        close_opt(name, "eta", eta, opt_num(case.get("eta")), 60.0);
        assert_eq!(
            opt_text(est::eta_text(&h, now, Lang::PtBr)),
            case.get("eta_text"),
            "{name}: eta_text"
        );

        close_rel(
            name,
            "charge_rate",
            est::charge_rate(&h),
            case.get("charge_rate").parse().unwrap(),
        );
        close_opt(
            name,
            "full_hours",
            est::time_to_full(&h),
            opt_num(case.get("full_hours")),
            1.0 / 60.0,
        );
        assert_eq!(
            opt_text(est::full_in_text(&h, Lang::PtBr)),
            case.get("full_in_text"),
            "{name}: full_in_text"
        );
        assert_eq!(
            opt_text(est::full_at_text(&h, now, Lang::PtBr)),
            case.get("full_at_text"),
            "{name}: full_at_text"
        );
        assert_eq!(
            est::usage_today_text(&h, now, Lang::PtBr),
            case.get("usage_today_text"),
            "{name}: usage_today_text"
        );
        assert_eq!(
            usage_text(&h, now),
            case.get("usage_by_day"),
            "{name}: usage_by_day"
        );

        let want_health = case.get("health");
        match est::health(&h) {
            None => assert_eq!(want_health, "aprendendo", "{name}: health"),
            Some(v) => close_opt(name, "health", Some(v), opt_num(want_health), 0.1),
        }
        assert_eq!(
            est::health_text(&h, Lang::PtBr) == "aprendendo",
            est::health(&h).is_none(),
            "{name}: health text"
        );

        assert_eq!(
            opt_text(est::smart_warning(&h, now, Lang::PtBr)),
            case.get("smart_warning"),
            "{name}: smart_warning"
        );
        assert_eq!(
            notices_text(&notices),
            case.get("notices"),
            "{name}: notices"
        );
        assert_eq!(
            if h.profile_learned(now) { "1" } else { "0" },
            case.get("profile_learned"),
            "{name}: profile_learned"
        );
        let u_now = h.profile(now)[est::hour_of_week(now + h.utc_offset)];
        close_opt(name, "u_now", Some(u_now), opt_num(case.get("u_now")), 1e-6);
    }
}

#[test]
fn english_runs_every_scenario_with_no_text_missing() {
    for case in load() {
        let name = case.name().to_string();
        let now: i64 = case.get("now").parse().unwrap();
        let (h, notices) = replay(&case, Lang::En);
        let (pt, pt_notices) = replay(&case, Lang::PtBr);
        let texts = [
            est::forecast_text(&h, Lang::En),
            est::eta_text(&h, now, Lang::En),
            est::full_in_text(&h, Lang::En),
            est::full_at_text(&h, now, Lang::En),
            est::smart_warning(&h, now, Lang::En),
            est::forecast_label(&h, Lang::En),
            Some(est::usage_today_text(&h, now, Lang::En)),
            Some(est::health_text(&h, Lang::En)),
        ];
        for t in texts.into_iter().flatten() {
            assert!(
                !t.is_empty() && !t.contains('{') && !t.contains("_"),
                "{name}: bad English text {t:?}"
            );
        }
        // same decisions in both languages
        assert_eq!(
            notices.len(),
            pt_notices.len(),
            "{name}: notices differ by language"
        );
        for (a, b) in notices.iter().zip(&pt_notices) {
            assert_eq!(
                (a.kind, a.at, a.text.is_some()),
                (b.kind, b.at, b.text.is_some()),
                "{name}"
            );
        }
        assert_eq!(est::eta(&h, now), est::eta(&pt, now), "{name}");
    }
}

#[test]
fn english_texts_read_naturally() {
    let case = load()
        .into_iter()
        .find(|c| c.name() == "eta_default_profile_today")
        .unwrap();
    let now: i64 = case.get("now").parse().unwrap();
    let (h, notices) = replay(&case, Lang::En);
    assert_eq!(est::forecast_text(&h, Lang::En).unwrap(), "~2.5 h of use");
    assert_eq!(
        est::eta_text(&h, now, Lang::En).unwrap(),
        "runs out today ~17:40"
    );
    assert_eq!(
        notices[0].text.as_deref(),
        Some("Won't last the day: charge before 15:00"),
        "{notices:?}"
    );
    let charging = load()
        .into_iter()
        .find(|c| c.name() == "charge_below_80")
        .unwrap();
    let (h, _) = replay(&charging, Lang::En);
    assert_eq!(
        est::full_in_text(&h, Lang::En).unwrap(),
        "full in ~1 h 40 min"
    );
}
