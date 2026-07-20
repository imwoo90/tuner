//! # Webhook Ingress and Axum API Module (index.md)
//!
//! ## Overview
//! Manages registration, signature verification, route handling, and WebSocket sessions for external runners.
//!
//! ## Module Components
//! - [`server`]: Binds TCP ports, sets up grace shutdown, and starts HTTP pipelines.
//! - [`api`]: Implements websocket session loops, handshake validations, and API routes.
//! - [`auth`]: Validates bearer tokens and HMAC sha256 checksums.
//! - [`manager`]: Persists and resolves registered webhook destinations.
//!
//! ## Search Tags
//! #webhook-ingress, #axum-server, #websocket-sessions, #hmac-validation

pub mod api;
pub mod auth;
pub mod manager;
pub mod models;
pub mod observer;
pub mod server;

pub use observer::WebhookObserver;
