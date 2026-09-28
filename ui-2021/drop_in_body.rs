// SOUNDNESS R756: the drop happens in a body that is NOT the reported function's own.
//
// `mir_spike::drop_edges` walked `tcx.hir_body_owners()` and filtered to `DefKind::Fn | DefKind::AssocFn`.
// A closure, an `async` block and an `async fn`'s coroutine are each their OWN body owner with their own
// MIR — so a `let` inside one of them emits its scope-exit `Drop` terminator in THAT body, and the filter
// meant the deep engine never looked. Measured on a one-line pair with observed drop counts: a closure
// body dropping an effectful guard read **0 violations** under `deny Fs <the enclosing fn>`, while the
// identical body with an explicit `drop(g)` read 1 — the explicit form is rescued by the separate HIR
// `mem::drop` route, which is what made the hole look covered. `candor-scan` got both right.
//
// The sibling fixture `coroutine_drop.rs` is a DIFFERENT question and neither covers the other: there the
// guard is an UPVAR of a closure/coroutine that is itself dropped from a normal `fn` body, so the walker's
// per-`TyKind` arms are what matter and the enclosing body is a plain `fn`. Here the guard is a LOCAL of
// the closure/coroutine body, and the question is whether that body is looked at at all.
//
// GUARD-DELETION, and the halves are independent:
//   * restore the `DefKind::Fn | DefKind::AssocFn` filter → every `in_*` / `escaping_closure_maker` row
//     below goes silent while every `*_stays_pure` control stays green. That is the original defect.
//   * keep the body but key the edge on the closure's own `LocalDefId` instead of
//     `enclosing_reportable_owner` → the same rows go silent AND a `::{closure#0}` unit appears in
//     their place. That is the HALF-FIX. It is why the gate assertions live in `tests/integration.sh`
//     9c-v and are read as a WHOLE violation SET: a name-prefix-scoped `deny` still FIRES under the
//     half-fix (on the closure unit), so the discriminator is the CALLER — `main` is charged only when
//     the edge is attributed to the owner.
//   * widen the filter to every body owner (drop `is_fn_like()`) → `uses_const_and_static` reaches a
//     `Const`/`Static` body, where `is_mir_available` says TRUE and `optimized_mir` ICEs with
//     *"do not use `optimized_mir` for constants"*. That is why the predicate is rustc's own.
#![allow(unused)]

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::net::TcpStream::connect("10.0.0.2:9"); // Net
    }
}

// A REAL destructor that performs nothing: the walker still runs over it, so every `*_stays_pure`
// control below exercises the new attribution path and must find nothing.
struct PureGuard;
impl Drop for PureGuard {
    fn drop(&mut self) {}
}

async fn ready() {}

// ---- a closure BODY ----
fn in_closure_body() {
    let c = || {
        let _g = Guard;
    };
    c(); // Net
}

// The over-approximation, stated: a closure that is never called still charges the fn whose body built
// it. That is not a new policy — it is what the enclosing-item attribution has always done to a CALL in
// the same position, which is the point of the pair below.
fn in_closure_body_uncalled() {
    let c = || {
        let _g = Guard;
    };
    let _ = &c; // Net
}
fn call_in_uncalled_closure() {
    let c = || {
        let _ = std::net::TcpStream::connect("10.0.0.2:9");
    };
    let _ = &c; // Net — true BEFORE R756's fix as well; the drop path now answers identically
}

fn in_nested_closure_body() {
    let c = || {
        let d = || {
            let _g = Guard;
        };
        d();
    };
    c(); // Net
}

// ---- a coroutine BODY ----
async fn in_async_fn() {
    let _g = Guard; // Net
}

// The guard is LIVE ACROSS an await point, so its drop sits on the coroutine's resume AND drop paths.
async fn in_async_fn_across_await() {
    let _g = Guard;
    ready().await; // Net
}

// An `async` block whose future is never polled: the local is never created, so nothing drops at
// runtime. Charged anyway, same direction as `in_closure_body_uncalled`.
fn in_async_block_unpolled() {
    let _fut = async {
        let _g = Guard;
    }; // Net
}

// ---- the ESCAPE case: the closure outlives the fn that built it ----
// The CREATOR is charged, matching where the engine already charges a closure body's calls. The fn that
// invokes the escaped closure cannot see what it was handed, and keeps its own honest `Unknown` — it
// does NOT silently inherit `Net`, and it must not silently read pure either.
fn escaping_closure_maker() -> Box<dyn FnOnce()> {
    Box::new(|| {
        let _g = Guard;
    }) // Net
}
fn escaping_closure_runner(f: Box<dyn FnOnce()>) {
    f(); // Unknown, not Net
}

// ---- a callback body handed to a std combinator ----
fn in_for_each_callback() {
    (0..1).for_each(|_| {
        let _g = Guard;
    }); // Net
}

// ---- OVER-CHARGE CONTROLS: a real but pure destructor in each new body kind ----
fn closure_over_pure_drop_stays_pure() {
    let c = || {
        let _g = PureGuard;
    };
    c(); // PURE
}
async fn async_fn_over_pure_drop_stays_pure() {
    let _g = PureGuard; // PURE
}
// `ManuallyDrop<Guard>` never runs Guard's destructor: charging it through the closure body would be a
// fabrication, exactly as it would through a plain fn body.
fn manually_drop_in_closure_stays_pure() {
    let c = || {
        let _g = std::mem::ManuallyDrop::new(Guard);
    };
    c(); // PURE
}

// ---- the ICE control: body owners `optimized_mir` is NOT defined on ----
const SIZE: usize = 4;
static NAMES: [u8; SIZE] = [0; SIZE];
fn uses_const_and_static() {
    let inline = const { 7u8 };
    let _ = (SIZE, NAMES.len(), inline); // PURE
}

fn main() {
    in_closure_body();
    in_closure_body_uncalled();
    call_in_uncalled_closure();
    in_nested_closure_body();
    in_async_block_unpolled();
    escaping_closure_runner(escaping_closure_maker());
    in_for_each_callback();
    closure_over_pure_drop_stays_pure();
    manually_drop_in_closure_stays_pure();
    uses_const_and_static();
}
