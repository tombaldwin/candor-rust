use crate::prelude::thread_local;
thread_local! { static X_e23: u8 = 0; }
#[cfg(unix)] pub fn gated_e23() {}
#[cfg(not(unix))] pub fn gated_e23() {}
pub fn p_go_e23() { tl_touch() }
