// Integration coverage for devirtualization (CRITIQUE §9): a concrete-receiver trait call
// resolves to its ONE impl, NOT CHA-expanded to every impl. Calling the pure `Quiet` must NOT
// inherit `Loud`'s Clock; the `dyn` call still soundly picks it up via CHA.
#![allow(unused)]

trait Greet {
    fn greet(&self);
}

struct Loud;
impl Greet for Loud {
    fn greet(&self) {
        let _ = std::time::SystemTime::now(); // Clock
    }
}

struct Quiet;
impl Greet for Quiet {
    fn greet(&self) {} // pure
}

fn use_loud(l: &Loud) {
    l.greet(); // concrete -> devirt -> Loud::greet -> Clock*
}

fn use_quiet(q: &Quiet) {
    q.greet(); // concrete -> devirt -> Quiet::greet -> pure (must NOT show Clock)
}

fn use_dyn(g: &dyn Greet) {
    g.greet(); // dyn -> CHA -> Clock* (sound over-approximation)
}

fn main() {
    use_loud(&Loud);
    use_quiet(&Quiet);
    use_dyn(&Loud);
}
