#[cfg(unix)] crate::wim! { crate::Q, crate::Tr_d11v4, go_d11v4 }
#[cfg(not(unix))] impl crate::Tr_d11v4 for crate::Q { fn go_d11v4(&self) {} }
pub fn pure_d11v4() {}
