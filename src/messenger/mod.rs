//! # Cross-Platform Messenger Adapter
//!
//! ## Overview
//! Bridges standard message bus envelopes and streaming agent outputs to external messaging
//! networks such as Telegram.
//!
//! ## Submodules
//! - [`telegram`] (`telegram/mod.rs`): Full Telegram bot ingress, streaming, and interactive UI adapter.
//! - [`web`] (`web/mod.rs`): Responsive Web UI / PWA control plane and bidirectional REST/WebSocket dashboard.
//!
//! ## Search Tags
//! #messenger-adapter, #telegram-adapter, #chat-bridge, #web-dashboard

pub mod telegram;
pub mod web;
