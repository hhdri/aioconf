//! The motherboard's Nuvoton fan chip (NCT67xx), through the `nct6775` hwmon
//! driver's sysfs files.
//!
//! The chip runs fan curves on its own in "SmartFan IV" mode, much like the
//! Kraken does. aioconf only writes the curve points and the temperature
//! source into the chip, so no daemon is needed. The BIOS reprograms the chip
//! at boot and often on resume, which is why the service reapplies them.
//!
//! This goes through the kernel driver rather than the chip's I/O ports,
//! because the driver and the ASUS firmware both access the chip and direct
//! port I/O would race them.
//!
//! The chip's step-up and step-down times are deliberately never written. On
//! an NCT6798D the driver cannot store the BIOS value of 0 (it reads back as
//! 400 ms), and after any write to them the header stops following its curve
//! until the next reboot. A slow sensor such as CPUTIN smooths the response
//! instead.

use crate::io::{self, CPath, Out};
use crate::Error;
use aioconf::config::{BoardFan, Config, Source, BOARD_HEADERS, BOARD_POINTS};
use core::fmt::Write;

/// `pwmN_enable` value for SmartFan IV, where the chip follows its curve.
const SMART_FAN_IV: i64 = 5;
const MAX_TEMPS: usize = 32;

/// The hwmon directory of the first `nct6*` chip, and its name.
pub fn find() -> Result<(CPath, [u8; 16], usize), Error> {
    let root = CPath::from("/sys/class/hwmon").unwrap();
    let mut found: Option<(CPath, [u8; 16], usize)> = None;
    io::list_dir(&root, |entry| {
        if found.is_some() {
            return;
        }
        let Ok(entry) = core::str::from_utf8(entry) else { return };
        let Some(name_path) = io::path(format_args!("/sys/class/hwmon/{entry}/name")) else { return };
        let mut buf = [0u8; 32];
        if let Ok(name) = io::read_file(&name_path, &mut buf) {
            let name = name.strip_suffix(b"\n").unwrap_or(name);
            if name.starts_with(b"nct6") && name.len() <= 16
                && let Some(dir) = io::path(format_args!("/sys/class/hwmon/{entry}")) {
                    let mut n = [0u8; 16];
                    n[..name.len()].copy_from_slice(name);
                    found = Some((dir, n, name.len()));
                }
        }
    })
    .map_err(|errno| Error::Sys { what: "list", path: root, errno })?;
    found.ok_or(Error::NoChip)
}

fn attr(dir: &CPath, args: core::fmt::Arguments) -> CPath {
    let mut p = *dir;
    let _ = p.write_str("/");
    let _ = core::fmt::write(&mut p, args);
    p
}

fn read(dir: &CPath, args: core::fmt::Arguments) -> Result<i64, Error> {
    let path = attr(dir, args);
    io::read_int(&path).map_err(|errno| Error::Sys { what: "read", path, errno })
}

fn write(dir: &CPath, args: core::fmt::Arguments, value: i64, dry_run: bool, o: &mut Out) -> Result<(), Error> {
    let path = attr(dir, args);
    if dry_run {
        let _ = writeln!(o, "would write {value} to {path}");
        return Ok(());
    }
    io::write_int(&path, value).map_err(|errno| Error::Sys { what: "write", path, errno })
}

/// The label of `tempN`, written into `buf`.
fn temp_label<'a>(dir: &CPath, n: usize, buf: &'a mut [u8; 64]) -> Option<&'a str> {
    let path = attr(dir, format_args!("temp{n}_label"));
    let text = io::read_file(&path, buf).ok()?;
    core::str::from_utf8(text.strip_suffix(b"\n").unwrap_or(text)).ok()
}

/// The `tempN` index a source names.
fn resolve(dir: &CPath, source: &Source) -> Result<usize, Error> {
    for n in 1..=MAX_TEMPS {
        let mut buf = [0u8; 64];
        if let Some(label) = temp_label(dir, n, &mut buf)
            && source.matches(n, label) {
                return Ok(n);
            }
    }
    Err(Error::UnknownSensor(*source))
}

/// Program every board header the config mentions.
pub fn apply(cfg: &Config, dry_run: bool, source_name: &str, o: &mut Out) -> Result<(), Error> {
    let (dir, name, name_len) = find()?;
    let chip = core::str::from_utf8(&name[..name_len]).unwrap_or("nct6775");
    for (i, slot) in cfg.board.iter().enumerate() {
        let Some(fan) = slot else { continue };
        let n = i + 1;
        read(&dir, format_args!("pwm{n}_enable")).map_err(|_| Error::NoHeader(n))?;
        program(&dir, n, fan, dry_run, o)?;
        let _ = writeln!(o, "pwm{n}  {fan}  ({source_name})");
    }
    if !dry_run {
        let _ = writeln!(o, "Applied to {chip} ({dir})");
    }
    Ok(())
}

fn program(dir: &CPath, n: usize, fan: &BoardFan, dry_run: bool, o: &mut Out) -> Result<(), Error> {
    if let Some(src) = &fan.source {
        let sel = resolve(dir, src)?;
        write(dir, format_args!("pwm{n}_temp_sel"), sel as i64, dry_run, o)?;
    }
    for (k, (temp, pwm)) in fan.chip_points().iter().enumerate().take(BOARD_POINTS) {
        let k = k + 1;
        write(dir, format_args!("pwm{n}_auto_point{k}_temp"), *temp as i64 * 1000, dry_run, o)?;
        write(dir, format_args!("pwm{n}_auto_point{k}_pwm"), *pwm as i64, dry_run, o)?;
    }
    write(dir, format_args!("pwm{n}_enable"), SMART_FAN_IV, dry_run, o)
}

fn mode_name(enable: i64) -> &'static str {
    match enable {
        0 => "full speed",
        1 => "manual",
        2 => "thermal cruise",
        3 => "fan speed cruise",
        4 => "smart fan III",
        5 => "curve (smart fan IV)",
        _ => "unknown mode",
    }
}

/// Show what each header is doing.
pub fn show(o: &mut Out) -> Result<(), Error> {
    let (dir, name, name_len) = find()?;
    let chip = core::str::from_utf8(&name[..name_len]).unwrap_or("nct6775");
    let _ = writeln!(o, "{chip} ({dir})");
    for n in 1..=BOARD_HEADERS {
        let Ok(enable) = read(&dir, format_args!("pwm{n}_enable")) else { continue };
        let rpm = read(&dir, format_args!("fan{n}_input")).unwrap_or(0);
        let pwm = read(&dir, format_args!("pwm{n}")).unwrap_or(0);
        let _ = write!(o, "pwm{n}  {:>3}%  {rpm:>5} rpm  {}", (pwm * 100 + 127) / 255, mode_name(enable));
        if enable == SMART_FAN_IV {
            let sel = read(&dir, format_args!("pwm{n}_temp_sel")).unwrap_or(0);
            let mut buf = [0u8; 64];
            let label = temp_label(&dir, sel as usize, &mut buf).unwrap_or("?");
            let now = read(&dir, format_args!("temp{sel}_input")).unwrap_or(0);
            let _ = write!(o, ", following {label} (now {}°C):", now / 1000);
            for k in 1..=BOARD_POINTS {
                let t = read(&dir, format_args!("pwm{n}_auto_point{k}_temp")).unwrap_or(0);
                let p = read(&dir, format_args!("pwm{n}_auto_point{k}_pwm")).unwrap_or(0);
                let sep = if k == 1 { " " } else { ", " };
                let _ = write!(o, "{sep}{}°C {}%", t / 1000, (p * 100 + 127) / 255);
            }
        }
        let _ = writeln!(o);
    }
    Ok(())
}
