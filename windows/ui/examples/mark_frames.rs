// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The render seam of the animated tray mark: writes PNG strips of a whole 2-second run for every
//! juice on both taskbar themes at 16 and 24 px, a 4x zoomed sheet of each run, and prints the
//! contrast of every juice against its taskbar.
//!
//! `cargo run -p juicebar-ui --example mark_frames -- --out <folder>`

use std::path::{Path, PathBuf};

use juicebar_ui::gfx::{Gfx, Pixels, write_png};
use juicebar_ui::icon::{
    IconLook, Juice, RUN_FRAMES, Run, contrast, juice_color, mark_pixels_with, over_background,
    run_frame, taskbar_background,
};

const SIZES: [u32; 2] = [16, 24];
const JUICES: [(Juice, &str); 3] = [
    (Juice::Purple, "purple"),
    (Juice::Green, "green"),
    (Juice::Red, "red"),
];
const RUNS: [(Run, &str); 2] = [(Run::Slosh, "slosh"), (Run::Bubbles, "bubbles")];
/// Pixels between two pictures of a strip (the taskbar's colour).
const GAP: u32 = 4;
/// The zoom of the sheet.
const ZOOM: u32 = 4;
/// The sheet shows every this-many-th picture of the run.
const SHEET_STEP: usize = 4;

/// One row of tiles on one taskbar colour, ready to place side by side.
struct Row {
    dark: bool,
    tiles: Vec<Pixels>,
}

/// Rows of tiles laid out left to right and top to bottom, each tile `zoom` times larger.
fn compose(rows: &[Row], tile: u32, zoom: u32) -> Pixels {
    let cell = tile * zoom;
    let cols = rows.iter().map(|r| r.tiles.len()).max().unwrap_or(0) as u32;
    let (w, h) = (
        GAP + cols * (cell + GAP),
        GAP + rows.len() as u32 * (cell + GAP),
    );
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (ri, row) in rows.iter().enumerate() {
        let bg = taskbar_background(row.dark);
        let bgp = [
            (bg.b * 255.0).round() as u8,
            (bg.g * 255.0).round() as u8,
            (bg.r * 255.0).round() as u8,
            255,
        ];
        let y0 = ri as u32 * (cell + GAP);
        for y in y0..y0 + cell + 2 * GAP {
            if y >= h {
                break;
            }
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                data[o..o + 4].copy_from_slice(&bgp);
            }
        }
        for (ti, t) in row.tiles.iter().enumerate() {
            let x0 = GAP + ti as u32 * (cell + GAP);
            for y in 0..cell {
                for x in 0..cell {
                    let s = (((y / zoom) * tile + x / zoom) * 4) as usize;
                    let o = (((y0 + GAP + y) * w + x0 + x) * 4) as usize;
                    data[o..o + 4].copy_from_slice(&t.data[s..s + 4]);
                }
            }
        }
    }
    Pixels {
        width: w,
        height: h,
        data,
    }
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(out) = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
    else {
        eprintln!("usage: mark_frames --out <folder>");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&out).expect("the output folder");
    let gfx = Gfx::new()?;
    let mut files = 0;

    for (run, run_name) in RUNS {
        for px in SIZES {
            let mut sheet: Vec<Row> = Vec::new();
            for (juice, juice_name) in JUICES {
                // Bubbles only rise while charging.
                let look = IconLook {
                    juice,
                    charging: run == Run::Bubbles,
                };
                for (dark, theme) in [(true, "dark"), (false, "light")] {
                    let tiles: Vec<Pixels> = (0..RUN_FRAMES)
                        .map(|i| {
                            let m = mark_pixels_with(&gfx, look, &run_frame(run, i), dark, px)?;
                            Ok(over_background(m, dark))
                        })
                        .collect::<windows::core::Result<_>>()?;
                    let row = Row { dark, tiles };
                    let strip = compose(std::slice::from_ref(&row), px, 1);
                    let name = format!("strip-{run_name}-{juice_name}-{theme}-{px}.png");
                    write_png(&out.join(name), &strip)?;
                    files += 1;
                    sheet.push(Row {
                        dark,
                        tiles: row.tiles.into_iter().step_by(SHEET_STEP).collect(),
                    });
                }
            }
            let name = format!("sheet4x-{run_name}-{px}.png");
            write_png(&out.join(name), &compose(&sheet, px, ZOOM))?;
            files += 1;
        }
    }

    println!("wrote {files} files to {}", display(&out));
    println!("contrast of the juice against the taskbar (WCAG, 3:1 wanted):");
    for (juice, name) in JUICES {
        for (dark, theme) in [(true, "dark"), (false, "light")] {
            let c = juice_color(juice, dark);
            println!(
                "  {name:<6} {theme:<5} #{:02x}{:02x}{:02x}  {:.2}:1",
                (c.r * 255.0).round() as u8,
                (c.g * 255.0).round() as u8,
                (c.b * 255.0).round() as u8,
                contrast(c, taskbar_background(dark))
            );
        }
    }
    Ok(())
}

fn display(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
