#[cfg(unix)] pub trait T_d11v2 { crate::wt! { fn go_d11v2(&self); } }
#[cfg(not(unix))] pub trait T_d11v2 { fn go_d11v2(&self) {} }
pub fn pure_d11v2() {}
