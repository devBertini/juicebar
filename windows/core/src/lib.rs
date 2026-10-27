// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! JuiceBar's core: pure logic, no device, file, clock or thread (spec/ARCHITECTURE.md).
//! The shell feeds it transport events and the time; it answers with actions.
#![forbid(unsafe_code)]

pub mod types;

// Protocol side (lane W1a): spec/PROTOCOL.md, spec/PROVIDERS.md.
pub mod hidpp;
pub mod logitech;
pub mod registry;

// Data side (lane W1b): spec/estimator.md, spec/ARCHITECTURE.md.
pub mod estimator;
pub mod snapshot;
pub mod store;
pub mod strings;
