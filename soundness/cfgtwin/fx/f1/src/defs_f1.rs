macro_rules! def_f1 { () => { macro_rules! kx_f1 { () => { { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } } }
def_f1!();
