// HONESTY probe (FFI/extern boundary): call libc::open directly. Kernel shows open(candor-mk-ffi). candor must
// DISCLOSE Unknown for the extern/FFI call, never read silent-pure. marker: candor-mk-ffi
fn raw_open() {
    let path = b"/tmp/candor-mk-ffi\0";
    let _fd = unsafe { libc::open(path.as_ptr() as *const libc::c_char, libc::O_CREAT | libc::O_WRONLY, 0o644) };
    if _fd >= 0 { unsafe { libc::close(_fd); } }
}
fn main() { raw_open(); }
