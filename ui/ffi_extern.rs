// R60 (SOUNDNESS.md): a call to a fn declared in a LOCAL `extern "C" { .. }` block has an unknowable
// body (it lives in another language), so it must disclose `Unknown` — never silent-pure. Before the
// fix this vanished with ZERO disclosure even though the engine's own callgraph proved it visited the
// call: both `record_resolved_call`'s honest routes (classify-by-crate-name, and the floor-to-invisible
// disclosure) are gated on the callee being NON-local, and a local `extern` declaration IS local.
#![allow(unused)]

extern "C" {
    fn system(cmd: *const std::os::raw::c_char) -> i32;
}

// #[link(name = "c")] is the bindgen-generated shape; the bare block above and this one must behave
// identically — `is_foreign_item` doesn't key on the attribute.
#[link(name = "c")]
extern "C" {
    fn getpid() -> i32;
}

fn run_cmd() {
    let c = std::ffi::CString::new("id").unwrap();
    unsafe {
        system(c.as_ptr()); // Unknown (native:extern fn) — unknowable FFI boundary
    }
}

fn get_pid() -> i32 {
    unsafe { getpid() } // Unknown (native:extern fn) too — `#[link]` changes nothing
}

// CONTROL: a caller inherits the Unknown transitively, exactly like any other unresolvable call.
fn caller() {
    run_cmd(); // -> Unknown*
}

// CONTROL: a genuinely pure fn with no extern call must never be flagged (no fabrication).
fn pure_math(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {}
