pub struct Poller;
impl Poller { pub fn wait(&self) -> u8 { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 1 } }
