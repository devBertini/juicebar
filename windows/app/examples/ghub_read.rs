// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Live check of the calibration's G HUB client (plan stage K1): asks the running Logitech G HUB for the battery of
//! every connected device, with `GET` requests only, and prints the model JuiceBar would calibrate and G HUB's
//! percent. Writes no file and nothing to G HUB.
//!
//! `cargo run -p juicebar --example ghub_read`

#[allow(dead_code)]
#[path = "../src/calib.rs"]
mod calib;

use juicebar_core::hidpp::catalog;

fn main() {
    let found = match calib::read_batteries(calib::GHUB_ADDR, calib::TOTAL_TIMEOUT, None) {
        Ok(f) => f,
        Err(e) => {
            println!("G HUB gave nothing: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "G HUB answered: {} connected device(s) with a battery",
        found.len()
    );
    for b in &found {
        let curve = catalog::rows()
            .iter()
            .find(|r| r.id == b.pid && r.curve.is_some())
            .and_then(|r| r.curve);
        println!(
            "pid {:04X} {} -> model {} : G HUB percent {}, charging {}",
            b.pid,
            b.name,
            curve.unwrap_or("(reports its own percent, not calibrated)"),
            b.percent,
            b.charging
        );
    }
}
