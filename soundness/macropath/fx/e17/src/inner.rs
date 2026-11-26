use fxdepe17::*;
thread_local! { static X_e17: u8 = 0; }
#[cfg(unix)] pub fn gated_e17() {}
#[cfg(not(unix))] pub fn gated_e17() {}
pub fn p_go_e17() { X_e17.with(|_| ()) }
