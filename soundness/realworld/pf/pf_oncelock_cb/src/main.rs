// ---------------------------------------------------------------------------------------------
// KNOWN-RED, AND ALLOWLISTED. This driver is EXPECTED to fail at HEAD; it is listed in
// `soundness/realworld/known_under.sh` (KNOWN_UNDER_PERFN) with its SOUNDNESS row, so `run_pf.sh`
// reports it as a KNOWN under-report and stays green. Two things follow for a reader hitting it:
//   * do NOT "fix" the driver. It is a runtime witness for an OPEN row; the red IS the finding.
//   * when the engine defect is fixed, the driver PASSES and the stale-entry ratchet in
//     `known_under.sh` turns the oracle RED until the allowlist entry is deleted in the same change.
//     That is deliberate: an allowlist consulted only in the failing branch is a gate that can never
//     go red again, and would absorb the next regression here silently and forever (SOUNDNESS R102).
// Before R102 this driver's red ABORTED the per-function disclosure-recall calibration outright
// (`recall/disclosure_recall_check.py`), so the recall numbers stopped being produced rather than
// being reported red — the §H aggregation shape, which is why the allowlist exists at all.
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
