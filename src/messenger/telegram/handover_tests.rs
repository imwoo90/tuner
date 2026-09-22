//! # Telegram Topic Handover Unit Tests
//!
//! ## Overview
//! Validates history extraction, filtering, summarization (objective, decisions, progress),
//! and prompt formatting for topic session handover.

#[cfg(test)]
mod tests {
    use super::super::handover::*;
    use crate::messenger::telegram::history::log_telegram_message;
    use tempfile::tempdir;
    use std::io::Write;

    #[test]
    fn test_extract_topic_history_missing_dir() {
        let tmp = tempdir().unwrap();
        let entries = extract_topic_history(tmp.path(), "non-existent-sid");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_extract_topic_history_filters_errors_and_commands() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "test-session-1";

        log_telegram_message(ws, sid, Some(10), Some("Dev"), "user", Some(1), "/new", true, None);
        log_telegram_message(ws, sid, Some(10), Some("Dev"), "user", Some(2), "/status@tuner_bot", true, None);
        log_telegram_message(ws, sid, Some(10), Some("Dev"), "bot", Some(3), "Failed api", false, Some("Network error"));
        log_telegram_message(ws, sid, Some(10), Some("Dev"), "user", Some(4), "Implement feature A", true, None);
        log_telegram_message(ws, sid, Some(10), Some("Dev"), "bot", Some(5), "Feature A implemented successfully", true, None);

        let entries = extract_topic_history(ws, sid);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "Implement feature A");
        assert_eq!(entries[1].text, "Feature A implemented successfully");
    }

    #[test]
    fn test_extract_topic_history_handles_invalid_utf8_and_partial_lines() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "session-corrupt";
        let target_dir = ws.join("brain").join(sid);
        std::fs::create_dir_all(&target_dir).unwrap();
        let file_path = target_dir.join("telegram_history.jsonl");

        let mut file = std::fs::File::create(&file_path).unwrap();
        let valid_entry = r#"{"timestamp":"2026-01-01T00:00:00Z","sender":"user","text":"Build robust parser","is_success":true}"#;
        writeln!(file, "{}", valid_entry).unwrap();
        // Write corrupted bytes
        file.write_all(&[0xff, 0xfe, 0xfd, b'\n']).unwrap();
        // Write partial JSON line without literal unclosed brace
        file.write_all(b"\x7b\"timestamp\":\"partial\n").unwrap();
        // Write another valid entry
        let valid_entry2 = r#"{"timestamp":"2026-01-01T00:01:00Z","sender":"bot","text":"Parser built","is_success":true}"#;
        writeln!(file, "{}", valid_entry2).unwrap();

        let entries = extract_topic_history(ws, sid);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "Build robust parser");
        assert_eq!(entries[1].text, "Parser built");
    }

    fn seed_test_history(ws: &std::path::Path, sid: &str) {
        let t = Some("RusTerm Monitoring");
        log_telegram_message(ws, sid, Some(42), t, "user", Some(10), "RusTerm 웹 시리얼 모니터링 기능 구현해줘", true, None);
        log_telegram_message(ws, sid, Some(42), t, "bot", Some(11), "WebSocket 스트리밍과 WebSerial 중 어떤 것을 쓸까요?", true, None);
        log_telegram_message(ws, sid, Some(42), t, "user", Some(12), "WebSocket 스트리밍 방식으로 결정하고 진행하자", true, None);
        log_telegram_message(ws, sid, Some(42), t, "bot", Some(13), "WebSocket 스트리밍 방식으로 결정했습니다. 백엔드 구현 완료.", true, None);
        log_telegram_message(ws, sid, Some(42), t, "user", Some(14), "프론트엔드 차트 컴포넌트 추가해줘", true, None);
        log_telegram_message(ws, sid, Some(42), t, "bot", Some(15), "차트 렌더링 컴포넌트 추가 완료했습니다. 60fps 출력 중입니다.", true, None);
    }

    #[test]
    fn test_build_topic_handover_success_flow() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "session-handover-test";

        seed_test_history(ws, sid);

        let handover = build_topic_handover(ws, sid, Some(42), Some("RusTerm Monitoring".to_string()), None, &[]).unwrap();

        assert_eq!(handover.topic_id, Some(42));
        assert_eq!(handover.topic_name, Some("RusTerm Monitoring".to_string()));
        assert_eq!(handover.previous_session_id, sid);
        assert!(handover.topic_objective.contains("RusTerm 웹 시리얼 모니터링 기능 구현해줘"));
        assert!(handover.agreed_decisions.iter().any(|d| d.contains("WebSocket 스트리밍 방식")));
        assert!(handover.current_progress.contains("차트 렌더링 컴포넌트 추가 완료했습니다"));
        assert!(handover.handover_prompt.contains("[TOPIC CONTEXT HANDOVER]"));
        assert!(handover.handover_prompt.contains("RusTerm Monitoring"));
    }

    #[test]
    fn test_build_topic_handover_retains_prior_summary_on_empty_session() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "empty-new-sid";

        let prior = "Objective: [Topic: RusTerm] Core Engine\nDecisions: WebSocket streaming; In-memory ring buffer\nProgress: Last assistant status: Ready";
        let handover = build_topic_handover(ws, sid, Some(42), Some("RusTerm".to_string()), Some(prior), &[]).unwrap();

        assert_eq!(handover.topic_objective, "[Topic: RusTerm] Core Engine");
        assert!(handover.agreed_decisions.contains(&"WebSocket streaming".to_string()));
        assert!(handover.agreed_decisions.contains(&"In-memory ring buffer".to_string()));
        assert!(handover.current_progress.contains("Ready"));
    }

    #[test]
    fn test_build_topic_handover_preserves_multiline_objective_with_greetings() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "session-greeting-obj";

        let multi = "안녕하세요!\nRusTerm 웹 시리얼 모니터링 기능 추가해주세요.";
        log_telegram_message(ws, sid, Some(10), Some("Dev"), "user", Some(1), multi, true, None);
        log_telegram_message(ws, sid, Some(10), Some("Dev"), "bot", Some(2), "작업을 시작합니다.", true, None);

        let handover = build_topic_handover(ws, sid, Some(10), Some("Dev".to_string()), None, &[]).unwrap();
        assert!(handover.topic_objective.contains("RusTerm 웹 시리얼 모니터링 기능 추가해주세요"));
    }

    #[test]
    fn test_agreed_decisions_ignores_markdown_headers_and_boilerplate() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "session-no-leak";

        log_telegram_message(ws, sid, Some(10), None, "user", Some(1), "Let's begin", true, None);
        let bot_msg = "\
### 2. Key Decisions & Agreed Specifications
1. Seamlessly continue the work in this topic without losing context.
- Actual technical decision: decided to use Tokio async runtime.
";
        log_telegram_message(ws, sid, Some(10), None, "bot", Some(2), bot_msg, true, None);

        let handover = build_topic_handover(ws, sid, Some(10), None, None, &[]).unwrap();
        for dec in &handover.agreed_decisions {
            assert!(!dec.starts_with('#'), "Markdown header leaked: {}", dec);
            assert!(!dec.contains("Seamlessly continue"), "Instruction leaked: {}", dec);
        }
        assert!(handover.agreed_decisions.iter().any(|d| d.contains("Tokio async runtime")));
    }

    #[test]
    fn test_handover_truncates_long_messages() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "session-long-text";

        let long_user = "A".repeat(1000);
        let long_bot = "B".repeat(1000);

        log_telegram_message(ws, sid, Some(7), None, "user", Some(1), &long_user, true, None);
        log_telegram_message(ws, sid, Some(7), None, "bot", Some(2), &long_bot, true, None);

        let handover = build_topic_handover(ws, sid, Some(7), None, None, &[]).unwrap();
        assert!(!handover.handover_prompt.contains(&"A".repeat(500)));
        assert!(!handover.handover_prompt.contains(&"B".repeat(500)));
        assert!(handover.handover_prompt.contains("[truncated]"));
    }

    #[test]
    fn test_handover_multibyte_korean_utf8_truncation() {
        let tmp = tempdir().unwrap();
        let ws = tmp.path();
        let sid = "session-korean-utf8";

        let korean_prompt = "우리가 RusTerm 웹 시리얼 프로젝트를 개발 중이야. 통신 속도는 115200 baud로 확정하고 전송 포맷은 JSON으로 합의했어. 매우 긴 한국어 문장이 포함되어 있으며 150바이트 경계선에서 글자가 잘리더라도 패닉이 발생하지 않아야 합니다. 계속해서 긴 문장을 이어 작성합니다.".repeat(3);
        log_telegram_message(ws, sid, Some(8), None, "user", Some(1), &korean_prompt, true, None);
        log_telegram_message(ws, sid, Some(8), None, "bot", Some(2), "네, 확인했습니다.", true, None);

        let res = build_topic_handover(ws, sid, Some(8), None, None, &[]);
        assert!(res.is_some());
        let handover = res.unwrap();
        assert!(!handover.agreed_decisions.is_empty());
    }
}
