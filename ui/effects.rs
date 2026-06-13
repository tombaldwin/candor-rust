// Classifier wires into the report and effects propagate transitively (`*` = via callee).
#![allow(unused)]

fn read_file() -> std::io::Result<String> {
    std::fs::read_to_string("/etc/hostname") // Fs
}

fn run_cmd() {
    let _ = std::process::Command::new("true").status(); // Exec ("true" is an unknown head → bare cliff)
}

fn run_curl() {
    // Spec §4 ⟨0.5⟩: a literal subprocess head refines the cliff — `curl` adds Net, Exec stays.
    let _ = std::process::Command::new("curl").arg("https://x").status(); // Exec + Net
}

fn read_env() -> Result<String, std::env::VarError> {
    std::env::var("HOME") // Env
}

fn caller() {
    let _ = read_file(); // -> Fs* (via callee)
    run_cmd(); // -> Exec*
}

fn main() {}
