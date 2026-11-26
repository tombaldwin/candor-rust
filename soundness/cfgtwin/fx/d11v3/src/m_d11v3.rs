#[cfg(unix)] crate::winh! { S_d11v3, go_d11v3 }
#[cfg(not(unix))] impl S_d11v3 { pub fn go_d11v3(&self) {} }
pub struct S_d11v3;
pub fn pure_d11v3() {}
