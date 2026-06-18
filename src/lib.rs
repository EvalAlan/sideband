#![allow(dead_code)]

pub mod api;
pub mod shared;
pub mod transport;
pub mod types;

// Re-export types
pub use types::{ApiContact, ApiMessage, ApiStatus};

// Re-export shared types
pub use shared::{ChatMessage, ContactFile, ContactsMap};

// Re-export API functions with clean names
pub use api::{
    api_add_contact as add_contact,
    api_cancel_transfer as cancel_transfer,
    api_delete_contact as delete_contact,
    api_init_profile as init_profile,
    api_list_contacts as list_contacts,
    api_list_messages as list_messages,
    api_list_transfers as list_transfers,
    api_resume_transfer as resume_transfer,
    api_send_file as send_file,
    api_send_message as send_message,
    api_status as status,
};

// Re-export shared functions for use by main.rs binary
pub use shared::*;
