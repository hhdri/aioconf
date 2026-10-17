//! Buffered output to a file descriptor, NUL-terminated paths, and whole-file
//! reads, all on top of raw syscalls.

use crate::sys::{self, Errno, Fd};
use core::fmt;

/// A buffered writer for stdout or stderr. Implements `core::fmt::Write`, so
/// `write!` works without std. Flushes when full and when dropped.
pub struct Out {
    fd: Fd,
    buf: [u8; 4096],
    len: usize,
}

impl Out {
    pub fn stdout() -> Self {
        Out { fd: 1, buf: [0; 4096], len: 0 }
    }

    pub fn stderr() -> Self {
        Out { fd: 2, buf: [0; 4096], len: 0 }
    }

    pub fn flush(&mut self) {
        // Nothing useful can be done if the terminal is gone.
        let _ = sys::write_all(self.fd, &self.buf[..self.len]);
        self.len = 0;
    }

    /// Write `data` as hex, without trailing zero bytes, after `prefix`.
    pub fn hex(&mut self, prefix: &str, data: &[u8]) {
        let end = data.iter().rposition(|&b| b != 0).map_or(2, |i| (i + 1).max(2)).min(data.len());
        let _ = fmt::Write::write_str(self, prefix);
        for b in &data[..end] {
            let _ = fmt::write(self, format_args!(" {b:02x}"));
        }
        let _ = fmt::Write::write_str(self, "\n");
    }
}

impl fmt::Write for Out {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut bytes = s.as_bytes();
        while !bytes.is_empty() {
            if self.len == self.buf.len() {
                self.flush();
            }
            let n = bytes.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
            self.len += n;
            bytes = &bytes[n..];
        }
        Ok(())
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        self.flush();
    }
}

/// A path in a fixed buffer, always NUL-terminated for the kernel.
#[derive(Clone, Copy)]
pub struct CPath {
    buf: [u8; 256],
    len: usize,
}

impl CPath {
    pub fn new() -> Self {
        CPath { buf: [0; 256], len: 0 }
    }

    pub fn from(s: &str) -> Option<Self> {
        let mut p = CPath::new();
        p.push(s.as_bytes()).then_some(p)
    }

    /// Append bytes. Fails on overflow or an embedded NUL.
    pub fn push(&mut self, s: &[u8]) -> bool {
        if self.len + s.len() >= self.buf.len() || s.contains(&0) {
            return false;
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
        true
    }

    /// The path with its trailing NUL, ready for a syscall.
    pub fn c(&self) -> &[u8] {
        &self.buf[..=self.len]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("<path>")
    }
}

impl fmt::Write for CPath {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.push(s.as_bytes()) { Ok(()) } else { Err(fmt::Error) }
    }
}

impl fmt::Display for CPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Read a whole file into `buf` and return the filled part. Fails with EIO if
/// the file does not fit.
pub fn read_file<'a>(path: &CPath, buf: &'a mut [u8]) -> Result<&'a [u8], Errno> {
    let fd = sys::open(path.c(), sys::O_RDONLY | sys::O_CLOEXEC)?;
    let mut len = 0;
    let result = loop {
        if len == buf.len() {
            break Err(sys::EIO);
        }
        match sys::read(fd, &mut buf[len..]) {
            Ok(0) => break Ok(()),
            Ok(n) => len += n,
            Err(e) => break Err(e),
        }
    };
    sys::close(fd);
    result.map(|()| &buf[..len])
}

/// A path built with `format_args!`, such as `{dir}/pwm{n}_enable`.
pub fn path(args: fmt::Arguments) -> Option<CPath> {
    let mut p = CPath::new();
    fmt::write(&mut p, args).ok().map(|()| p)
}

/// Call `f` with the name of every entry in a directory, using getdents64.
pub fn list_dir(dir: &CPath, mut f: impl FnMut(&[u8])) -> Result<(), Errno> {
    let fd = sys::open(dir.c(), sys::O_RDONLY | sys::O_DIRECTORY | sys::O_CLOEXEC)?;
    let mut buf = [0u8; 4096];
    let result = loop {
        let n = match sys::getdents64(fd, &mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        // struct linux_dirent64 { u64 ino; i64 off; u16 reclen; u8 type; char name[]; }
        let mut off = 0;
        while off < n {
            let reclen = u16::from_ne_bytes([buf[off + 16], buf[off + 17]]) as usize;
            let name_area = &buf[off + 19..off + reclen];
            let name = &name_area[..name_area.iter().position(|&b| b == 0).unwrap_or(name_area.len())];
            if name != b"." && name != b".." {
                f(name);
            }
            off += reclen;
        }
    };
    sys::close(fd);
    result
}

/// Read a sysfs attribute holding one integer.
pub fn read_int(path: &CPath) -> Result<i64, Errno> {
    let mut buf = [0u8; 64];
    let text = read_file(path, &mut buf)?;
    let text = text.strip_suffix(b"\n").unwrap_or(text);
    let (neg, digits) = match text.strip_prefix(b"-") {
        Some(d) => (true, d),
        None => (false, text),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(sys::EIO);
    }
    let v = digits.iter().fold(0i64, |a, &b| a * 10 + (b - b'0') as i64);
    Ok(if neg { -v } else { v })
}

/// Write one integer to a sysfs attribute.
pub fn write_int(path: &CPath, value: i64) -> Result<(), Errno> {
    let mut text = [0u8; 24];
    let mut n = text.len();
    let mut v = value.unsigned_abs();
    loop {
        n -= 1;
        text[n] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if value < 0 {
        n -= 1;
        text[n] = b'-';
    }
    let fd = sys::open(path.c(), sys::O_WRONLY | sys::O_CLOEXEC)?;
    let r = sys::write_all(fd, &text[n..]);
    sys::close(fd);
    r
}
