// Coverage for `is_dyn_receiver`'s OPAQUE-ALIAS branch (src/lib.rs): a LOCAL factory returning
// `impl Iterator` whose hidden type is itself `Box<dyn Iterator>` must still resolve `.next()` via
// the dyn path so an Unknown effect PROPAGATES to callers — the exact shape that silently dropped an
// effect on the `which` crate (`all_results().and_then(|mut i| i.next())`) before this branch
// existed. That fix landed with no fixture pinning it: a guard-deletion audit (2026-08-30) found
// `cargo test --workspace` fully green with the branch removed, and reproduced the ORIGINAL failure
// mode directly — with the branch deleted, `use_it` still self-reports `Unknown` (the direct call is
// still visibly unresolved) but that Unknown STOPS PROPAGATING to `main`, which drops out of the
// report entirely, i.e. reads as fully PURE despite transitively reaching unresolved dispatch. `main`
// carrying `Unknown*` below is the assertion that discriminates the fix from its absence.
#![allow(unused)]

struct EffIter;
impl Iterator for EffIter {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        let _ = std::time::SystemTime::now(); // Clock
        None
    }
}

fn make_iter() -> impl Iterator<Item = i32> {
    Box::new(EffIter) as Box<dyn Iterator<Item = i32>>
}

fn use_it() {
    let mut it = make_iter();
    it.next();
}

fn main() {
    use_it();
}
