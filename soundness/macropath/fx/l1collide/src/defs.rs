macro_rules! mkfn_l1collide { ($n:ident) => { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } pub(crate) use mkfn_l1collide;
macro_rules! mkfnp_l1collide { ($n:ident) => { pub fn $n() {} } } pub(crate) use mkfnp_l1collide;
macro_rules! mkm_l1collide { ($n:ident) => { pub fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } pub(crate) use mkm_l1collide;
macro_rules! mkmp_l1collide { ($n:ident) => { pub fn $n(&self) {} } } pub(crate) use mkmp_l1collide;
macro_rules! mktr_l1collide { ($n:ident) => { fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } pub(crate) use mktr_l1collide;
macro_rules! mktrp_l1collide { ($n:ident) => { fn $n(&self) {} } } pub(crate) use mktrp_l1collide;
