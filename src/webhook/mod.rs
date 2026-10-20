//! # Webhook Ingress and Axum API Module
//!
//! ## Overview
//! Manages webhook registration, HMAC/Bearer authentication, HTTP route handling, and encrypted
//! WebSocket sessions for external clients and triggers.
//!
//! ## Submodules
//! - [`api`] (`api/mod.rs`): Implements WebSocket session loops, E2E handshake validation, and API routes.
//! - [`auth`] (`auth.rs`): Validates Bearer tokens and HMAC-SHA256 payload signatures.
//! - [`manager`] (`manager.rs`): Persists and resolves registered webhook hook definitions.
//! - [`models`] (`models.rs`): Data structures for webhook hooks, payloads, and dispatch results.
//! - [`observer`] (`observer.rs`): Watches `webhooks.json` and manages the lifecycle of the webhook server.
//! - [`server`] (`server.rs`): Binds TCP ports, configures rate limiting, and runs the Axum HTTP server.
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
