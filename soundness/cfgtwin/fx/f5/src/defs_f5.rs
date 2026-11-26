#[macro_export] macro_rules! kp_f5 { () => { { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
