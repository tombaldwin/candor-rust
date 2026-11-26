use fxdepe5::thread_local;
thread_local! { static X_e5: u8 = 0; }
#[cfg(unix)] pub fn gated_e5() {}
#[cfg(not(unix))] pub fn gated_e5() {}
pub fn p_go_e5() { tl_touch() }
