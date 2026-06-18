// Generated-code + named-initializer seams. (1) A `#[derive(Clone)]` impl is COMPILER-GENERATED code
// that calls each field's clone — if a field's Clone is effectful, the derived clone must carry it (the
// generated body must be analyzed, like any other — the thread_local lesson was that generated code can
// be orphaned). (2) `Once::call_once`/`OnceLock::get_or_init` given a NAMED effectful fn must charge the
// caller via the callback-value edge. Pure controls in each family must stay pure.
#![allow(unused)]
use std::sync::{Once, OnceLock};

fn sink() {
    let _ = std::fs::read_to_string("/etc/hostname"); // Fs
}

// (1) derived Clone over an effectful-Clone field
struct Loud;
impl Clone for Loud {
    fn clone(&self) -> Loud {
        sink();
        Loud
    }
}
#[derive(Clone)]
struct HasLoud {
    w: Loud,
}
fn via_derived_clone(s: &HasLoud) -> HasLoud {
    s.clone() // Fs (derived HasLoud::clone calls Loud::clone)
}

// pure control: derived clone over a pure field
#[derive(Clone)]
struct HasPure {
    n: u8,
}
fn pure_derived_clone(s: &HasPure) -> HasPure {
    s.clone() // pure
}

// (2) Once::call_once with a NAMED effectful fn
static ONCE: Once = Once::new();
fn once_init() {
    sink();
}
fn via_call_once() {
    ONCE.call_once(once_init); // Fs (named init runs once)
}

// (3) OnceLock::get_or_init with a NAMED effectful fn
static CELL: OnceLock<u8> = OnceLock::new();
fn cell_init() -> u8 {
    sink();
    0
}
fn via_get_or_init() {
    let _ = CELL.get_or_init(cell_init); // Fs (named init runs on first get)
}

// pure control: get_or_init with a pure init
static CELL_PURE: OnceLock<u8> = OnceLock::new();
fn cell_init_pure() -> u8 {
    0
}
fn via_get_or_init_pure() {
    let _ = CELL_PURE.get_or_init(cell_init_pure); // pure
}

fn main() {}
