//! # Direct API Server
//!
//! Exposes E2E encrypted API and WebSocket servers.

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
