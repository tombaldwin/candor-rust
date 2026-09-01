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
