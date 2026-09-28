//! CALIBRATION — PLANTED NEGATIVE (a) — NO DESTRUCTOR ANYWHERE.
//!
//! Byte-identical to `pos/src/lib.rs` except that `impl Drop for H` is deleted. Nothing else moves.
//! Every row must therefore report LOOSE (the shape is still there) and TIGHT **0** (no leaf in the
//! crate has a destructor, so no fix to the drop/escape model can make a row follow).
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
