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
