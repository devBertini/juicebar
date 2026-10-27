// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! Idle cost of the transport: starts it, lets the enumeration settle, idles, and prints this
//! process's CPU time and private bytes at the start and the end.
//!
//! `cargo run --release -p juicebar-hid --example idle [seconds]` (default 60).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use juicebar_hid::{Transport, TransportEvent};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

struct Sample {
    cpu_ms: f64,
    private_bytes: usize,
}

fn filetime_100ns(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

fn sample() -> Sample {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: GetCurrentProcess returns a pseudo handle; the four out pointers are valid.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .expect("GetProcessTimes");
    }
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: PROCESS_MEMORY_COUNTERS_EX starts with a PROCESS_MEMORY_COUNTERS, as the API
    // documents, and `cb` is its full size.
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
        .expect("GetProcessMemoryInfo");
    }
    Sample {
        cpu_ms: (filetime_100ns(kernel) + filetime_100ns(user)) as f64 / 10_000.0,
        private_bytes: counters.PrivateUsage,
    }
}

fn print(label: &str, s: &Sample) {
    println!(
        "{label}: cpu {:.1} ms, private bytes {} ({:.2} MiB)",
        s.cpu_ms,
        s.private_bytes,
        s.private_bytes as f64 / 1_048_576.0
    );
}

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(60);
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let transport = Transport::start(Arc::new(move || {
        counter.fetch_add(1, Ordering::Relaxed);
    }))
    .expect("transport start");

    // Let the enumeration finish, then take the first sample.
    std::thread::sleep(Duration::from_secs(1));
    let (mut arrived, mut reports) = (0usize, 0usize);
    let mut count = |events: Vec<TransportEvent>| {
        for e in events {
            match e {
                TransportEvent::Arrived(_) => arrived += 1,
                TransportEvent::Report(..) => reports += 1,
                TransportEvent::Removed(_) => {}
            }
        }
    };
    count(transport.drain());
    let start = sample();
    print("start", &start);

    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    let end = sample();
    print("end  ", &end);
    count(transport.drain());
    println!(
        "idle {:.1} s: cpu delta {:.1} ms ({:.4} % of one core), private bytes delta {}",
        t0.elapsed().as_secs_f64(),
        end.cpu_ms - start.cpu_ms,
        (end.cpu_ms - start.cpu_ms) / (t0.elapsed().as_secs_f64() * 1000.0) * 100.0,
        end.private_bytes as i64 - start.private_bytes as i64,
    );
    println!(
        "endpoints arrived {arrived}, reports {reports}, wakes {}",
        wakes.load(Ordering::Relaxed)
    );
    transport.stop();
}
