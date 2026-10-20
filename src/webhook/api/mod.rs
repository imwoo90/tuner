//! # Webhook API Handler Declarations
//!
//! ## Overview
//! Registers and maps Axum HTTP/WebSocket routes to E2E cryptographic handshakes, session event
//! loops, and file upload/download endpoints.
//!
//! ## Submodules
//! - [`crypto`] (`crypto.rs`): X25519 key exchange and ChaCha20-Poly1305 session encryption.
//! - [`files`] (`files.rs`): Sanitized file upload staging and attachment reference parsing.
//! - [`handshake`] (`handshake.rs`): WebSocket authentication and E2E key negotiation protocol.
//! - [`server`] (`server.rs`): Standalone WebSocket API server lifecycle and state container.
//! - [`session_loop`] (`session_loop.rs`): Per-client WebSocket frame dispatch loop.
//! - [`websocket`] (`websocket.rs`): Callback handler traits and types for WebSocket API messages.
//!
//! ## Search Tags
//! #api-router, #webhook-endpoints, #service-registry, #e2e-websocket

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
