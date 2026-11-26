mod prelude { pub use fxdepe8::*; }
use prelude::*;
thread_local! { static X_e8: u8 = 0; }
#[cfg(unix)] pub fn gated_e8() {}
#[cfg(not(unix))] pub fn gated_e8() {}
pub fn p_go_e8() { tl_touch() }
