//! Raw Linux system calls. There is no libc underneath: each call is a single
//! `syscall` (x86_64) or `svc 0` (aarch64) instruction, and errors come back
//! as negative errno values straight from the kernel.

use core::arch::asm;
use core::fmt;

#[cfg(target_arch = "x86_64")]
mod nr {
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const CLOSE: usize = 3;
    pub const IOCTL: usize = 16;
    pub const NANOSLEEP: usize = 35;
    pub const GETDENTS64: usize = 217;
    pub const EXIT_GROUP: usize = 231;
    pub const OPENAT: usize = 257;
    pub const PPOLL: usize = 271;
}

#[cfg(target_arch = "aarch64")]
mod nr {
    pub const GETDENTS64: usize = 61;
    pub const READ: usize = 63;
    pub const WRITE: usize = 64;
    pub const CLOSE: usize = 57;
    pub const IOCTL: usize = 29;
    pub const NANOSLEEP: usize = 101;
    pub const EXIT_GROUP: usize = 94;
    pub const OPENAT: usize = 56;
    pub const PPOLL: usize = 73;
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("aioconf supports x86_64 and aarch64 Linux only");

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn syscall(n: usize, a: [usize; 5]) -> isize {
    let ret: isize;
    // The kernel clobbers rcx (return address) and r11 (saved rflags).
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") n as isize => ret,
            in("rdi") a[0], in("rsi") a[1], in("rdx") a[2], in("r10") a[3], in("r8") a[4],
            lateout("rcx") _, lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn syscall(n: usize, a: [usize; 5]) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "svc 0",
            in("x8") n,
            inlateout("x0") a[0] as isize => ret,
            in("x1") a[1], in("x2") a[2], in("x3") a[3], in("x4") a[4],
            options(nostack),
        );
    }
    ret
}

/// A kernel error number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(pub i32);

pub const ENOENT: Errno = Errno(2);
pub const EINTR: Errno = Errno(4);
pub const EIO: Errno = Errno(5);
pub const EACCES: Errno = Errno(13);

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            1 => "operation not permitted",
            2 => "no such file or directory",
            4 => "interrupted",
            5 => "input/output error",
            9 => "bad file descriptor",
            11 => "resource temporarily unavailable",
            13 => "permission denied",
            16 => "device or resource busy",
            19 => "no such device",
            20 => "not a directory",
            21 => "is a directory",
            22 => "invalid argument",
            25 => "not a hidraw device",
            32 => "broken pipe",
            108 | 110 => "device stopped responding",
            _ => "error",
        };
        write!(f, "{name} (errno {})", self.0)
    }
}

fn check(ret: isize) -> Result<usize, Errno> {
    if (-4095..0).contains(&ret) { Err(Errno(-ret as i32)) } else { Ok(ret as usize) }
}

/// Retry a call that the kernel interrupted with a signal.
fn retry(mut f: impl FnMut() -> isize) -> Result<usize, Errno> {
    loop {
        match check(f()) {
            Err(EINTR) => continue,
            r => return r,
        }
    }
}

pub type Fd = i32;

const AT_FDCWD: isize = -100;
pub const O_RDONLY: usize = 0;
pub const O_WRONLY: usize = 1;
pub const O_RDWR: usize = 2;
pub const O_CLOEXEC: usize = 0o2000000;
#[cfg(target_arch = "x86_64")]
pub const O_DIRECTORY: usize = 0o200000;
#[cfg(target_arch = "aarch64")]
pub const O_DIRECTORY: usize = 0o40000;

/// `path` must be NUL-terminated.
pub fn open(path: &[u8], flags: usize) -> Result<Fd, Errno> {
    debug_assert_eq!(path.last(), Some(&0));
    retry(|| unsafe { syscall(nr::OPENAT, [AT_FDCWD as usize, path.as_ptr() as usize, flags, 0, 0]) })
        .map(|fd| fd as Fd)
}

pub fn close(fd: Fd) {
    // Linux always releases the descriptor, even when close reports an error.
    let _ = unsafe { syscall(nr::CLOSE, [fd as usize, 0, 0, 0, 0]) };
}

pub fn read(fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
    retry(|| unsafe { syscall(nr::READ, [fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0]) })
}

pub fn write(fd: Fd, buf: &[u8]) -> Result<usize, Errno> {
    retry(|| unsafe { syscall(nr::WRITE, [fd as usize, buf.as_ptr() as usize, buf.len(), 0, 0]) })
}

pub fn write_all(fd: Fd, mut buf: &[u8]) -> Result<(), Errno> {
    while !buf.is_empty() {
        match write(fd, buf)? {
            0 => return Err(EIO),
            n => buf = &buf[n..],
        }
    }
    Ok(())
}

pub fn getdents64(fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
    retry(|| unsafe { syscall(nr::GETDENTS64, [fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0]) })
}

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: i64,
}

impl Timespec {
    fn from_ms(ms: u64) -> Self {
        Timespec { sec: (ms / 1000) as i64, nsec: ((ms % 1000) * 1_000_000) as i64 }
    }
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 1;

/// Wait up to `timeout_ms` for `fd` to become readable.
pub fn wait_readable(fd: Fd, timeout_ms: u64) -> Result<bool, Errno> {
    let mut pfd = PollFd { fd, events: POLLIN, revents: 0 };
    let ts = Timespec::from_ms(timeout_ms);
    let n = retry(|| unsafe {
        syscall(nr::PPOLL, [&mut pfd as *mut PollFd as usize, 1, &ts as *const Timespec as usize, 0, 8])
    })?;
    Ok(n > 0)
}

pub fn sleep_ms(ms: u64) {
    let ts = Timespec::from_ms(ms);
    let _ = unsafe { syscall(nr::NANOSLEEP, [&ts as *const Timespec as usize, 0, 0, 0, 0]) };
}

/// `struct hidraw_devinfo` from `<linux/hidraw.h>`.
#[repr(C)]
#[derive(Default)]
struct HidrawDevinfo {
    bustype: u32,
    vendor: i16,
    product: i16,
}

/// `HIDIOCGRAWINFO`, which is `_IOR('H', 0x03, struct hidraw_devinfo)`.
const HIDIOCGRAWINFO: usize = (2 << 30) | (8 << 16) | ((b'H' as usize) << 8) | 0x03;

/// USB vendor and product ID of an open hidraw node.
pub fn hidraw_ids(fd: Fd) -> Result<(u16, u16), Errno> {
    let mut info = HidrawDevinfo::default();
    retry(|| unsafe { syscall(nr::IOCTL, [fd as usize, HIDIOCGRAWINFO, &mut info as *mut _ as usize, 0, 0]) })?;
    Ok((info.vendor as u16, info.product as u16))
}

pub fn exit(code: i32) -> ! {
    unsafe {
        syscall(nr::EXIT_GROUP, [code as usize, 0, 0, 0, 0]);
    }
    // exit_group does not return; this only satisfies the `!` type.
    loop {
        core::hint::spin_loop();
    }
}
