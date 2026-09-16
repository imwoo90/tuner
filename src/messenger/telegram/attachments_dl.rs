//! # Telegram Attachment Direct Downloader
//!
//! Handles direct downloading of referenced files via Telegram inline callback buttons.
//! Packages multiple referenced files into a single compressed ZIP archive to avoid chat flooding,
//! and ensures documents are delivered to the originating forum topic rather than the General topic.
//!
//! ## Collaboration Graph
//! - [`handle_callback_query`](super::callbacks::handle_callback_query): Dispatches `dl_files:<token>` to this module.
//! - [`ReviewManager`](super::review::ReviewManager): Provides cached file metadata by token.
//!
//! ## Search Tags
//! #attachments, #zip-compression, #direct-download, #topic-routing

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use teloxide::prelude::*;
use super::review::{global_review_manager, ReviewFile};

/// Compresses a slice of `ReviewFile` items into an in-memory ZIP archive.
/// Deduplicates entry names if multiple files share identical basenames.
pub(crate) fn create_zip_archive(files: &[ReviewFile]) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buf);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut used_names = HashSet::new();
    for f in files {
        let p = Path::new(&f.path);
        let content = std::fs::read(p).unwrap_or_else(|_| f.content.as_bytes().to_vec());

        let mut entry_name = f.filename.clone();
        if entry_name.is_empty() {
            entry_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
        }

        if used_names.contains(&entry_name) {
            let stem = Path::new(&entry_name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
            let ext = Path::new(&entry_name)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            let mut counter = 2;
            loop {
                let candidate = format!("{}_{}{}", stem, counter, ext);
                if !used_names.contains(&candidate) {
                    entry_name = candidate;
                    break;
                }
                counter += 1;
            }
        }
        used_names.insert(entry_name.clone());

        zip.start_file(&entry_name, options)
            .map_err(|e| format!("Failed to create zip entry {}: {}", entry_name, e))?;
        zip.write_all(&content)
            .map_err(|e| format!("Failed to write zip content for {}: {}", entry_name, e))?;
    }

    zip.finish().map_err(|e| format!("Failed to finalize zip archive: {}", e))?;
    Ok(buf.into_inner())
}

/// Handles the `dl_files:<token>` inline button callback.
/// Delivers files directly to the calling forum topic (`msg.thread_id`).
async fn send_single_attachment(bot: &teloxide::Bot, msg: &teloxide::types::Message, path: PathBuf) {
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 50 * 1024 * 1024 {
            let text = "⚠️ 파일 크기가 50MB를 초과하여 텔레그램으로 직접 전송할 수 없습니다. 웹 뷰어에서 스트리밍 및 개별 다운로드하세요.";
            let mut req = bot.send_message(msg.chat.id, text);
            if let Some(tid) = msg.thread_id {
                req = req.message_thread_id(tid);
            }
            let _ = req.await;
            return;
        }
    }
    let mut req = bot.send_document(msg.chat.id, teloxide::types::InputFile::file(&path));
    if let Some(tid) = msg.thread_id {
        req = req.message_thread_id(tid);
    }
    let _ = req.await;
}

async fn send_multi_attachments(
    bot: &teloxide::Bot,
    msg: &teloxide::types::Message,
    valid: Vec<ReviewFile>,
) {
    match create_zip_archive(&valid) {
        Ok(zip_bytes) => {
            if zip_bytes.len() > 50 * 1024 * 1024 {
                let text = "⚠️ 압축 파일 크기가 50MB를 초과하여 텔레그램으로 전송할 수 없습니다. 웹 뷰어에서 개별 다운로드하세요.";
                let mut req = bot.send_message(msg.chat.id, text);
                if let Some(tid) = msg.thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await;
                return;
            }
            let input_file = teloxide::types::InputFile::memory(zip_bytes).file_name("attachments.zip");
            let mut req = bot.send_document(msg.chat.id, input_file);
            if let Some(tid) = msg.thread_id {
                req = req.message_thread_id(tid);
            }
            let _ = req.await;
        }
        Err(err) => {
            eprintln!("❌ Failed to create zip for download: {}", err);
            for f in valid {
                let path = PathBuf::from(&f.path);
                let mut req = bot.send_document(msg.chat.id, teloxide::types::InputFile::file(&path));
                if let Some(tid) = msg.thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await;
            }
        }
    }
}

pub(crate) async fn handle_dl_files_callback(
    bot: &teloxide::Bot,
    msg: &teloxide::types::Message,
    token: &str,
) {
    let mgr = global_review_manager();
    let files = match mgr.get_files(token).await {
        Some(f) if !f.is_empty() => f,
        _ => return,
    };

    let valid: Vec<_> = files
        .into_iter()
        .filter(|f| Path::new(&f.path).is_file())
        .collect();

    if valid.is_empty() {
        return;
    }

    if valid.len() == 1 {
        let path = PathBuf::from(&valid[0].path);
        send_single_attachment(bot, msg, path).await;
    } else {
        send_multi_attachments(bot, msg, valid).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_zip_archive_success() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("first.txt");
        let file2 = dir.path().join("second.txt");
        std::fs::write(&file1, b"hello from first").unwrap();
        std::fs::write(&file2, b"hello from second").unwrap();

        let review_files = vec![
            ReviewFile {
                filename: "first.txt".to_string(),
                path: file1.to_string_lossy().to_string(),
                content: "hello from first".to_string(),
                size_bytes: 16,
                language: "text".to_string(),
                is_binary: false,
            },
            ReviewFile {
                filename: "second.txt".to_string(),
                path: file2.to_string_lossy().to_string(),
                content: "hello from second".to_string(),
                size_bytes: 17,
                language: "text".to_string(),
                is_binary: false,
            },
        ];

        let zip_bytes = create_zip_archive(&review_files).expect("zip creation should succeed");
        assert!(!zip_bytes.is_empty());

        let reader = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(reader).expect("valid zip archive");
        assert_eq!(archive.len(), 2);

        {
            let mut entry1 = archive.by_name("first.txt").expect("first.txt exists");
            let mut content1 = String::new();
            std::io::Read::read_to_string(&mut entry1, &mut content1).unwrap();
            assert_eq!(content1, "hello from first");
        }

        {
            let mut entry2 = archive.by_name("second.txt").expect("second.txt exists");
            let mut content2 = String::new();
            std::io::Read::read_to_string(&mut entry2, &mut content2).unwrap();
            assert_eq!(content2, "hello from second");
        }
    }

    #[test]
    fn test_create_zip_archive_duplicate_names() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("sub1").join("common.txt");
        let file2 = dir.path().join("sub2").join("common.txt");
        std::fs::create_dir_all(file1.parent().unwrap()).unwrap();
        std::fs::create_dir_all(file2.parent().unwrap()).unwrap();
        std::fs::write(&file1, b"common 1").unwrap();
        std::fs::write(&file2, b"common 2").unwrap();

        let review_files = vec![
            ReviewFile {
                filename: "common.txt".to_string(),
                path: file1.to_string_lossy().to_string(),
                content: "common 1".to_string(),
                size_bytes: 8,
                language: "text".to_string(),
                is_binary: false,
            },
            ReviewFile {
                filename: "common.txt".to_string(),
                path: file2.to_string_lossy().to_string(),
                content: "common 2".to_string(),
                size_bytes: 8,
                language: "text".to_string(),
                is_binary: false,
            },
        ];

        let zip_bytes = create_zip_archive(&review_files).expect("zip creation should succeed");
        let reader = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(reader).expect("valid zip archive");
        assert_eq!(archive.len(), 2);

        assert!(archive.by_name("common.txt").is_ok());
        assert!(archive.by_name("common_2.txt").is_ok());
    }
}
