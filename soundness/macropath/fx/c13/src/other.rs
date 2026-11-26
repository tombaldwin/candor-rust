#[cfg(unix)]
#[macro_export] macro_rules! mk { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } }
#[cfg(not(unix))]
#[macro_export] macro_rules! mk { () => { () } }
#[macro_export] macro_rules! mknop { () => { () } }
