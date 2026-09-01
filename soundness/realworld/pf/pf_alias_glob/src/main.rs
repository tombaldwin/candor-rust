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
