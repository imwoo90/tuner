//! # Attachment Downloader and Storage Manager
//!
//! ## Overview
//! Checks incoming Telegram messages for files, images, or audio clips. Downloads them via HTTP
//! and stores them inside the workspace's `telegram_files/` directory.
//!
//! ## Search Tags
//! #attachment-downloads, #media-storage, #file-receiver

use teloxide::prelude::*;
use crate::config::CliConfig;
use std::path::PathBuf;

/// Scans text for `file://` URLs and Markdown links/image embeds with absolute paths,
/// validates that they are files within allowed roots, and returns a list of unique safe file paths.
pub fn extract_file_paths(text: &str, allowed_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen_canonical = std::collections::HashSet::new();
    let mut push_if_valid = |path: PathBuf| {
        if path.is_file() && crate::security::paths::is_path_safe(&path, allowed_roots) {
            let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen_canonical.insert(canon) {
                paths.push(path);
            }
        }
    };

    if let Ok(re) = regex::Regex::new(r#"file://[^\s\)\]`"'>]+"#) {
        for cap in re.captures_iter(text) {
            let raw = cap[0].trim_end_matches(&['.', ',', ';', '"', '\'', '>', ')', '`', ']'][..]);
            if let Ok(parsed_url) = url::Url::parse(raw)
                && let Ok(path) = parsed_url.to_file_path()
            {
                push_if_valid(path);
            }
        }
    }

    if let Ok(md_re) = regex::Regex::new(r"!?\[[^\]]*\]\((/[^)]+)\)") {
        for cap in md_re.captures_iter(text) {
            let raw_target = cap[1].split('#').next().unwrap_or("").trim();
            if raw_target.starts_with('/') && !raw_target.starts_with("//") {
                push_if_valid(PathBuf::from(raw_target));
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

fn build_allowed_roots(config: &CliConfig) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home_path = PathBuf::from(&home);
    let mut allowed_roots = vec![
        config.working_dir.clone(),
        home_path.join(".tuner"),
        home_path.join(".tuner-dev"),
        home_path.join("tuner"),
        home_path.join(".gemini/antigravity-cli"),
    ];
    if let Ok(th) = std::env::var("TUNER_HOME") {
        let th_path = PathBuf::from(th);
        if !allowed_roots.contains(&th_path) {
            allowed_roots.push(th_path);
        }
    }
    if let Ok(cwd) = std::env::current_dir()
        && !allowed_roots.contains(&cwd) {
            allowed_roots.push(cwd);
        }
    let ws_path = PathBuf::from("/workspace");
    if ws_path.is_dir() && !allowed_roots.contains(&ws_path) {
        allowed_roots.push(ws_path);
    }
    allowed_roots
}

/// Helper task to register matching file and image links in the In-App review viewer.
pub(crate) async fn send_file_attachments(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    raw_text: &str,
    config: &CliConfig,
) {
    let allowed_roots = build_allowed_roots(config);
    let review_files: Vec<PathBuf> = extract_file_paths(raw_text, &allowed_roots)
        .into_iter()
        .filter(|path| !is_blacklisted_file(path))
        .collect();

    if !review_files.is_empty() {
        let mgr = super::review::global_review_manager();
        if let Some((session_id, count)) = mgr.register_session(&review_files).await {
            send_review_button(bot, chat_id, thread_id, &session_id, count).await;
        }
    }
}

async fn send_review_button(
    bot: &Bot,
    chat_id: ChatId,
    thread_id: Option<i32>,
    session_id: &str,
    count: usize,
) {
    use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};
    let review_btn = InlineKeyboardButton::callback(
        format!("🔍 참조 파일 {}개 검토", count),
        format!("review:{}", session_id),
    );
    let download_btn = InlineKeyboardButton::callback("📥 직접 받기", format!("dl_files:{}", session_id));
    let keyboard = InlineKeyboardMarkup::new(vec![vec![review_btn, download_btn]]);

    let text = format!("📁 <b>답변에서 {}개의 파일이 참조되었습니다.</b>", count);

    let mut msg_req = bot
        .send_message(chat_id, text)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_markup(keyboard);
    if let Some(tid) = thread_id {
        msg_req = msg_req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(tid)));
    }
    if let Err(e) = msg_req.await {
        eprintln!("❌ Failed to send review button: {:?}", e);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

        // Test inline code backtick and bracket cleanup for explicit file:// URLs,
        // while ignoring bare backtick filenames/relative paths without file://
        let text_code = format!("Inline `file://{}` and [file://{}] and `test_file_1.rs`", file1.to_string_lossy(), file1.to_string_lossy());
        let paths_code = extract_file_paths(&text_code, &allowed_roots);
        assert_eq!(paths_code.len(), 1);
        assert_eq!(paths_code[0], file1);
    }

    #[test]
    fn test_extract_file_paths_carousel_and_markdown_images() {
        let dir = tempdir().unwrap();
        let allowed_roots = vec![dir.path().to_path_buf()];
        let img1 = dir.path().join("render_iso.png");
        let img2 = dir.path().join("render_top.png");
        std::fs::write(&img1, "png1").unwrap();
        std::fs::write(&img2, "png2").unwrap();
        let carousel_text = format!(
            "````carousel\n![Iso View]({})\n<!-- slide -->\n![Top View]({})\n````\nAlso relative ![rel](render_iso.png) and duplicate [iso](file://{})",
            img1.to_string_lossy(),
            img2.to_string_lossy(),
            img1.to_string_lossy()
        );
        let paths_carousel = extract_file_paths(&carousel_text, &allowed_roots);
        assert_eq!(paths_carousel.len(), 2);
        assert!(paths_carousel.contains(&img1));
        assert!(paths_carousel.contains(&img2));
    }
}
