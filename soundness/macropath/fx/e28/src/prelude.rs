#[cfg(feature = "loom")] pub use fxdepe28::thread_local;
#[cfg(not(feature = "loom"))] pub use std::thread_local;
