//! # Message Bus and Event Routing Module
//!
//! ## Overview
//! Coordinates asynchronous event routing, chat/topic lock pooling, and transport delivery
//! adapters across background runners, cron jobs, heartbeats, and webhooks.
//!
//! ## Submodules
//! - [`envelope`] (`envelope.rs`): Structs and enums defining the bus message envelope format.
//! - [`adapters`] (`adapters.rs`): Converters mapping domain task results into bus envelopes.
//! - [`lock_pool`] (`lock_pool.rs`): Per-chat and per-topic async mutex pool.
//! - [`bus`] (`bus.rs`): Core `MessageBus` dispatching unicast and broadcast events.
//! - [`observers_wire`] (`observers_wire.rs`): Wiring helpers connecting background observer runtimes to the bus.
//!
//! ## Search Tags
//! #message-bus, #event-routing, #lock-pool, #transport-adapters

pub mod envelope;
pub mod adapters;
pub mod lock_pool;
#[allow(clippy::module_inception)]
pub mod bus;
pub mod observers_wire;
