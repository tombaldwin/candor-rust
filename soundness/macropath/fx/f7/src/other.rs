#[macro_export] macro_rules! mk { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } }
pub(crate) use mk;
