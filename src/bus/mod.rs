//! # Message Bus and Event Routing Module
//!
//! Replaces the Python `ductor_bot.bus` framework in Rust, coordinating
//! async event routing, prompt injection locks, and transport adaptors.
//!
//! ## Submodules
//!
//! - [`envelope`]: Structs and enums defining the message format.
//! - [`adapters`]: Converters mapping various domain results into Envelopes.
//! - [`lock_pool`]: Mutex pool matching chat and topic keys.
//! - [`bus`]: Core MessageBus dispatching unicast/broadcast events.
//! - [`observers_wire`]: Hooks connecting background observer runtimes.

//! 
//! ## Search Tags
//! #mod

pub mod envelope;
pub mod adapters;
pub mod lock_pool;
pub mod bus;
pub mod observers_wire;
