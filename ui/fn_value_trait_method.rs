// A TRAIT METHOD named as a fn VALUE resolved to the trait DECLARATION, not to the impl that runs.
//
// `<S as T>::run` types as a `FnDef` whose `DefId` is the TRAIT's item. That `DefId` is local and an
// `AssocFn`, so the old "local + Fn/AssocFn ⇒ resolvable target" test said yes and edged to a body that
// does not exist: `register(<S as T>::run)` read silently PURE however effectful `S::run` was — while the
// identical `<S as T>::run()` CALL one line away was resolved correctly, because the call path already
// carries the rule this one broke ("NEVER edge to the bodyless trait method"). Two routes answering
// "which body runs" must not disagree.
//
// Three shapes, three distinct failure modes:
//   - no default body at all         -> the caller inherited NOTHING (silent pure);
//   - a PURE default the impl OVERRIDES -> the caller was charged the DEFAULT's effects, a wrong answer
//     that still prints a plausible one;
//   - a LOCAL impl of a NON-LOCAL trait -> `is_local` was tested on the TRAIT item, so the edge was
//     dropped on the floor before resolution ever happened.
//
// The fix asks rustc (`Instance::try_resolve`), then falls back to CHA over the trait's local impls when
// `Self` is generic — the same two steps, in the same order, as the call path's devirtualization.
//
// The over-charge controls are the point of the file: a PURE impl reached the same way, and a pure
// default that is genuinely NOT overridden, must both stay pure. A fix that edged to every impl of the
// trait would pass every positive row here and fail those two.
#![allow(unused)]

struct Effectful;
struct Harmless;

// --- no default body ---
trait Run {
    fn run();
}
impl Run for Effectful {
    fn run() {
        let _ = std::fs::read_to_string("/db"); // Fs
    }
}
impl Run for Harmless {
    fn run() {}
}

// --- a PURE default the effectful impl overrides ---
trait Hook {
    fn hook() {} // pure default
}
impl Hook for Effectful {
    fn hook() {
        let _ = std::fs::read_to_string("/db"); // Fs
    }
}
impl Hook for Harmless {} // takes the pure default

// --- a default that IS what runs (nothing overrides it) ---
trait Reporter {
    fn report() {
        let _ = std::env::var("PATH"); // Env
    }
}
impl Reporter for Harmless {}

// --- a LOCAL impl of a NON-LOCAL trait ---
struct Parsed;
impl std::str::FromStr for Parsed {
    type Err = ();
    fn from_str(_: &str) -> Result<Parsed, ()> {
        let _ = std::env::var("PATH"); // Env
        Ok(Parsed)
    }
}

fn register(f: fn()) {
    f() // opaque param -> honest, non-propagating Unknown on `register` itself
}

fn qualified_path() {
    register(<Effectful as Run>::run); // EXPECT Fs
}
fn short_path() {
    register(Effectful::run); // EXPECT Fs — the same target spelled the other way
}
fn overridden_default() {
    register(<Effectful as Hook>::hook); // EXPECT Fs, NOT the pure default's nothing
}
fn generic_self<X: Run>() {
    register(X::run); // Self unpinned -> CHA the local impls -> EXPECT Fs
}
fn local_impl_of_nonlocal_trait() {
    let f = <Parsed as std::str::FromStr>::from_str;
    let _ = f; // EXPECT Env
}
fn default_body_really_runs() {
    register(<Harmless as Reporter>::report); // EXPECT Env — the default IS the body here
}

// --- OVER-CHARGE CONTROLS ---
fn pure_impl_stays_pure() {
    register(Harmless::run); // EXPECT PURE — must not pick up the sibling impl's Fs
}
fn unoverridden_pure_default_stays_pure() {
    register(<Harmless as Hook>::hook); // EXPECT PURE
}

fn main() {
    qualified_path();
    short_path();
    overridden_default();
    generic_self::<Effectful>();
    local_impl_of_nonlocal_trait();
    default_body_really_runs();
    pure_impl_stays_pure();
    unoverridden_pure_default_stays_pure();
}
