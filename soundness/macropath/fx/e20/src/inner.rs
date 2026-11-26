mod prelude { pub use fxdepe20::*; }
use self::prelude::thread_local;
thread_local! { static X_e20: u8 = 0; }
#[cfg(unix)] pub fn gated_e20() {}
#[cfg(not(unix))] pub fn gated_e20() {}
pub fn p_go_e20() { tl_touch() }
