//! The bits of a runtime that libc and std would normally provide: the process
//! entry point, the panic handler, and the memory routines the compiler emits
//! calls to.

use crate::io::Out;
use crate::sys;
use core::ffi::c_void;
use core::fmt::Write;

const MAX_ARGS: usize = 64;

/// The ELF entry point. The kernel jumps here with the stack pointer at
/// `argc`, followed by the `argv` pointers. There is no return address, so
/// this only aligns the stack and calls into Rust.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "xor ebp, ebp",   // mark the outermost frame for debuggers
        "mov rdi, rsp",   // first argument: pointer to argc
        "and rsp, -16",   // SysV ABI: 16-byte aligned stack before a call
        "call {entry}",
        "ud2",
        entry = sym entry,
    );
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "mov x29, xzr",
        "mov x30, xzr",
        "mov x0, sp",
        "bl {entry}",
        "brk #0",
        entry = sym entry,
    );
}

unsafe extern "C" fn entry(sp: *const usize) -> ! {
    let argc = unsafe { *sp };
    let argv = unsafe { sp.add(1) as *const *const u8 };

    let mut args: [&str; MAX_ARGS] = [""; MAX_ARGS];
    let mut n = 0;
    // Skip argv[0], the program name.
    for i in 1..argc {
        if n == MAX_ARGS {
            let mut err = Out::stderr();
            let _ = writeln!(err, "aioconf: too many arguments");
            err.flush();
            sys::exit(2);
        }
        let ptr = unsafe { *argv.add(i) };
        let bytes = unsafe { cstr(ptr) };
        match core::str::from_utf8(bytes) {
            Ok(s) => args[n] = s,
            Err(_) => {
                let mut err = Out::stderr();
                let _ = writeln!(err, "aioconf: arguments must be valid UTF-8");
                err.flush();
                sys::exit(2);
            }
        }
        n += 1;
    }
    sys::exit(crate::main(&args[..n]))
}

/// The bytes of a NUL-terminated string, without the NUL.
unsafe fn cstr<'a>(ptr: *const u8) -> &'a [u8] {
    unsafe { core::slice::from_raw_parts(ptr, strlen(ptr.cast())) }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut err = Out::stderr();
    let _ = writeln!(err, "aioconf: internal error: {info}");
    err.flush();
    sys::exit(101)
}

// The compiler lowers struct copies, array initialization and slice
// comparisons to calls to these symbols, which libc normally provides. The
// loops use volatile accesses so the optimizer cannot turn them back into a
// call to themselves.

#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst as *mut u8, src as *const u8);
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "rep movsb",
            inout("rcx") n => _, inout("rdi") d => _, inout("rsi") s => _,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    for i in 0..n {
        unsafe { d.add(i).write_volatile(s.add(i).read_volatile()) };
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    let d = dst as *mut u8;
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "rep stosb",
            inout("rcx") n => _, inout("rdi") d => _, in("al") c as u8,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(target_arch = "x86_64"))]
    for i in 0..n {
        unsafe { d.add(i).write_volatile(c as u8) };
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let (d, s) = (dst as *mut u8, src as *const u8);
    if (d as usize) <= (s as usize) || (d as usize) >= (s as usize) + n {
        for i in 0..n {
            unsafe { d.add(i).write_volatile(s.add(i).read_volatile()) };
        }
    } else {
        for i in (0..n).rev() {
            unsafe { d.add(i).write_volatile(s.add(i).read_volatile()) };
        }
    }
    dst
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    let (a, b) = (a as *const u8, b as *const u8);
    for i in 0..n {
        let (x, y) = unsafe { (a.add(i).read_volatile(), b.add(i).read_volatile()) };
        if x != y {
            return x as i32 - y as i32;
        }
    }
    0
}

// LLVM recognizes a "count bytes until NUL" loop and emits a call to strlen.
#[unsafe(no_mangle)]
unsafe extern "C" fn strlen(s: *const core::ffi::c_char) -> usize {
    let s = s as *const u8;
    let mut n = 0;
    while unsafe { s.add(n).read_volatile() } != 0 {
        n += 1;
    }
    n
}

#[unsafe(no_mangle)]
unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    unsafe { memcmp(a, b, n) }
}

/// The precompiled `core` library references the unwinding personality
/// routine even though this binary is built with `panic = "abort"` and never
/// unwinds. This stub satisfies the linker and is never called.
#[unsafe(no_mangle)]
extern "C" fn rust_eh_personality() {}
