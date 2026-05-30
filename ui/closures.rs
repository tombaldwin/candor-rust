// Effects inside an inline closure are charged to the enclosing NAMED function
// (enclosing_named_fn walks out of closures), not lost or attributed to the anonymous closure.
#![allow(unused)]

fn in_closure() {
    ["/a", "/b"].iter().for_each(|p| {
        let _ = std::fs::read_to_string(p); // Fs, inside the closure -> charged to in_closure
    });
}

fn in_nested_closure() {
    let _ = Some(1).map(|_| std::process::Command::new("x").status()); // Exec inside closure
}

fn main() {}
