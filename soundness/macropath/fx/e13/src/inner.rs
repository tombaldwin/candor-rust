thread_local! { static X_e13: u8 = 0; }
#[cfg(unix)] pub fn gated_e13() {}
#[cfg(not(unix))] pub fn gated_e13() {}
pub fn p_go_e13() { let _ = vec![1]; }
