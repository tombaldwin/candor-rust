// Regression: an INHERITED method — a trait default/provided method a type does not override — is a
// RESOLVED call, NOT an `Unknown`. `Plain` inherits `Job::run`'s default body, so calling `run()` on
// a `Plain` must attribute that body's `Clock`, exactly as if it were written on `Plain`. This is the
// Rust analog of the JVM port's "inherited-concrete dispatch" fix: there, hand-rolled CHA scanned
// only a type's own/sub impls and missed the inherited body — a false `Unknown` that ALSO masked the
// real effect (an unresolved dispatch stops propagation). Here rustc's `Instance::try_resolve`
// (devirtualize) lands on the inherited default body for free, so the effect is attributed and there
// is no `Unknown` — no hand-rolled supertype walk needed. Guards against a regression that would let
// inherited dispatch fall back to a (masking) Unknown.
#![allow(unused)]

trait Job {
    fn run(&self) {
        let _ = std::time::SystemTime::now(); // Clock — in the DEFAULT (provided) body
    }
    fn id(&self) -> u32;
}

struct Plain;
impl Job for Plain {
    fn id(&self) -> u32 {
        1
    } // does NOT override run() -> inherits the default body -> Clock
}

fn use_inherited(p: &Plain) {
    p.run(); // resolves to the inherited Job::run default body -> Clock* (attributed, NOT Unknown)
}

fn main() {
    use_inherited(&Plain);
}
