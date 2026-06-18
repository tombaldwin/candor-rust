// CALIBRATED-Exec (std) probe: std::process spawns echo with a marker arg; the kernel shows execve.
fn run() {
    // marker: candor-oracle-exec-std
    let _ = std::process::Command::new("/bin/echo").arg("candor-oracle-exec-std").status();
}
fn main() { run(); }
