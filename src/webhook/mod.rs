//! # Webhook Module
//!
//! Handles registering webhook ingress endpoints, verifying HMAC signatures
//! and tokens, routing requests, and forwarding events to the `MessageBus`.

pub mod api;
pub mod auth;
pub mod manager;
pub mod models;
pub mod observer;
pub mod server;

pub use observer::WebhookObserver;
