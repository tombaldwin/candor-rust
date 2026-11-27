pub trait GetU { fn get(x: u8) -> usize; }
impl GetU for usize { fn get(_x: u8) -> usize { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } }
impl GetU for std::ops::Range<usize> { fn get(_x: u8) -> usize { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } }
