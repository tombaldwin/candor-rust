// main -> launch -> spawn -> Command  (Exec fires in the leaf; the CHILD opens the marker path)
fn spawn() {
    eprintln!("CFE spawn");
    let _ = std::process::Command::new("/bin/sh")
        .arg("-c").arg("echo x > /tmp/pf-exec-marker-9271").status();
    eprintln!("CFX spawn");
}
fn launch() { eprintln!("CFE launch"); spawn(); eprintln!("CFX launch"); }
fn main() { eprintln!("CFE main"); launch(); eprintln!("CFX main"); }
