//! The config file format and the curve syntax shared with the command line.
//!
//! ```text
//! # comment
//! pump = 27 20  28 50  29 75  30 100   # pairs: temperature °C, duty %
//! fan  = 40                            # one number: fixed duty %
//! ```

use crate::curve::{self, Point, MAX_POINTS, TABLE_LEN};
use crate::proto::Channel;
use core::fmt;

/// What a channel should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// One duty for every temperature (still 100% at 59 °C as a failsafe).
    Fixed(u8),
    /// Points to interpolate between.
    Curve(Points),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Points {
    pts: [Point; MAX_POINTS],
    len: usize,
}

impl Points {
    pub fn as_slice(&self) -> &[Point] {
        &self.pts[..self.len]
    }
}

impl Setting {
    /// The firmware table for this setting on `channel`.
    pub fn table(&self, channel: Channel) -> [u8; TABLE_LEN] {
        let (min, max) = channel.duty_range();
        match self {
            // liquidctl sends a fixed duty as a flat curve up to 58 °C.
            Setting::Fixed(d) => curve::table(&[Point::new(0, *d), Point::new(curve::CRITICAL_TEMP - 1, *d)], min, max),
            Setting::Curve(p) => curve::table(p.as_slice(), min, max),
        }
    }
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Setting::Fixed(d) => write!(f, "fixed {d}%"),
            Setting::Curve(p) => {
                f.write_str("curve")?;
                for (i, pt) in p.as_slice().iter().enumerate() {
                    let sep = if i == 0 { " " } else { ", " };
                    write!(f, "{sep}{}°C {}%", pt.temp, pt.duty)?;
                }
                Ok(())
            }
        }
    }
}

/// Fan headers on the motherboard's Nuvoton chip (nct6775 driver).
pub const BOARD_HEADERS: usize = 7;
/// Curve points the chip offers. A fifth point is the chip's critical
/// temperature, which is left as the firmware set it.
pub const BOARD_POINTS: usize = 4;
pub const SOURCE_LEN: usize = 32;

/// The temperature sensor a board header follows: an hwmon label such as
/// `SYSTIN` or `CPUTIN` (spaces written as `_`), or `tempN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    buf: [u8; SOURCE_LEN],
    len: usize,
}

impl Source {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    /// Whether this names the sensor `temp{index}` whose label is `label`.
    pub fn matches(&self, index: usize, label: &str) -> bool {
        let s = self.as_str();
        if let Some(n) = s.strip_prefix("temp")
            && let Some(n) = parse_u8(n) {
                return n as usize == index;
            }
        let label = label.trim_end();
        s.len() == label.len() && s.bytes().zip(label.bytes()).all(|(a, b)| a == if b == b' ' { b'_' } else { b })
    }
}

/// The chip's own curve for one header, in SmartFan IV mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardFan {
    pub source: Option<Source>,
    pub points: [Point; BOARD_POINTS],
    pub len: usize,
}

impl BoardFan {
    pub fn points(&self) -> &[Point] {
        &self.points[..self.len]
    }

    /// The four chip points as (°C, PWM 0-255). Unused points repeat the last
    /// one, since the chip always uses all of them.
    pub fn chip_points(&self) -> [(u8, u8); BOARD_POINTS] {
        let mut out = [(0, 0); BOARD_POINTS];
        for (i, slot) in out.iter_mut().enumerate() {
            let p = self.points[i.min(self.len - 1)];
            *slot = (p.temp, ((p.duty as u16 * 255 + 50) / 100) as u8);
        }
        out
    }
}

impl fmt::Display for BoardFan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.source.as_ref().map_or("current source", |s| s.as_str()))?;
        for (i, pt) in self.points().iter().enumerate() {
            let sep = if i == 0 { ": " } else { ", " };
            write!(f, "{sep}{}°C {}%", pt.temp, pt.duty)?;
        }
        Ok(())
    }
}

/// NVIDIA clock offsets, `gpu.mem_offset = 3000` and `gpu.gr_offset = 100`, in
/// NVML's units. aioconf only validates them; `aioconf-nvidia` applies them
/// through NVML, since a freestanding binary cannot load NVIDIA's library.
/// On the cards tested the offset is shared by every power state, so there is
/// one value per clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gpu {
    pub mem_offset: Option<i16>,
    pub gr_offset: Option<i16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub pump: Option<Setting>,
    pub fan: Option<Setting>,
    /// `board[0]` is header `pwm1`.
    pub board: [Option<BoardFan>; BOARD_HEADERS],
    pub gpu: Gpu,
}

impl Config {
    pub fn has_kraken(&self) -> bool {
        self.pump.is_some() || self.fan.is_some()
    }

    pub fn has_board(&self) -> bool {
        self.board.iter().any(Option::is_some)
    }

    pub fn has_gpu(&self) -> bool {
        self.gpu.mem_offset.is_some() || self.gpu.gr_offset.is_some()
    }
}

impl Config {
    pub fn get(&self, channel: Channel) -> Option<&Setting> {
        match channel {
            Channel::Pump => self.pump.as_ref(),
            Channel::Fan => self.fan.as_ref(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    MissingEquals,
    UnknownKey,
    Empty,
    NotANumber,
    OddCount,
    TooManyPoints,
    TempOutOfRange,
    DutyOutOfRange,
    BadHeader,
    BadSource,
    BoardCurve,
    BadGpuKey,
    GpuOffsetRange,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorKind::MissingEquals => "expected `pump = ...` or `fan = ...`",
            ErrorKind::UnknownKey => "unknown key, expected `pump` or `fan`",
            ErrorKind::Empty => "no duty given",
            ErrorKind::NotANumber => "not a whole number",
            ErrorKind::OddCount => "a curve needs pairs of temperature and duty",
            ErrorKind::TooManyPoints => "a curve can have at most 32 points",
            ErrorKind::TempOutOfRange => "temperature must be 0 to 100 °C",
            ErrorKind::DutyOutOfRange => "duty must be 0 to 100 %",
            ErrorKind::BadHeader => "board headers are board.pwm1 to board.pwm7",
            ErrorKind::BadSource => "a sensor name is letters, digits and _, at most 32 characters",
            ErrorKind::BoardCurve => "a board curve is 1 to 4 pairs of temperature and duty",
            ErrorKind::BadGpuKey => "GPU keys are gpu.mem_offset and gpu.gr_offset",
            ErrorKind::GpuOffsetRange => "a GPU offset is a whole number of MHz from -2000 to 6000",
        })
    }
}

/// A config error with its 1-based line number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub kind: ErrorKind,
}

pub fn parse(text: &str) -> Result<Config, Error> {
    let mut cfg = Config::default();
    for (i, raw) in text.lines().enumerate() {
        let err = |kind| Error { line: i + 1, kind };
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(err(ErrorKind::MissingEquals))?;
        let key = key.trim();
        if let Some(kind) = key.strip_prefix("gpu.") {
            let mhz = parse_i16(value.trim()).filter(|v| (-2000..=6000).contains(v)).ok_or(err(ErrorKind::GpuOffsetRange))?;
            match kind {
                "mem_offset" => cfg.gpu.mem_offset = Some(mhz),
                "gr_offset" => cfg.gpu.gr_offset = Some(mhz),
                _ => return Err(err(ErrorKind::BadGpuKey)),
            }
            continue;
        }
        if let Some(header) = key.strip_prefix("board.pwm") {
            let n = parse_u8(header).filter(|n| (1..=BOARD_HEADERS as u8).contains(n)).ok_or(err(ErrorKind::BadHeader))?;
            cfg.board[n as usize - 1] = Some(parse_board(value).map_err(err)?);
            continue;
        }
        let setting = parse_setting(value.split_whitespace()).map_err(err)?;
        match Channel::parse(key) {
            Some(Channel::Pump) => cfg.pump = Some(setting),
            Some(Channel::Fan) => cfg.fan = Some(setting),
            None => return Err(err(ErrorKind::UnknownKey)),
        }
    }
    Ok(cfg)
}

/// Parse `DUTY` or `TEMP DUTY [TEMP DUTY ...]`.
pub fn parse_setting<'a>(tokens: impl Iterator<Item = &'a str>) -> Result<Setting, ErrorKind> {
    let mut nums = [0u8; 2 * MAX_POINTS];
    let mut n = 0;
    for tok in tokens {
        if n == nums.len() {
            return Err(ErrorKind::TooManyPoints);
        }
        nums[n] = parse_u8(tok).ok_or(ErrorKind::NotANumber)?;
        n += 1;
    }
    match n {
        0 => Err(ErrorKind::Empty),
        1 => Ok(Setting::Fixed(duty(nums[0])?)),
        _ if n % 2 == 1 => Err(ErrorKind::OddCount),
        _ => {
            let mut pts = Points { pts: [Point::default(); MAX_POINTS], len: n / 2 };
            for (slot, pair) in pts.pts.iter_mut().zip(nums[..n].chunks_exact(2)) {
                if pair[0] > 100 {
                    return Err(ErrorKind::TempOutOfRange);
                }
                *slot = Point::new(pair[0], duty(pair[1])?);
            }
            Ok(Setting::Curve(pts))
        }
    }
}

/// Parse `[SOURCE] TEMP DUTY [TEMP DUTY ...]` for a board header.
pub fn parse_board(value: &str) -> Result<BoardFan, ErrorKind> {
    let mut tokens = value.split_whitespace().peekable();
    let source = match tokens.peek() {
        Some(t) if !t.bytes().all(|b| b.is_ascii_digit()) => {
            let t = tokens.next().unwrap_or("");
            if t.len() > SOURCE_LEN || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(ErrorKind::BadSource);
            }
            let mut buf = [0u8; SOURCE_LEN];
            buf[..t.len()].copy_from_slice(t.as_bytes());
            Some(Source { buf, len: t.len() })
        }
        _ => None,
    };
    let Setting::Curve(p) = parse_setting(tokens)? else { return Err(ErrorKind::BoardCurve) };
    let pts = p.as_slice();
    if pts.len() > BOARD_POINTS {
        return Err(ErrorKind::BoardCurve);
    }
    // The chip interpolates between consecutive points, so they must be in order.
    let mut points = [Point::default(); BOARD_POINTS];
    points[..pts.len()].copy_from_slice(pts);
    let sorted = &mut points[..pts.len()];
    for i in 1..sorted.len() {
        let mut j = i;
        while j > 0 && sorted[j].temp < sorted[j - 1].temp {
            sorted.swap(j, j - 1);
            j -= 1;
        }
    }
    Ok(BoardFan { source, points, len: pts.len() })
}

fn duty(d: u8) -> Result<u8, ErrorKind> {
    if d <= 100 { Ok(d) } else { Err(ErrorKind::DutyOutOfRange) }
}

fn parse_i16(s: &str) -> Option<i16> {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v = digits.bytes().fold(0i32, |a, b| a * 10 + (b - b'0') as i32);
    i16::try_from(if neg { -v } else { v }).ok()
}

fn parse_u8(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v = s.bytes().fold(0u16, |acc, b| acc * 10 + (b - b'0') as u16);
    u8::try_from(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = "pump = 27 20  28 50  29 75  30 100\nfan  = 27 10  28 50  29 75  30 100\n";

    #[test]
    fn example_config_parses() {
        let cfg = parse(include_str!("../contrib/aioconf.conf")).unwrap();
        assert!(cfg.pump.is_some() && cfg.fan.is_some());
    }

    #[test]
    fn original_script_curves() {
        let cfg = parse(ORIGINAL).unwrap();
        let pump = cfg.pump.unwrap().table(Channel::Pump);
        let fan = cfg.fan.unwrap().table(Channel::Fan);
        // 20..=27 °C: pump 20, fan 10; 28: 50; 29: 75; 30..=59: 100.
        assert_eq!(&pump[..10], &[20, 20, 20, 20, 20, 20, 20, 20, 50, 75]);
        assert_eq!(&fan[..10], &[10, 10, 10, 10, 10, 10, 10, 10, 50, 75]);
        assert!(pump[10..].iter().chain(&fan[10..]).all(|&d| d == 100));
    }

    #[test]
    fn fixed_duty_keeps_failsafe() {
        let t = Setting::Fixed(40).table(Channel::Fan);
        assert!(t[..39].iter().all(|&d| d == 40));
        assert_eq!(t[39], 100);
        // The pump is never set below 20%.
        assert!(Setting::Fixed(5).table(Channel::Pump)[..39].iter().all(|&d| d == 20));
    }

    #[test]
    fn comments_blank_lines_and_errors() {
        let cfg = parse("# hi\n\n  fan = 50 # trailing\n").unwrap();
        assert_eq!(cfg.fan, Some(Setting::Fixed(50)));
        assert_eq!(cfg.pump, None);
        assert_eq!(parse("pump 50").unwrap_err(), Error { line: 1, kind: ErrorKind::MissingEquals });
        assert_eq!(parse("\nlights = 1").unwrap_err().kind, ErrorKind::UnknownKey);
        assert_eq!(parse("fan = 30 40 50").unwrap_err().kind, ErrorKind::OddCount);
        assert_eq!(parse("fan = 30 101").unwrap_err().kind, ErrorKind::DutyOutOfRange);
        assert_eq!(parse("fan = 130 50").unwrap_err().kind, ErrorKind::TempOutOfRange);
        assert_eq!(parse("fan = -5").unwrap_err().kind, ErrorKind::NotANumber);
        assert_eq!(parse("fan =").unwrap_err().kind, ErrorKind::Empty);
    }

    #[test]
    fn board_headers() {
        let cfg = parse("board.pwm2 = SYSTIN 30 30  45 60  40 50\nboard.pwm1 = 35 40\n").unwrap();
        let b2 = cfg.board[1].unwrap();
        assert_eq!(b2.source.unwrap().as_str(), "SYSTIN");
        // Sorted by temperature, padded with the last point, duty scaled to 0-255.
        assert_eq!(b2.chip_points(), [(30, 77), (40, 128), (45, 153), (45, 153)]);
        let b1 = cfg.board[0].unwrap();
        assert_eq!(b1.source, None);
        assert_eq!(b1.chip_points(), [(35, 102); 4]);
        assert!(cfg.has_board() && !cfg.has_kraken());

        let src = parse_board("PECI_Agent_0_Calibration 40 50").unwrap().source.unwrap();
        assert!(src.matches(9, "PECI Agent 0 Calibration"));
        assert!(!src.matches(8, "PECI Agent 0"));
        assert!(parse_board("temp1 40 50").unwrap().source.unwrap().matches(1, "SYSTIN"));

        assert_eq!(parse("board.pwm8 = 30 40").unwrap_err().kind, ErrorKind::BadHeader);
        assert_eq!(parse("board.pwm1 = 50").unwrap_err().kind, ErrorKind::BoardCurve);
        assert_eq!(parse("board.pwm1 = 1 1 2 2 3 3 4 4 5 5").unwrap_err().kind, ErrorKind::BoardCurve);
        assert_eq!(parse("board.pwm1 = SYS-TIN 30 40").unwrap_err().kind, ErrorKind::BadSource);
        // Ramp times are deliberately not configurable; see src/board.rs.
        assert_eq!(parse("board.pwm1.ramp = 100 300").unwrap_err().kind, ErrorKind::BadHeader);
    }

    #[test]
    fn gpu_offsets() {
        let cfg = parse("gpu.mem_offset = 3000\ngpu.gr_offset = -100\ngpu.mem_offset = +2500\n").unwrap();
        assert_eq!(cfg.gpu, Gpu { mem_offset: Some(2500), gr_offset: Some(-100) });
        assert!(cfg.has_gpu() && !cfg.has_kraken() && !cfg.has_board());
        assert_eq!(parse("gpu.mem_offset.p2 = 100").unwrap_err().kind, ErrorKind::BadGpuKey);
        assert_eq!(parse("gpu.core_offset = 100").unwrap_err().kind, ErrorKind::BadGpuKey);
        assert_eq!(parse("gpu.mem_offset = 7000").unwrap_err().kind, ErrorKind::GpuOffsetRange);
        assert_eq!(parse("gpu.mem_offset = fast").unwrap_err().kind, ErrorKind::GpuOffsetRange);
    }

    #[test]
    fn display() {
        let cfg = parse(ORIGINAL).unwrap();
        assert_eq!(std::format!("{}", cfg.fan.unwrap()), "curve 27°C 10%, 28°C 50%, 29°C 75%, 30°C 100%");
        assert_eq!(std::format!("{}", Setting::Fixed(100)), "fixed 100%");
    }
}
