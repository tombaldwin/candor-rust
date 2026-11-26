pub struct S;
impl S { pub const C: () = { #[cfg(windows)] let _w: () = { #[macro_export] macro_rules! mk { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }; }; }
#[macro_export] macro_rules! mknop { () => { () } }
