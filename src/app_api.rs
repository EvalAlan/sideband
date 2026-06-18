#![allow(dead_code)]

// Re-export the crate's public API for FFI consumers
pub use crate::api::*;
pub use crate::types::*;

// Re-export transport for bootstrap
pub use crate::transport::tor::TorTransport;
