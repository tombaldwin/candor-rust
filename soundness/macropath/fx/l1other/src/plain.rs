pub fn go_l1other() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub fn call_l1other() { go_l1other() }
