// Dispatch over a trait PROVIDED (default) method — precision AND soundness, across all three shapes:
//
//   * INHERITED (Plain doesn't override): the call resolves to the default body, whose Clock MUST be
//     attributed. This is the Rust analog of the JVM port's "inherited-concrete dispatch" fix — but
//     rustc's `Instance::try_resolve` lands on the inherited body for free (no hand-rolled supertype
//     walk). A regression here would be a false `Unknown` that ALSO masks the effect.
//
//   * OVERRIDDEN (Heavy replaces run with a pure-of-Clock, Fs-only body): on a CONCRETE receiver the
//     default body provably never runs, so it must NOT be attributed — `use_overridden` is `Fs` only,
//     NOT `{Clock, Fs}`. Previously the unconditional base edge to the trait method (the default body)
//     fired alongside the devirtualized override edge, double-counting the default's Clock — a
//     confident false positive. The base edge is now suppressed when devirt proves a concrete override.
//
//   * DYNAMIC (`&dyn Job`): the runtime impl is unknown, so CHA over-approximates to every impl —
//     `{Clock, Fs}` is the sound answer (the default OR the override could run).
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

struct Heavy;
impl Job for Heavy {
    fn run(&self) {
        let _ = std::fs::read("/tmp/x"); // Fs — overrides the default; no Clock
    }
    fn id(&self) -> u32 {
        2
    }
}

fn use_inherited(p: &Plain) {
    p.run(); // -> inherited default body -> Clock* (attributed, NOT Unknown)
}

fn use_overridden(q: &Heavy) {
    q.run(); // -> Heavy::run override -> Fs* ONLY (the default's Clock must NOT leak in)
}

fn use_dyn(j: &dyn Job) {
    j.run(); // -> CHA over both impls -> { Clock*, Fs* } (sound over-approximation)
}

fn main() {
    use_inherited(&Plain);
    use_overridden(&Heavy);
    use_dyn(&Heavy);
}
