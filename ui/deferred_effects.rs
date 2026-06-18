// Deferred-effect seams: fire-and-forget (spawned closure), lazy-init (LazyLock initializer forced at an
// access site), and deferred-iterator (a custom Iterator whose next() does the effect, run by a for-loop).
// In each, the effect's call site is separated from where it actually fires. The deep engine must still
// charge it (no silent under-report — the cardinal sin); a std/pure analogue of each must stay pure (no
// fabrication). The pure controls also pin per-receiver resolution: a sibling effectful impl in the same
// crate must NOT bleed onto the pure one (the whole-program-CHA union artifact these seams hit elsewhere).
#![allow(unused)]
use std::sync::LazyLock;

fn sink() {
    let _ = std::fs::read_to_string("/etc/hostname"); // Fs
}

// ---- fire-and-forget: the effect runs in a closure handed to a spawn primitive ----
fn via_spawn() {
    std::thread::spawn(|| {
        sink();
    }); // Fs (in the spawned closure)
}
fn pure_spawn() {
    std::thread::spawn(|| {
        let _ = 1 + 1;
    }); // pure
}

// ---- lazy-init: the effect runs in a deferred initializer, forced at the access site ----
static LAZY_EFF: LazyLock<u8> = LazyLock::new(|| {
    sink();
    0u8
});
fn via_force() {
    let _ = *LAZY_EFF; // Fs (first force runs the initializer)
}
static LAZY_PURE: LazyLock<u8> = LazyLock::new(|| 0u8);
fn pure_force() {
    let _ = *LAZY_PURE; // pure
}

// ---- deferred-iterator: a custom Iterator whose next() does the effect, consumed by a for-loop ----
struct EffIter(bool);
impl Iterator for EffIter {
    type Item = ();
    fn next(&mut self) -> Option<()> {
        if self.0 {
            return None;
        }
        self.0 = true;
        sink();
        Some(())
    }
}
fn via_consume() {
    for _ in EffIter(false) {} // Fs (next() does the effect; the loop body is empty)
}
struct PureIter(bool);
impl Iterator for PureIter {
    type Item = ();
    fn next(&mut self) -> Option<()> {
        if self.0 {
            return None;
        }
        self.0 = true;
        Some(())
    }
}
fn pure_consume() {
    for _ in PureIter(false) {} // pure (sibling effectful EffIter must not bleed here)
}

fn main() {}
