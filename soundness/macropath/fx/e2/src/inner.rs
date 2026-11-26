mod prelude { pub use fxdepe2::*; }
use prelude::*;
thread_local! { static X_e2: u8 = 0; }
#[cfg(unix)] pub fn gated_e2() {}
#[cfg(not(unix))] pub fn gated_e2() {}
pub fn p_go_e2() { tl_touch() }
