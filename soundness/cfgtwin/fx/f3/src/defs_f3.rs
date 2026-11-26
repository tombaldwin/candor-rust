macro_rules! mkf_f3 { ($n:ident) => { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
