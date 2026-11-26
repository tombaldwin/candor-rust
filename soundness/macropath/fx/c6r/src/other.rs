#[macro_export] macro_rules! mk { () => { () } }
#[macro_export] macro_rules! mknop { () => { () } }
#[macro_export] macro_rules! wrap { () => { $crate::mk!() } }
#[macro_export] macro_rules! wrapnop { () => { $crate::mknop!() } }
