#[cfg(unix)] crate::wmod! { in_d11v8, go_d11v8 }
#[cfg(not(unix))] pub mod in_d11v8 { pub fn go_d11v8() {} }
pub mod inq_d11v8 { pub fn pure_d11v8() {} }
