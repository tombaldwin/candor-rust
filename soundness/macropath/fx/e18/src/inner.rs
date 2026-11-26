mod helpers { pub fn h_e18() {} }
use helpers::*;
thread_local! { static X_e18: u8 = 0; }
#[cfg(unix)] pub fn gated_e18() {}
#[cfg(not(unix))] pub fn gated_e18() {}
pub fn p_go_e18() { tl_touch() }
