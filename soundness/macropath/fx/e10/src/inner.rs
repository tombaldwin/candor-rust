use crate::prelude::*;
thread_local! { static X_e10: u8 = 0; }
#[cfg(unix)] pub fn gated_e10() {}
#[cfg(not(unix))] pub fn gated_e10() {}
pub fn p_go_e10() { tl_touch() }
