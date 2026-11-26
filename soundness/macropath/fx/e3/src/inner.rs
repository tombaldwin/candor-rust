thread_local! { static X_e3: u8 = 0; }
#[cfg(unix)] pub fn gated_e3() {}
#[cfg(not(unix))] pub fn gated_e3() {}
pub fn p_go_e3() { tl_touch() }
