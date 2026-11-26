thread_local! { static X_e11: u8 = 0; }
#[cfg(unix)] pub fn gated_e11() {}
#[cfg(not(unix))] pub fn gated_e11() {}
pub fn p_go_e11() { tl_touch() }
