use crate::prelude::thread_local;
thread_local! { static X_e28: u8 = 0; }
#[cfg(unix)] pub fn gated_e28() {}
#[cfg(not(unix))] pub fn gated_e28() {}
pub fn p_go_e28() { X_e28.with(|_| ()) }
