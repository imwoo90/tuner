//! # Cross-Platform Messenger Adapter (index.md)
//!
//! ## Overview
//! Bridges standard message bus envelopes to alternative messaging networks like Matrix.
//!
//! ## Module Components
//! - [`matrix`]: Full Matrix client transport implementing event synchronizers.
//!
//! ## Search Tags
//! #messenger-adapter, #matrix-adapter, #chat-bridge

pub mod matrix;
pub mod telegram;
