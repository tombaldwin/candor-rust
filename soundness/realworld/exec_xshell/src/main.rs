// MACRO-FAMILY probe: xshell's `cmd!` macro builds a command, `.run()` spawns it. Same shape as the duct
// under-report (macro result → untypeable receiver → terminal verb's effect dropped). The kernel sees the
// execve; candor must predict Exec or disclose uncertainty.
use xshell::{cmd, Shell};
fn run_sh() {
    let sh = Shell::new().unwrap();
    // marker: candor-oracle-xshell
    let _ = cmd!(sh, "/bin/echo candor-oracle-xshell").quiet().run();
}
fn main() { run_sh(); }
