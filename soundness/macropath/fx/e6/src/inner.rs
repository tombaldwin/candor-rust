thread_local! { static X_e6: u8 = 0; }
#[cfg(unix)] pub fn gated_e6() {}
#[cfg(not(unix))] pub fn gated_e6() {}
pub fn p_go_e6() { X_e6.with(|_| ()) }
