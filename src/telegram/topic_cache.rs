//! # Forum Topic Metadata Cache
//!
//! Manages a local cache mapping Telegram forum topic names to IDs. Accelerates message routing
//! in multi-topic workspace environments without querying the API repeatedly.

use std::sync::Mutex;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct BotInfo {
    pub username: Option<String>,
}

pub struct TopicNameCache {
    names: Mutex<HashMap<(i64, i64), String>>,
}

impl TopicNameCache {
    pub fn new() -> Self {
        Self {
            names: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, chat_id: i64, topic_id: i64, name: String) {
        let mut map = self.names.lock().unwrap();
        map.insert((chat_id, topic_id), name);
    }

    pub fn find_by_id(&self, chat_id: i64, topic_id: i64) -> Option<String> {
        let map = self.names.lock().unwrap();
        map.get(&(chat_id, topic_id)).cloned()
    }

    pub fn find_by_name(&self, chat_id: i64, name: &str) -> Option<i64> {
        let target_name = name.strip_prefix('@').unwrap_or(name);
        let map = self.names.lock().unwrap();
        for (&(cid, tid), tname) in map.iter() {
            if cid == chat_id && tname.eq_ignore_ascii_case(target_name) {
                return Some(tid);
            }
        }
        None
    }
}

pub(crate) fn handle_forum_topic_events(msg: &teloxide::types::Message, cache: &TopicNameCache, chat_id: i64) -> bool {
    let tid = match msg.thread_id { Some(t) => t as i64, None => return false };
    match &msg.kind {
        teloxide::types::MessageKind::ForumTopicCreated(c) => {
            cache.insert(chat_id, tid, c.forum_topic_created.name.clone());
            true
        }
        teloxide::types::MessageKind::ForumTopicEdited(e) => {
            if let Some(ref name) = e.forum_topic_edited.name {
                cache.insert(chat_id, tid, name.clone());
            }
            true
        }
        _ => false,
    }
}
