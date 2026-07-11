//! # Webhook Authentication Tests
//!
//! Tests for Bearer token verification, sliding-window RateLimiter, and HMAC signature algorithms.

use crate::webhook::auth::{validate_bearer_token, validate_hmac_signature, RateLimiter};
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn sign_sha256(body: &[u8], secret: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

#[tokio::test]
async fn test_rate_limiter_allows_within_limit() {
    let rl = RateLimiter::new(5);
    for _ in 0..5 {
        assert!(rl.check().await);
    }
}

#[tokio::test]
async fn test_rate_limiter_blocks_above_limit() {
    let rl = RateLimiter::new(2);
    assert!(rl.check().await);
    assert!(rl.check().await);
    assert!(!rl.check().await);
}

#[test]
fn test_validate_bearer_token() {
    assert!(validate_bearer_token("Bearer my-secret", "my-secret"));
    assert!(!validate_bearer_token("Bearer wrong", "my-secret"));
    assert!(!validate_bearer_token("my-secret", "my-secret"));
    assert!(!validate_bearer_token("", "my-secret"));
}

#[test]
fn test_validate_hmac_sha256_hex() {
    let body = b"hello world";
    let secret = "my-secret";
    let sig = sign_sha256(body, secret);
    assert!(validate_hmac_signature(
        body, &sig, secret, "sha256", "hex", "", "", ""
    ));
}

#[test]
fn test_validate_hmac_sig_prefix() {
    let body = b"hello world";
    let secret = "my-secret";
    let sig = format!("sha256={}", sign_sha256(body, secret));
    assert!(validate_hmac_signature(
        body, &sig, secret, "sha256", "hex", "sha256=", "", ""
    ));
}

#[test]
fn test_sig_regex_stripe_style() {
    let body = b"charge.succeeded";
    let secret = "whsec_test";
    let timestamp = "1614000000";
    let signed_payload = format!("{}.", timestamp);
    let mut payload = signed_payload.into_bytes();
    payload.extend_from_slice(body);

    let sig = sign_sha256(&payload, secret);
    let header = format!("t={},v1={}", timestamp, sig);
    assert!(validate_hmac_signature(
        body,
        &header,
        secret,
        "sha256",
        "hex",
        "",
        r"v1=([a-f0-9]+)",
        r"t=(\d+)"
    ));
}
