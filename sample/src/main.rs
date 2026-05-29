//! Conformance sample for effect_audit, written in the capability discipline.
//!
//! Run the conformance check with:
//!   EFFECT_AUDIT_STRICT=1 cargo dylint --lib-path <…/libeffect_audit@…dylib>
//!
//! A function declares the effects it may perform by taking the matching capability
//! token as a parameter (`&Fs`, `&Env`, …). The checker flags any function whose
//! *inferred* effect set exceeds what its parameters *declare*.

mod caps {
    //! Unforgeable capability tokens. Each has a PRIVATE field, so it cannot be
    //! constructed outside this module — code elsewhere can only *receive* one as a
    //! parameter. `acquire()` is the single minting point; in a real program it is
    //! called exactly once at the entry point, and every effect is reachable only by
    //! threading these tokens down from there. This is the type system enforcing the
    //! spec's "no ambient capability" pillar — `&Fs` can never be forged.
    pub struct Fs(());
    pub struct Env(());
    pub struct Clock(());
    pub struct Exec(());

    /// The sole place capabilities enter the program.
    pub fn acquire() -> (Fs, Env, Clock, Exec) {
        (Fs(()), Env(()), Clock(()), Exec(()))
    }
}
use caps::{Clock, Env, Exec, Fs};

// ── CONFORMANT: declares Fs, performs Fs. ─────────────────────────────────────
fn read_config(_fs: &Fs, path: &str) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

// ── VIOLATION (direct): performs Fs but declares no capability. ───────────────
fn sneaky_read(path: &str) -> std::io::Result<String> {
    std::fs::read_to_string(path) // inferred {Fs}, declared {} -> undeclared Fs
}

// ── VIOLATION (helper): a "pure-looking" helper that secretly reads the clock. ─
fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0) // inferred {Clock}, declared {} -> undeclared Clock
}

// ── VIOLATION (transitive): declares Fs, but Clock leaks in via now_ms(). ──────
fn stamp(_fs: &Fs, path: &str) {
    let t = now_ms(); // pulls Clock in transitively
    let _ = std::fs::write(path, t.to_string()); // declares Fs; inferred {Clock, Fs}
}

// ── OVER-DECLARATION: declares Clock but never reads it. ──────────────────────
fn greet(_clock: &Clock, name: &str) -> String {
    format!("hello {name}") // declared {Clock}, inferred {} -> over-declared
}

// ── CONFORMANT (transitive): declares Exec + Fs, performs both. ───────────────
fn run_tool(_exec: &Exec, fs: &Fs) {
    let _ = std::process::Command::new("ls").status(); // Exec (direct)
    let _ = read_config(fs, "settings"); // Fs (via callee, fs threaded through)
}

// ── CONFORMANT: declares Env, performs Env. ───────────────────────────────────
fn home(_env: &Env) -> Option<String> {
    std::env::var("HOME").ok()
}

fn main() {
    // Capabilities are minted exactly once, here, then threaded down by reference.
    let (fs, env, clock, exec) = caps::acquire();

    let _ = sneaky_read("x");
    let _ = now_ms();
    stamp(&fs, "out");
    let _ = greet(&clock, "world");
    run_tool(&exec, &fs);
    let _ = home(&env);

    // The point of unforgeable tokens: the line below does NOT compile, because `Fs`'s
    // field is private to `caps` — no code outside that module can mint a capability.
    //     let forged = Fs(());   // error[E0603]: tuple struct constructor `Fs` is private
}
