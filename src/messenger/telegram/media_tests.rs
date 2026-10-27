#![allow(clippy::await_holding_lock, clippy::type_complexity)]
#[cfg(test)]
mod tests {
    use crate::telegram::TEST_ENV_MUTEX;
    use crate::telegram::test_helpers::helpers::*;
    use crate::session::key::SessionKey as SK;
    use crate::session::manager::SessionManager as SM;
    use std::sync::Arc;
    use teloxide::Bot;

    #[allow(clippy::too_many_arguments)]
    async fn handle_message(
        bot: Bot,
        msg: teloxide::types::Message,
        config: Arc<crate::config::CliConfig>,
        sessions: Arc<SM>,
        cli: Arc<crate::cli::antigravity::AntigravityCli>,
        cron_manager: Arc<crate::cron::manager::CronManager>,
        topic_cache: Arc<crate::telegram::TopicNameCache>,
        bot_info: Arc<crate::telegram::BotInfo>,
        media_group_manager: Arc<crate::telegram::media_group::MediaGroupManager>,
    ) -> Result<(), teloxide::RequestError> {
        crate::telegram::handle_message(
            bot,
            msg,
            config,
            sessions,
            cli,
            (cron_manager, topic_cache),
            (bot_info, media_group_manager),
        )
        .await
    }

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
        assert_eq!(sess.pending_attachments[0], "telegram_files/mock_media_101_test_doc.pdf");

        let text_msg = make_msg(r#"{"message_id":102,"date":123457,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"Check files"}"#);

        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated).await.unwrap();

        handle_message(bot, text_msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let sess_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(sess_after.pending_attachments.is_empty());

        let prompt_log = wait_for_prompt(&temp, "Check files").await;
        assert!(prompt_log.contains("telegram_files/mock_media_101_test_doc.pdf"), "prompt_log is: {:?}", prompt_log);
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
        mgr.update_session(&updated).await.unwrap();

        let msg = make_msg(r#"{"message_id":103,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-123","file_unique_id":"unique-123","file_name":"test_doc.pdf"},"caption":"Analyze this file"}"#);

        let res = handle_message(bot, msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await;
        assert!(res.is_ok());

        let sess_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(sess_after.pending_attachments.is_empty());

        let prompt_log = wait_for_prompt(&temp, "Analyze this file").await;
        assert!(prompt_log.contains("telegram_files/mock_media_103_test_doc.pdf"), "prompt_log is: {:?}", prompt_log);
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
        mgr.update_session(&updated).await.unwrap();

        let msg1 = make_msg(r#"{"message_id":104,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-1","file_unique_id":"unique-1","file_name":"file1.pdf"},"media_group_id":"group-999"}"#);
        let msg2 = make_msg(r#"{"message_id":105,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-2","file_unique_id":"unique-2","file_name":"file2.pdf"},"media_group_id":"group-999","caption":"Analyze album"}"#);

        handle_message(bot.clone(), msg1, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();
        handle_message(bot, msg2, cfg, mgr, cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let prompt_log = wait_for_prompt(&temp, "Analyze album").await;
        assert!(prompt_log.contains("telegram_files/mock_media_104_file1.pdf"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("telegram_files/mock_media_105_file2.pdf"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("Analyze album"), "prompt_log is: {:?}", prompt_log);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_voice_note_immediate_turn_execution() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env, temp) = setup_env();
        let key = SK::telegram(123, None);
        mock_brain_dir(&temp, "conv-xyz");

        let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated).await.unwrap();

        let voice_msg = make_msg(r#"{"message_id":106,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"voice":{"file_id":"voice-1","file_unique_id":"u-voice-1","duration":5,"mime_type":"audio/ogg"}}"#);
        let res = handle_message(bot, voice_msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await;
        assert!(res.is_ok());

        let sess_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(sess_after.pending_attachments.is_empty());

        let prompt_log = wait_for_prompt(&temp, "Voice message attached").await;
        assert!(prompt_log.contains("telegram_files/mock_media_106.ogg"), "prompt_log is: {:?}", prompt_log);
        assert!(prompt_log.contains("Voice message attached"), "prompt_log is: {:?}", prompt_log);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pending_attachments_survive_commands_and_bare_model_directive() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env, temp) = setup_env();
        let key = SK::telegram(123, None);
        mock_brain_dir(&temp, "conv-xyz");

        let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated).await.unwrap();

        let doc_msg = make_msg(r#"{"message_id":107,"date":123456,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-7","file_unique_id":"unique-7","file_name":"queued.pdf"}}"#);
        handle_message(bot.clone(), doc_msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        let status_msg = make_msg(r#"{"message_id":108,"date":123457,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"/status"}"#);
        handle_message(bot.clone(), status_msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        let bare_model_msg = make_msg(r#"{"message_id":109,"date":123458,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"@opus"}"#);
        handle_message(bot.clone(), bare_model_msg, cfg.clone(), mgr.clone(), cli.clone(), cron_mgr.clone(), topic_cache.clone(), bot_info.clone(), mgm.clone()).await.unwrap();

        let sess_mid = mgr.get_active(&key).await.unwrap().unwrap();
        assert_eq!(sess_mid.pending_attachments, vec!["telegram_files/mock_media_107_queued.pdf".to_string()]);

        let prompt_msg = make_msg(r#"{"message_id":110,"date":123459,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"Summarize queued file"}"#);
        handle_message(bot, prompt_msg, cfg, mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let sess_after = mgr.get_active(&key).await.unwrap().unwrap();
        assert!(sess_after.pending_attachments.is_empty());

        let prompt_log = wait_for_prompt(&temp, "Summarize queued file").await;
        assert!(prompt_log.contains("telegram_files/mock_media_107_queued.pdf"), "prompt_log is: {:?}", prompt_log);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reply_to_media_injects_file_and_cron_transport_logs_history() {
        let _guard = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let (mgr, cfg, cli, bot, cron_mgr, topic_cache, bot_info, mgm, _env, temp) = setup_env();
        let key = SK::telegram(123, None);
        mock_brain_dir(&temp, "conv-xyz");

        let (sess, _) = mgr.resolve_session(&key, &cfg.provider, "opus").await.unwrap();
        let mut updated = sess.clone();
        updated.set_session_id("antigravity", "conv-xyz");
        mgr.update_session(&updated).await.unwrap();

        // Pre-seed a cached file on disk for message_id 90 to verify 0ms local cache hit on reply
        let tg_dir = cfg.working_dir.join("telegram_files");
        std::fs::create_dir_all(&tg_dir).unwrap();
        std::fs::write(tg_dir.join("media_123_90_schematic_v2.pdf"), "cached-pdf").unwrap();

        let reply_msg = make_msg(r#"{"message_id":111,"date":123460,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"text":"Inspect this schematic","reply_to_message":{"message_id":90,"date":123400,"chat":{"id":123,"type":"private"},"from":{"id":100,"is_bot":false,"first_name":"I"},"document":{"file_id":"file-90","file_unique_id":"u-90","file_name":"schematic_v2.pdf"}}}"#);
        handle_message(bot.clone(), reply_msg, cfg.clone(), mgr.clone(), cli, cron_mgr, topic_cache, bot_info, mgm).await.unwrap();

        let prompt_log = wait_for_prompt(&temp, "Inspect this schematic").await;
        assert!(prompt_log.contains("telegram_files/media_123_90_schematic_v2.pdf"), "prompt_log is: {:?}", prompt_log);

        assert_eq!(crate::messenger::telegram::media::sanitize_filename("../../Makefile"), "Makefile");
        assert_eq!(crate::messenger::telegram::media::sanitize_filename("my bom (v2).csv"), "my_bom_v2_.csv");

        // Verify TelegramTransport::deliver with_context logs cron output into topic history
        use crate::bus::bus::Transport;
        let transport = crate::telegram::TelegramTransport::with_targets(bot, vec![123])
            .with_context(cfg.clone(), mgr.clone());
        let env = crate::bus::adapters::from_cron_result("Daily Quiz", "Cron quiz payload 123", "success", Some(123), None, Some("tg"));
        transport.deliver(&env).await.unwrap();
        let err_env = crate::bus::adapters::from_cron_result("Daily Quiz", "Cron error 500", "error:exit_1", Some(123), None, Some("tg"));
        transport.deliver(&err_env).await.unwrap();

        let entries = crate::messenger::telegram::handover_helpers::extract_topic_history(&cfg.working_dir, "conv-xyz");
        assert!(entries.iter().any(|e| e.sender == "bot" && e.text.contains("Cron quiz payload 123")));
        assert!(!entries.iter().any(|e| e.text.contains("Cron error 500")));
    }
}
