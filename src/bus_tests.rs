//! # Message Bus Test Suite
//!
//! Exposes and runs unit and integration tests for the message bus framework.
#![allow(clippy::module_inception)]

mod envelope_tests;
mod lock_pool_tests;
mod adapters_1_tests;
mod adapters_2_tests;
mod bus_tests;
mod observers_wire_tests;
