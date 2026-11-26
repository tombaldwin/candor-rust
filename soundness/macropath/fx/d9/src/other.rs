#[cfg(unix)]
#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
#[cfg(not(unix))]
#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() {} } }
#[macro_export] macro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }
