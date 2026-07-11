//! # E2E Crypto Session Tests
//!
//! Tests Curve25519-XSalsa20-Poly1305 E2E session handshake, encryption, decryption, and integrity check.

use crate::webhook::api::crypto::E2ESession;
use base64::Engine;
use serde_json::json;

fn make_pair() -> (E2ESession, E2ESession) {
    let mut server = E2ESession::new();
    let mut client = E2ESession::new();
    server.set_remote_key(&client.local_pk_b64).unwrap();
    client.set_remote_key(&server.local_pk_b64).unwrap();
    (server, client)
}

#[test]
fn test_keypair_is_32_bytes() {
    let session = E2ESession::new();
    let pk_bytes = base64::prelude::BASE64_STANDARD
        .decode(&session.local_pk_b64)
        .unwrap();
    assert_eq!(pk_bytes.len(), 32);
}

#[test]
fn test_round_trip() {
    let (server, client) = make_pair();
    let msg = json!({"type": "text_delta", "data": "hello world"});
    let encrypted = server.encrypt(&msg).unwrap();
    let decrypted = client.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, msg);
}

#[test]
fn test_bidirectional() {
    let (server, client) = make_pair();
    let msg1 = json!({"type": "result", "text": "response"});
    let encrypted1 = server.encrypt(&msg1).unwrap();
    assert_eq!(client.decrypt(&encrypted1).unwrap(), msg1);

    let msg2 = json!({"type": "message", "text": "input"});
    let encrypted2 = client.encrypt(&msg2).unwrap();
    assert_eq!(server.decrypt(&encrypted2).unwrap(), msg2);
}

#[test]
fn test_tampered_ciphertext_rejects() {
    let (server, client) = make_pair();
    let msg = json!({"key": "value"});
    let encrypted = server.encrypt(&msg).unwrap();
    let mut raw = base64::prelude::BASE64_STANDARD.decode(&encrypted).unwrap();
    let last_idx = raw.len() - 1;
    raw[last_idx] ^= 0xFF;
    let tampered = base64::prelude::BASE64_STANDARD.encode(raw);
    assert!(client.decrypt(&tampered).is_err());
}

#[test]
fn test_unique_nonces() {
    let (server, _) = make_pair();
    let msg = json!({"same": "data"});
    assert_ne!(server.encrypt(&msg).unwrap(), server.encrypt(&msg).unwrap());
}
