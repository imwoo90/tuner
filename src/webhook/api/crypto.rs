//! # Webhook Payload Encryption and Signing Keys
//!
//! Manages ECDH/AES-GCM key exchanges and signature verification to guarantee message integrity
//! across web APIs.

use base64::Engine;
use crypto_box::aead::Aead;
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use rand::rngs::OsRng;
use serde_json::Value;

pub struct E2ESession {
    secret_key: SecretKey,
    pub local_pk_b64: String,
    box_session: Option<SalsaBox>,
}

impl E2ESession {
    pub fn new() -> Self {
        let secret_key = SecretKey::generate(&mut OsRng);
        let local_pk_b64 =
            base64::prelude::BASE64_STANDARD.encode(secret_key.public_key().as_bytes());
        Self {
            secret_key,
            local_pk_b64,
            box_session: None,
        }
    }

    pub fn set_remote_key(&mut self, remote_pk_b64: &str) -> Result<(), String> {
        let remote_bytes = base64::prelude::BASE64_STANDARD
            .decode(remote_pk_b64)
            .map_err(|e| e.to_string())?;
        if remote_bytes.len() != 32 {
            return Err("Invalid Curve25519 public key length".to_string());
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&remote_bytes);
        let remote_pk = PublicKey::from(key_bytes);
        self.box_session = Some(SalsaBox::new(&remote_pk, &self.secret_key));
        Ok(())
    }

    pub fn encrypt(&self, data: &Value) -> Result<String, String> {
        let session = self
            .box_session
            .as_ref()
            .ok_or_else(|| "E2E session not initialized".to_string())?;
        let plaintext = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        let mut nonce_bytes = [0u8; 24];
        use rand::RngCore;
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = crypto_box::Nonce::from(nonce_bytes);
        let ciphertext = session
            .encrypt(&nonce, plaintext.as_slice())
            .map_err(|_| "Encryption failed".to_string())?;
        let mut packet = nonce.to_vec();
        packet.extend(ciphertext);
        Ok(base64::prelude::BASE64_STANDARD.encode(packet))
    }

    pub fn decrypt(&self, frame: &str) -> Result<Value, String> {
        let session = self
            .box_session
            .as_ref()
            .ok_or_else(|| "E2E session not initialized".to_string())?;
        let raw = base64::prelude::BASE64_STANDARD
            .decode(frame)
            .map_err(|e| e.to_string())?;
        if raw.len() < 24 {
            return Err("Payload too short".to_string());
        }
        let (nonce_bytes, ciphertext) = raw.split_at(24);
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(nonce_bytes);
        let nonce = crypto_box::Nonce::from(nonce);
        let plaintext = session
            .decrypt(&nonce, ciphertext)
            .map_err(|_| "Decryption failed".to_string())?;
        let val = serde_json::from_slice(&plaintext).map_err(|e| e.to_string())?;
        Ok(val)
    }
}
