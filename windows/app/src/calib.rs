// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! Calibration of the voltage curves (plan stage K1): for two weeks after the first start of this version, each
//! voltage reading of a Logitech device is written beside the percent Logitech G HUB shows for the same device, so
//! `scripts/fit_curves.py` can fit curves of JuiceBar's own from the owner's measurements.
//!
//! * G HUB is read over its local websocket (`ws://localhost:9010`, subprotocol `json`), loopback only, with `GET`
//!   requests only: nothing is ever written to G HUB and nothing leaves the computer.
//! * The client is our own (RFC 6455): TCP, the HTTP upgrade with a random key, masked client text frames, unmasked
//!   server frames, a 2 s total timeout. It runs on a short-lived worker thread, so the UI thread never waits.
//! * One line per device per 10 minutes goes to `<data>\calibration\<model>.tsv`:
//!   `<unix seconds>\t<model>\t<millivolts>\t<ghub percent>\t<charging 0/1>`. G HUB absent or answering something
//!   unknown writes nothing; the next reading tries again.
//!
//! The file depends on `std` and the core's catalog only, so the example `ghub_read` includes it unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use juicebar_core::hidpp::catalog;

/// How long calibration runs after the first start of the version, seconds.
pub const WINDOW_SECS: i64 = 14 * 24 * 3600;
/// The least time between two lines of one device, seconds.
pub const MIN_GAP_SECS: i64 = 600;
/// G HUB's websocket: loopback, never anything else.
pub const GHUB_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 9010);
/// The whole exchange with G HUB (connect, upgrade, requests, answers) gets this long.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(2);
/// The largest message accepted from G HUB (its device list is a few KiB).
const MAX_MESSAGE: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------- the schedule

/// Calibration is on while `until` (unix seconds) is in the future.
pub fn is_on(until: Option<i64>, now: i64) -> bool {
    until.is_some_and(|u| now < u)
}

/// The end of calibration for a start at `now`: the moment settings already hold (a past one too: the window
/// never opens again), else `now` plus 14 days. The flag says the moment is new and must be saved.
pub fn window_at_start(current: Option<i64>, now: i64) -> (i64, bool) {
    match current {
        Some(until) => (until, false),
        None => (now + WINDOW_SECS, true),
    }
}

/// `dd/mm` of a unix moment in the zone `utc_offset` seconds from UTC.
pub fn day_month(unix: i64, utc_offset: i32) -> String {
    let days = (unix + i64::from(utc_offset)).div_euclid(86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let _ = era;
    format!("{day:02}/{month:02}")
}

/// What decides whether a device may be sampled now: at least `MIN_GAP_SECS` since its last line, and no sample
/// of it in flight.
#[derive(Debug, Default)]
pub struct Gate {
    last: BTreeMap<String, i64>,
    busy: BTreeSet<String>,
}

impl Gate {
    /// Claims the device for a sample at `now`; false when a line was written less than ten minutes ago or a
    /// sample is running.
    pub fn begin(&mut self, key: &str, now: i64) -> bool {
        if self.busy.contains(key) {
            return false;
        }
        if let Some(last) = self.last.get(key)
            && (0..MIN_GAP_SECS).contains(&(now - last))
        {
            return false;
        }
        self.busy.insert(key.to_string());
        true
    }

    /// Releases the device; only a line written starts the ten minutes.
    pub fn finish(&mut self, key: &str, now: i64, wrote: bool) {
        self.busy.remove(key);
        if wrote {
            self.last.insert(key.to_string(), now);
        }
    }
}

// ---------------------------------------------------------------- the model of a device

/// A device the calibration covers: the curve it is fitted into and the product ids G HUB may list it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub curve: &'static str,
    pub pids: Vec<u16>,
}

/// The model behind a device key (`logitech-hidpp:4086`): the catalog's curve id and every product id of that
/// model (the three G733 dongles are one device). `None` for a device with no curve (it reports a percent itself)
/// and for other providers' keys. G HUB's device list carries the product id as the decimal `pid` field
/// (16518 = 0x4086 for the G703).
pub fn model_of(key: &str) -> Option<Model> {
    let id = key.strip_prefix("logitech-hidpp:")?;
    let id = u16::from_str_radix(id, 16).ok()?;
    let rows = catalog::rows();
    let row = rows.iter().find(|r| r.id == id && r.curve.is_some())?;
    Some(Model {
        curve: row.curve?,
        pids: rows
            .iter()
            .filter(|r| r.model == row.model)
            .map(|r| r.id)
            .collect(),
    })
}

/// One calibration line.
pub fn line(at: i64, model: &str, millivolts: u16, percent: f64, charging: bool) -> String {
    format!(
        "{at}\t{model}\t{millivolts}\t{percent}\t{}",
        u8::from(charging)
    )
}

/// Appends one line to `<dir>\<model>.tsv`, creating the folder and the file.
pub fn append_line(dir: &Path, model: &str, line: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{model}.tsv")))?;
    file.write_all(format!("{line}\n").as_bytes())
}

// ---------------------------------------------------------------- the sampler

/// Takes calibration samples on short-lived worker threads.
pub struct Calibrator {
    dir: PathBuf,
    until: i64,
    addr: SocketAddr,
    gate: Arc<Mutex<Gate>>,
}

impl Calibrator {
    pub fn new(dir: PathBuf, until: i64) -> Calibrator {
        Calibrator::with_addr(dir, until, GHUB_ADDR)
    }

    pub fn with_addr(dir: PathBuf, until: i64, addr: SocketAddr) -> Calibrator {
        Calibrator {
            dir,
            until,
            addr,
            gate: Arc::new(Mutex::new(Gate::default())),
        }
    }

    /// A voltage reading of the device `key` at wall time `wall`. When calibration is on, the device has a model
    /// and the ten minutes have passed, a worker reads G HUB's percent and writes the line. True when a worker
    /// started.
    pub fn sample(&self, key: &str, millivolts: u16, charging: bool, wall: i64) -> bool {
        if !is_on(Some(self.until), wall) {
            return false;
        }
        let Some(model) = model_of(key) else {
            return false;
        };
        let begun = self.gate.lock().is_ok_and(|mut g| g.begin(key, wall));
        if !begun {
            return false;
        }
        let (dir, addr, gate, key_owned) = (
            self.dir.clone(),
            self.addr,
            Arc::clone(&self.gate),
            key.to_string(),
        );
        let started = std::thread::Builder::new()
            .name("juicebar-calibration".into())
            .spawn(move || {
                let wrote = take_sample(addr, &dir, &model, millivolts, charging, wall);
                if let Ok(mut g) = gate.lock() {
                    g.finish(&key_owned, wall, wrote);
                }
            });
        if started.is_err()
            && let Ok(mut g) = self.gate.lock()
        {
            g.finish(key, wall, false);
        }
        started.is_ok()
    }
}

/// Reads G HUB's percent for the model and appends the line; false when G HUB gave nothing usable.
pub fn take_sample(
    addr: SocketAddr,
    dir: &Path,
    model: &Model,
    millivolts: u16,
    charging: bool,
    wall: i64,
) -> bool {
    let Ok(found) = read_batteries(addr, TOTAL_TIMEOUT, Some(&model.pids)) else {
        return false;
    };
    let Some(b) = found.first() else {
        return false;
    };
    let text = line(
        wall,
        model.curve,
        millivolts,
        b.percent,
        charging || b.charging,
    );
    append_line(dir, model.curve, &text).is_ok()
}

// ---------------------------------------------------------------- G HUB's answers

/// The battery of one device G HUB holds, as it answered.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceBattery {
    pub id: String,
    pub pid: u16,
    pub name: String,
    pub percent: f64,
    pub charging: bool,
}

/// One connection to G HUB: the active devices' batteries, with `GET` requests only. `pids` keeps only the
/// devices with those product ids. A device that is not `ACTIVE`, or whose battery G HUB will not give, is left
/// out. Any failure of the exchange is an error and nothing is returned.
pub fn read_batteries(
    addr: SocketAddr,
    timeout: Duration,
    pids: Option<&[u16]>,
) -> io::Result<Vec<DeviceBattery>> {
    let mut ws = Ws::connect(addr, timeout)?;
    let out = batteries_over(&mut ws, pids);
    ws.close();
    out
}

fn batteries_over(ws: &mut Ws, pids: Option<&[u16]>) -> io::Result<Vec<DeviceBattery>> {
    let list = get(ws, "1", "/devices/list")?;
    let infos = list
        .get("payload")
        .and_then(|p| p.get("deviceInfos"))
        .and_then(Json::as_arr)
        .ok_or_else(|| invalid("G HUB's device list has no deviceInfos"))?;
    let mut out = Vec::new();
    for (n, info) in infos.iter().enumerate() {
        let (Some(id), Some(pid)) = (
            info.get("id").and_then(Json::as_str),
            info.get("pid")
                .and_then(Json::as_f64)
                .and_then(|p| u16::try_from(p as i64).ok()),
        ) else {
            continue;
        };
        let active = info.get("state").and_then(Json::as_str) == Some("ACTIVE");
        let wanted = pids.is_none_or(|w| w.contains(&pid));
        let id_is_plain = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !(active && wanted && id_is_plain) {
            continue;
        }
        let answer = get(ws, &format!("b{}", n + 2), &format!("/battery/{id}/state"))?;
        let Some(payload) = answer.get("payload") else {
            continue;
        };
        let percent = payload.get("percentage").and_then(Json::as_f64);
        let Some(percent) = percent.filter(|p| (0.0..=100.0).contains(p)) else {
            continue;
        };
        out.push(DeviceBattery {
            id: id.to_string(),
            pid,
            name: info
                .get("displayName")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            percent,
            charging: payload.get("charging").and_then(Json::as_bool) == Some(true),
        });
    }
    Ok(out)
}

/// One `GET`: the answer with the same `msgId`, other messages (G HUB's events) skipped. An answer whose
/// `result.code` is not `SUCCESS` comes back without a payload.
fn get(ws: &mut Ws, msg_id: &str, path: &str) -> io::Result<Json> {
    ws.send_text(&format!(
        "{{\"msgId\":\"{msg_id}\",\"verb\":\"GET\",\"path\":\"{path}\"}}"
    ))?;
    loop {
        let text = ws.recv_text()?;
        let Some(json) = Json::parse(&text) else {
            continue;
        };
        if json.get("msgId").and_then(Json::as_str) != Some(msg_id) {
            continue;
        }
        let ok = json
            .get("result")
            .and_then(|r| r.get("code"))
            .and_then(Json::as_str)
            == Some("SUCCESS");
        return Ok(if ok { json } else { Json::Obj(Vec::new()) });
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

// ---------------------------------------------------------------- JSON

/// A parsed JSON value (just enough to read G HUB's answers).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// The whole text as one value; `None` for anything else.
    pub fn parse(text: &str) -> Option<Json> {
        let mut p = JsonParser {
            b: text.as_bytes(),
            i: 0,
        };
        let v = p.value(0)?;
        p.space();
        (p.i == p.b.len()).then_some(v)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) if n.is_finite() => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}

const MAX_DEPTH: usize = 32;

impl JsonParser<'_> {
    fn space(&mut self) {
        while self
            .b
            .get(self.i)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            return true;
        }
        false
    }

    fn word(&mut self, w: &str, v: Json) -> Option<Json> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            return Some(v);
        }
        None
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.space();
        match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                if self.eat(b'}') {
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    fields.push((key, self.value(depth + 1)?));
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b'}').then_some(Json::Obj(fields));
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b']').then_some(Json::Arr(items));
                }
            }
            b'"' => self.string().map(Json::Str),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'n' => self.word("null", Json::Null),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while self
                    .b
                    .get(self.i)
                    .is_some_and(|c| matches!(c, b'+' | b'-' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.b[start..self.i]).ok()?;
                text.parse::<f64>().ok().map(Json::Num)
            }
            _ => None,
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.b.get(self.i..self.i + 4)?;
        let text = std::str::from_utf8(digits).ok()?;
        let n = u32::from_str_radix(text, 16).ok()?;
        self.i += 4;
        Some(n)
    }

    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'"' | b'\\' | b'/' => char::from(e),
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            if (0xD800..0xDC00).contains(&hi)
                                && self.b[self.i..].starts_with(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo & 0x3FF))
                                    .unwrap_or('\u{FFFD}')
                            } else {
                                char::from_u32(hi).unwrap_or('\u{FFFD}')
                            }
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return None,
                c => out.push(c),
            }
        }
    }
}

// ---------------------------------------------------------------- SHA-1 and base64 (the upgrade handshake)

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (hv, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hv = hv.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (slot, v) in out.chunks_mut(4).zip(h) {
        slot.copy_from_slice(&v.to_be_bytes());
    }
    out
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The `Sec-WebSocket-Accept` value a server must answer to a client's `Sec-WebSocket-Key` (RFC 6455, 4.2.2).
pub fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{key}{WS_GUID}").as_bytes()))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(8) {
        // A fresh RandomState carries keys the system seeded; the counter and the clock keep two calls apart.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        let bytes = hasher.finish().to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    out
}

/// The upgrade request for `key` (the subprotocol G HUB speaks is `json`).
pub fn upgrade_request(host: &str, key: &str) -> String {
    format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: json\r\n\r\n"
    )
}

/// Checks the server's answer to the upgrade: status 101, `Upgrade: websocket`, the accept value for `key`, and
/// no subprotocol other than `json`.
pub fn check_upgrade(response: &str, key: &str) -> Result<(), String> {
    let mut lines = response.split("\r\n");
    let status = lines.next().unwrap_or("");
    if status.split(' ').nth(1) != Some("101") {
        return Err(format!("the upgrade was refused: {status}"));
    }
    let (mut upgrade, mut accept, mut protocol) = (None, None, None);
    for l in lines {
        let Some((name, value)) = l.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match name.trim().to_ascii_lowercase().as_str() {
            "upgrade" => upgrade = Some(value),
            "sec-websocket-accept" => accept = Some(value),
            "sec-websocket-protocol" => protocol = Some(value),
            _ => {}
        }
    }
    if !upgrade.is_some_and(|u| u.eq_ignore_ascii_case("websocket")) {
        return Err("the answer is not a websocket upgrade".into());
    }
    if accept.as_deref() != Some(accept_key(key).as_str()) {
        return Err("the accept key does not match".into());
    }
    if protocol.is_some_and(|p| p != "json") {
        return Err("the server chose a subprotocol that was not offered".into());
    }
    Ok(())
}

// ---------------------------------------------------------------- frames

const OP_CONT: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// One client frame: FIN set, masked with `mask` (RFC 6455, 5.2 and 5.3).
pub fn client_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => out.push(0x80 | n as u8),
        n if n <= usize::from(u16::MAX) => {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// A frame from the server.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Reads one server frame. A masked frame, a reserved bit, a fragmented or oversized control frame and a
/// message over `MAX_MESSAGE` are protocol errors.
pub fn read_frame(r: &mut impl Read) -> io::Result<Frame> {
    let mut head = [0u8; 2];
    r.read_exact(&mut head)?;
    let (fin, reserved, opcode) = (head[0] & 0x80 != 0, head[0] & 0x70, head[0] & 0x0F);
    let masked = head[1] & 0x80 != 0;
    let mut len = u64::from(head[1] & 0x7F);
    if len == 126 {
        let mut b = [0u8; 2];
        r.read_exact(&mut b)?;
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b)?;
        len = u64::from_be_bytes(b);
    }
    if reserved != 0 || masked {
        return Err(invalid("the server sent a masked frame or reserved bits"));
    }
    if len > MAX_MESSAGE as u64 || (opcode >= 8 && (len > 125 || !fin)) {
        return Err(invalid("the server sent an oversized or fragmented frame"));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    Ok(Frame {
        fin,
        opcode,
        payload,
    })
}

// ---------------------------------------------------------------- the connection

/// Reads from a stream until a deadline.
struct Timed<'a> {
    stream: &'a TcpStream,
    end: Instant,
}

impl Read for Timed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self
            .end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "G HUB did not answer in time")
            })?;
        self.stream.set_read_timeout(Some(left))?;
        let mut s = self.stream;
        s.read(buf)
    }
}

/// A websocket client connection with a deadline for everything it does.
pub struct Ws {
    stream: TcpStream,
    end: Instant,
}

impl Ws {
    /// Connects to `addr` and upgrades, all within `timeout`.
    pub fn connect(addr: SocketAddr, timeout: Duration) -> io::Result<Ws> {
        let end = Instant::now() + timeout;
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_nodelay(true)?;
        let left = end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "connect took too long"))?;
        stream.set_write_timeout(Some(left))?;
        let mut ws = Ws { stream, end };
        let key = base64(&random_bytes::<16>());
        let request = upgrade_request(&format!("localhost:{}", addr.port()), &key);
        (&ws.stream).write_all(request.as_bytes())?;
        let response = ws.read_head()?;
        check_upgrade(&response, &key).map_err(|e| invalid(&e))?;
        Ok(ws)
    }

    /// The HTTP answer up to the blank line, byte by byte so nothing of the first frame is swallowed.
    fn read_head(&mut self) -> io::Result<String> {
        let mut head = Vec::new();
        let mut reader = Timed {
            stream: &self.stream,
            end: self.end,
        };
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() > 8192 {
                return Err(invalid("the upgrade answer is too long"));
            }
            reader.read_exact(&mut byte)?;
            head.push(byte[0]);
        }
        String::from_utf8(head).map_err(|_| invalid("the upgrade answer is not text"))
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let frame = client_frame(opcode, payload, random_bytes::<4>());
        (&self.stream).write_all(&frame)
    }

    pub fn send_text(&mut self, text: &str) -> io::Result<()> {
        self.send_frame(OP_TEXT, text.as_bytes())
    }

    /// The next complete text message: fragments are joined, a ping is answered with a pong, a close is an error.
    pub fn recv_text(&mut self) -> io::Result<String> {
        let mut message: Vec<u8> = Vec::new();
        let mut started = false;
        loop {
            let frame = read_frame(&mut Timed {
                stream: &self.stream,
                end: self.end,
            })?;
            match frame.opcode {
                OP_TEXT | OP_BINARY if !started => {
                    started = true;
                    message = frame.payload;
                }
                OP_CONT if started => message.extend_from_slice(&frame.payload),
                OP_PING => {
                    self.send_frame(OP_PONG, &frame.payload)?;
                    continue;
                }
                OP_PONG => continue,
                OP_CLOSE => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "G HUB closed the connection",
                    ));
                }
                _ => return Err(invalid("an unexpected websocket frame")),
            }
            if message.len() > MAX_MESSAGE {
                return Err(invalid("a message over the limit"));
            }
            if frame.fin {
                return String::from_utf8(message)
                    .map_err(|_| invalid("a message that is not text"));
            }
        }
    }

    /// Says goodbye (close, normal) and drops the connection; failures do not matter.
    pub fn close(mut self) {
        let _ = self.send_frame(OP_CLOSE, &1000u16.to_be_bytes());
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    #[test]
    fn sha1_and_base64_match_their_published_vectors() {
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        let hex: String = sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn the_accept_key_is_the_one_of_rfc_6455() {
        // RFC 6455, 1.3: the opening handshake's example.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn the_upgrade_answer_is_checked_against_the_key() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let good = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nsec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Protocol: json\r\n\r\n";
        assert_eq!(check_upgrade(good, key), Ok(()));
        assert!(check_upgrade(&good.replace("s3pPLM", "s3pXXX"), key).is_err());
        assert!(check_upgrade(&good.replace("101 Switching", "200 OK"), key).is_err());
        assert!(check_upgrade(&good.replace("websocket\r\nConn", "nothing\r\nConn"), key).is_err());
        assert!(check_upgrade(&good.replace(": json", ": soap"), key).is_err());
        let no_protocol = good.replace("Sec-WebSocket-Protocol: json\r\n", "");
        assert_eq!(check_upgrade(&no_protocol, key), Ok(()));
        let request = upgrade_request("localhost:9010", key);
        assert!(request.starts_with("GET / HTTP/1.1\r\nHost: localhost:9010\r\n"));
        assert!(request.contains("Sec-WebSocket-Protocol: json\r\n"));
        assert!(request.contains("Sec-WebSocket-Version: 13\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
    }

    #[test]
    fn a_client_frame_is_masked_as_in_rfc_6455() {
        // RFC 6455, 5.7: a single-frame masked text message "Hello".
        let frame = client_frame(OP_TEXT, b"Hello", [0x37, 0xfa, 0x21, 0x3d]);
        assert_eq!(
            frame,
            [
                0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58
            ]
        );
    }

    #[test]
    fn long_payloads_use_the_extended_length_fields() {
        let mask = [1, 2, 3, 4];
        let f = client_frame(OP_TEXT, &[b'a'; 200], mask);
        assert_eq!(f[..4], [0x81, 0xFE, 0x00, 0xC8]);
        assert_eq!(f.len(), 4 + 4 + 200);
        let f = client_frame(OP_TEXT, &vec![b'a'; 70_000], mask);
        assert_eq!(f[..2], [0x81, 0xFF]);
        assert_eq!(f[2..10], 70_000u64.to_be_bytes());
        assert_eq!(f.len(), 10 + 4 + 70_000);
        let empty = client_frame(OP_CLOSE, &[], mask);
        assert_eq!(empty, [0x88, 0x80, 1, 2, 3, 4]);
    }

    fn server_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![if fin { 0x80 } else { 0 } | opcode];
        if payload.len() < 126 {
            out.push(payload.len() as u8);
        } else {
            out.push(126);
            out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn server_frames_are_read_and_masked_ones_refused() {
        let mut bytes = io::Cursor::new(server_frame(true, OP_TEXT, b"hi"));
        assert_eq!(
            read_frame(&mut bytes).unwrap(),
            Frame {
                fin: true,
                opcode: OP_TEXT,
                payload: b"hi".to_vec()
            }
        );
        let long = vec![b'x'; 300];
        let mut bytes = io::Cursor::new(server_frame(true, OP_TEXT, &long));
        assert_eq!(read_frame(&mut bytes).unwrap().payload.len(), 300);
        let mut masked = io::Cursor::new(vec![0x81, 0x82, 0, 0, 0, 0, b'h', b'i']);
        assert!(read_frame(&mut masked).is_err());
        let mut reserved = io::Cursor::new(vec![0xC1, 0x00]);
        assert!(read_frame(&mut reserved).is_err());
        let mut fragmented_ping = io::Cursor::new(vec![OP_PING, 0x00]);
        assert!(read_frame(&mut fragmented_ping).is_err());
        let mut huge = vec![0x81, 127];
        huge.extend_from_slice(&(MAX_MESSAGE as u64 + 1).to_be_bytes());
        assert!(read_frame(&mut io::Cursor::new(huge)).is_err());
        let mut cut = io::Cursor::new(vec![0x81, 0x05, b'a']);
        assert!(read_frame(&mut cut).is_err());
    }

    #[test]
    fn json_reads_nested_values_escapes_and_refuses_garbage() {
        let j = Json::parse(
            r#" {"a": [1, -2.5e1, true, null], "s": "x\"\u00e9\ud83d\ude00\n", "o": {"k": false}} "#,
        )
        .unwrap();
        assert_eq!(
            j.get("a").and_then(Json::as_arr).map(<[Json]>::len),
            Some(4)
        );
        assert_eq!(
            j.get("a").unwrap().as_arr().unwrap()[1].as_f64(),
            Some(-25.0)
        );
        assert_eq!(
            j.get("s").and_then(Json::as_str),
            Some("x\"\u{e9}\u{1F600}\n")
        );
        assert_eq!(
            j.get("o").and_then(|o| o.get("k")).and_then(Json::as_bool),
            Some(false)
        );
        assert_eq!(j.get("missing"), None);
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":}",
            "nul",
            "1 2",
            "\"\\x\"",
            "{\"a\" 1}",
        ] {
            assert_eq!(Json::parse(bad), None, "{bad:?}");
        }
        let deep = "[".repeat(40) + &"]".repeat(40);
        assert_eq!(Json::parse(&deep), None);
    }

    #[test]
    fn the_calibration_window_opens_once_and_closes_for_good() {
        let now = 1_793_035_800;
        assert_eq!(window_at_start(None, now), (now + WINDOW_SECS, true));
        assert_eq!(window_at_start(Some(now - 5), now), (now - 5, false));
        let until = now + WINDOW_SECS;
        assert!(is_on(Some(until), now));
        assert!(is_on(Some(until), until - 1));
        assert!(!is_on(Some(until), until));
        assert!(!is_on(Some(until), until + 86_400));
        assert!(!is_on(None, now));
        assert_eq!(WINDOW_SECS, 1_209_600);
    }

    #[test]
    fn the_end_date_is_day_and_month_in_the_local_zone() {
        assert_eq!(day_month(0, 0), "01/01");
        assert_eq!(day_month(951_782_400, 0), "29/02");
        assert_eq!(day_month(951_782_400 - 1, 0), "28/02");
        assert_eq!(day_month(951_782_400, -10_800), "28/02");
        // 2026-10-26 14:30 in Sao Paulo, plus the 14 days.
        assert_eq!(day_month(1_793_035_800, -10_800), "26/10");
        assert_eq!(day_month(1_793_035_800 + WINDOW_SECS, -10_800), "09/11");
        assert_eq!(day_month(1_798_761_599, 0), "31/12");
    }

    #[test]
    fn a_device_is_sampled_at_most_once_in_ten_minutes_and_one_at_a_time() {
        let mut g = Gate::default();
        assert!(g.begin("a", 1000));
        assert!(!g.begin("a", 1001), "a sample of it is running");
        assert!(g.begin("b", 1001), "another device is independent");
        g.finish("a", 1000, true);
        assert!(!g.begin("a", 1000 + 599));
        assert!(g.begin("a", 1000 + 600));
        // A sample that wrote nothing starts no wait.
        g.finish("a", 1600, false);
        assert!(g.begin("a", 1601));
        g.finish("a", 1601, true);
        // A clock that went back does not block the device for good.
        assert!(g.begin("a", 1));
    }

    #[test]
    fn the_line_has_five_tab_separated_fields() {
        assert_eq!(
            line(1_793_035_800, "g703", 3968, 77.0, false),
            "1793035800\tg703\t3968\t77\t0"
        );
        assert_eq!(line(1, "g733", 3752, 45.5, true), "1\tg733\t3752\t45.5\t1");
    }

    #[test]
    fn a_device_key_maps_to_its_curve_and_every_product_id_of_the_model() {
        let g703 = model_of("logitech-hidpp:4086").unwrap();
        assert_eq!(g703.curve, "g703");
        assert_eq!(g703.pids, vec![0x4086]);
        // 16518 is the decimal pid G HUB lists for the G703.
        assert!(g703.pids.contains(&16518));
        let g733 = model_of("logitech-hidpp:0AB5").unwrap();
        assert_eq!(g733.curve, "g733");
        assert_eq!(g733.pids, vec![0x0AB5, 0x0AFE, 0x0B1F]);
        assert_eq!(model_of("logitech-hidpp:0AFE").unwrap().curve, "g733");
        // The G915 X reports a percent itself: no curve, nothing to calibrate.
        assert_eq!(model_of("logitech-hidpp:40B5"), None);
        assert_eq!(model_of("game-controllers:054C:0CE6"), None);
        assert_eq!(model_of("logitech-hidpp:ZZZZ"), None);
        assert_eq!(model_of("logitech-hidpp:4086:abc"), None);
    }

    // ---- a fake G HUB on loopback

    struct Hub {
        devices: String,
        batteries: Vec<(&'static str, String)>,
        /// Send a ping and a fragmented message before the first answer.
        chatty: bool,
        /// Answer every request with something unknown.
        nonsense: bool,
    }

    fn client_message(s: &mut TcpStream) -> io::Result<Option<(u8, Vec<u8>)>> {
        let mut head = [0u8; 2];
        s.read_exact(&mut head)?;
        assert!(head[1] & 0x80 != 0, "client frames are masked");
        let mut len = usize::from(head[1] & 0x7F);
        if len == 126 {
            let mut b = [0u8; 2];
            s.read_exact(&mut b)?;
            len = usize::from(u16::from_be_bytes(b));
        }
        let mut mask = [0u8; 4];
        s.read_exact(&mut mask)?;
        let mut payload = vec![0u8; len];
        s.read_exact(&mut payload)?;
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        Ok(Some((head[0] & 0x0F, payload)))
    }

    fn serve(hub: Hub) -> (SocketAddr, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            let (mut s, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                s.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let key = head
                .lines()
                .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .to_string();
            seen.push(head);
            let reply = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: json\r\n\r\n",
                accept_key(&key)
            );
            s.write_all(reply.as_bytes()).unwrap();
            // G HUB greets with an OPTIONS message.
            let greeting = br#"{"msgId":"","verb":"OPTIONS","path":"/","origin":"backend"}"#;
            s.write_all(&server_frame(true, OP_TEXT, greeting)).unwrap();
            let mut chatty = hub.chatty;
            while let Ok(Some((op, payload))) = client_message(&mut s) {
                if op == OP_CLOSE {
                    seen.push("close".into());
                    break;
                }
                if op == OP_PONG {
                    seen.push("pong".into());
                    continue;
                }
                let text = String::from_utf8(payload).unwrap();
                seen.push(text.clone());
                let request = Json::parse(&text).unwrap();
                let id = request
                    .get("msgId")
                    .and_then(Json::as_str)
                    .unwrap()
                    .to_string();
                let path = request
                    .get("path")
                    .and_then(Json::as_str)
                    .unwrap()
                    .to_string();
                if chatty {
                    chatty = false;
                    s.write_all(&server_frame(true, OP_PING, b"p")).unwrap();
                    let half = br#"{"msgId":"zzz","verb":"EV"#;
                    s.write_all(&server_frame(false, OP_TEXT, half)).unwrap();
                    s.write_all(&server_frame(true, OP_CONT, br#"ENT"}"#))
                        .unwrap();
                }
                let (code, payload) = if hub.nonsense {
                    ("WHAT", String::new())
                } else if path == "/devices/list" {
                    ("SUCCESS", hub.devices.clone())
                } else if let Some(b) = hub
                    .batteries
                    .iter()
                    .find(|(d, _)| path == format!("/battery/{d}/state"))
                {
                    ("SUCCESS", b.1.clone())
                } else {
                    ("INVALID_ARG", "null".into())
                };
                let payload = if payload.is_empty() {
                    "null".into()
                } else {
                    payload
                };
                let answer = format!(
                    r#"{{"msgId":"{id}","verb":"GET","path":"{path}","origin":"backend","result":{{"code":"{code}","what":""}},"payload":{payload}}}"#
                );
                s.write_all(&server_frame(true, OP_TEXT, answer.as_bytes()))
                    .unwrap();
            }
            seen
        });
        (addr, handle)
    }

    fn standard_hub() -> Hub {
        // The shape measured on 2026-10-27 (docs/research/probe-windows.md), serials and paths left out.
        let devices = r#"{"deviceInfos":[
            {"id":"dev00000000","pid":2814,"state":"NOT_CONNECTED","displayName":"G733"},
            {"id":"dev00000001","pid":16518,"state":"ACTIVE","displayName":"G703 LIGHTSPEED"},
            {"id":"dev00000002","pid":16565,"state":"ACTIVE","displayName":"G915 X LIGHTSPEED"},
            {"id":"dev00000003","pid":16565,"state":"ACTIVE","displayName":"No battery"}
        ]}"#;
        Hub {
            devices: devices.to_string(),
            batteries: vec![
                (
                    "dev00000001",
                    r#"{"@type":"x","deviceId":"dev00000001","percentage":72,"charging":false}"#
                        .into(),
                ),
                (
                    "dev00000002",
                    r#"{"percentage":49.5,"charging":true}"#.into(),
                ),
            ],
            chatty: false,
            nonsense: false,
        }
    }

    #[test]
    fn the_client_reads_the_active_devices_batteries_from_a_fake_g_hub() {
        let (addr, server) = serve(Hub {
            chatty: true,
            ..standard_hub()
        });
        let found = read_batteries(addr, Duration::from_secs(2), None).unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            found,
            vec![
                DeviceBattery {
                    id: "dev00000001".into(),
                    pid: 0x4086,
                    name: "G703 LIGHTSPEED".into(),
                    percent: 72.0,
                    charging: false
                },
                DeviceBattery {
                    id: "dev00000002".into(),
                    pid: 0x40B5,
                    name: "G915 X LIGHTSPEED".into(),
                    percent: 49.5,
                    charging: true
                },
            ]
        );
        assert!(seen[0].contains("Sec-WebSocket-Protocol: json\r\n"));
        assert!(seen[1].contains(r#""verb":"GET""#) && seen[1].contains("/devices/list"));
        assert!(
            seen.iter().any(|s| s == "pong"),
            "the ping was answered: {seen:?}"
        );
        assert_eq!(seen.last().map(String::as_str), Some("close"));
        // Only GET requests were ever sent.
        assert!(
            seen.iter()
                .filter(|s| s.starts_with('{'))
                .all(|s| s.contains(r#""verb":"GET""#)),
            "{seen:?}"
        );
    }

    #[test]
    fn the_product_id_filter_keeps_one_model() {
        let (addr, server) = serve(standard_hub());
        let found = read_batteries(addr, Duration::from_secs(2), Some(&[0x4086])).unwrap();
        let seen = server.join().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, 0x4086);
        assert!(
            !seen.iter().any(|s| s.contains("dev00000002")),
            "no request for a device that was not wanted: {seen:?}"
        );
    }

    #[test]
    fn a_sample_writes_the_line_with_gs_hubs_percent() {
        let dir = temp_dir("sample");
        let (addr, server) = serve(standard_hub());
        let model = model_of("logitech-hidpp:4086").unwrap();
        assert!(take_sample(addr, &dir, &model, 3968, false, 1_793_035_800));
        server.join().unwrap();
        let text = fs::read_to_string(dir.join("g703.tsv")).unwrap();
        assert_eq!(text, "1793035800\tg703\t3968\t72\t0\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn g_hub_absent_or_unintelligible_writes_nothing() {
        let dir = temp_dir("nothing");
        let model = model_of("logitech-hidpp:4086").unwrap();
        // Nobody listens: connect is refused at once.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = closed.local_addr().unwrap();
        drop(closed);
        assert!(!take_sample(addr, &dir, &model, 3968, false, 1));
        // An answer that is not SUCCESS.
        let (addr, server) = serve(Hub {
            nonsense: true,
            ..standard_hub()
        });
        assert!(!take_sample(addr, &dir, &model, 3968, false, 1));
        server.join().unwrap();
        // The model is not among the active devices.
        let (addr, server) = serve(standard_hub());
        let g733 = model_of("logitech-hidpp:0AB5").unwrap();
        assert!(!take_sample(addr, &dir, &g733, 3752, false, 1));
        server.join().unwrap();
        assert!(!dir.exists(), "no file and no folder were made");
    }

    #[test]
    fn a_server_that_never_answers_gives_up_inside_the_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let start = Instant::now();
        let r = read_batteries(addr, Duration::from_millis(300), None);
        assert!(r.is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(listener);
    }

    #[test]
    fn the_calibrator_samples_a_device_once_per_ten_minutes_and_only_while_on() {
        let dir = temp_dir("calibrator");
        let wall = 1_793_035_800;
        let (addr, server) = serve(standard_hub());
        let cal = Calibrator::with_addr(dir.clone(), wall + WINDOW_SECS, addr);
        assert!(cal.sample("logitech-hidpp:4086", 3968, false, wall));
        // Wait for the worker: the line is there and the gate has released the device.
        let file = dir.join("g703.tsv");
        let start = Instant::now();
        while !file.exists() && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        server.join().unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("{wall}\tg703\t3968\t72\t0\n")
        );
        while cal
            .gate
            .lock()
            .unwrap()
            .busy
            .contains("logitech-hidpp:4086")
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!cal.sample("logitech-hidpp:4086", 3960, false, wall + 599));
        // A device with no curve and a moment after the window start nothing.
        assert!(!cal.sample("logitech-hidpp:40B5", 3900, false, wall + 700));
        assert!(!cal.sample("logitech-hidpp:4086", 3960, false, wall + WINDOW_SECS));
        assert_eq!(fs::read_to_string(&file).unwrap().lines().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "juicebar-calib-test-{}-{name}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
