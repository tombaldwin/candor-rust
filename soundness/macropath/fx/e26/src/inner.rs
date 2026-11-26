use crate::prelude::thread_local;
thread_local! { static X_e26: u8 = 0; }
#[cfg(unix)] pub fn gated_e26() {}
#[cfg(not(unix))] pub fn gated_e26() {}
pub fn p_go_e26() { X_e26.with(|_| ()) }
