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
//
// FINDING C — the union-fabrication guard under MULTI-SITE, MULTI-EFFECT pressure (stronger than B,
// which has one effect + one effectful site). One HOF `apply` is passed callbacks with DISTINCT effects
// from three sites: `site_fs` (Fs), `site_env` (Env), `site_pure` (pure). Each site must inherit ONLY
// its own callback's effect — `site_fs` = { Fs }, `site_env` = { Env } (NOT { Fs, Env }), `site_pure` =
// PURE. A regression that unions callback effects onto `apply` would leak { Fs, Env } to ALL three sites
// (and `apply` itself); this fixture catches that cross-site contamination the single-effect B cannot.
#![allow(unused)]

fn fs_helper() {
    let _ = std::fs::read_to_string("/db"); // Fs
}

// --- Finding A ---
fn just_address() -> usize {
    fs_helper as *const () as usize // cast away from callability -> PURE (no Fs)
}
// The "stays callable" half of Finding A's claim (`f as fn()`) had NO fixture at all — every existing
// case either casts AWAY (just_address) or passes the fn by name without a `Cast` node at all
// (passes_cb: an implicit FnDef->fn() coercion at an argument position never visits the `ExprKind::Cast`
// arm of the cast_away match). An EXPLICIT `as fn()` cast is the one shape that actually exercises the
// match's `TyKind::FnPtr` arm — guard-deletion confirms it's load-bearing: dropping `FnPtr` from that
// match silently drops this edge (Fs vanishes) while every other fixture in this file stays green.
fn keeps_via_fn_ptr_cast() -> fn() {
    fs_helper as fn() // explicit cast to a callable fn-pointer type -> Fs still flows (not cast away)
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

// --- Finding C — multi-site, multi-effect cross-contamination guard ---
fn cb_fs() {
    let _ = std::fs::read_to_string("/db"); // Fs
}
fn cb_env() {
    let _ = std::env::var("PATH"); // Env
}
fn cb_pure() {} // PURE
fn apply(g: fn()) {
    g() // opaque param -> non-propagating Unknown on `apply`
}
fn site_fs() {
    apply(cb_fs); // EXPECT { Fs } only
}
fn site_env() {
    apply(cb_env); // EXPECT { Env } only (must NOT pick up Fs from the sibling site)
}
fn site_pure() {
    apply(cb_pure); // EXPECT PURE (must NOT pick up Fs or Env)
}

fn main() {}
