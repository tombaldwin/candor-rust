use crate::prelude::thread_local;
thread_local! { static X_e24: u8 = 0; }
#[cfg(unix)] pub fn gated_e24() {}
#[cfg(not(unix))] pub fn gated_e24() {}
pub fn p_go_e24() { tl_touch() }
