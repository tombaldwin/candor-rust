#[macro_export] macro_rules! thread_local { ($($t:tt)*) => { pub fn go_h9() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
