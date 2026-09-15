use super::commands_remote::{
    render_remote_dashboard, RemoteHubStatus, ServiceStatus, binary_exists,
};

#[test]
fn test_render_remote_dashboard_all_active() {
    let status = RemoteHubStatus {
        profile: "default".to_string(),
        remote_workspace: std::path::PathBuf::from("/home/wimvm/.tuner/profiles/default/remote_workspace"),
        antigravity: ServiceStatus::Active,
        antigravity_url: Some("https://antigravity.google.com/r/test-uuid-v2".to_string()),
        antigravity_uptime: Some("1h 25m".to_string()),
        antigravity_pid: Some(12345),
        tailscale: ServiceStatus::Active,
        tailscale_ip: Some("100.111.32.91".to_string()),
        ssh: ServiceStatus::Active,
    };

    let (text, markup) = render_remote_dashboard(&status);
    assert!(text.contains("Google Antigravity</b> : 🟢 <b>Active</b>"));
    assert!(text.contains("https://antigravity.google.com/r/test-uuid-v2"));
    assert!(text.contains("1h 25m (PID: 12345)"));
    assert!(text.contains("Tailscale</b>          : 🟢 <b>Connected</b> (<code>100.111.32.91</code>)"));
    assert!(text.contains("SSH Service</b>        : 🟢 <b>Listening</b> (<code>Port 22</code>)"));

    // Check keyboard buttons
    let rows = markup.inline_keyboard;
    assert!(!rows.is_empty());
    // Row 1 should have Web IDE url button and Stop button
    let row1 = &rows[0];
    assert_eq!(row1.len(), 2);
    assert_eq!(row1[0].text, "🔗 Web IDE 열기");
    assert_eq!(row1[1].text, "⏹️ Antigravity 중지");

    // Row 2 should have restart button
    let row2 = &rows[1];
    assert_eq!(row2[0].text, "🔄 Antigravity 재시작");

    // Last row should have refresh
    let last_row = &rows[rows.len() - 1];
    assert_eq!(last_row[0].text, "🔄 새로고침");
}

#[test]
fn test_render_remote_dashboard_inactive_antigravity() {
    let status = RemoteHubStatus {
        profile: "default".to_string(),
        remote_workspace: std::path::PathBuf::from("/home/wimvm/.tuner/profiles/default/remote_workspace"),
        antigravity: ServiceStatus::Inactive,
        antigravity_url: None,
        antigravity_uptime: None,
        antigravity_pid: None,
        tailscale: ServiceStatus::Inactive,
        tailscale_ip: None,
        ssh: ServiceStatus::Active,
    };

    let (text, markup) = render_remote_dashboard(&status);
    assert!(text.contains("Google Antigravity</b> : 🔴 <b>Inactive</b>"));
    assert!(text.contains("Tailscale</b>          : 🔴 <b>Inactive</b> (Disconnected)"));
    assert!(text.contains("SSH Service</b>        : 🟢 <b>Listening</b> (<code>Port 22</code>)"));

    let rows = markup.inline_keyboard;
    // Row 1 should have start button
    let row1 = &rows[0];
    assert_eq!(row1[0].text, "🚀 Antigravity 시작");

    // Row 2 should have Tailscale connect button
    let row2 = &rows[1];
    assert_eq!(row2[0].text, "🔒 Tailscale 연결");
}

#[test]
fn test_render_remote_dashboard_not_installed() {
    let status = RemoteHubStatus {
        profile: "default".to_string(),
        remote_workspace: std::path::PathBuf::from("/home/wimvm/.tuner/profiles/default/remote_workspace"),
        antigravity: ServiceStatus::NotInstalled,
        antigravity_url: None,
        antigravity_uptime: None,
        antigravity_pid: None,
        tailscale: ServiceStatus::NotInstalled,
        tailscale_ip: None,
        ssh: ServiceStatus::NotInstalled,
    };

    let (text, markup) = render_remote_dashboard(&status);
    assert!(text.contains("Google Antigravity</b> : ⚠️ <b>Not Installed</b>"));
    assert!(text.contains("Tailscale</b>          : ⚠️ <b>Not Installed</b>"));
    assert!(text.contains("SSH Service</b>        : ⚠️ <b>Not Installed</b>"));

    let rows = markup.inline_keyboard;
    assert_eq!(rows[0][0].text, "📥 Antigravity 설치 안내");
}

#[test]
fn test_binary_exists_common_commands() {
    // ls should always exist on unix
    assert!(binary_exists("ls") || binary_exists("sh"));
    // definitely nonexistent binary
    assert!(!binary_exists("definitely_nonexistent_binary_xyz_12345"));
}

#[test]
fn test_resolve_profile_info() {
    use std::path::Path;
    let p_default = Path::new("/home/wimvm/.tuner/profiles/default/workspace");
    let (profile, remote_ws, screen_name, instance_name) = super::commands_remote_hub::resolve_profile_info(p_default);
    assert_eq!(profile, "default");
    assert_eq!(remote_ws, Path::new("/home/wimvm/.tuner/profiles/default/remote_workspace"));
    assert_eq!(screen_name, "tuner-agy-remote-default");
    assert_eq!(instance_name, "default-remote");

    let p_seojin = Path::new("/home/wimvm/.tuner/profiles/seojin/workspace");
    let (profile_s, remote_ws_s, screen_name_s, instance_name_s) = super::commands_remote_hub::resolve_profile_info(p_seojin);
    assert_eq!(profile_s, "seojin");
    assert_eq!(remote_ws_s, Path::new("/home/wimvm/.tuner/profiles/seojin/remote_workspace"));
    assert_eq!(screen_name_s, "tuner-agy-remote-seojin");
    assert_eq!(instance_name_s, "seojin-remote");
}
