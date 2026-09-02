// ---------------------------------------------------------------------------------------------
// PREDICTED GREEN. This driver was added at `1aeeaba` as a deliberately-RED runtime witness for
// SOUNDNESS R101 and was allowlisted in `soundness/realworld/known_under.sh`. R101's OnceLock half is
// FIXED, the driver PASSES, the ratchet printed `✗ STALE ALLOWLIST ENTRY` on the first run after the
// fix, and the entry left in the same commit — which is what the ratchet exists to force. The
// allowlist is now EMPTY.
//
// SO A RED HERE IS A NEW FINDING, and nothing will absorb it. Read it as a regression in the
// callable-static route, not as a known gap: the shape below is a callback installed from OUTSIDE
// through interior mutability and invoked later, and the engine must say `Unknown` for `fire`.
//
// THE DISCRIMINATOR IS `fire`. `install` has RETURNED before the effect and is not on the stack;
// `main` legitimately reaches the closure body through the `install()` edge, so `main` carrying `Fs`
// is not what this driver tests. Pre-fix `fire` was ABSENT from `functions[]` entirely — silent on
// `deny Fs`, `deny Unknown`, `deny Fs Unknown` and scoped `deny Fs fire`.
//
// PAIRED CONTROL: pf_oncelock_cb_ctl — the SAME opaque callable reached through a fn-typed parameter,
// the sibling path that always answered correctly. It passes BY DISCLOSURE (`inferred: ['Unknown']`),
// and the fix converged this driver onto that exact answer rather than writing a second rule. The
// pair still separates "blind to interior mutability" from "blind to opaque callables": if this one
// goes red while the control stays green, the static route regressed and the parameter route did not.
// ---------------------------------------------------------------------------------------------
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
