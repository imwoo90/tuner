//! # Webhook API Routes Declarations
//!
//! Declares router paths, handlers, and endpoints for public web services like handshake registers,
//! task submissions, and file access.

pub mod crypto;
pub mod files;
pub mod handshake;
pub mod server;
pub mod session_loop;
pub mod websocket;
pub mod tasks;
pub mod tasks_models;

pub use crypto::E2ESession;
pub use files::{is_image_path, parse_file_refs, prepare_destination, sanitize_filename};
pub use handshake::perform_handshake;
pub use server::{ApiServer, ApiServerState};
pub use websocket::{ActiveStateGetter, ApiAbortHandler, ApiMessageHandler, ApiResult};
