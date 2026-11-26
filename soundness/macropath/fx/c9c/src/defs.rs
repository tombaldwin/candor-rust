#[macro_export] macro_rules! mk { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } }
#[macro_export] macro_rules! mknop { () => { () } }
#[macro_export] macro_rules! wrap { () => { mk!() } }
#[macro_export] macro_rules! wrapnop { () => { nop!() } }
