mod prelude { pub use fxdepe21::*; }
use crate::inner::prelude::thread_local;
thread_local! { static X_e21: u8 = 0; }
#[cfg(unix)] pub fn gated_e21() {}
#[cfg(not(unix))] pub fn gated_e21() {}
pub fn p_go_e21() { tl_touch() }
