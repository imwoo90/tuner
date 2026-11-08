use crate::upgrade::{
    has_any_active_work, is_idle_upgrade_window, is_newer_version, record_user_activity,
    user_idle_secs, GithubRelease,
};

#[test]
fn test_version_comparison() {
    assert!(is_newer_version("0.1.0", "0.1.1"));
    assert!(is_newer_version("0.1.0", "v0.2.0"));
    assert!(is_newer_version("1.0.0", "2.0.0"));
    assert!(is_newer_version("0.1.0", "0.1.0.1"));

    assert!(!is_newer_version("0.2.0", "0.1.0"));
    assert!(!is_newer_version("0.1.1", "0.1.1"));
    assert!(!is_newer_version("v1.2.3", "1.2.3"));
    assert!(!is_newer_version("1.0.0", "0.9.9"));
}

#[test]
fn test_hybrid_idle_upgrade_window() {
    // 1. Night window (03:00..06:00): requires >= 30m (1800s) idle and 0 active work
    assert!(!is_idle_upgrade_window(3, 1799, false));
    assert!(is_idle_upgrade_window(3, 1800, false));
    assert!(is_idle_upgrade_window(4, 2400, false));
    assert!(is_idle_upgrade_window(5, 1800, false));
    // Boundary at 06:00 transitions to daytime rules (requires >= 3h / 10800s)
    assert!(!is_idle_upgrade_window(6, 1800, false));
    assert!(!is_idle_upgrade_window(2, 1800, false));

    // 2. Daytime long idle (any hour): requires >= 3h (10800s) idle and 0 active work
    assert!(!is_idle_upgrade_window(14, 10799, false));
    assert!(is_idle_upgrade_window(14, 10800, false));
    assert!(is_idle_upgrade_window(22, 15000, false));

    // 3. Hard safety guard: active work always blocks upgrade regardless of hour or idle duration
    assert!(!is_idle_upgrade_window(4, 36000, true));
    assert!(!is_idle_upgrade_window(15, 36000, true));
}

#[tokio::test]
async fn test_activity_tracking_and_active_work_guard() {
    record_user_activity();
    assert!(user_idle_secs() < 5);

    let cli = crate::cli::antigravity::AntigravityCli::new(crate::config::CliConfig::default());
    assert!(!has_any_active_work(&cli).await);

    cli.sessions.set_running("test-session-1", true).await;
    assert!(has_any_active_work(&cli).await);
    cli.sessions.set_running("test-session-1", false).await;
    assert!(!has_any_active_work(&cli).await);
}

#[test]
fn test_github_release_deserialization() {
    let json_data = r#"{
        "tag_name": "v0.1.1",
        "body": "Fixed critical issues",
        "assets": [
            {
                "name": "tuner-linux-amd64",
                "browser_download_url": "https://example.com/download/tuner-linux-amd64"
            }
        ]
    }"#;

    let release: Result<GithubRelease, _> = serde_json::from_str(json_data);
    assert!(release.is_ok());
    let release = release.unwrap();
    assert_eq!(release.tag_name, "v0.1.1");
    assert_eq!(release.body.as_deref(), Some("Fixed critical issues"));
    assert_eq!(release.assets.len(), 1);
    assert_eq!(release.assets[0].name, "tuner-linux-amd64");
    assert_eq!(release.assets[0].browser_download_url, "https://example.com/download/tuner-linux-amd64");
}
