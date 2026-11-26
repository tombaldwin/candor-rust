#[cfg(unix)] crate::wuse! { go_d11v5, crate::eff_d11v5 }
#[cfg(not(unix))] pub use crate::pur_d11v5 as go_d11v5;
pub use crate::pur_d11v5 as keep_d11v5;
