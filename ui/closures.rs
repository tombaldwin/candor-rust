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

fn reads(p: &str) -> usize {
    let _ = std::fs::metadata(p); // Fs (read)
    p.len()
}

// `reads` is handed to a combinator BY NAME (a `FnDef` value, not called here). It's invoked inside
// `Option::map` (std, unseen), so without following the reference its Fs would be lost — now the
// edge propagates it, exactly as an inline closure's body would be charged.
fn passes_fn_by_name() {
    let _ = Some("/a").map(reads); // Fs reaches here through the passed callback
}

fn main() {}
