macro_rules! mkfn { ($n:ident) => { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
macro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }
pub(crate) use mkfn;
pub(crate) use mkfnp;
