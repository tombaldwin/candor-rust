#[cfg(unix)] crate::wmdef! { k_d11v7 }
#[cfg(not(unix))] macro_rules! k_d11v7 { () => {} }
