//! # Webhook Authentication and Rate Limiting
//!
//! Handles bearer tokens, HMAC signature verification, and sliding-window rate limiting.

use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use std::collections::VecDeque;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

#[derive(Clone)]
pub struct RateLimiter {
    max_per_minute: usize,
    timestamps: Arc<Mutex<VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(max_per_minute: usize) -> Self {
        Self {
            max_per_minute,
            timestamps: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn check(&self) -> bool {
        let mut ts = self.timestamps.lock().await;
        let now = Instant::now();
        let minute_ago = now.checked_sub(Duration::from_secs(60)).unwrap_or(now);

        while let Some(&t) = ts.front() {
            if t < minute_ago {
                ts.pop_front();
            } else {
                break;
            }
        }

        if ts.len() >= self.max_per_minute {
            false
        } else {
            ts.push_back(now);
            true
        }
    }

    pub async fn reset(&self) {
        self.timestamps.lock().await.clear();
    }
}

pub fn safe_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        false
    } else {
        a.ct_eq(b).into()
    }
}

pub fn validate_bearer_token(auth_header: &str, expected_token: &str) -> bool {
    if expected_token.is_empty() {
        return false;
    }
    let Some(prefix) = auth_header.get(..7) else { return false; };
    if !prefix.eq_ignore_ascii_case("Bearer ") { return false; }
    let Some(token) = auth_header.get(7..) else { return false; };
    safe_compare(token.as_bytes(), expected_token.as_bytes())
}

fn extract_signature(
    sig_value: &str,
    sig_prefix: &str,
    sig_regex: &str,
    cache: &std::sync::OnceLock<Option<regex::Regex>>,
) -> Option<String> {
    if !sig_regex.is_empty() {
        let re_opt = cache.get_or_init(|| regex::Regex::new(sig_regex).ok());
        let re = re_opt.as_ref()?;
        let caps = re.captures(sig_value)?;
        caps.get(1).map(|m| m.as_str().to_string())
    } else if !sig_prefix.is_empty() {
        if sig_value.starts_with(sig_prefix) {
            Some(sig_value[sig_prefix.len()..].to_string())
        } else {
            Some(sig_value.to_string())
        }
    } else {
        Some(sig_value.to_string())
    }
}

fn compute_hmac(algo: &str, secret: &[u8], payload: &[u8]) -> Option<Vec<u8>> {
    match algo {
        "sha256" => {
            let mut mac = Hmac::<Sha256>::new_from_slice(secret).ok()?;
            mac.update(payload);
            Some(mac.finalize().into_bytes().to_vec())
        }
        "sha512" => {
            let mut mac = Hmac::<Sha512>::new_from_slice(secret).ok()?;
            mac.update(payload);
            Some(mac.finalize().into_bytes().to_vec())
        }
        "sha1" => {
            let mut mac = Hmac::<Sha1>::new_from_slice(secret).ok()?;
            mac.update(payload);
            Some(mac.finalize().into_bytes().to_vec())
        }
        _ => None,
    }
}

pub fn validate_hmac_signature(
    body: &[u8],
    sig_value: &str,
    secret: &str,
    hmac_algorithm: &str,
    hmac_encoding: &str,
    hmac_sig_prefix: &str,
    hmac_sig_regex: &str,
    hmac_sig_regex_cached: &std::sync::OnceLock<Option<regex::Regex>>,
    hmac_payload_prefix_regex: &str,
    hmac_payload_prefix_regex_cached: &std::sync::OnceLock<Option<regex::Regex>>,
) -> bool {
    if sig_value.is_empty() || secret.is_empty() {
        return false;
    }

    let Some(extracted_sig) = extract_signature(
        sig_value,
        hmac_sig_prefix,
        hmac_sig_regex,
        hmac_sig_regex_cached,
    ) else {
        return false;
    };

    let mut signed_payload = body.to_vec();
    if !hmac_payload_prefix_regex.is_empty() {
        let re_opt = hmac_payload_prefix_regex_cached
            .get_or_init(|| regex::Regex::new(hmac_payload_prefix_regex).ok());
        if let Some(re) = re_opt {
            if let Some(caps) = re.captures(sig_value) {
                if let Some(m) = caps.get(1) {
                    let mut prefix_bytes = m.as_str().as_bytes().to_vec();
                    prefix_bytes.push(b'.');
                    prefix_bytes.extend_from_slice(body);
                    signed_payload = prefix_bytes;
                }
            }
        }
    }

    let decoded_sig = match hmac_encoding {
        "base64" => base64::prelude::BASE64_STANDARD.decode(&extracted_sig).ok(),
        _ => hex::decode(&extracted_sig).ok(),
    };

    let Some(expected_bytes) = decoded_sig else {
        return false;
    };

    let Some(computed_bytes) = compute_hmac(hmac_algorithm, secret.as_bytes(), &signed_payload)
    else {
        return false;
    };

    safe_compare(&computed_bytes, &expected_bytes)
}

pub fn validate_hook_auth(
    hook: &crate::webhook::models::WebhookEntry,
    auth_header: &str,
    sig_value: &str,
    body: &[u8],
    global_token: &str,
) -> bool {
    if hook.auth_mode == "hmac" {
        validate_hmac_signature(
            body,
            sig_value,
            &hook.hmac_secret,
            &hook.hmac_algorithm,
            &hook.hmac_encoding,
            &hook.hmac_sig_prefix,
            &hook.hmac_sig_regex,
            &hook.hmac_sig_regex_cached,
            &hook.hmac_payload_prefix_regex,
            &hook.hmac_payload_prefix_regex_cached,
        )
    } else {
        let expected = if hook.token.is_empty() {
            global_token
        } else {
            &hook.token
        };
        if expected.is_empty() {
            return false;
        }
        validate_bearer_token(auth_header, expected)
    }
}
