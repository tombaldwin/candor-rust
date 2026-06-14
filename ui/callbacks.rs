// Per-call-site closure flow + fn-value-cast precision (regression tests).
//
// FINDING A — a `FnDef` cast AWAY from callability (`f as usize` / `f as *const ()`) is NOT a call,
// so the casting fn must NOT inherit the callee's effects. A cast that STAYS callable (`f as fn()`),
// or passing the fn by name to a combinator, DOES keep the effect (the callback edge is deliberate).
//
// FINDING B — a generic HOF's callback effects flow PER CALL SITE to the caller that passed the
// callback, never unioned onto the HOF and leaked to EVERY caller. `handler_io` (passed an effectful
// callback) is `Fs`; `domain_calc` (passed only a PURE callback) stays PURE; the HOF itself carries an
// honest, NON-PROPAGATING `Unknown` (it invokes an opaque param) that does not re-pollute either.
#![allow(unused)]

fn fs_helper() {
    let _ = std::fs::read_to_string("/db"); // Fs
}

// --- Finding A ---
fn just_address() -> usize {
    fs_helper as *const () as usize // cast away from callability -> PURE (no Fs)
}
fn passes_cb() {
    register(fs_helper); // passed by name to a HOF -> Fs flows
}
fn register(f: fn()) {
    f();
}

// --- Finding B ---
fn with_retry<F: Fn() -> i32>(f: F) -> i32 {
    f() // invokes an opaque param -> honest, non-propagating Unknown on `with_retry`
}
fn fetch_remote() -> i32 {
    let _ = std::fs::read_to_string("/db");
    1
} // Fs
fn compute_local() -> i32 {
    4
} // PURE
fn handler_io() -> i32 {
    with_retry(fetch_remote) // EXPECT Fs (per-site edge handler_io -> fetch_remote)
}
fn domain_calc() -> i32 {
    with_retry(compute_local) // EXPECT PURE (must NOT inherit the sibling's Fs, nor the HOF's Unknown)
}

fn main() {}
