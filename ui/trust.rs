// Trust contract (SPEC §4 / PRINCIPLES #1): candor marks dispatch it CANNOT resolve as `Unknown`,
// never silently pure — except conventionally-pure std traits, which would flood the report.
#![allow(unused)]

// (a) a call through a closure / `impl Fn` parameter — body invisible -> Unknown.
fn via_callback(f: impl Fn()) {
    f();
}

// (b) a call through a fn-pointer -> Unknown.
fn via_fn_ptr(f: fn()) {
    f();
}

// (c) a `dyn` call over a NON-LOCAL, NON-exempt trait (Iterator) -> Unknown.
fn via_dyn_iter(it: &mut dyn Iterator<Item = u8>) {
    let _ = it.next();
}

// (d) ...but formatting a `dyn std::error::Error` is the pure-std-trait exemption -> NOT Unknown.
fn format_error(e: &dyn std::error::Error) -> String {
    e.to_string()
}

fn main() {}
