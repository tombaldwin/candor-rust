// CONTROL for pf_alias_field: the field's type is spelled directly. One variable — the module alias.
struct Holder { c: std::process::Command }
impl Holder {
    fn run_plain(&mut self) {
        eprintln!("CFE run_plain");
        let _ = self.c.status();
        eprintln!("CFX run_plain");
    }
}
fn build_cmd() -> std::process::Command {
    eprintln!("CFE build_cmd");
    let mut c = std::process::Command::new("/bin/sh");
    c.arg("-c").arg("echo x > /tmp/pf-alfieldc-9271");
    eprintln!("CFX build_cmd");
    c
}
fn spawn_it() {
    eprintln!("CFE spawn_it");
    let mut h = Holder { c: build_cmd() };
    h.run_plain();
    eprintln!("CFX spawn_it");
}
fn main() { eprintln!("CFE main"); spawn_it(); eprintln!("CFX main"); }
