//! # Heartbeat and Quiet Hours Tests
//!
//! This module contains TDD tests for the Tuner heartbeat observer,
//! including timezone quiet windows, cooldown checks, and ACK token suppression.

#[cfg(test)]
mod tests {
    use crate::heartbeat::quiet::{is_within_quiet_hours, should_suppress_heartbeat};
    use chrono::{NaiveTime, DateTime, TimeZone};
    use chrono_tz::Tz;

    #[test]
    fn test_is_quiet_hours_no_wrap() {
        // Quiet hours: 01:00 to 05:00
        let start = NaiveTime::from_hms_opt(1, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(5, 0, 0).unwrap();
        
        let tz: Tz = "UTC".parse().unwrap();
        let t1 = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T03:00:00Z").unwrap().naive_utc());
        let t2 = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T06:00:00Z").unwrap().naive_utc());

        assert!(is_within_quiet_hours(&t1, start, end));
        assert!(!is_within_quiet_hours(&t2, start, end));
    }

    #[test]
    fn test_is_quiet_hours_boundary_start_is_quiet() {
        // quiet 21-08: 21 is boundary start, should be quiet (inclusive)
        let start = NaiveTime::from_hms_opt(21, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        let tz: Tz = "UTC".parse().unwrap();
        let t = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T21:00:00Z").unwrap().naive_utc());
        assert!(is_within_quiet_hours(&t, start, end));
    }

    #[test]
    fn test_is_quiet_hours_boundary_end_is_not_quiet() {
        // quiet 21-08: 08 is boundary end, should NOT be quiet (exclusive)
        let start = NaiveTime::from_hms_opt(21, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        let tz: Tz = "UTC".parse().unwrap();
        let t = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T08:00:00Z").unwrap().naive_utc());
        assert!(!is_within_quiet_hours(&t, start, end));
    }

    #[test]
    fn test_is_quiet_hours_midnight_in_wrap_window() {
        // quiet 21-08: midnight (00:00) should be quiet
        let start = NaiveTime::from_hms_opt(21, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        let tz: Tz = "UTC".parse().unwrap();
        let t = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T00:00:00Z").unwrap().naive_utc());
        assert!(is_within_quiet_hours(&t, start, end));
    }

    #[test]
    fn test_is_quiet_hours_same_start_end_never_quiet() {
        // same start/end means never quiet
        let same = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        let tz: Tz = "UTC".parse().unwrap();
        let t = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T08:00:00Z").unwrap().naive_utc());
        assert!(!is_within_quiet_hours(&t, same, same));
        let t2 = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T12:00:00Z").unwrap().naive_utc());
        assert!(!is_within_quiet_hours(&t2, same, same));
    }

    #[test]
    fn test_is_quiet_hours_wrapping() {
        // Quiet hours: 22:00 to 07:00 (crosses midnight)
        let start = NaiveTime::from_hms_opt(22, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(7, 0, 0).unwrap();

        let tz: Tz = "UTC".parse().unwrap();
        let t1 = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T23:30:00Z").unwrap().naive_utc());
        let t2 = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T04:00:00Z").unwrap().naive_utc());
        let t3 = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T12:00:00Z").unwrap().naive_utc());

        assert!(is_within_quiet_hours(&t1, start, end));
        assert!(is_within_quiet_hours(&t2, start, end));
        assert!(!is_within_quiet_hours(&t3, start, end));
    }

    #[test]
    fn test_suppress_heartbeat_ack() {
        let ack_token = "HEARTBEAT_OK";

        assert!(should_suppress_heartbeat("HEARTBEAT_OK", ack_token));
        assert!(should_suppress_heartbeat("  HEARTBEAT_OK \n", ack_token));
        assert!(!should_suppress_heartbeat("Warning: Disk full. HEARTBEAT_OK", ack_token));
    }

    #[tokio::test]
    async fn test_heartbeat_scheduler_tick() {
        // Prepare Scheduler configurations and mock components
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = std::sync::Arc::new(crate::session::manager::SessionManager::new(
            temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None
        ));
        let cfg = std::sync::Arc::new(crate::config::CliConfig {
            provider: "antigravity".to_string(),
            allowed_user_ids: vec![100],
            telegram_heartbeat_enabled: true,
            telegram_heartbeat_interval_minutes: Some(30),
            telegram_heartbeat_quiet_start: Some(22),
            telegram_heartbeat_quiet_end: Some(7),
            telegram_heartbeat_ack_token: Some("HEARTBEAT_OK".to_string()),
            ..Default::default()
        });

        let cli = std::sync::Arc::new(crate::cli::antigravity::AntigravityCli::new((*cfg).clone()));
        let scheduler = crate::heartbeat::scheduler::HeartbeatScheduler::new(cfg, mgr, cli);

        // Under quiet hour boundary check (e.g. at 23:00)
        let tz: Tz = "UTC".parse().unwrap();
        let time_quiet = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T23:00:00Z").unwrap().naive_utc());
        assert!(scheduler.should_skip_tick(&time_quiet));

        // Outside quiet hours (e.g. at 12:00)
        let time_active = tz.from_utc_datetime(&DateTime::parse_from_rfc3339("2026-07-11T12:00:00Z").unwrap().naive_utc());
        assert!(!scheduler.should_skip_tick(&time_active));
    }

    #[test]
    fn test_heartbeat_cooling_down() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mgr = std::sync::Arc::new(crate::session::manager::SessionManager::new(
            temp.path().to_path_buf(), 30, 4, false, "UTC".to_string(), None
        ));
        let cfg = std::sync::Arc::new(crate::config::CliConfig {
            telegram_heartbeat_interval_minutes: Some(30),
            ..Default::default()
        });
        let cli = std::sync::Arc::new(crate::cli::antigravity::AntigravityCli::new((*cfg).clone()));
        let scheduler = crate::heartbeat::scheduler::HeartbeatScheduler::new(cfg, mgr, cli);

        // Case 1: last_active is 10 minutes ago (within 30 mins cooldown)
        let ten_mins_ago = chrono::Utc::now() - chrono::Duration::minutes(10);
        let session_warm = crate::session::data::SessionData {
            last_active: ten_mins_ago.to_rfc3339(),
            ..Default::default()
        };
        assert!(scheduler.is_cooling_down(&session_warm));

        // Case 2: last_active is 40 minutes ago (exceeds 30 mins cooldown)
        let forty_mins_ago = chrono::Utc::now() - chrono::Duration::minutes(40);
        let session_cold = crate::session::data::SessionData {
            last_active: forty_mins_ago.to_rfc3339(),
            ..Default::default()
        };
        assert!(!scheduler.is_cooling_down(&session_cold));
    }
}
