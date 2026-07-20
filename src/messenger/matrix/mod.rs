//! # Matrix protocol Bridge (index.md)
//!
//! ## Overview
//! Connects the local message bus to Matrix networks. Handles encryption, credentials, and state sync.
//!
//! ## Module Components
//! - [`bot`]: Long-polling loop receiving events.
//! - [`transport`]: Transmits message envelopes to Matrix room APIs.
//! - [`credentials`]: Restores access tokens.
//!
//! ## Search Tags
//! #matrix-bridge, #matrix-transport, #token-restore

pub mod id_map;
pub mod formatting;
pub mod typing;
pub mod message_queue;
pub mod transport;
pub mod credentials;
pub mod bot;






