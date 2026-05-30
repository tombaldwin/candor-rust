// Classifier wires into the report and effects propagate transitively (`*` = via callee).
#![allow(unused)]

fn read_file() -> std::io::Result<String> {
    std::fs::read_to_string("/etc/hostname") // Fs
}

fn run_cmd() {
    let _ = std::process::Command::new("true").status(); // Exec
}

fn read_env() -> Result<String, std::env::VarError> {
    std::env::var("HOME") // Env
}

fn caller() {
    let _ = read_file(); // -> Fs* (via callee)
    run_cmd(); // -> Exec*
}

fn main() {}
