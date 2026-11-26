mod prelude { pub use fxdepe22::*; }
use self::prelude::thread_local;
thread_local! { static X_e22: u8 = 0; }
#[cfg(unix)] pub fn gated_e22() {}
#[cfg(not(unix))] pub fn gated_e22() {}
pub fn p_go_e22() { tl_touch() }
