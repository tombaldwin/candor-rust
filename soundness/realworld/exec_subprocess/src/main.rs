// HONESTY probe (UNCALIBRATED Exec): the `subprocess` crate spawns echo with a marker arg. The kernel
// shows execve. candor doesn't model subprocess, so it must DISCLOSE (invisible/Unknown) — silent-pure
// here is a real κ-ledger disclosure hole.
fn run() {
    // marker: candor-oracle-subprocess
    let _ = subprocess::Exec::cmd("/bin/echo").arg("candor-oracle-subprocess").join();
}
fn main() { run(); }
