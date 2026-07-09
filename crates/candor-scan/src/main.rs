//! candor-scan — a STABLE-Rust effect scanner. Produces the same candor report JSON the nightly
//! `rustc_private` lint does, but with a purely syntactic backend: walk the crate's `.rs` files, parse
//! with `syn`, build a name-resolved-enough call graph, classify path-qualified calls with the shared
//! `candor-classify`, and propagate transitively. No nightly, no `rustc-dev`, no dylint — `cargo install`
//! and run anywhere.
//!
//! THE PRECISION TRADE vs the lint, stated plainly. This is syntactic, so it sees what's written, not
//! what's resolved. It CATCHES: path-qualified effect calls (`std::fs::read`, `reqwest::Client::execute`,
//! `Command::new`), including `use`-aliased prefixes; intra-crate calls (matched by name) for transitive
//! propagation; macro bodies; and local-type/local-trait method dispatch. It DISCLOSES `Unknown` where it
//! can see the boundary it can't see through: an invoked fn-value/callback (scan.rs), an FFI `extern`
//! call (scan.rs), an untrusted chained dep report (deps.rs). It MISSES (silently, by design — no
//! `Unknown`): effects reached only through external-trait dispatch or an uninferrable receiver,
//! desugared operators/`?`/`.await`/Drop-glue of external types, and cross-crate propagation by stable
//! identity. So on resolution-heavy code it under-reports relative to the lint. Use the lint when you
//! need the soundness contract; use this when you need zero-friction, stable, installable triage.
//! Shares the lint's classifier — one source of truth.
//!
//! CALL RESOLUTION. The local call graph is name-resolved, not type-resolved. A qualified `Type::method`
//! call (or an associated-fn call `RequestBuilder::new()`) is matched on its 2-segment tail, but ONLY when
//! that tail is UNAMBIGUOUS, which keeps same-named methods on different types distinct. A bare
//! free-function call falls back to a unique leaf. A `.method()` call whose receiver type is inferred to a
//! LOCAL type resolves through that type's `Type::method` tail (so `x.go()` reaches a local `S::go`); an
//! external or un-inferrable receiver leaves the bare `.method()` with no definite target, so it
//! under-reports rather than guess (this is what stops `range.start()` — on the external `FloatRange` —
//! linking to a unique local `Clipboard::start`). We deliberately do NOT link a many-way-ambiguous name:
//! on a real crate that would link every `.new()` to all 100+ `*::new` defs and smear one type's effect
//! across the whole graph. Under-reporting an ambiguous edge is the honest failure
//! mode; fabricating one is never ok. The shared resolver is `resolve_target`.
//!
//! Usage:  candor-scan [<crate-dir>] [--out <prefix>] [--json] [--include-tests]
//!   default dir = ".", default prefix = "<dir>/.candor/report"; writes <prefix>.<crate>.scan.json (+ a
//!   callgraph sidecar so `cargo candor callers <fn>` works on the stable report too). `--json` prints
//!   the report to stdout instead. By DEFAULT only the crate's library/binary source is scanned —
//!   `tests/`, `benches/`, `examples/`, `test/`, the root `build.rs` (the Cargo build script — but NOT a
//!   `src/build.rs` source module), and `#[cfg(test)]` modules are skipped, so
//!   the report describes what the CRATE does, not what its harness does (`--include-tests` keeps them).
//!   See eval/calibration for accuracy on 35 real crates.

pub(crate) use std::collections::{BTreeMap, BTreeSet, HashMap};
pub(crate) use std::path::Path;
pub(crate) use candor_report::ReportEntry;
pub(crate) use syn::visit::Visit;

mod model;
mod lang;
mod lazy;
mod deps;
mod collector;
mod decls;
mod cache;
mod config;
mod gate;
mod propagate;
mod scan;

// One flat crate namespace: main.rs was a single 7.9k-line file until 2026-07; the
// split into modules kept every item reachable exactly as before (byte-identical
// reports gated the move), so the modules re-export crate-wide.
pub(crate) use model::*;
pub(crate) use lang::*;
pub(crate) use lazy::*;
pub(crate) use deps::*;
pub(crate) use collector::*;
pub(crate) use decls::*;
pub(crate) use cache::*;
pub(crate) use config::*;
pub(crate) use gate::*;
pub(crate) use propagate::*;
pub(crate) use scan::*;

fn main() {
    // Deeply-nested expressions / method chains recurse in syn's parser AND the single-threaded visitor
    // (Pass B) without depth limits; on the default ~8 MB stack a ~1000-deep file ABORTED the process
    // (SIGABRT) instead of degrading (adversarial review). Run the whole scan on a generous stack — and
    // give rayon's parse workers the same — so a pathological/generated file is handled, not a crash.
    // (A truly adversarial million-deep file still aborts; that's a DoS edge, not real code.)
    const BIG_STACK: usize = 256 * 1024 * 1024;
    rayon::ThreadPoolBuilder::new().stack_size(BIG_STACK).build_global().ok();
    let worker = std::thread::Builder::new()
        .stack_size(BIG_STACK)
        .spawn(scan_main)
        .expect("spawn scan worker thread");
    // scan_main drives its own exit codes via process::exit; a normal return → 0, a panic → 101.
    if worker.join().is_err() {
        std::process::exit(101);
    }
}

#[cfg(test)]
mod tests;
