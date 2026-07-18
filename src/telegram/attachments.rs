//! # Telegram File Attachments Handler
//!
//! Scans bot responses for `file://` links, validates their safety
//! against allowed directory roots, and automatically sends them
//! as Telegram document attachments.

use teloxide::prelude::*;
use crate::config::CliConfig;
use std::path::PathBuf;

/// Scans text for `file://` URLs, validates that they are files within allowed roots,
/// and returns a list of unique safe file paths.
pub fn extract_file_paths(text: &str, allowed_roots: &[PathBuf]) -> Vec<PathBuf> {
    let re = match regex::Regex::new(r"file://[^\s\)]+") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut paths = Vec::new();
    for cap in re.captures_iter(text) {
        let url_str = cap[0].to_string();
        if let Ok(parsed_url) = url::Url::parse(&url_str) {
            if let Ok(path) = parsed_url.to_file_path() {
                if path.is_file() && crate::security::paths::is_path_safe(&path, allowed_roots) {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
    }
    paths
}

/// Helper task to send any matching file links as document attachments.
pub(crate) async fn send_file_attachments(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    raw_text: &str,
    config: &CliConfig,
) {
    let allowed_roots = vec![
        config.working_dir.clone(),
        PathBuf::from("/home/wimvm/.tuner"),
        PathBuf::from("/home/wimvm/tuner"),
    ];
    let file_paths = extract_file_paths(raw_text, &allowed_roots);
    for path in file_paths {
        let mut req = bot.send_document(chat_id, teloxide::types::InputFile::file(&path));
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        if let Err(e) = req.await {
            eprintln!("Failed to send document {:?}: {:?}", path, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_extract_file_paths_valid_and_invalid() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("test_file_1.rs");
        std::fs::write(&file1, "content").unwrap();

        let allowed_roots = vec![dir.path().to_path_buf()];

        // Test absolute file:// link
        let text = format!("Please review [test_file_1.rs](file://{}) and some invalid [nonexistent](file://{})", file1.to_string_lossy(), dir.path().join("nonexistent.rs").to_string_lossy());
        let paths = extract_file_paths(&text, &allowed_roots);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], file1);

        // Test file:// link with line numbers fragment
        let text_frag = format!("Error at [line](file://{}#L10-L20)", file1.to_string_lossy());
        let paths_frag = extract_file_paths(&text_frag, &allowed_roots);
        assert_eq!(paths_frag.len(), 1);
        assert_eq!(paths_frag[0], file1);
    }
}
