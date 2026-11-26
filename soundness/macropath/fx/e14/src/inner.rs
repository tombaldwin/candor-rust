use fxdepe14::*;
thread_local! { static X_e14: u8 = 0; }
#[cfg(unix)] pub fn gated_e14() {}
#[cfg(not(unix))] pub fn gated_e14() {}
pub fn p_go_e14() { tl_touch() }
