//! # Unit Tests for Model Quota and Whitelist Command Protection
//!
//! Tests quota parsing from ANSI terminal output, progress bar generation, HTML rendering,
//! and slash command whitelist escaping to protect against Antigravity modal traps.

#[cfg(test)]
mod tests {
    use crate::messenger::telegram::commands_usage::{
        format_progress_bar, parse_quota_report, render_quota_message,
        ModelQuotaGroup, QuotaLimit, QuotaReport,
    };
    use crate::messenger::telegram::commands_registry::{
        escape_non_workflow_slash, get_bot_commands, is_lock_free_command, is_workflow_slash_command,
    };

    #[test]
    fn test_format_progress_bar() {
        assert_eq!(format_progress_bar(0.0), "[░░░░░░░░░░░░░░░░░░░░]");
        assert_eq!(format_progress_bar(50.0), "[██████████░░░░░░░░░░]");
        assert_eq!(format_progress_bar(100.0), "[████████████████████]");
        assert_eq!(format_progress_bar(10.0), "[██░░░░░░░░░░░░░░░░░░]");
    }

    const SAMPLE_AGY_USAGE: &str = r#"
Models & Quota

Account: user@example.com

GEMINI MODELS (Flash, Pro)
Weekly Limit Remaining: 95.0%
[██████████████████░░]
Resets in 3 days, 14 hours

Five Hour Limit Remaining: 100.0%
[████████████████████]
Resets in 4 hours, 50 minutes

CLAUDE AND GPT MODELS (Opus, Sonnet, GPT)
Weekly Limit Remaining: 80.0%
[████████████████░░░░]
Resets in 5 days, 2 hours

Five Hour Limit Remaining: 75.5%
[███████████████░░░░░]
Resets in 1 hour, 10 minutes

Press Esc or q to exit
"#;

    #[test]
    fn test_parse_quota_report_full() {
        let report = parse_quota_report(SAMPLE_AGY_USAGE);
        assert_eq!(report.account.as_deref(), Some("user@example.com"));

        let gemini = report.gemini.expect("Gemini models quota expected");
        let g_weekly = gemini.weekly.expect("Gemini weekly quota expected");
        assert_eq!(g_weekly.pct, 95.0);
        assert_eq!(g_weekly.detail, "Resets in 3 days, 14 hours");

        let g_five = gemini.five_hour.expect("Gemini five hour quota expected");
        assert_eq!(g_five.pct, 100.0);
        assert_eq!(g_five.detail, "Resets in 4 hours, 50 minutes");

        let claude = report.claude.expect("Claude models quota expected");
        let c_weekly = claude.weekly.expect("Claude weekly quota expected");
        assert_eq!(c_weekly.pct, 80.0);
        assert_eq!(c_weekly.detail, "Resets in 5 days, 2 hours");

        let c_five = claude.five_hour.expect("Claude five hour quota expected");
        assert_eq!(c_five.pct, 75.5);
        assert_eq!(c_five.detail, "Resets in 1 hour, 10 minutes");
    }

    #[test]
    fn test_parse_quota_report_with_ansi_escapes() {
        let raw_with_ansi = format!("\x1b[1mAccount:\x1b[0m user@google.com\n\x1b[32mGEMINI MODELS\x1b[0m\nWeekly Limit Remaining: 88.5%\n[█████████████████░░░]\nResets tomorrow\n");
        let report = parse_quota_report(&raw_with_ansi);
        assert_eq!(report.account.as_deref(), Some("user@google.com"));
        let gemini = report.gemini.expect("Gemini expected");
        let w = gemini.weekly.expect("Weekly expected");
        assert_eq!(w.pct, 88.5);
        assert_eq!(w.detail, "Resets tomorrow");
    }

    #[test]
    fn test_render_quota_message_formatting() {
        let report = QuotaReport {
            account: Some("tester@dev.com".into()),
            gemini: Some(ModelQuotaGroup {
                weekly: Some(QuotaLimit { pct: 90.0, detail: "Resets in 2 days".into() }),
                five_hour: Some(QuotaLimit { pct: 100.0, detail: "".into() }),
            }),
            claude: None,
            raw_fallback: String::new(),
        };
        let msg = render_quota_message(&report);
        assert!(msg.contains("📊 <b>[tuner] Model Quota & Usage</b>"));
        assert!(msg.contains("tester@dev.com"));
        assert!(msg.contains("GEMINI MODELS (Flash, Pro)"));
        assert!(msg.contains("Weekly Limit: <code>[██████████████████░░]</code> 90.0%"));
        assert!(msg.contains("Resets in 2 days"));
        assert!(msg.contains("5-Hour Limit: <code>[████████████████████]</code> 100.0%"));
    }

    #[test]
    fn test_workflow_slash_command_whitelist() {
        assert!(is_workflow_slash_command("/goal overnight build something"));
        assert!(is_workflow_slash_command("/plan refactor architecture"));
        assert!(is_workflow_slash_command("/learn record correction"));
        assert!(is_workflow_slash_command("/grill_me"));
        assert!(is_workflow_slash_command("/grill-me"));
        assert!(is_workflow_slash_command("/teamwork_preview"));
        assert!(is_workflow_slash_command("/teamwork-preview"));
        assert!(is_workflow_slash_command("/browser check docs"));
        assert!(is_workflow_slash_command("/boost solve puzzle"));
        assert!(is_workflow_slash_command("/schedule every 10m"));

        assert!(!is_workflow_slash_command("/etc/nginx/nginx.conf"));
        assert!(!is_workflow_slash_command("/usage"));
        assert!(!is_workflow_slash_command("/unknown_command"));
        assert!(!is_workflow_slash_command("plain text"));
    }

    #[test]
    fn test_escape_non_workflow_slash() {
        // Workflow commands stay unescaped
        assert_eq!(escape_non_workflow_slash("/goal overnight task"), "/goal overnight task");
        assert_eq!(escape_non_workflow_slash("/plan something"), "/plan something");
        assert_eq!(escape_non_workflow_slash("/learn fix"), "/learn fix");

        // Non-workflow slash inputs are prepended with a single space
        assert_eq!(escape_non_workflow_slash("/etc/hosts"), " /etc/hosts");
        assert_eq!(escape_non_workflow_slash("/usage 설명해줘"), " /usage 설명해줘");
        assert_eq!(escape_non_workflow_slash("/usage 동작 되는거 확인했어."), " /usage 동작 되는거 확인했어.");
        assert_eq!(escape_non_workflow_slash("/var/log/syslog"), " /var/log/syslog");

        // Normal inputs without leading slash are untouched
        assert_eq!(escape_non_workflow_slash("일반 텍스트"), "일반 텍스트");
        assert_eq!(escape_non_workflow_slash("hello /world"), "hello /world");
    }

    #[test]
    fn test_usage_command_registry_and_topic_lock() {
        // /usage must NOT be lock-free: it runs under topic lock in its own turn
        assert!(!is_lock_free_command("/usage"));
        assert!(!is_lock_free_command("/usage help"));

        // Normal lock-free commands must remain lock-free
        assert!(is_lock_free_command("/status"));
        assert!(is_lock_free_command("/stop"));
        assert!(is_lock_free_command("/abort"));

        // Bot commands must include "usage" and "cron"
        let cmds = get_bot_commands();
        assert!(cmds.iter().any(|c| c.command == "usage"));
        assert!(cmds.iter().any(|c| c.command == "cron"));
    }

    #[test]
    fn test_load_home_defaults_cron_jobs() {
        let content = std::fs::read_to_string("_home_defaults/cron_jobs.json").unwrap();
        let jobs_res: Result<crate::cron::manager::CronJob, _> =
            serde_json::from_str(&serde_json::from_str::<serde_json::Value>(&content).unwrap()["jobs"][0].to_string());
        println!("Jobs parse result: {:?}", jobs_res);
        assert!(jobs_res.is_ok());
    }
}
