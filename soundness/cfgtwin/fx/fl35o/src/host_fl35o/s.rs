pub struct T; impl T { pub fn go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } pub fn quiet(&self) {} }
