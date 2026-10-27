// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! JuiceBar's Windows drawing crate: the tray mark and the flyout.
//! See `spec/design/DESIGN.md`. Pure parts (model, layout, path parsing) have no Win32 in them.

pub mod diag;
pub mod draw;
pub mod flyout;
pub mod gfx;
pub mod glyphs;
pub mod icon;
pub mod layout;
pub mod model;
pub mod render;
pub mod svgpath;
pub mod system;
