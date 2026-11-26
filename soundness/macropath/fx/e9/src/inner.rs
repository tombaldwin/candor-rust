use crate::prelude::*;
thread_local! { static X_e9: u8 = 0; }
#[cfg(unix)] pub fn gated_e9() {}
#[cfg(not(unix))] pub fn gated_e9() {}
pub fn p_go_e9() { tl_touch() }
