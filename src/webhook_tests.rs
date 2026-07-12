//! # Webhook and API Server Tests
//!
//! Integration and unit tests for Axum webhook/api routing, auth, models and session.

#[cfg(test)]
pub mod adversarial_tests;
#[cfg(test)]
pub mod api_server_tests;
#[cfg(test)]
pub mod auth_tests;
#[cfg(test)]
pub mod crypto_tests;
#[cfg(test)]
pub mod manager_tests;
#[cfg(test)]
pub mod models_tests;
#[cfg(test)]
pub mod observer_tests;
#[cfg(test)]
pub mod server_e2e_tests;
#[cfg(test)]
pub mod server_files_tests;
#[cfg(test)]
pub mod server_tests;
