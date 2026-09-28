//! CALIBRATION — the OWNED-DROP half of `drop_relevant`, with its own negatives.
//!
//! `drop_relevant` is `drop_types ∪ owned_drops.keys()`, and the second half is the one a purely
//! `impl Drop`-keyed probe misses in the OTHER direction: a shape on a `W` that owns an `H` through
//! a field really can move a row, because constructing a `W` runs `H`'s glue.
//!
//! Paired with R718's rule, which is the negative: a BORROWED field is not an owned one, and a type
//! whose only path to a destructor is `&H` must stay TIGHT 0. Each pair below differs in exactly one
//! thing — the `&`.
#![allow(dead_code, unused_variables, clippy::all)]

pub struct H(pub u32);
impl H {
    pub fn new() -> H {
        H(0)
    }
}
impl Drop for H {
    fn drop(&mut self) {}
}

/// OWNS an `H` directly.
pub struct W {
    pub h: H,
}
/// OWNS a `W`, so owns an `H` TRANSITIVELY — the fixpoint's job.
pub struct V {
    pub w: W,
}
/// OWNS `H`s through a container ELEMENT.
pub struct C {
    pub hs: Vec<H>,
}

/// R718 CONTROL: BORROWS an `H`. Its drop runs in whoever owns the referent, never here.
pub struct B<'a> {
    pub h: &'a H,
}
/// R718 CONTROL, container edition.
pub struct CB<'a> {
    pub hs: Vec<&'a H>,
}

// ---- R297 pair: the only difference is that `B` reaches its `H` through a `&`.
pub fn r297_owned(slot: &mut W, v: W) {
    *slot = v;
}
pub fn r297_transitive(slot: &mut V, v: V) {
    *slot = v;
}
pub fn r297_borrowed<'a>(slot: &mut B<'a>, v: B<'a>) {
    *slot = v;
}

// ---- R200d pair: a container PARAMETER over an owning type, and over a borrowing one.
pub fn r200d_container(c: Option<C>, n: u32) -> u32 {
    n
}
pub fn r200d_borrowed(c: Option<CB<'_>>, n: u32) -> u32 {
    n
}
