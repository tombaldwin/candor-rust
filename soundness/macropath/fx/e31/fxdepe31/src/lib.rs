pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() }
#[macro_export] macro_rules! thread_local { (static $n:ident : $t:ty = $e:expr ;) => { pub fn tl_touch() { $crate::emit() } } }
