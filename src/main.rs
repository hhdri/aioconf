//! `aioconf`: pump and fan curves for the NZXT Kraken 2023.
//!
//! This binary is freestanding. It has no libc and no dependencies: it starts
//! at its own `_start`, talks to the kernel with raw syscalls, and exchanges
//! 64-byte HID reports with the cooler through `/dev/hidrawN`.

#![no_std]
#![no_main]
// Errors carry a fixed-size path buffer instead of a heap allocation, since
// there is no allocator. They are returned once per run, so size is fine.
#![allow(clippy::result_large_err)]

mod board;
mod device;
mod io;
mod rt;
mod sys;

use aioconf::config::{self, Config, Setting};
use aioconf::curve::{FIRST_TEMP, TABLE_LEN};
use aioconf::proto::{self, Channel, Status};
use core::fmt::{self, Write};
use device::Kraken;
use io::{CPath, Out};

const DEFAULT_CONFIG: &str = "/etc/aioconf.conf";
const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
aioconf: pump and fan curves for the NZXT Kraken 2023 (USB 1e71:300e)

Usage: aioconf [OPTIONS] [COMMAND]

Commands:
  status                   Show liquid temperature, pump and fan speed (default)
  watch [SECONDS]          Show status every SECONDS (default 0.5) until Ctrl-C
  apply [FILE]             Apply the curves in FILE (default /etc/aioconf.conf),
                           for the Kraken and for motherboard fan headers
  set <pump|fan> <CURVE>   Set one channel. CURVE is a duty in %, or pairs of
                           liquid temperature in °C and duty in %
  full                     Run pump and fan at 100%
  board                    Show the motherboard fan headers and their curves
  init                     Initialize the device and show the firmware version
  list                     List the coolers found

Options:
  -d, --device <PATH>      Use this hidraw node instead of searching for one
  -n, --dry-run            Show what would be sent without opening the device
  -v, --verbose            Print every HID report sent and received
  -h, --help               Show this help
  -V, --version            Show the version

Examples:
  aioconf set fan 27 10 28 50 29 75 30 100
  aioconf set pump 60
  aioconf watch 1
";

pub enum Error {
    Usage(&'static str),
    Sys { what: &'static str, path: CPath, errno: sys::Errno },
    NotFound,
    WrongDevice { path: CPath, vendor: u16, product: u16 },
    NoReply(&'static str),
    Config { source: CPath, err: config::Error },
    Setting(config::ErrorKind),
    NoChip,
    NoHeader(usize),
    UnknownSensor(config::Source),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(msg) => write!(f, "{msg}\nTry 'aioconf --help'."),
            Error::Sys { what, path, errno } => {
                write!(f, "cannot {what} {path}: {errno}")?;
                if *errno == sys::ENOENT && path.as_str() == DEFAULT_CONFIG {
                    f.write_str("\nCreate it from contrib/aioconf.conf, or pass a file: aioconf apply FILE")?;
                }
                if *errno == sys::EACCES {
                    f.write_str("\nRun with sudo, or install the udev rule so your user can reach the cooler.")?;
                }
                Ok(())
            }
            Error::NotFound => write!(
                f,
                "no {} ({:04x}:{:04x}) found under /sys/class/hidraw",
                proto::MODEL,
                proto::VENDOR_ID,
                proto::PRODUCT_ID
            ),
            Error::WrongDevice { path, vendor, product } => write!(
                f,
                "{path} is {vendor:04x}:{product:04x}, not an {} ({:04x}:{:04x})",
                proto::MODEL,
                proto::VENDOR_ID,
                proto::PRODUCT_ID
            ),
            Error::NoReply(what) => write!(f, "the cooler did not answer the {what} request"),
            Error::Config { source, err } => write!(f, "{source}, line {}: {}", err.line, err.kind),
            Error::Setting(kind) => write!(f, "{kind}"),
            Error::NoChip => f.write_str(
                "no Nuvoton fan chip found under /sys/class/hwmon\nLoad its driver with: sudo modprobe nct6775",
            ),
            Error::NoHeader(n) => write!(f, "the motherboard fan chip has no header pwm{n}"),
            Error::UnknownSensor(src) => write!(
                f,
                "the motherboard fan chip has no sensor named {}\nRun 'aioconf board' and see the temp*_label files of the chip",
                src.as_str()
            ),
        }
    }
}

struct Opts<'a> {
    device: Option<&'a str>,
    dry_run: bool,
    verbose: bool,
}

pub fn main(args: &[&str]) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(e) => {
            let mut err = Out::stderr();
            let _ = writeln!(err, "aioconf: {e}");
            if matches!(e, Error::Usage(_)) { 2 } else { 1 }
        }
    }
}

fn run(args: &[&str]) -> Result<(), Error> {
    let mut opts = Opts { device: None, dry_run: false, verbose: false };
    let mut pos: [&str; 64] = [""; 64];
    let mut npos = 0;

    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        match a {
            "-h" | "--help" | "help" => {
                let _ = Out::stdout().write_str(HELP);
                return Ok(());
            }
            "-V" | "--version" => {
                let _ = writeln!(Out::stdout(), "aioconf {VERSION}");
                return Ok(());
            }
            "-v" | "--verbose" => opts.verbose = true,
            "-n" | "--dry-run" => opts.dry_run = true,
            "-d" | "--device" => {
                i += 1;
                opts.device = Some(*args.get(i).ok_or(Error::Usage("--device needs a path"))?);
            }
            _ if a.starts_with("--device=") => opts.device = Some(&a["--device=".len()..]),
            _ if a.starts_with('-') && a.len() > 1 => return Err(Error::Usage("unknown option")),
            _ => {
                pos[npos] = a;
                npos += 1;
            }
        }
        i += 1;
    }

    let (cmd, rest) = match &pos[..npos] {
        [] => ("status", &[][..]),
        [cmd, rest @ ..] => (*cmd, rest),
    };
    match cmd {
        "status" => status(&opts, rest),
        "watch" => watch(&opts, rest),
        "apply" => apply_file(&opts, rest),
        "set" => set(&opts, rest),
        "full" => {
            no_args(rest)?;
            let cfg = Config { pump: Some(Setting::Fixed(100)), fan: Some(Setting::Fixed(100)), ..Config::default() };
            apply(&opts, &cfg, "full speed")
        }
        "board" => {
            no_args(rest)?;
            board::show(&mut Out::stdout())
        }
        "init" => init(&opts, rest),
        "list" => list(rest),
        _ => Err(Error::Usage("unknown command")),
    }
}

fn no_args(rest: &[&str]) -> Result<(), Error> {
    if rest.is_empty() { Ok(()) } else { Err(Error::Usage("too many arguments")) }
}

fn status(opts: &Opts, rest: &[&str]) -> Result<(), Error> {
    no_args(rest)?;
    let dev = Kraken::open(opts.device, opts.verbose)?;
    let st = dev.status()?;
    print_status(&mut Out::stdout(), &dev.path, &st);
    Ok(())
}

fn print_status(o: &mut Out, path: &CPath, st: &Status) {
    let _ = writeln!(o, "{} ({path})", proto::MODEL);
    match st.liquid {
        Some(t) => {
            let _ = writeln!(o, "├── Liquid temperature  {t:>6}  °C");
        }
        None => {
            let _ = writeln!(o, "├── Liquid temperature     n/a  (sensor fault; reset or update the firmware)");
        }
    }
    let _ = writeln!(o, "├── Pump speed          {:>6}  rpm", st.pump_rpm);
    let _ = writeln!(o, "├── Pump duty           {:>6}  %", st.pump_duty);
    let _ = writeln!(o, "├── Fan speed           {:>6}  rpm", st.fan_rpm);
    let _ = writeln!(o, "└── Fan duty            {:>6}  %", st.fan_duty);
}

fn watch(opts: &Opts, rest: &[&str]) -> Result<(), Error> {
    let interval_ms = match rest {
        [] => 500,
        [s] => parse_seconds_ms(s).ok_or(Error::Usage("interval must be seconds, like 0.5 or 2"))?,
        _ => return Err(Error::Usage("too many arguments")),
    }
    .max(100);
    let dev = Kraken::open(opts.device, opts.verbose)?;
    loop {
        let st = dev.status()?;
        let mut o = Out::stdout();
        // Home the cursor and clear the screen, then draw the whole frame in one write.
        let _ = write!(o, "\x1b[H\x1b[2JEvery {}.{}s, Ctrl-C to stop\n\n", interval_ms / 1000, interval_ms % 1000 / 100);
        print_status(&mut o, &dev.path, &st);
        o.flush();
        sys::sleep_ms(interval_ms);
    }
}

/// `"0.5"` to `500`, `"2"` to `2000`. Up to millisecond precision.
fn parse_seconds_ms(s: &str) -> Option<u64> {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if (whole.is_empty() && frac.is_empty()) || whole.len() > 6 || frac.len() > 3 {
        return None;
    }
    let digits = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(frac) {
        return None;
    }
    let mut ms = whole.bytes().fold(0u64, |a, b| a * 10 + (b - b'0') as u64) * 1000;
    let mut scale = 100;
    for b in frac.bytes() {
        ms += (b - b'0') as u64 * scale;
        scale /= 10;
    }
    Some(ms)
}

fn apply_file(opts: &Opts, rest: &[&str]) -> Result<(), Error> {
    let file = match rest {
        [] => DEFAULT_CONFIG,
        [f] => *f,
        _ => return Err(Error::Usage("too many arguments")),
    };
    let path = CPath::from(file).ok_or(Error::Usage("config path is too long"))?;
    let mut buf = [0u8; 16384];
    let bytes = io::read_file(&path, &mut buf).map_err(|errno| Error::Sys { what: "read", path, errno })?;
    let text = core::str::from_utf8(bytes).map_err(|_| Error::Sys { what: "read", path, errno: sys::EIO })?;
    let cfg = config::parse(text).map_err(|err| Error::Config { source: path, err })?;
    if !cfg.has_kraken() && !cfg.has_board() && !cfg.has_gpu() {
        return Err(Error::Config { source: path, err: config::Error { line: 0, kind: config::ErrorKind::Empty } });
    }
    apply(opts, &cfg, path.as_str())
}

fn set(opts: &Opts, rest: &[&str]) -> Result<(), Error> {
    let [channel, curve @ ..] = rest else {
        return Err(Error::Usage("usage: aioconf set <pump|fan> <DUTY | TEMP DUTY ...>"));
    };
    let channel = Channel::parse(channel).ok_or(Error::Usage("channel must be pump or fan"))?;
    let setting = config::parse_setting(curve.iter().copied()).map_err(Error::Setting)?;
    let mut cfg = Config::default();
    match channel {
        Channel::Pump => cfg.pump = Some(setting),
        Channel::Fan => cfg.fan = Some(setting),
    }
    apply(opts, &cfg, "command line")
}

fn apply(opts: &Opts, cfg: &Config, source: &str) -> Result<(), Error> {
    let mut o = Out::stdout();
    // Apply each part even if the other fails, so a missing motherboard driver
    // does not leave the Kraken unconfigured, then report the first error.
    let kraken = if cfg.has_kraken() { apply_kraken(opts, cfg, source, &mut o) } else { Ok(()) };
    let board = if cfg.has_board() { board::apply(cfg, opts.dry_run, source, &mut o) } else { Ok(()) };
    for (name, value) in [("mem_offset", cfg.gpu.mem_offset), ("gr_offset", cfg.gpu.gr_offset)] {
        if let Some(mhz) = value {
            let _ = writeln!(o, "gpu   {name} {mhz:+} MHz  (applied by aioconf-nvidia)");
        }
    }
    kraken.and(board)
}

fn apply_kraken(opts: &Opts, cfg: &Config, source: &str, o: &mut Out) -> Result<(), Error> {
    let dev = if opts.dry_run { None } else { Some(Kraken::open(opts.device, opts.verbose)?) };
    for ch in Channel::ALL {
        let Some(setting) = cfg.get(ch) else { continue };
        let table = setting.table(ch);
        let report = proto::set_curve(ch, &table);
        match &dev {
            Some(d) => d.send(&report)?,
            None => o.hex("would send", &report),
        }
        let _ = writeln!(o, "{:<4}  {setting}  ({source})", ch.name());
        let _ = write!(o, "      ");
        print_table(o, &table);
    }
    if let Some(d) = &dev {
        let _ = writeln!(o, "Applied to {}", d.path);
    }
    Ok(())
}

/// The 40-degree table as runs, like `20-27°C 20%, 28°C 50%, 30-59°C 100%`.
fn print_table(o: &mut Out, table: &[u8; TABLE_LEN]) {
    let mut start = 0;
    while start < TABLE_LEN {
        let mut end = start;
        while end + 1 < TABLE_LEN && table[end + 1] == table[start] {
            end += 1;
        }
        let sep = if start == 0 { "" } else { ", " };
        let (lo, hi) = (FIRST_TEMP as usize + start, FIRST_TEMP as usize + end);
        let _ = if lo == hi {
            write!(o, "{sep}{lo}°C {}%", table[start])
        } else {
            write!(o, "{sep}{lo}-{hi}°C {}%", table[start])
        };
        start = end + 1;
    }
    let _ = writeln!(o);
}

fn init(opts: &Opts, rest: &[&str]) -> Result<(), Error> {
    no_args(rest)?;
    let dev = Kraken::open(opts.device, opts.verbose)?;
    let [major, minor, patch] = dev.init()?;
    let _ = writeln!(Out::stdout(), "{} ({})\n└── Firmware version  {major}.{minor}.{patch}", proto::MODEL, dev.path);
    Ok(())
}

fn list(rest: &[&str]) -> Result<(), Error> {
    no_args(rest)?;
    let mut o = Out::stdout();
    let mut n = 0;
    device::for_each_kraken(|name| {
        let _ = writeln!(
            o,
            "/dev/{}  {} ({:04x}:{:04x})",
            core::str::from_utf8(name).unwrap_or("?"),
            proto::MODEL,
            proto::VENDOR_ID,
            proto::PRODUCT_ID
        );
        n += 1;
    })?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}
