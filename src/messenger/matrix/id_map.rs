//! Bidirectional mapping between Matrix room_id strings and integer chat_ids.
//!
//! Provides unique, deterministic, collision-resistant 64-bit ID mapping.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};
use std::sync::Mutex;

/// Bidirectional room_id ↔ int mapping with collision detection.
#[derive(Debug)]
pub struct MatrixIdMap {
    room_to_int: Mutex<HashMap<String, i64>>,
    int_to_room: Mutex<HashMap<i64, String>>,
    path: PathBuf,
}

impl MatrixIdMap {
    /// Create a new `MatrixIdMap` persisting data in the given directory.
    pub fn new(store_path: &Path) -> Self {
        let path = store_path.join("room_id_map.json");
        let mut map = Self {
            room_to_int: Mutex::new(HashMap::new()),
            int_to_room: Mutex::new(HashMap::new()),
            path,
        };
        map.load();
        map
    }

    /// Convert a room_id to a deterministic 64-bit integer ID.
    pub fn room_to_int(&self, room_id: &str) -> i64 {
        {
            let r_to_i = self.room_to_int.lock().unwrap();
            if let Some(&id) = r_to_i.get(room_id) {
                return id;
            }
        }

        let mut hasher = Sha256::new();
        hasher.update(room_id.as_bytes());
        let hash_bytes = hasher.finalize();
        let mut h = u64::from_be_bytes(hash_bytes[0..8].try_into().unwrap()) as i64;

        loop {
            let i_to_r = self.int_to_room.lock().unwrap();
            if let Some(existing) = i_to_r.get(&h) {
                if existing == room_id {
                    break;
                }
                let mut hasher = Sha256::new();
                hasher.update(format!("{}:{}", room_id, h).as_bytes());
                let hash_bytes = hasher.finalize();
                h = u64::from_be_bytes(hash_bytes[0..8].try_into().unwrap()) as i64;
            } else {
                break;
            }
        }

        {
            let mut r_to_i = self.room_to_int.lock().unwrap();
            let mut i_to_r = self.int_to_room.lock().unwrap();
            r_to_i.insert(room_id.to_string(), h);
            i_to_r.insert(h, room_id.to_string());
        }
        self.save();
        h
    }

    /// Retrieve the room_id string for a given integer chat_id.
    pub fn int_to_room(&self, chat_id: i64) -> Option<String> {
        let i_to_r = self.int_to_room.lock().unwrap();
        i_to_r.get(&chat_id).cloned()
    }

    fn load(&mut self) {
        if !self.path.exists() {
            return;
        }
        if let Ok(content) = fs::read_to_string(&self.path) {
            if let Ok(data) = serde_json::from_str::<HashMap<String, i64>>(&content) {
                let mut r_to_i = self.room_to_int.lock().unwrap();
                let mut i_to_r = self.int_to_room.lock().unwrap();
                for (room_id, int_id) in data {
                    r_to_i.insert(room_id.clone(), int_id);
                    i_to_r.insert(int_id, room_id);
                }
            }
        }
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let r_to_i = self.room_to_int.lock().unwrap();
        if let Ok(content) = serde_json::to_string_pretty(&*r_to_i) {
            let _ = fs::write(&self.path, content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_room_to_int_deterministic() {
        let dir = tempdir().unwrap();
        let m = MatrixIdMap::new(dir.path());
        let a = m.room_to_int("!abc:server");
        let b = m.room_to_int("!abc:server");
        assert_eq!(a, b);
    }

    #[test]
    fn test_different_rooms_different_ids() {
        let dir = tempdir().unwrap();
        let m = MatrixIdMap::new(dir.path());
        let a = m.room_to_int("!room1:server");
        let b = m.room_to_int("!room2:server");
        assert_ne!(a, b);
    }

    #[test]
    fn test_int_to_room_roundtrip() {
        let dir = tempdir().unwrap();
        let m = MatrixIdMap::new(dir.path());
        let int_id = m.room_to_int("!test:example.com");
        assert_eq!(m.int_to_room(int_id), Some("!test:example.com".to_string()));
    }

    #[test]
    fn test_int_to_room_unknown() {
        let dir = tempdir().unwrap();
        let m = MatrixIdMap::new(dir.path());
        assert_eq!(m.int_to_room(999999), None);
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = tempdir().unwrap();
        let m1 = MatrixIdMap::new(dir.path());
        let int_id = m1.room_to_int("!persist:server");

        let m2 = MatrixIdMap::new(dir.path());
        assert_eq!(m2.room_to_int("!persist:server"), int_id);
        assert_eq!(m2.int_to_room(int_id), Some("!persist:server".to_string()));
    }

    #[test]
    fn test_corrupt_file_starts_fresh() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("room_id_map.json");
        fs::write(&path, "{bad json").unwrap();
        let m = MatrixIdMap::new(dir.path());
        let int_id = m.room_to_int("!new:server");
        assert_ne!(int_id, 0);
    }

    #[test]
    fn test_multiple_rooms_all_unique() {
        let dir = tempdir().unwrap();
        let m = MatrixIdMap::new(dir.path());
        let mut ids = std::collections::HashSet::new();
        for i in 0..50 {
            ids.insert(m.room_to_int(&format!("!room{}:server", i)));
        }
        assert_eq!(ids.len(), 50);
    }
}
