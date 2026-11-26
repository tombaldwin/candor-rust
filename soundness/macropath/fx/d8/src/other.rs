#[macro_export] macro_rules! mktr { ($n:ident) => { fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
#[macro_export] macro_rules! mktrp { ($n:ident) => { fn $n(&self) {} } }
