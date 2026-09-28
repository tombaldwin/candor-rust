//! CALIBRATION — PLANTED POSITIVE.
//!
//! One function per open drop-glue row, each the row's own RE-VERIFIED cell, on a type `H` that has
//! a local `impl Drop`. The probe MUST find a TIGHT hit for every row here. A probe that has never
//! been shown to fire is not evidence.
//!
//! This crate compiles. `reach.sh` runs `cargo check` on it, so the fixtures cannot rot into
//! something that only looks like Rust.
#![allow(dead_code, unused_variables, clippy::all)]

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct H(pub u32);
impl H {
    pub fn new() -> H {
        H(0)
    }
}
impl Drop for H {
    fn drop(&mut self) {}
}

pub fn fallible(n: u32) -> Result<u32, ()> {
    if n == 0 {
        Err(())
    } else {
        Ok(n)
    }
}

// ---- R189: two exits, one an explicit `return`, each escaping the SAME leaf under a different name.
pub fn r189(n: u32) -> Result<H, ()> {
    let a = H::new();
    let b = H::new();
    if n == 0 {
        return Ok(a);
    }
    Ok(b)
}

// ---- R198: a `try_fold` accumulator whose closure tail is a bare `Ok(acc)`.
pub fn r198(v: &[u32]) -> Result<Vec<H>, ()> {
    v.iter().try_fold(Vec::new(), |mut acc, i| {
        fallible(*i)?;
        acc.push(H::new());
        Ok(acc)
    })
}

// ---- R200 (decl side): a by-value parameter that is a CONTAINER over the drop leaf.
pub fn eat_opt(h: Option<H>, n: u32) -> Result<u32, ()> {
    Ok(n)
}

// ---- R200 (call site): the wrapper argument.
pub fn r200c(n: u32) -> Result<u32, ()> {
    eat_opt(Some(H::new()), n)
}

// ---- R201: a hand-written future OWNING a construction, awaited, output used.
pub struct FutH {
    pub h: Option<H>,
    pub n: u32,
}
impl Future for FutH {
    type Output = u32;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
        let n = self.n;
        self.h.take();
        Poll::Ready(n)
    }
}
pub async fn r201(n: u32) -> Result<u32, ()> {
    let v = FutH { h: Some(H::new()), n }.await;
    Ok(v)
}
/// The RVALUE-RECEIVER spelling: `H::new()` is moved into the future, which `.await` consumes, so
/// the `H` dies in THIS frame. Paired with `r201_receiver_place` in `neg-shape`, which differs in
/// exactly one thing — the receiver is a binding, so it is borrowed.
impl H {
    pub fn fut(self) -> FutH {
        FutH { h: Some(self), n: 0 }
    }
}
pub async fn r201_receiver_rvalue() -> Result<u32, ()> {
    let v = H::new().fut().await;
    Ok(v)
}

// ---- R209(a): a `?` inside MACRO TOKENS, and it is the body's sole exit.
macro_rules! idm {
    ($e:expr) => {
        $e
    };
}
pub fn r209a(n: u32) -> Result<u32, ()> {
    let g = H::new();
    idm!(fallible(n)?);
    Ok(n)
}

// ---- R297: assignment THROUGH A PLACE. No construction site in this body at all.
pub fn r297(slot: &mut H, v: H) {
    *slot = v;
}

// ---- R300: a `fn` ITEM in a body, invoked here, whose leaf ALSO escapes from the enclosing body.
pub fn r300(n: u32) -> Result<H, ()> {
    fn inner() -> H {
        H::new()
    }
    let x = inner();
    Ok(H::new())
}

// ---- R323: two bindings of ONE closure name, both invoked and discarded.
pub fn r323() {
    let c = || H::new();
    let _ = c();
    let c = || H::new();
    let _ = c();
}
