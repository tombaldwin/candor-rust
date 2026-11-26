#[macro_export] macro_rules! dfix_h5b { () => { fn go_h5b(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
