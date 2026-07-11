//! # Transport-Agnostic Composite Session Key
//!
//! This module defines [`SessionKey`] which represents a composite session identifier
//! containing the transport mechanism, chat identifier, and optional topic/channel identifier.

use std::str::FromStr;

/// Composite session identifier: transport + chat + optional topic/channel.
#[derive(Clone, Debug, Hash, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionKey {
    pub transport: String,
    pub chat_id: i64,
    pub topic_id: Option<i64>,
}

impl Default for SessionKey {
    fn default() -> Self {
        Self {
            transport: "tg".to_string(),
            chat_id: 0,
            topic_id: None,
        }
    }
}

impl SessionKey {
    /// Create a new session key for Telegram.
    pub fn telegram(chat_id: i64, topic_id: Option<i64>) -> Self {
        Self {
            transport: "tg".to_string(),
            chat_id,
            topic_id,
        }
    }

    /// Generate a JSON-serializable key for `sessions.json` persistence.
    pub fn storage_key(&self) -> String {
        match self.topic_id {
            None => format!("{}:{}", self.transport, self.chat_id),
            Some(tid) => format!("{}:{}:{}", self.transport, self.chat_id, tid),
        }
    }
}

impl FromStr for SessionKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let p: Vec<&str> = s.split(':').collect();
        let err = |name, val: &str| format!("Invalid {}: {}", name, val);
        match p.len() {
            1 => Ok(Self {
                transport: "tg".to_string(),
                chat_id: p[0].parse().map_err(|_| err("chat_id", p[0]))?,
                topic_id: None,
            }),
            2 => {
                if let Ok(cid) = p[0].parse::<i64>() {
                    let tid = p[1].parse().map_err(|_| err("topic_id", p[1]))?;
                    Ok(Self { transport: "tg".to_string(), chat_id: cid, topic_id: Some(tid) })
                } else {
                    let cid = p[1].parse().map_err(|_| err("chat_id", p[1]))?;
                    Ok(Self { transport: p[0].to_string(), chat_id: cid, topic_id: None })
                }
            }
            3 => {
                let cid = p[1].parse().map_err(|_| err("chat_id", p[1]))?;
                let tid = p[2].parse().map_err(|_| err("topic_id", p[2]))?;
                Ok(Self { transport: p[0].to_string(), chat_id: cid, topic_id: Some(tid) })
            }
            _ => Err(format!("Invalid key format: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_key() {
        let k1 = SessionKey::telegram(12345, None);
        assert_eq!(k1.storage_key(), "tg:12345");

        let k2 = SessionKey::telegram(12345, Some(99));
        assert_eq!(k2.storage_key(), "tg:12345:99");
    }

    #[test]
    fn test_parse_legacy_keys() {
        let k1 = SessionKey::from_str("12345").unwrap();
        assert_eq!(k1.transport, "tg");
        assert_eq!(k1.chat_id, 12345);
        assert_eq!(k1.topic_id, None);

        let k2 = SessionKey::from_str("-100123").unwrap();
        assert_eq!(k2.transport, "tg");
        assert_eq!(k2.chat_id, -100123);
        assert_eq!(k2.topic_id, None);

        let k3 = SessionKey::from_str("12345:99").unwrap();
        assert_eq!(k3.transport, "tg");
        assert_eq!(k3.chat_id, 12345);
        assert_eq!(k3.topic_id, Some(99));
    }

    #[test]
    fn test_parse_new_prefixed_keys() {
        let k1 = SessionKey::from_str("tg:12345").unwrap();
        assert_eq!(k1.transport, "tg");
        assert_eq!(k1.chat_id, 12345);
        assert_eq!(k1.topic_id, None);

        let k2 = SessionKey::from_str("tg:12345:99").unwrap();
        assert_eq!(k2.transport, "tg");
        assert_eq!(k2.chat_id, 12345);
        assert_eq!(k2.topic_id, Some(99));

        let k3 = SessionKey::from_str("mx:98765").unwrap();
        assert_eq!(k3.transport, "mx");
        assert_eq!(k3.chat_id, 98765);
        assert_eq!(k3.topic_id, None);
    }
}
