//! The NZXT Kraken 2023 HID protocol (USB `1e71:300e`, sold as the Kraken
//! and the Kraken Base).
//!
//! Every message in either direction is a 64-byte HID report whose first byte
//! is the report ID. On Linux they are exchanged verbatim with `write(2)` and
//! `read(2)` on the `/dev/hidrawN` node. The layouts here follow liquidctl's
//! `kraken3` driver, where the protocol was reverse-engineered.

use crate::curve::TABLE_LEN;

pub const VENDOR_ID: u16 = 0x1e71;
pub const PRODUCT_ID: u16 = 0x300e;
pub const MODEL: &str = "NZXT Kraken 2023";
pub const REPORT_LEN: usize = 64;

pub type Report = [u8; REPORT_LEN];

/// A cooling channel the firmware can drive from a temperature curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Pump,
    Fan,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Pump, Channel::Fan];

    pub fn name(self) -> &'static str {
        match self {
            Channel::Pump => "pump",
            Channel::Fan => "fan",
        }
    }

    pub fn parse(s: &str) -> Option<Channel> {
        match s {
            "pump" => Some(Channel::Pump),
            "fan" | "fans" => Some(Channel::Fan),
            _ => None,
        }
    }

    /// Bytes 1..4 of a curve report, selecting the channel.
    fn id(self) -> [u8; 3] {
        match self {
            Channel::Pump => [0x01, 0x01, 0x00],
            Channel::Fan => [0x02, 0x01, 0x01],
        }
    }

    /// Duty limits in %. The pump floor matches liquidctl.
    pub fn duty_range(self) -> (u8, u8) {
        match self {
            Channel::Pump => (20, 100),
            Channel::Fan => (0, 100),
        }
    }
}

/// A report that starts with `head` and is zero-padded to 64 bytes.
pub fn report(head: &[u8]) -> Report {
    let mut r = [0; REPORT_LEN];
    r[..head.len()].copy_from_slice(head);
    r
}

/// Initialization: set the status report interval, then start the device.
/// The interval byte is `1 + round((secs - 0.5) / 0.25)`, so `0x01` is 0.5 s.
/// The `nzxt_kraken3` kernel driver already sends these when it binds.
pub fn init() -> [Report; 2] {
    [report(&[0x70, 0x02, 0x01, 0xb8, 0x01]), report(&[0x70, 0x01])]
}

pub fn firmware_request() -> Report {
    report(&[0x10, 0x01])
}

pub fn status_request() -> Report {
    report(&[0x74, 0x01])
}

/// Set a channel's duty table: `0x72`, the channel ID, then one duty in % per
/// degree from 20 °C to 59 °C.
pub fn set_curve(channel: Channel, table: &[u8; TABLE_LEN]) -> Report {
    let mut r = report(&[0x72]);
    r[1..4].copy_from_slice(&channel.id());
    r[4..4 + TABLE_LEN].copy_from_slice(table);
    r
}

/// A temperature in tenths of a degree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tenths(pub u16);

impl core::fmt::Display for Tenths {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Format into a small buffer first so width and alignment flags work.
        let mut buf = [0u8; 8];
        let mut n = 0;
        let whole = self.0 / 10;
        let mut digits = [0u8; 5];
        let mut d = 0;
        let mut w = whole;
        loop {
            digits[d] = b'0' + (w % 10) as u8;
            d += 1;
            w /= 10;
            if w == 0 {
                break;
            }
        }
        while d > 0 {
            d -= 1;
            buf[n] = digits[d];
            n += 1;
        }
        buf[n] = b'.';
        buf[n + 1] = b'0' + (self.0 % 10) as u8;
        n += 2;
        f.pad(core::str::from_utf8(&buf[..n]).unwrap_or("?"))
    }
}

/// A status report (`0x75 0x01`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    /// `None` when the firmware reports `ff ff`, a known sensor fault.
    pub liquid: Option<Tenths>,
    pub pump_rpm: u16,
    pub pump_duty: u8,
    pub fan_rpm: u16,
    pub fan_duty: u8,
}

pub fn parse_status(r: &Report) -> Option<Status> {
    if r[0] != 0x75 || r[1] != 0x01 {
        return None;
    }
    let liquid = if r[15] == 0xff && r[16] == 0xff {
        None
    } else {
        Some(Tenths(r[15] as u16 * 10 + r[16] as u16))
    };
    Some(Status {
        liquid,
        pump_rpm: u16::from_le_bytes([r[17], r[18]]),
        pump_duty: r[19],
        fan_rpm: u16::from_le_bytes([r[23], r[24]]),
        fan_duty: r[25],
    })
}

/// A firmware report (`0x11 0x01`), giving `major.minor.patch`.
pub fn parse_firmware(r: &Report) -> Option<[u8; 3]> {
    if r[0] != 0x11 || r[1] != 0x01 {
        return None;
    }
    Some([r[0x11], r[0x12], r[0x13]])
}

/// Vendor and product ID from a hidraw device's sysfs `uevent` file, which has
/// a line like `HID_ID=0003:00001E71:0000300E`.
pub fn parse_uevent_ids(uevent: &[u8]) -> Option<(u16, u16)> {
    for line in uevent.split(|&b| b == b'\n') {
        let Some(id) = line.strip_prefix(b"HID_ID=") else { continue };
        let mut fields = id.split(|&b| b == b':');
        let _bus = fields.next()?;
        let vendor = parse_hex(fields.next()?)?;
        let product = parse_hex(fields.next()?)?;
        return Some((u16::try_from(vendor).ok()?, u16::try_from(product).ok()?));
    }
    None
}

fn parse_hex(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 8 {
        return None;
    }
    let mut v = 0u32;
    for &c in s {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        v = v << 4 | d as u32;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{table, Point};

    #[test]
    fn fan_curve_report_layout() {
        let t = table(&[Point::new(27, 10), Point::new(28, 50), Point::new(29, 75), Point::new(30, 100)], 0, 100);
        let r = set_curve(Channel::Fan, &t);
        assert_eq!(&r[..4], &[0x72, 0x02, 0x01, 0x01]);
        assert_eq!(&r[4..12], &[10; 8]);
        assert_eq!(&r[12..14], &[50, 75]);
        assert_eq!(&r[14..44], &[100; 30]);
        assert!(r[44..].iter().all(|&b| b == 0));
    }

    #[test]
    fn pump_curve_report_header() {
        let r = set_curve(Channel::Pump, &[20; TABLE_LEN]);
        assert_eq!(&r[..4], &[0x72, 0x01, 0x01, 0x00]);
    }

    #[test]
    fn status_parsing() {
        let mut r = report(&[0x75, 0x01]);
        r[15] = 22;
        r[16] = 2;
        r[17..19].copy_from_slice(&936u16.to_le_bytes());
        r[19] = 20;
        r[23..25].copy_from_slice(&682u16.to_le_bytes());
        r[25] = 20;
        let s = parse_status(&r).unwrap();
        assert_eq!(s.liquid, Some(Tenths(222)));
        assert_eq!((s.pump_rpm, s.pump_duty, s.fan_rpm, s.fan_duty), (936, 20, 682, 20));
        assert_eq!(std::format!("{}", Tenths(222)), "22.2");
        assert_eq!(std::format!("{:>6}", Tenths(5)), "   0.5");

        r[15] = 0xff;
        r[16] = 0xff;
        assert_eq!(parse_status(&r).unwrap().liquid, None);
        assert_eq!(parse_status(&report(&[0x11, 0x01])), None);
    }

    #[test]
    fn firmware_parsing() {
        let mut r = report(&[0x11, 0x01]);
        r[0x11..0x14].copy_from_slice(&[2, 1, 0]);
        assert_eq!(parse_firmware(&r), Some([2, 1, 0]));
    }

    #[test]
    fn uevent_parsing() {
        let u = b"DRIVER=nzxt_kraken3\nHID_ID=0003:00001E71:0000300E\nHID_NAME=NZXT Inc. NZXT Kraken Base\n";
        assert_eq!(parse_uevent_ids(u), Some((0x1e71, 0x300e)));
        assert_eq!(parse_uevent_ids(b"HID_NAME=x\n"), None);
    }
}
