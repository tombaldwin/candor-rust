mod prelude { pub use std::thread_local; }
use self::prelude::thread_local;
thread_local! { static X_e27: u8 = 0; }
#[cfg(unix)] pub fn gated_e27() {}
#[cfg(not(unix))] pub fn gated_e27() {}
pub fn p_go_e27() { X_e27.with(|_| ()) }
