//! Finding and talking to the cooler through `/dev/hidrawN`.

use crate::io::{self, CPath, Out};
use crate::sys::{self, Fd};
use crate::Error;
use aioconf::proto::{self, Report, REPORT_LEN};

/// How many reports to read while waiting for a reply. The device also sends
/// unsolicited status reports, so a reply may not be the first thing to arrive.
const MAX_READS: usize = 12;
const READ_TIMEOUT_MS: u64 = 1000;

/// Call `f` with the name (`hidrawN`) of every hidraw node whose USB IDs match
/// the cooler, found by reading `/sys/class/hidraw/*/device/uevent`. This needs
/// no special permissions.
pub fn for_each_kraken(mut f: impl FnMut(&[u8])) -> Result<(), Error> {
    let dir = CPath::from("/sys/class/hidraw").unwrap();
    io::list_dir(&dir, |name| {
        if name.starts_with(b"hidraw") && is_kraken(name) {
            f(name);
        }
    })
    .map_err(|errno| Error::Sys { what: "list", path: dir, errno })
}

fn is_kraken(name: &[u8]) -> bool {
    let mut path = CPath::from("/sys/class/hidraw/").unwrap();
    if !(path.push(name) && path.push(b"/device/uevent")) {
        return false;
    }
    let mut buf = [0u8; 2048];
    match io::read_file(&path, &mut buf) {
        Ok(text) => proto::parse_uevent_ids(text) == Some((proto::VENDOR_ID, proto::PRODUCT_ID)),
        Err(_) => false,
    }
}

/// The `/dev/hidrawN` path of the first cooler found.
pub fn find() -> Result<CPath, Error> {
    let mut found = None;
    for_each_kraken(|name| {
        if found.is_none() {
            let mut p = CPath::from("/dev/").unwrap();
            if p.push(name) {
                found = Some(p);
            }
        }
    })?;
    found.ok_or(Error::NotFound)
}

/// An open connection to the cooler. Closes the descriptor when dropped.
pub struct Kraken {
    fd: Fd,
    pub path: CPath,
    verbose: bool,
}

impl Kraken {
    /// Open `device`, or the first cooler found, and check with the
    /// `HIDIOCGRAWINFO` ioctl that it really is one before writing anything.
    pub fn open(device: Option<&str>, verbose: bool) -> Result<Kraken, Error> {
        let path = match device {
            Some(p) => CPath::from(p).ok_or(Error::Usage("device path is too long"))?,
            None => find()?,
        };
        let fd = sys::open(path.c(), sys::O_RDWR | sys::O_CLOEXEC)
            .map_err(|errno| Error::Sys { what: "open", path, errno })?;
        let k = Kraken { fd, path, verbose };
        let (vendor, product) =
            sys::hidraw_ids(fd).map_err(|errno| Error::Sys { what: "identify", path, errno })?;
        if (vendor, product) != (proto::VENDOR_ID, proto::PRODUCT_ID) {
            return Err(Error::WrongDevice { path, vendor, product });
        }
        Ok(k)
    }

    pub fn send(&self, r: &Report) -> Result<(), Error> {
        if self.verbose {
            Out::stderr().hex("->", r);
        }
        match sys::write(self.fd, r) {
            Ok(REPORT_LEN) => Ok(()),
            Ok(_) => Err(Error::Sys { what: "write to", path: self.path, errno: sys::EIO }),
            Err(errno) => Err(Error::Sys { what: "write to", path: self.path, errno }),
        }
    }

    /// The next report from the device, or `None` after a timeout.
    pub fn recv(&self, timeout_ms: u64) -> Result<Option<Report>, Error> {
        let ready = sys::wait_readable(self.fd, timeout_ms)
            .map_err(|errno| Error::Sys { what: "wait for", path: self.path, errno })?;
        if !ready {
            return Ok(None);
        }
        let mut r = [0u8; REPORT_LEN];
        sys::read(self.fd, &mut r).map_err(|errno| Error::Sys { what: "read from", path: self.path, errno })?;
        if self.verbose {
            Out::stderr().hex("<-", &r);
        }
        Ok(Some(r))
    }

    /// Send `request` and return the first reply that `parse` accepts.
    pub fn query<T>(&self, request: &Report, what: &'static str, parse: fn(&Report) -> Option<T>) -> Result<T, Error> {
        self.send(request)?;
        for _ in 0..MAX_READS {
            match self.recv(READ_TIMEOUT_MS)? {
                Some(r) => {
                    if let Some(v) = parse(&r) {
                        return Ok(v);
                    }
                }
                None => break,
            }
        }
        Err(Error::NoReply(what))
    }

    pub fn status(&self) -> Result<proto::Status, Error> {
        self.query(&proto::status_request(), "status", proto::parse_status)
    }

    pub fn init(&self) -> Result<[u8; 3], Error> {
        for r in proto::init() {
            self.send(&r)?;
        }
        self.query(&proto::firmware_request(), "firmware", proto::parse_firmware)
    }
}

impl Drop for Kraken {
    fn drop(&mut self) {
        sys::close(self.fd);
    }
}
