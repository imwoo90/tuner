//! # Named Session Registry and Alias Resolver
//!
//! Allows assigning human-readable names (aliases) to active session IDs. Enables resuming
//! or managing specific workspaces easily via mnemonic keys.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use chrono::Utc;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NamedSession {
    pub name: String,
    pub chat_id: i64,
    pub status: String,
    pub last_prompt: String,
}

pub struct NamedSessionRegistry {
    path: PathBuf,
    lock: std::sync::Mutex<()>,
}

fn get_random_seed() -> u64 {
    let mut seed = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let mut buf = [0u8; 8];
        if std::io::Read::read_exact(&mut f, &mut buf).is_ok() {
            seed = u64::from_ne_bytes(buf);
        }
    }
    seed
}

impl NamedSessionRegistry {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: std::sync::Mutex::new(()) }
    }

    pub fn load(&self) -> Result<HashMap<String, NamedSession>, String> {
        if !self.path.exists() { return Ok(HashMap::new()); }
        let content = fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        if content.trim().is_empty() { return Ok(HashMap::new()); }
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }

    pub fn save(&self, map: &HashMap<String, NamedSession>) -> Result<(), String> {
        let temp = self.path.with_extension("tmp");
        let content = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&temp, content).map_err(|e| e.to_string())?;
        fs::rename(&temp, &self.path).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn generate_unique_name(&self, map: &HashMap<String, NamedSession>) -> Result<String, String> {
        let adjectives = ["bold", "swift", "clever", "quiet", "bright", "wild", "calm", "sharp", "gentle", "proud"];
        let nouns = ["owl", "fox", "wolf", "bear", "hawk", "deer", "lynx", "hare", "swan", "otter"];
        let mut seed = get_random_seed();
        let a: u64 = 6364136223846793005;
        let c: u64 = 1442695040888963407;
        for _ in 0..100 {
            seed = seed.wrapping_mul(a).wrapping_add(c);
            let adj = adjectives[(seed as usize) % adjectives.len()];
            seed = seed.wrapping_mul(a).wrapping_add(c);
            let noun = nouns[(seed as usize) % nouns.len()];
            let name = format!("{}{}", adj, noun);
            if !map.contains_key(&name) {
                return Ok(name);
            }
        }
        Err("Failed to generate unique name".to_string())
    }

    pub fn recover_crash(&self) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let mut map = self.load()?;
        let mut chg = false;
        for s in map.values_mut() {
            if !s.name.starts_with("ia-") && s.status == "running" {
                s.status = "idle".to_string();
                chg = true;
            }
        }
        if chg { self.save(&map)?; }
        Ok(())
    }

    pub fn create_session(&self, chat_id: i64, status: &str, last_prompt: &str) -> Result<NamedSession, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let mut map = self.load()?;
        let count = map.values().filter(|s| s.chat_id == chat_id).count();
        if count >= 10 {
            return Err("Max 10 named sessions per chat allowed".to_string());
        }
        let name = self.generate_unique_name(&map)?;
        let mut prompt = last_prompt.to_string();
        if status == "running" && prompt.len() > 4000 {
            prompt = prompt.chars().take(4000).collect();
        }
        let ns = NamedSession {
            name: name.clone(),
            chat_id,
            status: status.to_string(),
            last_prompt: prompt,
        };
        map.insert(name, ns.clone());
        self.save(&map)?;
        Ok(ns)
    }

    pub fn update_session_status(&self, name: &str, status: &str, last_prompt: Option<&str>) -> Result<NamedSession, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let mut map = self.load()?;
        if let Some(s) = map.get_mut(name) {
            s.status = status.to_string();
            if let Some(p) = last_prompt {
                s.last_prompt = p.to_string();
            }
            if s.status == "running" && s.last_prompt.len() > 4000 {
                s.last_prompt = s.last_prompt.chars().take(4000).collect();
            }
            let cloned = s.clone();
            self.save(&map)?;
            Ok(cloned)
        } else {
            Err(format!("Named session not found: {}", name))
        }
    }

    pub fn get_session(&self, name: &str) -> Result<Option<NamedSession>, String> {
        let _guard = self.lock.lock().map_err(|e| e.to_string())?;
        let map = self.load()?;
        Ok(map.get(name).cloned())
    }
}
