#![allow(dead_code)]

// Compile the existing CLI/TUI implementation as the library root too.
// A function named `main` is just another function in a library target.
// This keeps the Android cdylib path from duplicating half of src/main.rs.
include!("main.rs");

pub mod api;
pub mod types;

pub use api::*;
pub use types::{ApiContact, ApiMessage, ApiStatus};
