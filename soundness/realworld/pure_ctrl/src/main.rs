// NEGATIVE control: pure computation, no syscall-observable effect. The kernel sees no marker; candor
// must predict NOTHING (no fabrication). Sanity-checks the harness in both directions.
use std::hint::black_box;

fn compute() -> u64 {
    let mut x = 1u64;
    for i in 1..50 {
        x = x.wrapping_mul(i).wrapping_add(7);
    }
    black_box(x)
}

fn main() {
    let _ = compute();
}
