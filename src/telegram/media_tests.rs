#[cfg(test)]
mod tests {
    use crate::telegram::{handle_message, TEST_ENV_MUTEX};
    use crate::telegram::test_helpers::helpers::*;
    use crate::session::key::SessionKey as SK;
    use crate::session::manager::SessionManager as SM;
    use std::sync::Arc;
    use teloxide::Bot;

    const MOCK_AGY_SCRIPT: &str = r#"#!/usr/bin/env python3
import sys
import os
import json

home = os.environ.get("HOME", ".")
log_path = os.path.join(home, "received_prompts.txt")

with open(log_path, "a") as f:
    f.write(f"ARGS: {' '.join(sys.argv[1:])}\n")

session_id = None
for i, arg in enumerate(sys.argv):
    if arg == "--conversation" and i + 1 < len(sys.argv):
        session_id = sys.argv[i+1]

sys.stdout.write("\n> ")
sys.stdout.flush()

buffer = []
while True:
    char = sys.stdin.read(1)
    if not char:
        break
    buffer.append(char)
    if char in ('\r', '\n'):
        line = "".join(buffer).strip()
        if line:
            with open(log_path, "a") as f:
                f.write(f"PROMPT_STDIN: {line}\n")
            if session_id:
                transcript_path = os.path.join(
                    home, ".gemini", "antigravity-cli", "brain",
                    session_id, ".system_generated", "logs", "transcript_full.jsonl"
                )
                if os.path.exists(os.path.dirname(transcript_path)):
                    completion_entry = {"source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "tool_calls": []}
                    with open(transcript_path, "a") as tf:
                        tf.write(json.dumps(completion_entry) + "\n")
            sys.stdout.write('{"result":"ok","is_error":false}\n')
            sys.stdout.write("\n> ")
            sys.stdout.flush()
        buffer = []
"#;

    fn setup_env() -> (
        Arc<SM>,
        Arc<crate::config::CliConfig>,
        Arc<crate::cli::antigravity::AntigravityCli>,
        Bot,
        Arc<crate::cron::manager::CronManager>,
        Arc<crate::telegram::TopicNameCache>,
        Arc<crate::telegram::BotInfo>,
        Arc<crate::telegram::media_group::MediaGroupManager>,
        EnvGuard,
        tempfile::TempDir,
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, guard) = setup_e2e_env(&temp_dir, MOCK_AGY_SCRIPT);
        (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, guard, temp_dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_media_scenario_b_pending_silent() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env, temp) = setup_env();
        let key = SK::telegram(123, None);
        mock_brain_dir(&temp, "conv-xyz");
        
        let msg = make_msg(r#"{"message_id":101,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-123","file_unique_id":"unique-123","file_name":"test_doc.pdf"}}"#);
        handle_message(bot.clone(), msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        let sess = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(sess.pending_attachments.len(), 1);
        assert_eq!(sess.pending_attachments[0], "telegram_files/mock_media_101.pdf");

        let text_msg = make_msg(r#"{"message_id":102,"date":123457,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"Check files"}"#);

        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated, 0.0, 0).await.unwrap();

        handle_message(bot, text_msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let sess_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(sess_after.pending_attachments.is_empty());

        let prompt_log = wait_for_prompt(&temp, "Check files").await;
        assert!(prompt_log.contains("telegram_files/mock_media_101.pdf"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("Check files"), "prompt_log is: {:?}", prompt_log);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_media_scenario_c_captioned_immediate() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env, temp) = setup_env();
        let key = SK::telegram(123, None);
        mock_brain_dir(&temp, "conv-xyz");

        let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated, 0.0, 0).await.unwrap();

        let msg = make_msg(r#"{"message_id":103,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-123","file_unique_id":"unique-123","file_name":"test_doc.pdf"},"caption":"Analyze this file"}"#);

        let res = handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await;
        assert!(res.is_ok());

        let sess_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(sess_after.pending_attachments.is_empty());

        let prompt_log = wait_for_prompt(&temp, "Analyze this file").await;
        assert!(prompt_log.contains("telegram_files/mock_media_103.pdf"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("Analyze this file"), "prompt_log is: {:?}", prompt_log);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_media_group_scenario_a_debounce() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env, temp) = setup_env();
        let key = SK::telegram(123, None);
        mock_brain_dir(&temp, "conv-xyz");

        let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated, 0.0, 0).await.unwrap();

        let msg1 = make_msg(r#"{"message_id":104,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-1","file_unique_id":"unique-1","file_name":"file1.pdf"},"media_group_id":"group-999"}"#);
        let msg2 = make_msg(r#"{"message_id":105,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-2","file_unique_id":"unique-2","file_name":"file2.pdf"},"media_group_id":"group-999","caption":"Analyze album"}"#);

        handle_message(bot.clone(), msg1, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();
        handle_message(bot, msg2, cfg, mgr, cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let prompt_log = wait_for_prompt(&temp, "Analyze album").await;
        assert!(prompt_log.contains("telegram_files/mock_media_104.pdf"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("telegram_files/mock_media_105.pdf"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("Analyze album"), "prompt_log is: {:?}", prompt_log);
    }
}
