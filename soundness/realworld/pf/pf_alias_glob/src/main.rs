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
// R99 OPEN SHAPE 1 — EXPECTED TO FAIL AT HEAD, and that is the point of the driver: it turns a
// known-open row into something the syscall oracle keeps honest, so a regression cannot be silent.
// main -> body -> put -> glb::write, where `glb` GLOB-re-exports std::fs.
// PAIRED CONTROL: pf_alias_glob_ctl — the same submodule re-exporting `write` BY NAME (R99 mech 1,
// closed by b00956b). One variable: glob vs named. The fully-direct arm is pf_alias_use_ctl.
mod glb;
fn put() {
    eprintln!("CFE put");
    let _ = glb::write("/tmp/pf-aliasglob-9271", b"x");
    eprintln!("CFX put");
}
fn body() { eprintln!("CFE body"); put(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
