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
//
// The CHA fallback carries its own pair, at the bottom of the file: when `Self` is unpinned, an impl
// that TAKES the default is reachable and its body must be charged (`generic_self_keeps_default`), and
// when every impl overrides it the default is dead and must NOT be (`generic_all_override_no_default`).
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

// --- CHA over an UNPINNED `Self` must also carry the DEFAULT body ---
//
// `cha_targets` reads `impl_item_implementor_ids`, which lists only the methods an impl DEFINES — an
// `impl Sig for SigTaker {}` that inherits the default contributes NOTHING to it. So CHA alone answers
// "every body that can run here" with the OVERRIDERS ONLY, and the default's effects vanish. MEASURED
// before this row existed: `generic_self_keeps_default` reported `Fs` and silently dropped `Exec` —
// the same class of silent under-report this file exists to close, introduced by the fix that closes
// it, in the opposite direction. Two distinguishable effects, one per body, so neither can mask the
// other (a single shared effect would have gone green either way).
trait Sig {
    fn sig() {
        let _ = std::process::Command::new("true").status(); // Exec — the default; `SigTaker` runs it
    }
}
struct SigOverrider;
impl Sig for SigOverrider {
    fn sig() {
        let _ = std::fs::read_to_string("/db"); // Fs
    }
}
struct SigTaker;
impl Sig for SigTaker {} // takes the effectful default

fn generic_self_keeps_default<X: Sig>() {
    register(X::sig); // EXPECT Exec AND Fs — `X` could be either impl
}

// --- OVER-CHARGE CONTROLS ---
fn pure_impl_stays_pure() {
    register(Harmless::run); // EXPECT PURE — must not pick up the sibling impl's Fs
}
fn unoverridden_pure_default_stays_pure() {
    register(<Harmless as Hook>::hook); // EXPECT PURE
}
// The control for the row above: when EVERY impl overrides the default, the default is unreachable and
// charging it would be a fabrication. A fix that unconditionally added the trait item back would pass
// `generic_self_keeps_default` and fail here.
trait AllOverride {
    fn m() {
        let _ = std::process::Command::new("true").status(); // Exec — dead, every impl overrides it
    }
}
struct Ov1;
impl AllOverride for Ov1 {
    fn m() {
        let _ = std::fs::read_to_string("/db"); // Fs
    }
}
struct Ov2;
impl AllOverride for Ov2 {
    fn m() {}
}
fn generic_all_override_no_default<X: AllOverride>() {
    register(X::m); // EXPECT Fs ONLY — never the unreachable default's Exec
}

fn main() {
    qualified_path();
    short_path();
    overridden_default();
    generic_self::<Effectful>();
    local_impl_of_nonlocal_trait();
    default_body_really_runs();
    generic_self_keeps_default::<SigOverrider>();
    generic_all_override_no_default::<Ov1>();
    pure_impl_stays_pure();
    unoverridden_pure_default_stays_pure();
}
