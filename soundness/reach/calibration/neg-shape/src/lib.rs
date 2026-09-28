//! CALIBRATION — PLANTED NEGATIVE (c) — THE ROWS' OWN CHARGED CONTROLS.
//!
//! Every function here is on `H`, which HAS a local `impl Drop`, so the drop-relevance half of the
//! conjunction is satisfied throughout. What each one fails is the ROW's own discriminator — the
//! single-variable control the row itself records as CHARGED, or the case its remedy deliberately
//! keeps uncharged. All of them must be TIGHT **0**.
//!
//! This fixture exists because drop-relevance alone was not enough. Four of these came from hits
//! this probe scored on the real census and that a hand audit threw out:
//!
//!   * `allocator_api2::repeat` and `anyhow::render` — inline, operand-local constructions at two
//!     exits, which R189's own remedy keeps unioned. -> `r189_inline`, `r189_one_name`
//!   * `async_nats::send_request` — `self.subscribe(..).await?`, where the leaf came from the
//!     method's RETURN type, i.e. the await's output, handed to the caller. -> `r201_output_leaf`
//!   * `async_compression::poll_partial_flush_buf` — `Poll::Ready(Ok(Buffer{..}))`, a wrapper
//!     argument to an enum-variant CONSTRUCTOR, which cannot drop it. -> `r200c_variant_ctor`
//!   * `async_executor::run` and `async_nats::send_request` again — `runner.runnable(..).await`
//!     and `subscriber.next().await`, where the leaf is the method's RECEIVER, a binding the future
//!     BORROWS. -> `r201_receiver_place`, and its one-variable twin `pos::r201_receiver_rvalue`
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

// ---- R189: two exits, but the constructions are INLINE in the exit operands (operand-local).
pub fn r189_inline(n: u32) -> Result<H, ()> {
    if n == 0 {
        return Ok(H::new());
    }
    Ok(H::new())
}
// ---- R189: two exits, but ONE binding — there is no second name for the intersection to separate.
pub fn r189_one_name(n: u32) -> Result<H, ()> {
    let a = H::new();
    if n == 0 {
        return Ok(a);
    }
    Ok(a)
}

// ---- R198: the closure tail has an explicit `Err` arm. The row's own one-variable control.
pub fn r198_err_arm(v: &[u32]) -> Result<Vec<H>, ()> {
    v.iter().try_fold(Vec::new(), |mut acc, i| {
        acc.push(H::new());
        if *i < 99 {
            Ok(acc)
        } else {
            Err(())
        }
    })
}

// ---- R200 (decl): the row's three stated-fine parameter forms.
pub fn r200d_bare(h: H, n: u32) -> u32 {
    n
}
pub fn r200d_boxed(h: Box<H>, n: u32) -> u32 {
    n
}
pub fn r200d_ref(h: &Option<H>, n: u32) -> u32 {
    n
}

// ---- R200 (call site): an enum-variant / tuple-struct CONSTRUCTOR wraps, it does not drop.
pub fn r200c_variant_ctor() -> Poll<Result<Option<H>, ()>> {
    Poll::Ready(Ok(Some(H::new())))
}
// ---- R200 (call site): passed by REFERENCE, so not by value.
pub fn takes_ref(h: &Option<H>, n: u32) -> u32 {
    n
}
pub fn r200c_byref(n: u32) -> u32 {
    takes_ref(&Some(H::new()), n)
}

// ---- R201: the leaf is the method's RETURN type — the await's OUTPUT, which leaves this frame.
pub struct Src;
impl Src {
    pub async fn make(&self) -> H {
        H::new()
    }
}
pub async fn r201_output_leaf(s: &Src) -> Result<u32, ()> {
    let h = s.make().await;
    Ok(0)
}
// ---- R201: an INLINE `async` block really does pass its value out — the arm's proven-safe case.
pub async fn r201_async_block(n: u32) -> Result<u32, ()> {
    let v = async { H::new() }.await;
    Ok(n)
}
// ---- R201: statement position, output discarded. CHARGED today.
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
pub async fn r201_discarded(n: u32) {
    FutH { h: Some(H::new()), n }.await;
}
// ---- R201: the future is BOUND first. The row records this spelling as CHARGED.
pub async fn r201_bound_first(n: u32) -> Result<u32, ()> {
    let f = FutH { h: Some(H::new()), n };
    let v = f.await;
    Ok(v)
}

// ---- R201: the receiver is a PLACE, so the future borrows it rather than owning it. One variable
// apart from `pos::r201_receiver_rvalue`.
pub struct Runner(pub H);
impl Runner {
    pub fn new() -> Runner {
        Runner(H::new())
    }
    pub async fn step(&mut self) -> u32 {
        0
    }
}
pub async fn r201_receiver_place() -> Result<u32, ()> {
    let mut r = Runner::new();
    let v = r.step().await;
    Ok(v)
}

// ---- R209(a): a macro-borne `?`, but a SECOND `?` written outside the macro rescues it.
macro_rules! idm {
    ($e:expr) => {
        $e
    };
}
pub fn r209a_second_q(n: u32) -> Result<u32, ()> {
    let g = H::new();
    idm!(fallible(n)?);
    let m = fallible(n)?;
    Ok(m)
}

// ---- R209(a): a `?` used as a PREFIX sigil, not the try operator. `tracing`'s field capture and
// a `?Sized` bound relaxation. `h2` scored five tight hits on `trace_span!("…", ?stream.id)`.
macro_rules! trace_span {
    ($($t:tt)*) => {
        ()
    };
}
macro_rules! boundy {
    ($($t:tt)*) => {
        ()
    };
}
pub fn r209a_prefix_sigil(n: u32) -> Result<u32, ()> {
    let g = H::new();
    trace_span!("queue_frame", ?n);
    boundy!(T: ?Sized);
    Ok(n)
}

// ---- R201: the construction is in a CLOSURE BODY handed to the awaited callee. It happens
// wherever the callee runs the closure, not in this frame. `async_std::File::open` scored here.
pub async fn spawn_blocking<F: FnOnce() -> H>(f: F) -> H {
    f()
}
pub async fn r201_closure_arg() -> Result<u32, ()> {
    let v = spawn_blocking(move || H::new()).await;
    Ok(0)
}

// ---- R297: a DEFERRED `let mut g;` with no initialiser and no prior assignment — the row's only
// proven-safe exemption.
pub fn r297_deferred(n: u32) -> H {
    let mut g;
    g = H::new();
    g
}

// ---- R300: a body-local `fn`, but NOTHING escapes from the enclosing body. The 2026-09-27
// re-verification: the escape is a second NECESSARY ingredient, and `t01` is charged without it.
pub fn r300_no_escape(n: u32) {
    fn inner() -> H {
        H::new()
    }
    let x = inner();
}

// ---- R323: a SINGLE closure binding. The control that moved ABSENT -> charged with R304's fix.
pub fn r323_single() {
    let c = || H::new();
    let _ = c();
}
