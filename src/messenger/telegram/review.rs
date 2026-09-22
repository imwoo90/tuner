//! # In-App File Review Manager
//!
//! Provides ephemeral, secure file snapshot sessions for Telegram WebApp and mobile file review.
//! Captures referenced code, text, images, video, and audio files, preparing sessions for the
//! mobile-responsive dark-mode viewer and direct download.

use super::review_store::{
    check_record_files, default_storage_path, load_records, save_records, ReviewSessionRecord,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReviewFile {
    pub filename: String,
    pub path: String,
    pub content: String,
    pub size_bytes: usize,
    pub language: String,
    #[serde(default)]
    pub is_binary: bool,
}

const SESSION_TTL: Duration = Duration::from_secs(1800); // 30 minutes

struct SessionEntry {
    created_at: Instant,
    files: Vec<ReviewFile>,
}

pub struct ReviewManager {
    sessions: Arc<Mutex<HashMap<String, SessionEntry>>>,
    records: Arc<Mutex<HashMap<String, ReviewSessionRecord>>>,
    storage_path: Arc<Mutex<PathBuf>>,
    tunnel_url: Arc<Mutex<Option<String>>>,
    server_port: Arc<Mutex<Option<u16>>>,
}

static INSTANCE: OnceLock<Arc<ReviewManager>> = OnceLock::new();

pub fn global_review_manager() -> Arc<ReviewManager> {
    INSTANCE.get_or_init(|| Arc::new(ReviewManager::new())).clone()
}

impl ReviewManager {
    pub fn new() -> Self {
        let path = default_storage_path();
        let loaded = load_records(&path);
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            records: Arc::new(Mutex::new(loaded)),
            storage_path: Arc::new(Mutex::new(path)),
            tunnel_url: Arc::new(Mutex::new(None)),
            server_port: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn init_storage(&self, path: PathBuf) {
        let loaded = load_records(&path);
        let mut recs = self.records.lock().await;
        for (k, v) in loaded {
            recs.insert(k, v);
        }
        let mut p = self.storage_path.lock().await;
        *p = path;
    }

    pub async fn register_session(&self, paths: &[PathBuf]) -> Option<(String, usize)> {
        let valid_paths: Vec<String> = paths
            .iter()
            .filter(|p| p.is_file())
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        if valid_paths.is_empty() {
            return None;
        }

        let session_id = hex::encode(rand::random::<[u8; 16]>());
        let count = valid_paths.len();
        let record = ReviewSessionRecord {
            created_at_secs: super::review_store::current_unix_secs(),
            file_paths: valid_paths,
        };

        {
            let mut recs = self.records.lock().await;
            recs.insert(session_id.clone(), record);
            let p = self.storage_path.lock().await;
            save_records(&p, &recs);
        }

        Some((session_id, count))
    }

    pub async fn get_session_record(&self, session_id: &str) -> Option<ReviewSessionRecord> {
        let recs = self.records.lock().await;
        recs.get(session_id).cloned()
    }

    pub async fn activate_session(&self, paths: &[PathBuf]) -> Option<String> {
        let mut files = Vec::new();
        let mut used_names = HashSet::new();

        for path in paths {
            if let Some(file) = process_path_for_review(path, &mut used_names) {
                files.push(file);
            }
        }

        if files.is_empty() {
            return None;
        }

        let token = hex::encode(rand::random::<[u8; 16]>());
        let mut map = self.sessions.lock().await;
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.created_at) < SESSION_TTL);
        map.insert(token.clone(), SessionEntry { created_at: now, files });

        Some(token)
    }

    pub async fn create_session(&self, paths: &[PathBuf]) -> Option<(String, usize)> {
        let (_session_id, count) = self.register_session(paths).await?;
        let token = self.activate_session(paths).await?;
        Some((token, count))
    }

    pub async fn get_files(&self, token_or_id: &str) -> Option<Vec<ReviewFile>> {
        let mut map = self.sessions.lock().await;
        let now = Instant::now();
        map.retain(|_, v| now.duration_since(v.created_at) < SESSION_TTL);

        if let Some(entry) = map.get(token_or_id) {
            return Some(entry.files.clone());
        }

        drop(map);

        // Fallback: check if it's a persistent session_id
        let rec = self.get_session_record(token_or_id).await?;
        let (valid_paths, _) = check_record_files(&rec);
        if valid_paths.is_empty() {
            return None;
        }
        let token = self.activate_session(&valid_paths).await?;
        let map = self.sessions.lock().await;
        map.get(&token).map(|e| e.files.clone())
    }

    pub async fn get_file_path(&self, token: &str, filename: &str) -> Option<PathBuf> {
        let files = self.get_files(token).await?;
        for f in files {
            if f.filename == filename {
                let p = PathBuf::from(&f.path);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    pub async fn get_html(&self, token: &str) -> Option<String> {
        let files = self.get_files(token).await?;
        let json_data = serde_json::to_string(&files).unwrap_or_else(|_| "[]".to_string());
        let safe_json = json_data
            .replace("</script", "<\\/script")
            .replace("</SCRIPT", "<\\/SCRIPT");
        let template = include_str!("review_viewer.html");
        let rendered = template
            .replace("__FILES_JSON__", &safe_json)
            .replace("__TOKEN__", token);
        Some(rendered)
    }

    pub async fn ensure_server_running(&self) -> u16 {
        super::review_server::ensure_server_running(&self.server_port).await
    }

    pub async fn warmup(&self) {
        super::review_server::warmup(&self.server_port, &self.tunnel_url).await
    }

    pub async fn get_review_url(&self, token: &str) -> String {
        super::review_server::get_review_url(&self.server_port, &self.tunnel_url, token).await
    }
}

fn resolve_unique_filename(path: &StdPath, used_names: &mut HashSet<String>) -> String {
    let mut filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    if used_names.contains(&filename) {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e))
            .unwrap_or_default();
        let mut counter = 2;
        loop {
            let candidate = format!("{}_{}{}", stem, counter, ext);
            if !used_names.contains(&candidate) {
                filename = candidate;
                break;
            }
            counter += 1;
        }
    }
    used_names.insert(filename.clone());
    filename
}

fn process_path_for_review(path: &StdPath, used_names: &mut HashSet<String>) -> Option<ReviewFile> {
    if !path.is_file() {
        return None;
    }
    let filename = resolve_unique_filename(path, used_names);
    let language = detect_language(path);
    let metadata = std::fs::metadata(path).ok();
    let size_bytes = metadata.as_ref().map(|m| m.len() as usize).unwrap_or(0);

    let is_media = matches!(language.as_str(), "video" | "audio" | "image" | "pdf");
    let (content, is_binary) = if is_media {
        (String::new(), true)
    } else if size_bytes > 10_000_000 {
        (
            format!("// File exceeds 10MB ({} bytes). Download directly to inspect.", size_bytes),
            false,
        )
    } else {
        match std::fs::read_to_string(path) {
            Ok(text) => (text, false),
            Err(_) => (String::new(), true),
        }
    };

    Some(ReviewFile {
        filename,
        path: path.to_string_lossy().to_string(),
        size_bytes: if is_binary { size_bytes } else { content.len() },
        content,
        language,
        is_binary,
    })
}

pub fn detect_language(path: &StdPath) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "json" | "jsonl" => "json",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "sh" | "bash" => "bash",
        "html" | "htm" => "html",
        "css" => "css",
        "md" | "markdown" => "markdown",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => "video",
        "mp3" | "wav" | "ogg" | "m4a" | "flac" | "aac" => "audio",
        "png" | "jpg" | "jpeg" | "gif" | "webp" => "image",
        "svg" => "svg",
        "pdf" => "pdf",
        _ => "plaintext",
    }
    .to_string()
}
