// ---------------------------------------------------------------------------------------------
// WAS KNOWN-RED AND ALLOWLISTED; NOW GREEN, AND THE ALLOWLIST ENTRY IS GONE. This driver was added
// at `1aeeaba` as a runtime witness for an OPEN row and was expected to fail. When the fix landed it
// went green, `known_under.sh`'s ratchet printed `✗ STALE ALLOWLIST ENTRY` and failed the oracle,
// and the entry was removed in the same commit as the fix — which is the entire point of the
// ratchet: a suppression that outlives its defect absorbs the next regression here silently and
// forever (SOUNDNESS R102). So a red here now is a NEW finding and must be read as one.
// ---------------------------------------------------------------------------------------------
// R99 SHAPE 2, CLOSED — Pass A's decl indexes could not see `mod_aliases` (a crate-wide fact that
// does not exist until every file is walked), so a struct FIELD typed through a module alias was
// unresolved and the method that USES it was omitted. `alias_expand_decls` re-expands at the merge.
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
