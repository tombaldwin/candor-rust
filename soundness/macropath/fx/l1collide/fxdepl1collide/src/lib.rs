pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
#[macro_export] macro_rules! mkfn_l1collide { ($n:ident) => { pub fn $n() { $crate::emit() } } }
