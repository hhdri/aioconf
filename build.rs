// The binary brings its own entry point (`_start`) and talks to the kernel with
// raw syscalls, so it is linked without the C runtime and without libc, as a
// static, non-PIE executable. These flags apply to the binary only; the library
// and its unit tests build normally against std.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        for arg in ["-nostartfiles", "-nostdlib", "-static", "-no-pie"] {
            println!("cargo:rustc-link-arg-bins={arg}");
        }
    }
}
