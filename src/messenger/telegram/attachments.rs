//! # Attachment Downloader and Storage Manager
//!
//! ## Overview
//! Checks incoming Telegram messages for files, images, or audio clips. Downloads them via HTTP
//! and stores them inside the workspace's `telegram_files/` directory.
//!
//! ## Collaboration Graph
//! - Integrates with [`process_text_with_files`](super::process_text_with_files) to inject paths into agent inputs.
//!
//! ## Search Tags
//! #attachment-downloads, #media-storage, #file-receiver

use teloxide::prelude::*;
use crate::config::CliConfig;
use std::path::PathBuf;

/// Scans text strictly for `file://` URLs, validates that they are files within allowed roots,
/// and returns a list of unique safe file paths.
pub fn extract_file_paths(text: &str, allowed_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(re) = regex::Regex::new(r"file://[^\s\)]+") {
        for cap in re.captures_iter(text) {
            let raw = cap[0].trim_end_matches(&['.', ',', ';', '"', '\'', '>', ')'][..]);
            if let Ok(parsed_url) = url::Url::parse(raw) {
                if let Ok(path) = parsed_url.to_file_path() {
                    if path.is_file() && crate::security::paths::is_path_safe(&path, allowed_roots) {
                        if !paths.contains(&path) {
                            paths.push(path);
                        }
                    }
                }
            }
        }
    }
    paths
}

fn is_blacklisted_file(path: &std::path::Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name == "Cargo.lock" || name == "package-lock.json" || name == "yarn.lock" {
        return true;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "log" || ext == "jsonl" || ext == "lock" {
        return true;
    }
    let p_str = path.to_string_lossy();
    if p_str.contains("/target/") || p_str.contains("/.git/") || p_str.contains("/node_modules/") {
        return true;
    }
    false
}

fn is_image_file(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// Helper task to send matching file links as In-App review buttons or photos.
pub(crate) async fn send_file_attachments(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    raw_text: &str,
    config: &CliConfig,
) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home_path = PathBuf::from(&home);
    let mut allowed_roots = vec![
        config.working_dir.clone(),
        home_path.join(".tuner"),
        home_path.join("tuner"),
        home_path.join(".gemini/antigravity-cli"),
    ];
    if let Ok(cwd) = std::env::current_dir() {
        if !allowed_roots.contains(&cwd) {
            allowed_roots.push(cwd);
        }
    }
    let ws_path = PathBuf::from("/workspace");
    if ws_path.is_dir() && !allowed_roots.contains(&ws_path) {
        allowed_roots.push(ws_path);
    }
    let file_paths = extract_file_paths(raw_text, &allowed_roots);
    let mut images = Vec::new();
    let mut code_files = Vec::new();

    for path in file_paths {
        if is_blacklisted_file(&path) {
            continue;
        }
        if is_image_file(&path) {
            images.push(path);
        } else {
            code_files.push(path);
        }
    }

    for img in images {
        let mut req = bot.send_photo(chat_id, teloxide::types::InputFile::file(&img));
        if let Some(tid) = thread_id {
            req = req.message_thread_id(tid);
        }
        let _ = req.await;
    }

    if !code_files.is_empty() {
        let mgr = super::review::global_review_manager();
        if let Some((token, count)) = mgr.create_session(&code_files).await {
            let review_url = mgr.get_review_url(&token).await;
            send_review_button(bot, chat_id, thread_id, &token, count, &review_url).await;
        }
    }
}

async fn send_review_button(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    token: &str,
    count: usize,
    review_url: &str,
) {
    use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, WebAppInfo};
    let btn_text = format!("🔍 참조 파일 {}개 검토 (In-App)", count);
    let review_btn = if review_url.starts_with("https://") {
        if let Ok(parsed_url) = review_url.parse() {
            InlineKeyboardButton::web_app(btn_text, WebAppInfo { url: parsed_url })
        } else {
            InlineKeyboardButton::callback(btn_text, format!("dl_files:{}", token))
        }
    } else {
        InlineKeyboardButton::callback(btn_text, format!("dl_files:{}", token))
    };

    let download_btn = InlineKeyboardButton::callback("📥 직접 받기", format!("dl_files:{}", token));
    let keyboard = InlineKeyboardMarkup::new(vec![vec![review_btn, download_btn]]);

    let text = if review_url.starts_with("https://") {
        format!("📁 <b>답변에서 {}개의 파일이 참조되었습니다.</b>", count)
    } else {
        format!(
            "📁 <b>답변에서 {}개의 파일이 참조되었습니다.</b>\n(로컬 뷰어: {})",
            count, review_url
        )
    };

    let mut msg_req = bot.send_message(chat_id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        msg_req = msg_req.message_thread_id(tid);
    }
    if let Err(e) = msg_req.await {
        eprintln!("❌ Failed to send review button: {:?}", e);
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

        // Test that relative markdown links without file:// are ignored
        let text_rel = "Check [source](test_file_1.rs) for implementation";
        let paths_rel = extract_file_paths(text_rel, &allowed_roots);
        assert_eq!(paths_rel.len(), 0);

        // Test trailing punctuation cleanup
        let text_punct = format!("Refer to file://{}.", file1.to_string_lossy());
        let paths_punct = extract_file_paths(&text_punct, &allowed_roots);
        assert_eq!(paths_punct.len(), 1);
        assert_eq!(paths_punct[0], file1);
    }
}
