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
// R99 OPEN SHAPE 2 — EXPECTED TO FAIL AT HEAD. Pass A's decl indexes do not see `mod_aliases`, so a
// struct FIELD typed through the module alias is unresolved and the method that USES it is omitted.
// The discriminating function is `run_aliased`: it is the only one whose receiver type is spelled
// through the alias. `build_cmd` is bracketed but has RETURNED before the exec, so it is not on the
// stack; `main`/`spawn_it` legitimately carry Exec through the constructor edge, which is why the
// FAIL, when it comes, names `run_aliased` alone.
// PAIRED CONTROL: pf_alias_field_ctl — same struct, field typed std::process::Command.
mod facade;
struct Holder { c: facade::Command }
impl Holder {
    fn run_aliased(&mut self) {
        eprintln!("CFE run_aliased");
        let _ = self.c.status();
        eprintln!("CFX run_aliased");
    }
}
fn build_cmd() -> std::process::Command {
    eprintln!("CFE build_cmd");
    let mut c = std::process::Command::new("/bin/sh");
    c.arg("-c").arg("echo x > /tmp/pf-alfield-9271");
    eprintln!("CFX build_cmd");
    c
}
fn spawn_it() {
    eprintln!("CFE spawn_it");
    let mut h = Holder { c: build_cmd() };
    h.run_aliased();
    eprintln!("CFX spawn_it");
}
fn main() { eprintln!("CFE main"); spawn_it(); eprintln!("CFX main"); }
