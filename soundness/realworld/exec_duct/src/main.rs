// CALIBRATED-Exec recall probe: duct spawns /bin/echo with a marker arg. The kernel shows the execve;
// candor must predict Exec.
fn run_cmd() {
    // marker: candor-oracle-exec
    let _ = duct::cmd!("/bin/echo", "candor-oracle-exec").stdout_null().run();
}

fn main() {
    run_cmd();
}
