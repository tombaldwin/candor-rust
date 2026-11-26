use super::prelude::thread_local;
thread_local! { static X_e25: u8 = 0; }
#[cfg(unix)] pub fn gated_e25() {}
#[cfg(not(unix))] pub fn gated_e25() {}
pub fn p_go_e25() { tl_touch() }
