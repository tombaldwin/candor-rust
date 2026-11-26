#[cfg(unix)] crate::wfn! { pub fn go_d11v1(); }
#[cfg(not(unix))] pub fn go_d11v1() {}
pub fn pure_d11v1() {}
