//! # Webhook API Handler Declarations
//!
//! ## Overview
//! Registers and maps Axum paths to handshake protocols, WebSocket event loops, task registers,
//! and asset file endpoints.
//!
//! ## Collaboration Graph
//! - Composes route routers loaded by [`super::server::WebhookServer::start`].
//!
//! ## Search Tags
//! #api-router, #webhook-endpoints, #service-registry

pub mod crypto;
pub mod files;
pub mod handshake;
pub mod server;
pub mod session_loop;
pub mod websocket;

pub use crypto::E2ESession;
pub use files::{is_image_path, parse_file_refs, prepare_destination, sanitize_filename};
pub use handshake::perform_handshake;
pub use server::{ApiServer, ApiServerState};
pub use websocket::{ActiveStateGetter, ApiAbortHandler, ApiMessageHandler, ApiResult};
