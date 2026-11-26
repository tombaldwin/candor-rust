use fxdepe15::*;
thread_local! { static X_e15: u8 = 0; }
#[cfg(unix)] pub fn gated_e15() {}
#[cfg(not(unix))] pub fn gated_e15() {}
pub fn p_go_e15() { tl_touch() }
