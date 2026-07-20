//! # Matrix Network Messaging Adapter
//!
//! Implements communication adapters for the Matrix protocol. Resolves room IDs, synchronizes state,
//! handles connection sessions, and formats incoming/outgoing envelopes.

pub mod id_map;
pub mod formatting;
pub mod typing;
pub mod message_queue;
pub mod transport;
pub mod credentials;
pub mod bot;






