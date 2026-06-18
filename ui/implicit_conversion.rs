// Implicit-conversion seam (the 6th cross-engine class). An effect reachable through an IMPLICIT
// trait-method invocation — no visible call at the use site — must still be charged. The deep engine
// is type-aware (HIR), so it resolves these natively; the `core::fmt` formatting path is the one true
// hole (the `<T as Display>::fmt` call lives behind a fn-pointer in `Arguments`, not inline) and is
// closed explicitly. This fixture pins BOTH directions: a LOCAL effectful impl reached implicitly must
// carry the effect (no silent under-report — the cardinal sin); a std/pure impl must stay pure (no
// fabrication). Mirrors candor-scan's implicit-conversion fixtures (the engine's syntactic counterpart).
#![allow(unused)]
use std::fmt;
use std::ops::{Add, Deref};

fn sink() {
    let _ = std::fs::read_to_string("/etc/hostname"); // Fs
}

// ---- (1) format!/Display/to_string — the fmt call is behind core::fmt Arguments ----
struct Loud;
impl fmt::Display for Loud {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        sink();
        write!(f, "loud")
    }
}
fn via_format(l: &Loud) -> String {
    format!("{}", l) // Fs (via Loud::fmt)
}
fn via_to_string(l: &Loud) -> String {
    l.to_string() // Fs (via Loud::fmt)
}

// ---- (2) `?` -> From::from (error-path conversion) ----
struct E1;
struct E2;
impl From<E1> for E2 {
    fn from(_e: E1) -> E2 {
        sink();
        E2
    }
}
fn may_fail() -> Result<(), E1> {
    Err(E1)
}
fn via_question() -> Result<(), E2> {
    may_fail()?; // Fs (desugars to E2::from)
    Ok(())
}

// ---- (3) `.into()` -> From::from ----
fn via_into(e: E1) -> E2 {
    e.into() // Fs (via E2::from)
}

// ---- (4) auto-deref -> Deref::deref ----
struct W;
impl Deref for W {
    type Target = str;
    fn deref(&self) -> &str {
        sink();
        "w"
    }
}
fn via_deref(w: &W) -> usize {
    w.len() // Fs (auto-deref through W::deref)
}

// ---- (5) operator overload -> Add::add ----
struct N;
impl Add for N {
    type Output = N;
    fn add(self, _rhs: N) -> N {
        sink();
        N
    }
}
fn via_add(a: N, b: N) -> N {
    a + b // Fs (via N::add)
}

// ---- (6) Drop-glue via a value binding ----
struct G;
impl Drop for G {
    fn drop(&mut self) {
        sink();
    }
}
fn via_drop_binding() {
    let _g = G; // Fs (Drop runs at scope end)
}

// ================= PURE CONTROLS — must stay pure (no fabrication) =================

// std Display (non-local impl) through the same format path.
fn pure_format_std(n: i32) -> String {
    format!("{}", n) // pure
}

// a LOCAL but effect-free Display impl — resolves locally, carries nothing.
struct Quiet;
impl fmt::Display for Quiet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "quiet")
    }
}
fn pure_format_local(q: &Quiet) -> String {
    format!("{}", q) // pure
}

// std operator — must not light up.
fn pure_add(a: i32, b: i32) -> i32 {
    a + b // pure
}

// std cross-type `From`/`.into()` (i32 -> i64) — a real conversion, but non-local, so pure.
fn pure_into(n: i32) -> i64 {
    n.into() // pure
}

// `?` with a genuine std error conversion (ParseIntError -> Box<dyn Error>) — non-local From, pure.
fn pure_question(s: &str) -> Result<i32, Box<dyn std::error::Error>> {
    let n: i32 = s.parse()?; // std From<ParseIntError> for Box<dyn Error>, pure
    Ok(n)
}

// a LOCAL but effect-free Deref impl — resolves locally through auto-deref, carries nothing.
struct PureW;
impl Deref for PureW {
    type Target = str;
    fn deref(&self) -> &str {
        "pure"
    }
}
fn pure_deref_local(w: &PureW) -> usize {
    w.len() // pure (auto-deref through a pure PureW::deref)
}

fn main() {}
