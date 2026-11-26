#[macro_export(local_inner_macros)]
macro_rules! wrap { () => { macro_rules! q { () => { mk!() } } macro_rules! qn { () => { mknop!() } } } }
