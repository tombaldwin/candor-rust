thread_local! { static X_e4: u8 = 0; }
#[cfg(unix)] pub fn gated_e4() {}
#[cfg(not(unix))] pub fn gated_e4() {}
pub fn p_go_e4() { use std::fmt::Write as _; let mut s = String::new(); let _ = write!(s, "x"); }
