// R6 probe: compound-assignment operators (`+=` etc.) desugar to an AssignOp trait call
// (`AddAssign::add_assign`, `SubAssign::sub_assign`, …). An effectful local impl reached only through
// the compound-assign sugar must be charged (no silent under-report); a std `+=` stays pure.
#![allow(unused)]
use std::ops::{AddAssign, SubAssign};

fn sink() {
    let _ = std::fs::read_to_string("/etc/hostname"); // Fs
}

struct N(u8);
impl AddAssign for N {
    fn add_assign(&mut self, _rhs: N) {
        sink();
    }
}
impl SubAssign for N {
    fn sub_assign(&mut self, _rhs: N) {
        sink();
    }
}

fn via_add_assign(mut a: N, b: N) {
    a += b; // Fs (via N::add_assign)
}
fn via_sub_assign(mut a: N, b: N) {
    a -= b; // Fs (via N::sub_assign)
}

// pure control: std compound-assign on a primitive
fn pure_add_assign(mut a: i32, b: i32) {
    a += b; // pure
}

fn main() {}
