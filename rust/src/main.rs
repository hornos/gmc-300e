//! gmc300e - control a GQ GMC-300E / GMC-300Re Geiger counter over USB.
//!
//! Rust port of ../gmc300e.sh. No third-party crates: serial I/O is done with
//! raw POSIX calls (termios/poll) so the binary stays dependency-free.
//!
//! Protocol: GQ-RFC1201 v1.40 / GQ-RFC1801 v1.00. The counter presents a
//! CH340 USB-serial adapter (1a86:7523) and talks 57600 8N1. Commands are
//! host-initiated, start with '<' and end with '>>'; multi-byte integers are
//! big-endian.

use std::env;
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Minimal libc bindings (Linux/glibc). Keeps the crate dependency-free.
// ---------------------------------------------------------------------------

#[repr(C, align(8))]
struct Termios {
    // Opaque storage; large enough for Linux' 60-byte struct termios.
    bytes: [u8; 64],
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

extern "C" {
    fn tcgetattr(fd: RawFd, termios: *mut Termios) -> i32;
    fn tcsetattr(fd: RawFd, action: i32, termios: *const Termios) -> i32;
    fn cfmakeraw(termios: *mut Termios);
    fn cfsetspeed(termios: *mut Termios, speed: u32) -> i32;
    fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
    fn read(fd: RawFd, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: RawFd, buf: *const c_void, count: usize) -> isize;
    fn signal(sig: i32, handler: usize) -> usize;
    fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
}

const TCSANOW: i32 = 0;
const POLLIN: i16 = 0x0001;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
const SIG_DFL: usize = 0;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: i32) {
    STOP.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// output helpers
// ---------------------------------------------------------------------------

fn log(msg: impl std::fmt::Display) {
    println!("{msg}");
}

fn warn(msg: impl std::fmt::Display) {
    eprintln!("warning: {msg}");
}

fn note(verbose: bool, msg: impl std::fmt::Display) {
    if verbose {
        eprintln!("[i] {msg}");
    }
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    process::exit(1);
}

fn color_code(level: &str) -> &'static str {
    match level {
        "green" => "\x1b[32m",
        "yellow" => "\x1b[33m",
        "red" => "\x1b[31m",
        _ => "\x1b[0m",
    }
}

// ---------------------------------------------------------------------------
// serial port
// ---------------------------------------------------------------------------

/// glibc's cfsetspeed() takes the numeric baud rate (B57600 == 57600), not
/// the raw kernel cbaud bit pattern (0x1001). Only standard rates are accepted.
fn speed_constant(baud: u32) -> Option<u32> {
    match baud {
        0 | 50 | 75 | 110 | 134 | 150 | 200 | 300 | 600 | 1200 | 1800 | 2400 | 4800 | 9600
        | 19200 | 38400 | 57600 | 115200 | 230400 | 460800 | 500000 | 576000 | 921600 | 1000000
        | 1152000 | 1500000 | 2000000 | 2500000 | 3000000 | 3500000 | 4000000 => Some(baud),
        _ => None,
    }
}

struct Serial {
    fd: RawFd,
    _file: File,
    verbose: bool,
}

impl Serial {
    fn from_file(file: File, baud: u32, verbose: bool) -> Self {
        let fd = file.as_raw_fd();
        let speed =
            speed_constant(baud).unwrap_or_else(|| die(format!("unsupported baud rate: {baud}")));
        let mut t = Termios { bytes: [0; 64] };
        if unsafe { tcgetattr(fd, &mut t) } != 0 {
            die("cannot read serial port attributes");
        }
        unsafe {
            cfmakeraw(&mut t);
            if cfsetspeed(&mut t, speed) != 0 {
                die("cannot set serial port speed");
            }
            // cfmakeraw gives CS8/no parity/one stop bit; mirror the shell's
            // "min 0 time 2" (0.2 s inter-byte read timeout).
            t.bytes[17 + 5] = 2; // VTIME
            t.bytes[17 + 6] = 0; // VMIN
            if tcsetattr(fd, TCSANOW, &t) != 0 {
                die("cannot configure serial port at requested baud");
            }
        }
        note(verbose, format!("opened serial port at {baud} 8N1"));
        Self {
            fd,
            _file: file,
            verbose,
        }
    }

    fn write_only(&mut self, bytes: &[u8]) {
        let mut done = 0;
        while done < bytes.len() {
            let n = unsafe {
                write(
                    self.fd,
                    bytes[done..].as_ptr() as *const c_void,
                    bytes.len() - done,
                )
            };
            if n <= 0 {
                die("serial write failed");
            }
            done += n as usize;
        }
    }

    fn tx(&mut self, cmd: &str) {
        self.write_only(cmd.as_bytes());
        note(self.verbose, format!("TX  {cmd}"));
    }

    fn tx_bytes(&mut self, vals: &[u8]) {
        self.write_only(vals);
        note(
            self.verbose,
            format!(
                "TX  <raw bytes: {}>",
                vals.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
    }

    /// Read whatever arrives within `millis`, like `timeout N od` did in bash.
    fn read_for(&mut self, millis: u64, collect: bool) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_millis(millis);
        let mut out = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
            let mut pfd = PollFd {
                fd: self.fd,
                events: POLLIN,
                revents: 0,
            };
            let rc = unsafe { poll(&mut pfd, 1, timeout_ms) };
            if rc == 0 {
                // Like `timeout N od`, keep reading until the whole window
                // expires: the counter can pause midway through a 256-byte
                // GETCFG reply.
                continue;
            }
            if rc < 0 {
                break;
            }
            let mut buf = [0u8; 4096];
            let n = unsafe { read(self.fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            if collect {
                out.extend_from_slice(&buf[..n as usize]);
            }
        }
        out
    }

    fn drain(&mut self, secs: f64) {
        self.read_for((secs * 1000.0) as u64, false);
    }

    fn rx_hex(&mut self, secs: f64) -> String {
        let bytes = self.read_for((secs * 1000.0) as u64, true);
        let hex = bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join("");
        note(
            self.verbose,
            format!("RX  {}", if hex.is_empty() { "<empty>" } else { &hex }),
        );
        hex
    }
}

// ---------------------------------------------------------------------------
// byte/hex/text helpers
// ---------------------------------------------------------------------------

fn hex_to_str(hex: &str) -> String {
    let mut bytes = Vec::new();
    let h = hex.as_bytes();
    let mut i = 0;
    while i + 1 < h.len() {
        if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&h[i..i + 2]).unwrap_or(""), 16) {
            bytes.push(b);
        }
        i += 2;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn u16be(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn display_unit_name(u: u8) -> String {
    match u {
        0 => "CPM".to_string(),
        1 => "µSv/h".to_string(),
        2 => "mR/h".to_string(),
        _ => format!("unknown(0x{u:02x})"),
    }
}

fn round6(v: f64) -> f64 {
    (v * 1_000_000.0).round() / 1_000_000.0
}

/// `od -An -f` prints floats with C's "%g" (six significant digits). The bash
/// controller parses that text, so mimic it when reading calibration floats.
fn od_float(v: f32) -> f64 {
    if v == 0.0 {
        return 0.0;
    }
    format!("{v:.5e}").parse::<f64>().unwrap_or(v as f64)
}

// ---------------------------------------------------------------------------
// time helpers
// ---------------------------------------------------------------------------

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// (full year, month, day, hour, minute, second, UTC offset in seconds)
fn local_parts(epoch: i64) -> (i32, i32, i32, i32, i32, i32, i64) {
    let t = epoch;
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    if unsafe { localtime_r(&t, &mut tm) }.is_null() {
        die("cannot convert host time");
    }
    (
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tm.tm_gmtoff,
    )
}

fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe - 719468) as i64
}

fn fmt_datetime(yy: i32, mm: i32, dd: i32, hh: i32, mi: i32, ss: i32) -> String {
    format!("20{yy:02}-{mm:02}-{dd:02} {hh:02}:{mi:02}:{ss:02}")
}

// ---------------------------------------------------------------------------
// application state
// ---------------------------------------------------------------------------

struct Ctx {
    prog: String,
    orig_args: Vec<String>,
    port: String,
    baud: u32,
    verbose: bool,
    csv: bool,
    color: bool,
    yes: bool,
    safe_max: String,
    danger_min: String,
    safe_max_v: f64,
    danger_min_v: f64,
    serial: Option<Serial>,
    display_unit: u8,
    cal_points: Vec<(f64, f64)>,
    display_factor: String,
}

impl Ctx {
    fn new() -> Self {
        let prog = env::args()
            .next()
            .and_then(|p| {
                Path::new(&p)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "gmc300e".to_string());
        let orig_args: Vec<String> = env::args().collect();
        let port = env::var("GMC_PORT").unwrap_or_default();
        let baud = env::var("GMC_BAUD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(57600);
        let safe_max = env::var("GMC_SAFE_MAX_USV").unwrap_or_else(|_| "1".to_string());
        let danger_min = env::var("GMC_DANGER_MIN_USV").unwrap_or_else(|_| "10".to_string());
        let safe_max_v = safe_max.parse().unwrap_or(1.0);
        let danger_min_v = danger_min.parse().unwrap_or(10.0);
        let mut color = io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none();
        if let Ok(v) = env::var("GMC_COLOR") {
            if v == "1" {
                color = true;
            } else if v == "0" {
                color = false;
            }
        }
        Self {
            prog,
            orig_args,
            port,
            baud,
            verbose: env::var("GMC_VERBOSE").map(|v| v != "0").unwrap_or(false),
            csv: false,
            color,
            yes: false,
            safe_max,
            danger_min,
            safe_max_v,
            danger_min_v,
            serial: None,
            display_unit: 0,
            cal_points: vec![(0.0, 0.0)],
            display_factor: "154".to_string(),
        }
    }

    fn ser(&mut self) -> &mut Serial {
        self.serial
            .as_mut()
            .unwrap_or_else(|| die("serial port is not open"))
    }

    fn detect_port(&mut self) {
        if !self.port.is_empty() {
            return;
        }
        let by_id = "/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0";
        if Path::new(by_id).exists() {
            self.port = by_id.to_string();
            return;
        }
        if let Ok(entries) = std::fs::read_dir("/dev") {
            let mut matches: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("ttyUSB"))
                        .unwrap_or(false)
                })
                .collect();
            matches.sort();
            if let Some(p) = matches.first() {
                self.port = p.to_string_lossy().into_owned();
                return;
            }
        }
        self.port = "/dev/ttyUSB0".to_string();
    }

    fn group_has_user(grp: &str, user: &str) -> bool {
        let groups = std::fs::read_to_string("/etc/group").unwrap_or_default();
        let line = groups.lines().find(|l| l.split(':').next() == Some(grp));
        line.and_then(|l| l.split(':').nth(3))
            .map(|members| members.split(',').any(|m| m == user))
            .unwrap_or(false)
    }

    fn reexec_with_uucp(&self) -> ! {
        let user = env::var("USER").unwrap_or_default();
        if Self::group_has_user("uucp", &user) {
            let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
            let cmd = self
                .orig_args
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(" ");
            note(
                self.verbose,
                "no write access, re-executing with the uucp group",
            );
            let err = Command::new("newgrp")
                .arg("uucp")
                .arg("-c")
                .arg(cmd)
                .env("GMC_REEXEC", "1")
                .exec();
            die(format!("newgrp failed: {err}"));
        }
        die(format!(
            "no write access to {} (log out and back in, use 'newgrp uucp', run with sudo, or set GMC_PORT)",
            self.port
        ));
    }

    fn open_port(&mut self) {
        if self.serial.is_some() {
            return;
        }
        self.detect_port();
        if !Path::new(&self.port).exists() {
            die(format!(
                "serial port {} does not exist (is the counter plugged in?)",
                self.port
            ));
        }
        let file = match OpenOptions::new().read(true).write(true).open(&self.port) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                if env::var("GMC_REEXEC").unwrap_or_default() != "1" {
                    self.reexec_with_uucp();
                }
                die(format!("cannot open {}: {e}", self.port));
            }
            Err(e) => die(format!("cannot open {}: {e}", self.port)),
        };
        let mut serial = Serial::from_file(file, self.baud, self.verbose);
        // A GETCPM nudge helps a just-plugged-in unit (as in GeigerLog).
        serial.tx("<GETCPM>>");
        serial.drain(0.3);
        self.serial = Some(serial);
    }

    fn ask(&mut self, cmd: &str, secs: f64) -> String {
        self.ser().tx(cmd);
        self.ser().rx_hex(secs)
    }

    fn ask_expect(&mut self, cmd: &str, bytes: usize, secs: f64, tries: usize) -> String {
        let mut h = String::new();
        for i in 1..=tries {
            h = self.ask(cmd, secs);
            if h.len() >= bytes * 2 {
                return h;
            }
            note(
                self.verbose,
                format!("retry {i}/{tries} for {cmd} (got {} byte(s))", h.len() / 2),
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        h
    }

    fn need_bytes(h: &str, n: usize, what: &str) {
        if h.len() < n * 2 {
            die(format!(
                "incomplete/no response to {what}: got {} byte(s), expected {n} (hex: {})",
                h.len() / 2,
                if h.is_empty() { "none" } else { h }
            ));
        }
    }

    fn ack_ok(h: &str, what: &str) {
        if h != "aa" && h != "00" {
            die(format!(
                "{what} failed (expected aa or 00, got '{}')",
                if h.is_empty() { "none" } else { h }
            ));
        }
    }

    fn confirm_danger(&self, desc: &str) {
        if !self.yes {
            die(format!(
                "{desc} is destructive. Re-run with --yes at the end to confirm."
            ));
        }
    }

    fn read_calibration(&mut self, cfg: &[u8]) {
        let mut points = vec![(0.0_f64, 0.0_f64)];
        for i in 0..3 {
            let off = 8 + i * 6;
            let cpm = u16be(&cfg[off..off + 2]) as f64;
            let usv = od_float(f32::from_le_bytes([
                cfg[off + 2],
                cfg[off + 3],
                cfg[off + 4],
                cfg[off + 5],
            ]));
            if usv.is_finite() && cpm > 0.0 {
                points.push((cpm, usv));
            }
        }
        if points.len() > 1 {
            let (sum, n) = points[1..]
                .iter()
                .filter(|(_, u)| *u > 0.0)
                .fold((0.0_f64, 0usize), |(s, n), (c, u)| (s + c / u, n + 1));
            self.display_factor = if n > 0 {
                format!("{:.6}", sum / n as f64)
            } else {
                "154".to_string()
            };
        }
        self.cal_points = points;
    }

    fn cpm_to_usv(&self, cpm: f64) -> f64 {
        let mut pc = 0.0;
        let mut pu = 0.0;
        let mut found = false;
        let mut result = 0.0;
        for &(ec, eu) in &self.cal_points {
            if !found && cpm <= ec {
                if ec == pc {
                    result = eu;
                } else {
                    let m = (eu - pu) / (ec - pc);
                    result = cpm * m + (eu - m * ec);
                }
                found = true;
                break;
            }
            pc = ec;
            pu = eu;
        }
        if !found {
            result = if pc > 0.0 {
                cpm * (pu / pc)
            } else {
                cpm / 153.846154
            };
        }
        round6(result)
    }

    fn format_display_value(&self, cpm: u64, decimals: usize) -> String {
        match self.display_unit {
            0 => cpm.to_string(),
            1 | 2 => {
                let mut usv = self.cpm_to_usv(cpm as f64);
                if self.display_unit == 2 {
                    usv = round6(usv / 10.0);
                }
                format!("{usv:.*}", decimals)
            }
            _ => "?".to_string(),
        }
    }

    fn level_for_usv(&self, u: f64) -> &'static str {
        if u >= self.danger_min_v {
            "red"
        } else if u >= self.safe_max_v {
            "yellow"
        } else {
            "green"
        }
    }

    fn level_label(&self, level: &str) -> String {
        match level {
            "green" => format!("NORMAL[<{}µSv/h]", self.safe_max),
            "yellow" => format!("CAUTION[>={}µSv/h]", self.safe_max),
            "red" => format!("EVACUATE[>={}µSv/h]", self.danger_min),
            _ => "UNKNOWN".to_string(),
        }
    }

    fn colored_log(&self, level: &str, msg: &str) {
        if self.color {
            println!("{}{}{}", color_code(level), msg, "\x1b[0m");
        } else {
            log(msg);
        }
    }

    // -----------------------------------------------------------------------
    // commands
    // -----------------------------------------------------------------------

    fn cmd_version(&mut self) {
        let h = self.ask_expect("<GETVER>>", 14, 0.8, 3);
        Self::need_bytes(&h, 14, "GETVER");
        log(hex_to_str(&h[..28]));
    }

    fn cmd_cpm(&mut self) {
        let h = self.ask_expect("<GETCPM>>", 2, 0.8, 3);
        Self::need_bytes(&h, 2, "GETCPM");
        log(u16be(&[
            u8::from_str_radix(&h[..2], 16).unwrap_or(0),
            u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
        ]));
    }

    fn cmd_cps(&mut self) {
        let h = self.ask_expect("<GETCPS>>", 2, 0.8, 3);
        Self::need_bytes(&h, 2, "GETCPS");
        let v = u16::from_str_radix(&h[..4], 16).unwrap_or(0) & 0x3fff;
        log(v);
    }

    fn cmd_maxcps(&mut self) {
        let h = self.ask_expect("<GETMAXCPS>>", 2, 0.8, 3);
        Self::need_bytes(&h, 2, "GETMAXCPS");
        log(u16::from_str_radix(&h[..4], 16).unwrap_or(0));
    }

    fn cmd_volt(&mut self) {
        let h = self.ask_expect("<GETVOLT>>", 1, 0.8, 3);
        Self::need_bytes(&h, 1, "GETVOLT");
        let v = u8::from_str_radix(&h[..2], 16).unwrap_or(0);
        log(format!("{}.{} V", v / 10, v % 10));
    }

    fn cmd_serial(&mut self) {
        let h = self.ask_expect("<GETSERIAL>>", 7, 0.8, 3);
        Self::need_bytes(&h, 7, "GETSERIAL");
        log(&h[..14]);
    }

    fn cmd_datetime(&mut self) {
        let h = self.ask_expect("<GETDATETIME>>", 7, 0.8, 3);
        Self::need_bytes(&h, 7, "GETDATETIME");
        let b: Vec<u8> = (0..14)
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0))
            .collect();
        log(fmt_datetime(
            b[0] as i32,
            b[1] as i32,
            b[2] as i32,
            b[3] as i32,
            b[4] as i32,
            b[5] as i32,
        ));
    }

    fn cmd_config(&mut self) {
        let h = self.ask_expect("<GETCFG>>", 256, 2.0, 3);
        Self::need_bytes(&h, 256, "GETCFG");
        let bytes: Vec<u8> = (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0))
            .collect();
        for off in (0..256).step_by(16) {
            let chunk = bytes[off..off + 16]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            println!("{off:3}: {chunk}");
        }
        if self.verbose {
            let power = bytes[0];
            log("-- decoded --");
            log(format!(
                "power      : {}  (raw {power}; 0=ON,255=OFF)",
                if power == 0 { "ON" } else { "OFF" }
            ));
            log(format!("alarm      : {}", bytes[1]));
            log(format!("speaker    : {}", bytes[2]));
            log(format!(
                "display    : {}  (config byte 3; 0=CPM,1=uSv/h,2=mR/h)",
                display_unit_name(bytes[3])
            ));
            log(format!(
                "save type  : {}  (0=off,1=CPS/s,2=CPM/min,...)",
                bytes[32]
            ));
            log(format!(
                "battery    : {}  (0=rechargeable,1=non-rechargeable)",
                bytes[56]
            ));
            log(format!("baud code  : {}", bytes[57]));
        }
    }

    fn cmd_status(&mut self) {
        let ver = hex_to_str(&self.ask_expect("<GETVER>>", 14, 0.8, 3)[..28]);
        let addr = self.ask_expect("<GETCFG>>", 256, 2.0, 3);
        Self::need_bytes(&addr, 256, "GETCFG");
        let cfg: Vec<u8> = (0..addr.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&addr[i..i + 2], 16).unwrap_or(0))
            .collect();
        let power = cfg[0];
        self.display_unit = cfg[3];
        let unit = display_unit_name(self.display_unit);
        self.read_calibration(&cfg);

        let volt = self.ask_expect("<GETVOLT>>", 1, 0.8, 3);
        Self::need_bytes(&volt, 1, "GETVOLT");
        let vv = u8::from_str_radix(&volt[..2], 16).unwrap_or(0);
        let cpm = self.ask_expect("<GETCPM>>", 2, 0.8, 3);
        Self::need_bytes(&cpm, 2, "GETCPM");
        let cps = self.ask_expect("<GETCPS>>", 2, 0.8, 3);
        Self::need_bytes(&cps, 2, "GETCPS");
        let dt = self.ask_expect("<GETDATETIME>>", 7, 0.8, 3);
        Self::need_bytes(&dt, 7, "GETDATETIME");
        let serial = self.ask_expect("<GETSERIAL>>", 7, 0.8, 3);
        Self::need_bytes(&serial, 7, "GETSERIAL");

        let cpsv = u16::from_str_radix(&cps[..4], 16).unwrap_or(0) & 0x3fff;
        let cpmn = u16::from_str_radix(&cpm[..4], 16).unwrap_or(0) as u64;
        let disp = self.format_display_value(cpmn, 2);
        let precise = self.format_display_value(cpmn, 3);
        let db: Vec<u8> = (0..dt.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&dt[i..i + 2], 16).unwrap_or(0))
            .collect();
        let clock = fmt_datetime(
            db[0] as i32,
            db[1] as i32,
            db[2] as i32,
            db[3] as i32,
            db[4] as i32,
            db[5] as i32,
        );
        if self.csv {
            log("model,power,battery_v,cpm,cps,display_unit,display_value,display_precise,datetime,serial");
            log(format!(
                "\"{ver}\",{},{}.{},{},{},{},{},{},\"{clock}\",{}",
                if power == 0 { "ON" } else { "OFF" },
                vv / 10,
                vv % 10,
                cpmn,
                cpsv,
                unit,
                disp,
                precise,
                &serial[..14]
            ));
        } else {
            log(format!("model    : {ver}"));
            log(format!(
                "power    : {}",
                if power == 0 { "ON" } else { "OFF" }
            ));
            log(format!("battery  : {}.{} V", vv / 10, vv % 10));
            log(format!("radiation: {cpmn} CPM   {cpsv} CPS"));
            log(format!("display  : {unit}  ({disp}, precise {precise})"));
            log(format!("clock    : {clock}"));
            log(format!("serial   : {}", &serial[..14]));
        }
    }

    fn cmd_displayunit(&mut self) {
        let h = self.ask_expect("<GETCFG>>", 256, 2.0, 3);
        Self::need_bytes(&h, 256, "GETCFG");
        let u = u8::from_str_radix(&h[6..8], 16).unwrap_or(0);
        log(format!(
            "{} (config byte 3 = 0x{u:02x})",
            display_unit_name(u)
        ));
    }

    fn cmd_displayvalue(&mut self) {
        let cfg = self.ask_expect("<GETCFG>>", 256, 2.0, 3);
        Self::need_bytes(&cfg, 256, "GETCFG");
        self.display_unit = u8::from_str_radix(&cfg[6..8], 16).unwrap_or(0);
        let unitname = display_unit_name(self.display_unit);
        self.read_calibration(&decode_hex(&cfg));
        let h = self.ask_expect("<GETCPM>>", 2, 0.8, 3);
        Self::need_bytes(&h, 2, "GETCPM");
        let cpmn = u16::from_str_radix(&h[..4], 16).unwrap_or(0) as u64;
        let disp = self.format_display_value(cpmn, 2);
        let precise = self.format_display_value(cpmn, 3);
        let usv = self.cpm_to_usv(cpmn as f64);
        let level = self.level_for_usv(usv);
        let label = self.level_label(level);
        if self.csv {
            log("device_cpm,display_unit,display_value,display_precise,level,level_label");
            log(format!(
                "{cpmn},{unitname},{disp},{precise},{level},{label}"
            ));
        } else {
            self.colored_log(
                level,
                &format!("device CPM {cpmn}  |  {unitname} {disp}  (precise {precise})  |  LEVEL {label}"),
            );
        }
    }

    fn poll_device_cpm(&mut self, secs: f64) -> (Option<u64>, Vec<u8>) {
        let mut cpm = None;
        let mut hb = Vec::new();
        for _ in 0..2 {
            self.ser().tx("<GETCPM>>");
            let h = self.ser().rx_hex(secs);
            let bytes = decode_hex(&h);
            let mut i = 0;
            while i + 1 < bytes.len() {
                let word = u16be(&bytes[i..i + 2]);
                i += 2;
                if word & 0x8000 != 0 {
                    hb.extend_from_slice(&word.to_be_bytes());
                } else if cpm.is_none() {
                    cpm = Some(word as u64);
                }
            }
            if cpm.is_some() {
                break;
            }
        }
        note(
            self.verbose,
            format!(
                "polled device CPM: {} (heartbeat bytes preserved: {})",
                cpm.map(|v| v.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                hb.len()
            ),
        );
        (cpm, hb)
    }

    fn cmd_live(&mut self, secs: u64) {
        // Install the handler before the (possibly slow) startup reads so an
        // early Ctrl-C cannot be missed while GETCFG retries.
        STOP.store(false, Ordering::SeqCst);
        unsafe {
            signal(SIGINT, on_signal as *const () as usize);
            signal(SIGTERM, on_signal as *const () as usize);
        }
        let ver = hex_to_str(&self.ask_expect("<GETVER>>", 14, 0.8, 3)[..28]);
        let cfg = self.ask_expect("<GETCFG>>", 256, 2.0, 3);
        Self::need_bytes(&cfg, 256, "GETCFG");
        self.display_unit = u8::from_str_radix(&cfg[6..8], 16).unwrap_or(0);
        let unitname = display_unit_name(self.display_unit);
        self.read_calibration(&decode_hex(&cfg));
        note(
            self.verbose,
            format!(
                "device display unit: {unitname} (config byte 3 = {:02x}), {} CPM/uSv/h representative",
                self.display_unit, self.display_factor
            ),
        );

        self.ser().tx("<HEARTBEAT1>>");
        self.ser().drain(0.4);
        let started = now_epoch();
        let (dev, hb) = self.poll_device_cpm(0.4);
        let mut carry = hb;
        let mut dev_cpm = dev;
        let mut next_poll = started + 1;
        if STOP.load(Ordering::SeqCst) {
            unsafe {
                signal(SIGINT, SIG_DFL);
                signal(SIGTERM, SIG_DFL);
            }
            self.ser().tx("<HEARTBEAT0>>");
            return;
        }

        if self.csv {
            log("epoch,time,cps,cumulative,avg_cpm,device_cpm,display_unit,display_value,display_precise,level,level_label");
        } else {
            log(format!("Live tracking - {ver}"));
            log(format!(
                "device display unit: {unitname}  (calibration: {} CPM/uSv/h representative)",
                self.display_factor
            ));
            log("device CPM polled every 1s; 'precise' shows sub-display-step movement");
            println!(
                "levels: {} | {} | {}",
                self.level_label("green"),
                self.level_label("yellow"),
                self.level_label("red")
            );
            println!(
                "timestamp  CPS  total  hb CPM  dev CPM    {:>8}    {:>8}  {:<20}",
                unitname,
                format!("{unitname}+"),
                "LEVEL"
            );
            log("----------------------------------------------------------------------------------------");
        }

        let mut total: u64 = 0;
        let mut n: u64 = 0;
        while !STOP.load(Ordering::SeqCst) {
            let now = now_epoch();
            if secs > 0 && now - started >= secs as i64 {
                break;
            }
            if now >= next_poll {
                let (polled, hb) = self.poll_device_cpm(0.25);
                carry.extend_from_slice(&hb);
                if let Some(v) = polled {
                    dev_cpm = Some(v);
                }
                next_poll = now + 1;
            }
            let mut bytes = carry;
            bytes.extend_from_slice(&self.ser().read_for(1100, true));
            let mut i = 0;
            while i + 1 < bytes.len() {
                let v = (u16be(&bytes[i..i + 2]) & 0x3fff) as u64;
                i += 2;
                total += v;
                n += 1;
                let avg_cpm = total * 60 / n;
                let disp_cpm = dev_cpm.unwrap_or(avg_cpm);
                let disp = self.format_display_value(disp_cpm, 2);
                let precise = self.format_display_value(disp_cpm, 3);
                let usv = self.cpm_to_usv(disp_cpm as f64);
                let level = self.level_for_usv(usv);
                let label = self.level_label(level);
                let row_epoch = now_epoch();
                let (_, _, _, hh, mi, ss, _) = local_parts(row_epoch);
                let stamp = format!("{hh:02}:{mi:02}:{ss:02}");
                if self.csv {
                    log(format!(
                        "{row_epoch},{stamp},{v},{total},{avg_cpm},{},{},{},{},{},{}",
                        dev_cpm.map(|x| x.to_string()).unwrap_or_default(),
                        unitname,
                        disp,
                        precise,
                        level,
                        label
                    ));
                } else {
                    let row = format!(
                        "{stamp}   {v:>3}  {total:>5}  {avg_cpm:>5}  {:>7}    {disp:>8}    {precise:>8}  {label:<20}",
                        dev_cpm
                            .map(|x| x.to_string())
                            .unwrap_or_else(|| "-".to_string())
                    );
                    self.colored_log(level, &row);
                }
            }
            carry = bytes[i..].to_vec();
        }

        unsafe {
            signal(SIGINT, SIG_DFL);
            signal(SIGTERM, SIG_DFL);
        }
        self.ser().tx("<HEARTBEAT0>>");
        note(
            self.verbose,
            format!("heartbeat stopped; {n} samples, {total} counts"),
        );
    }

    fn write_datetime(&mut self, vals: &[u8; 6]) {
        self.ser().write_only(b"<SETDATETIME");
        self.ser().tx_bytes(vals);
        self.ser().write_only(b">>");
        let mut h = self.ser().rx_hex(0.8);
        if h.starts_with("00") {
            h = h[2..].to_string();
        }
        if !h.is_empty() && h != "00" && h != "aa" {
            warn(format!("unexpected SETDATETIME reply: {h}"));
        }
    }

    fn cmd_setdatetime(&mut self, args: &[String]) {
        let vals = if args.len() >= 6 {
            let mut v = [0u8; 6];
            for (i, a) in args.iter().take(6).enumerate() {
                v[i] = a
                    .parse()
                    .unwrap_or_else(|_| die("setdatetime: values must be integers"));
            }
            v
        } else {
            let (y, m, d, hh, mi, ss, _) = local_parts(now_epoch());
            note(
                self.verbose,
                format!(
                    "using computer time: 20{:02}-{:02}-{:02} {:02}:{:02}:{:02}",
                    y % 100,
                    m,
                    d,
                    hh,
                    mi,
                    ss
                ),
            );
            [
                (y % 100) as u8,
                m as u8,
                d as u8,
                hh as u8,
                mi as u8,
                ss as u8,
            ]
        };
        self.write_datetime(&vals);
        log(format!(
            "clock set to 20{:02}-{:02}-{:02} {:02}:{:02}:{:02}",
            vals[0], vals[1], vals[2], vals[3], vals[4], vals[5]
        ));
    }

    fn cmd_synctime(&mut self) {
        let (y, m, d, hh, mi, ss, _) = local_parts(now_epoch());
        note(
            self.verbose,
            format!(
                "host local time: 20{:02}-{:02}-{:02} {:02}:{:02}:{:02}",
                y % 100,
                m,
                d,
                hh,
                mi,
                ss
            ),
        );
        self.write_datetime(&[
            (y % 100) as u8,
            m as u8,
            d as u8,
            hh as u8,
            mi as u8,
            ss as u8,
        ]);
        let h = self.ask("<GETDATETIME>>", 0.8);
        Self::need_bytes(&h, 7, "GETDATETIME");
        let b = decode_hex(&h);
        let dev = fmt_datetime(
            b[0] as i32,
            b[1] as i32,
            b[2] as i32,
            b[3] as i32,
            b[4] as i32,
            b[5] as i32,
        );
        let now_epoch = now_epoch();
        let (_, _, _, _, _, _, gmtoff) = local_parts(now_epoch);
        let dev_epoch = days_from_civil(2000 + b[0] as i32, b[1] as i32, b[2] as i32) * 86400
            + b[3] as i64 * 3600
            + b[4] as i64 * 60
            + b[5] as i64
            - gmtoff;
        let (y2, m2, d2, h2, i2, s2, _) = local_parts(now_epoch);
        log(format!(
            "host : {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            y2, m2, d2, h2, i2, s2
        ));
        log(format!("unit : {dev}"));
        let delta = (now_epoch - dev_epoch).abs();
        if delta > 3 {
            warn(format!("unit clock still differs from host by {delta}s"));
        } else {
            log(format!("clocks agree (within {delta}s)"));
        }
    }

    fn cmd_setfield(&mut self, name: &str, cmdword: &str, args: &[String]) {
        let val: u16 = args
            .first()
            .and_then(|s| s.parse().ok())
            .filter(|v: &u16| *v <= 255)
            .unwrap_or_else(|| die(format!("{name}: value must be 0..255")));
        self.ser().write_only(format!("<{cmdword}").as_bytes());
        self.ser().tx_bytes(&[val as u8]);
        self.ser().write_only(b">>");
        let h = self.ser().rx_hex(0.8);
        if !h.is_empty() && h != "00" && h != "aa" {
            warn(format!("unexpected {cmdword} reply: {h}"));
        }
        log(format!("{name} = {val}"));
    }

    fn cmd_history(&mut self, args: &[String]) {
        let addr: u32 = args
            .first()
            .map(|s| {
                s.parse()
                    .unwrap_or_else(|_| die("history: ADDR must be an integer"))
            })
            .unwrap_or(0);
        let len: u32 = args
            .get(1)
            .map(|s| {
                s.parse()
                    .unwrap_or_else(|_| die("history: LEN must be an integer"))
            })
            .unwrap_or(64);
        if !(1..=4096).contains(&len) {
            die("history: LEN must be 1..4096");
        }
        self.ser().write_only(b"<SPIR");
        self.ser().tx_bytes(&[
            ((addr >> 16) & 0xff) as u8,
            ((addr >> 8) & 0xff) as u8,
            (addr & 0xff) as u8,
            ((len >> 8) & 0xff) as u8,
            (len & 0xff) as u8,
        ]);
        self.ser().write_only(b">>");
        let h = self.ser().rx_hex(2.0);
        if h.is_empty() {
            die("no history data returned");
        }
        log(format!("history @ 0x{addr:06x}, {} byte(s):", h.len() / 2));
        let chars: Vec<char> = h.chars().collect();
        let mut off = 0;
        while off < chars.len() {
            let end = (off + 32).min(chars.len());
            let line: String = chars[off..end].iter().collect();
            log(format!("{:06x}: {line}", addr + (off / 2) as u32));
            off += 32;
        }
    }

    fn cmd_on(&mut self) {
        let h = self.ask("<POWERON>>", 0.8);
        if !h.is_empty() && h != "00" && h != "aa" {
            warn(format!("unexpected POWERON reply: {h}"));
        }
        log("power ON sent");
    }

    fn cmd_off(&mut self) {
        self.ser().tx("<POWEROFF>>");
        log("power OFF sent (the counter may go quiet now)");
        std::thread::sleep(Duration::from_millis(300));
    }

    fn cmd_reboot(&mut self) {
        let h = self.ask("<REBOOT>>", 2.0);
        if !h.is_empty() && h != "00" && h != "aa" {
            warn(format!("unexpected REBOOT reply: {h}"));
        }
        log("reboot command sent");
    }

    fn cmd_key(&mut self, args: &[String]) {
        let n = args.first().and_then(|s| s.parse::<u8>().ok());
        let n = match n {
            Some(v @ 0..=3) => v,
            _ => die("key: N must be 0..3 (S1..S4)"),
        };
        self.ser().tx(&format!("<KEY{n}>>"));
        log(format!("key S{} sent", n + 1));
    }

    fn cmd_alarm(&mut self, args: &[String]) {
        match args.first().map(|s| s.as_str()) {
            Some("on") => Self::ack_ok(&self.ask("<ALARM1>>", 0.6), "ALARM1"),
            Some("off") => Self::ack_ok(&self.ask("<ALARM0>>", 0.6), "ALARM0"),
            _ => die(format!("usage: {} alarm on|off", self.prog)),
        }
        log(format!("alarm {}", args[0]));
    }

    fn cmd_speaker(&mut self, args: &[String]) {
        match args.first().map(|s| s.as_str()) {
            Some("on") => Self::ack_ok(&self.ask("<SPEAKER1>>", 0.6), "SPEAKER1"),
            Some("off") => Self::ack_ok(&self.ask("<SPEAKER0>>", 0.6), "SPEAKER0"),
            _ => die(format!("usage: {} speaker on|off", self.prog)),
        }
        log(format!("speaker {}", args[0]));
    }

    fn cmd_echo(&mut self, args: &[String]) {
        let h = match args.first().map(|s| s.as_str()) {
            Some("on") => self.ask("<EchoON>>", 0.5),
            Some("off") => self.ask("<EchoOFF>>", 0.5),
            _ => die(format!("usage: {} echo on|off", self.prog)),
        };
        log(format!(
            "echo {} (reply: {}; 300E usually does not answer)",
            args[0],
            if h.is_empty() { "none" } else { &h }
        ));
    }

    fn cmd_cfgupdate(&mut self) {
        let h = self.ask("<CFGUPDATE>>", 0.8);
        Self::ack_ok(&h, "CFGUPDATE");
        log("configuration reloaded");
    }

    fn cmd_ecfg(&mut self) {
        self.confirm_danger("ECFG (erase configuration)");
        let h = self.ask("<ECFG>>", 0.8);
        Self::ack_ok(&h, "ECFG");
        log("configuration erased");
    }

    fn cmd_factoryreset(&mut self) {
        self.confirm_danger("FACTORYRESET");
        let h = self.ask("<FACTORYRESET>>", 0.8);
        Self::ack_ok(&h, "FACTORYRESET");
        log("factory defaults restored");
    }

    fn cmd_wcfg(&mut self, args: &[String]) {
        let addr: u16 = args
            .first()
            .and_then(|s| s.parse().ok())
            .filter(|v: &u16| *v <= 255)
            .unwrap_or_else(|| die("wcfg: ADDR must be 0..255"));
        let val: u16 = args
            .get(1)
            .and_then(|s| s.parse().ok())
            .filter(|v: &u16| *v <= 255)
            .unwrap_or_else(|| die("wcfg: VALUE must be 0..255"));
        self.confirm_danger(&format!("WCFG (write config byte {addr} = {val})"));
        self.ser().write_only(b"<WCFG");
        self.ser().tx_bytes(&[addr as u8, val as u8]);
        self.ser().write_only(b">>");
        let h = self.ser().rx_hex(0.8);
        Self::ack_ok(&h, "WCFG");
        log(format!("config[{addr}] = {val}"));
    }

    fn cmd_raw(&mut self, args: &[String]) {
        let cmd = args.first().cloned().unwrap_or_default();
        if cmd.is_empty() {
            die(format!("usage: {} raw '<COMMAND>>'", self.prog));
        }
        if !cmd.starts_with('<') || !cmd.ends_with(">>") {
            warn("command does not look like '<CMD>>'");
        }
        let h = self.ask(&cmd, 1.0);
        log(format!(
            "hex  : {}",
            if h.is_empty() { "<empty>" } else { &h }
        ));
        log(format!("ascii: {}", hex_to_str(&h)));
    }
}

fn decode_hex(h: &str) -> Vec<u8> {
    let b = h.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let s = std::str::from_utf8(&b[i..i + 2]).unwrap_or("");
        out.push(u8::from_str_radix(s, 16).unwrap_or(0));
        i += 2;
    }
    out
}

// ---------------------------------------------------------------------------
// help / arguments
// ---------------------------------------------------------------------------

fn help(prog: &str) -> String {
    format!(
        r#"{prog} - GQ GMC-300E Geiger counter controller (USB / CH340, 57600 8N1)

USAGE
  {prog} [global options] COMMAND [command options]

GLOBAL OPTIONS
  -p, --port DEVICE     serial device (default: auto-detect via
                        /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0,
                        falling back to /dev/ttyUSB0)
  -b, --baud RATE       baud rate (default: 57600, env GMC_BAUD)
  -v, --verbose         trace every command and raw hex response
      --csv             machine-readable output for 'live' and 'status'
      --color           force ANSI colour in live rows (auto on a TTY)
      --no-color        disable ANSI colour in live rows
  -h, --help            show this help
      --version         show script version

INFO / READ
  version               model + firmware string            (<GETVER>>, 14 ASCII)
  status                one-shot summary: model, power, battery, CPM, CPS,
                        display unit + converted value, clock, serial number
  cpm                   current counts per minute           (<GETCPM>>, 2 bytes)
  cps                   current counts per second           (<GETCPS>>, 2 bytes,
                        top bit masked)
  maxcps                highest CPS since power-on          (<GETMAXCPS>>)
  volt                  battery voltage                     (<GETVOLT>>, 1 byte)
  serial                serial number as hex                (<GETSERIAL>>, 7 bytes)
  datetime              counter's date and time             (<GETDATETIME>>)
  config                full 256-byte config hex dump; with -v also decodes
                        the known fields                    (<GETCFG>>)
  displayunit           read the unit the LCD is set to: CPM, µSv/h or mR/h
                        (config byte 3). 'live' uses this same unit.
  displayvalue          read the instrument's CPM and print the converted value
                        in the LCD's unit (2 decimals, like the screen) plus a
                        3-decimal 'precise' value so small changes are visible.
  temp                  raw temperature response (not fitted on 300E)
  gyro                  raw gyro response (not fitted on 300E)
  history [ADDR] [LEN]  read flash history, hex dump        (<SPIR...>>);
                        ADDR default 0, LEN default 64

LIVE / STREAMING
  live [SECONDS]        real-time tracking using the hardware heartbeat.
                        Reads the unit set on the device LCD (config byte 3)
                        before starting and shows a converted rate in that
                        same unit, so the live view matches the display.
                        The converted value uses the instrument's own CPM
                        (polled every second), not the heartbeat-derived
                        average, because the LCD uses the instrument's CPM too.
                        Prints timestamp, CPS, cumulative counts, heartbeat
                        CPM, device CPM, the LCD-style value (2 decimals), a
                        more precise 3-decimal value and a LEVEL once per
                        second. Rows are green/NORMAL, yellow/CAUTION or
                        red/EVACUATE, with the cut-off shown in brackets in
                        the LEVEL column. Defaults are NORMAL < 1 uSv/h,
                        CAUTION >= 1 uSv/h, EVACUATE >= 10 uSv/h; change them
                        with GMC_SAFE_MAX_USV / GMC_DANGER_MIN_USV. Runs until
                        Ctrl-C, or for SECONDS if given. Heartbeat is always
                        switched off on exit.
                        Example:  {prog} live 60
                                  {prog} --csv live > counts.csv

POWER / DEVICE
  on                    power on / wake the tube             (<POWERON>>)
  off                   power off the unit                   (<POWEROFF>>)
  reboot                soft reboot                          (<REBOOT>>)
  key N                 press virtual key S1..S4 (N = 0..3)  (<KEYn>>)

MEASUREMENT SETTINGS
  alarm on|off          enable/disable the alarm             (<ALARM1/0>>)
  speaker on|off        enable/disable the clicks            (<SPEAKER1/0>>)
  echo on|off           serial debug echo (no reply on 300E) (<EchoON/OFF>>)

CLOCK
  synctime              sync the counter's clock to this host's local date and
                        time, then read it back and report the difference.
                        This is the preferred way to set the clock.
  setdatetime [YY MM DD hh mm ss]
                        set the counter's clock to an explicit date/time. With
                        no arguments, the computer's local time is used (same
                        as 'synctime', without the read-back check). Bytes are
                        sent raw, not as an ASCII parameter string.
                                                              (<SETDATETIME...>>)
  setyear NN            set year (00 = 2000)                 (<SETDATEYY..>>)
  setmonth NN           set month                            (<SETDATEMM..>>)
  setday NN             set day                              (<SETDATEDD..>>)
  sethour NN            set hour                             (<SETTIMEHH..>>)
  setminute NN          set minute                           (<SETTIMEMM..>>)
  setsecond NN          set second                           (<SETTIMESS..>>)

ADVANCED / DESTRUCTIVE (require --yes)
  cfgupdate             reload configuration                 (<CFGUPDATE>>)
  wcfg ADDR VALUE       write one config byte (0..255)       (<WCFG..>>)
  ecfg                  erase all configuration (0xFF)       (<ECFG>>)
  factoryreset          restore factory defaults             (<FACTORYRESET>>)

RAW ACCESS
  raw '<COMMAND>>'      send any protocol command and print the response as
                        both hex and ASCII. Use this for commands this script
                        does not wrap. Example: {prog} raw '<GETVER>>'

PERMISSIONS
  The device node is owned by root:uucp. If your login session does not have
  the uucp group yet, this script automatically re-executes itself through
  'newgrp uucp'. If that fails, log out and back in, or run:
      sudo usermod -aG uucp "$USER"
  You can also run the script with sudo, or point GMC_PORT at a device node
  your account can already write.

EXAMPLES
  {prog} status
  {prog} cpm
  {prog} displayunit
  {prog} displayvalue
  {prog} live
  {prog} live 30
  {prog} --csv live 3600 > hour.csv
  {prog} synctime
  {prog} setdatetime
  {prog} speaker off
  {prog} --yes wcfg 1 0
  {prog} raw '<GETCPM>>'

NOTES
  * Commands are host-initiated only. Reading a command's reply with a short
    timeout is normal: the counter sends exactly the documented byte count.
  * GETCPS replies have bit 15 set as a marker; bit 14..0 are the count.
  * GETCFG byte 0 is power state on the 300 series: 0 = ON, 255 = OFF.
  * GETCFG byte 3 selects the LCD radiation unit: 0 = CPM, 1 = uSv/h,
    2 = mR/h. Conversion uses the device's own three calibration points
    (config offsets 8, 14, 20) with piecewise-linear interpolation, then
    divides by 10 for mR/h, matching the instrument's 328 uSv/h = 32.8 mR/h
    ranges. 'displayvalue' converts the instrument's current CPM exactly the
    same way; use it to compare directly with the LCD.
  * The LCD and 'live' can differ slightly because the instrument's CPM is a
    rolling value that changes every few seconds, while the heartbeat CPM is
    a running average since 'live' started. On this calibration ~15 CPM is one
    0.01 mR/h display step; near a rounding boundary only a few counts are
    needed to flip the displayed step, so small differences are normal.
  * Live row colours use dose rate: green/NORMAL < GMC_SAFE_MAX_USV (default
    1 uSv/h = 0.1 mR/h), yellow/CAUTION from there to GMC_DANGER_MIN_USV
    (default 10 uSv/h = 1 mR/h), red/EVACUATE at or above that. The LEVEL
    column spells this out with the cut-off in brackets. These are
    conservative log/alert aids, not radiation-safety advice; adjust the env
    vars for your own limits. Use --no-color for plain logs; --csv is always
    uncoloured.
  * A freshly woken counter can report 0 CPM for a short time. The tube can
    also need a couple of minutes to recover after a high dose. 'on' or a
    physical key press wakes it; 'reboot' clears latched states.
  * After 'off' the unit stops answering until you send 'on' (or press a key).
  * 'key N' really presses a button: S2/S3 can open an on-screen menu where
    serial commands are ignored until you press S1 ('key 0') to go back.
  * ECFG, WCFG and FACTORYRESET change stored configuration. Wrong values can
    make the counter misbehave; use --yes only when you intend the change.

Version 1.0. Protocol: GQ-RFC1201 v1.40 / GQ-RFC1801 v1.00.
"#
    )
}

fn main() {
    let mut ctx = Ctx::new();
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].clone();
        match a.as_str() {
            "-h" | "--help" => {
                print!("{}", help(&ctx.prog));
                return;
            }
            "--version" => {
                log(format!("{} 1.0", ctx.prog));
                return;
            }
            "-v" | "--verbose" => {
                ctx.verbose = true;
                i += 1;
            }
            "--csv" => {
                ctx.csv = true;
                ctx.color = false;
                i += 1;
            }
            "--color" => {
                ctx.color = true;
                i += 1;
            }
            "--no-color" => {
                ctx.color = false;
                i += 1;
            }
            "--yes" => {
                ctx.yes = true;
                i += 1;
            }
            "-p" | "--port" => {
                if i + 1 >= args.len() {
                    die(format!("{a} needs an argument"));
                }
                ctx.port = args[i + 1].clone();
                i += 2;
            }
            "-b" | "--baud" => {
                if i + 1 >= args.len() {
                    die(format!("{a} needs an argument"));
                }
                ctx.baud = args[i + 1]
                    .parse()
                    .unwrap_or_else(|_| die(format!("invalid baud rate: {}", args[i + 1])));
                i += 2;
            }
            "--" => {
                i += 1;
                break;
            }
            _ if a.starts_with('-') => die(format!("unknown option: {a} (try --help)")),
            _ => break,
        }
    }
    let rest = args.split_off(i);
    if rest.is_empty() {
        print!("{}", help(&ctx.prog));
        return;
    }
    let cmd = rest[0].clone();
    let sub = &rest[1..];

    if cmd == "help" {
        print!("{}", help(&ctx.prog));
        return;
    }
    ctx.open_port();

    match cmd.as_str() {
        "version" | "ver" => ctx.cmd_version(),
        "status" => ctx.cmd_status(),
        "cpm" => ctx.cmd_cpm(),
        "cps" => ctx.cmd_cps(),
        "maxcps" => ctx.cmd_maxcps(),
        "volt" | "battery" => ctx.cmd_volt(),
        "serial" | "--serial" => ctx.cmd_serial(),
        "datetime" | "date" => ctx.cmd_datetime(),
        "config" | "cfg" => ctx.cmd_config(),
        "displayunit" | "unit" => ctx.cmd_displayunit(),
        "displayvalue" | "dose" => ctx.cmd_displayvalue(),
        "history" | "hist" => ctx.cmd_history(sub),
        "temp" => ctx.cmd_raw(&["<GETTEMP>>".to_string()]),
        "gyro" => ctx.cmd_raw(&["<GETGYRO>>".to_string()]),
        "live" | "track" | "monitor" => {
            let secs: u64 = sub
                .first()
                .map(|s| {
                    s.parse()
                        .unwrap_or_else(|_| die("live: SECONDS must be a non-negative integer"))
                })
                .unwrap_or(0);
            ctx.cmd_live(secs);
        }
        "on" | "wake" => ctx.cmd_on(),
        "off" => ctx.cmd_off(),
        "reboot" => ctx.cmd_reboot(),
        "key" => ctx.cmd_key(sub),
        "alarm" => ctx.cmd_alarm(sub),
        "speaker" => ctx.cmd_speaker(sub),
        "echo" => ctx.cmd_echo(sub),
        "synctime" => ctx.cmd_synctime(),
        "setdatetime" => ctx.cmd_setdatetime(sub),
        "setyear" => ctx.cmd_setfield("setyear", "SETDATEYY", sub),
        "setmonth" => ctx.cmd_setfield("setmonth", "SETDATEMM", sub),
        "setday" => ctx.cmd_setfield("setday", "SETDATEDD", sub),
        "sethour" => ctx.cmd_setfield("sethour", "SETTIMEHH", sub),
        "setminute" => ctx.cmd_setfield("setminute", "SETTIMEMM", sub),
        "setsecond" => ctx.cmd_setfield("setsecond", "SETTIMESS", sub),
        "cfgupdate" => ctx.cmd_cfgupdate(),
        "wcfg" => ctx.cmd_wcfg(sub),
        "ecfg" => ctx.cmd_ecfg(),
        "factoryreset" => ctx.cmd_factoryreset(),
        "raw" => ctx.cmd_raw(sub),
        _ => die(format!("unknown command '{cmd}' (try --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> Ctx {
        let mut c = Ctx::new();
        c.cal_points = vec![(0.0, 0.0), (60.0, 0.39), (240.0, 1.56), (1000.0, 6.5)];
        c.display_factor = "153.846154".to_string();
        c.display_unit = 2;
        c.safe_max = "1".to_string();
        c.danger_min = "10".to_string();
        c.safe_max_v = 1.0;
        c.danger_min_v = 10.0;
        c
    }

    #[test]
    fn od_float_matches_six_significant_digits() {
        assert_eq!(od_float(0.39_f32), 0.39);
        assert_eq!(od_float(1.56_f32), 1.56);
        assert_eq!(od_float(6.5_f32), 6.5);
    }

    #[test]
    fn piecewise_conversion_hits_calibration_points() {
        let c = test_ctx();
        assert_eq!(c.cpm_to_usv(0.0), 0.0);
        assert_eq!(c.cpm_to_usv(60.0), 0.39);
        assert_eq!(c.cpm_to_usv(240.0), 1.56);
        assert_eq!(c.cpm_to_usv(1000.0), 6.5);
        assert_eq!(c.cpm_to_usv(30.0), 0.195);
        assert_eq!(c.cpm_to_usv(500.0), 3.25);
    }

    #[test]
    fn mrh_display_values_match_lcd_resolution() {
        let c = test_ctx();
        assert_eq!(c.format_display_value(60, 2), "0.04");
        assert_eq!(c.format_display_value(60, 3), "0.039");
        assert_eq!(c.format_display_value(240, 2), "0.16");
        assert_eq!(c.format_display_value(1000, 2), "0.65");
    }

    #[test]
    fn levels_and_labels_match_bash() {
        let c = test_ctx();
        assert_eq!(c.level_for_usv(0.9), "green");
        assert_eq!(c.level_for_usv(1.0), "yellow");
        assert_eq!(c.level_for_usv(9.9), "yellow");
        assert_eq!(c.level_for_usv(10.0), "red");
        assert_eq!(c.level_label("green"), "NORMAL[<1µSv/h]");
        assert_eq!(c.level_label("yellow"), "CAUTION[>=1µSv/h]");
        assert_eq!(c.level_label("red"), "EVACUATE[>=10µSv/h]");
    }

    #[test]
    fn unit_names_are_stable() {
        assert_eq!(display_unit_name(0), "CPM");
        assert_eq!(display_unit_name(1), "µSv/h");
        assert_eq!(display_unit_name(2), "mR/h");
        assert_eq!(display_unit_name(7), "unknown(0x07)");
    }
}
