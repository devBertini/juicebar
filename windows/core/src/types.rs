// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The types every module shares: what a provider reports (spec/PROVIDERS.md) and the time the shell passes in.

/// A moment, always passed in by the shell: the core never reads a clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Now {
    /// Wall time, seconds since the Unix epoch, UTC.
    pub wall: f64,
    /// Monotonic seconds since the shell started (for timeouts and deadlines).
    pub mono: f64,
    /// The local offset from UTC in seconds at `wall` (America/Sao_Paulo: -10800).
    pub utc_offset: i32,
}

/// A device's stable identity: provider id plus the fields that keep it apart, hashed when one is a serial or an
/// address (spec/PROVIDERS.md, "Identity without secrets").
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceKey(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Mouse,
    Keyboard,
    Headset,
    Earbuds,
    Gamepad,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Connection {
    Receiver,
    Dongle,
    Bluetooth,
    Usb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub key: DeviceKey,
    /// The provider id: `logitech-hidpp`, `os-bluetooth`, `game-controllers`, ...
    pub provider: &'static str,
    pub name: String,
    pub kind: Kind,
    pub connection: Connection,
    pub brand: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Coarse {
    Full,
    Good,
    Low,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Charging {
    Discharging,
    Charging,
    Full,
    Unknown,
}

/// How a percentage was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    /// The device reports a percentage.
    Exact,
    /// Computed from millivolts through a curve of spec/curves.tsv.
    Derived,
    /// Only a coarse level: no forecast.
    Coarse,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatteryReading {
    /// Wall time, seconds since the Unix epoch, UTC.
    pub at: f64,
    pub percent: Option<f64>,
    pub coarse: Option<Coarse>,
    pub millivolts: Option<u16>,
    pub charging: Charging,
    pub quality: Quality,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Presence {
    pub online: bool,
    /// Wall time, seconds since the Unix epoch, UTC.
    pub at: f64,
}

/// What a provider pushes to the registry.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderEvent {
    Appeared(DeviceInfo),
    Disappeared(DeviceKey),
    Presence(DeviceKey, Presence),
    Battery(DeviceKey, BatteryReading),
}

/// What a provider declares about itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capabilities {
    pub pushes_presence: bool,
    pub pushes_battery: bool,
    /// The shortest interval `refresh` may be asked at, in seconds; `None` when the provider pushes battery.
    pub min_refresh_seconds: Option<u32>,
}

/// A HID endpoint as the transport reports it (spec/ARCHITECTURE.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    /// Session-local id the transport assigns.
    pub id: u32,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub product: String,
    pub bluetooth: bool,
}

/// What the shell feeds the core.
#[derive(Clone, Debug, PartialEq)]
pub enum TransportEvent {
    Arrived(Endpoint),
    Removed(u32),
    Report(u32, Vec<u8>),
}

/// What the core asks the shell to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Write a report to an endpoint.
    Send(u32, Vec<u8>),
    /// A provider event for the registry and the UI.
    Emit(ProviderEvent),
    /// Append these lines to a device's history file (the store's format).
    Persist(DeviceKey, Vec<String>),
    /// Call `tick` at this monotonic moment (the only timer the shell keeps).
    Wake(f64),
}
