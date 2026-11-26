use fxdepe1::*;
thread_local! { static X_e1: u8 = 0; }
#[cfg(unix)] pub fn gated_e1() {}
#[cfg(not(unix))] pub fn gated_e1() {}
pub fn p_go_e1() { tl_touch() }
