// ---------------------------------------------------------------------------------------------
// WAS KNOWN-RED AND ALLOWLISTED; NOW GREEN, AND THE ALLOWLIST ENTRY IS GONE. This driver was added
// at `1aeeaba` as a runtime witness for an OPEN row and was expected to fail. When the fix landed it
// went green, `known_under.sh`'s ratchet printed `✗ STALE ALLOWLIST ENTRY` and failed the oracle,
// and the entry was removed in the same commit as the fix — which is the entire point of the
// ratchet: a suppression that outlives its defect absorbs the next regression here silently and
// forever (SOUNDNESS R102). So a red here now is a NEW finding and must be read as one.
// ---------------------------------------------------------------------------------------------
// R99 SHAPE 1, CLOSED — the driver that was red until `collect_module_glob` recorded the module's
// single external glob (with the names the module declares itself, which shadow it). It turns a row
// that WAS open into something the syscall oracle keeps honest, so a regression cannot be silent.
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
