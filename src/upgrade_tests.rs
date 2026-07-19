use crate::upgrade::{is_newer_version, GithubRelease};

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

#[test]
fn test_development_install_detection() {
    assert!(crate::upgrade::is_dev_install());
}
