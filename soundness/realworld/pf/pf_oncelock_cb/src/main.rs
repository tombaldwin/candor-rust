// ---------------------------------------------------------------------------------------------
// KNOWN-RED. This driver is EXPECTED to fail at HEAD and two things follow that a reader hitting the
// red needs to know before "fixing" it:
//   * `run_pf.sh` has no KNOWN_UNDER allowlist. Its sibling `soundness/realworld/run.sh` does (see the
//     `KNOWN_UNDER=()` block and its comment: "tracked so the oracle is a clean gate — green on known
//     gaps, red only on NEW findings"). Two oracles, one question, and only one of them answers it.
//   * because of that, `soundness/realworld/recall/disclosure_recall_check.py:117` sees the control
//     pass already red and ABORTS the per-function calibration entirely — so the recall numbers for
//     this oracle stop being produced, not merely reported red. That is an aggregation failure, and
//     it is the reason this file says so here rather than leaving it to be rediscovered.
// ---------------------------------------------------------------------------------------------
// R101 SHAPE — EXPECTED TO FAIL AT HEAD, and deliberately included so the open row has a runtime
// witness. A callback is installed from outside through interior mutability and invoked later:
// the `OnceLock::get()` if-let binder never marks the binding fn-typed, so `fire`'s call resolves
// as a phantom free-fn and disappears — silent on `deny Unknown` and even on bare `pure`.
// The discriminator is `fire`. `install` has RETURNED before the effect and is not on the stack;
// `main` legitimately reaches the closure body through the `install()` edge.
// PAIRED CONTROL: pf_oncelock_cb_ctl — the sibling path that answers the same question CORRECTLY,
// a plain fn-typed parameter, which discloses `Unknown` / unresolved and so passes by disclosure.
use std::sync::OnceLock;
static CB: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
fn install() {
    eprintln!("CFE install");
    let _ = CB.set(Box::new(|| { let _ = std::fs::write("/tmp/pf-oncecb-9271", b"x"); }));
    eprintln!("CFX install");
}
fn fire() {
    eprintln!("CFE fire");
    if let Some(f) = CB.get() { f(); }
    eprintln!("CFX fire");
}
fn main() { eprintln!("CFE main"); install(); fire(); eprintln!("CFX main"); }
