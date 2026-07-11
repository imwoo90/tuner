//! # Webhook Models Tests
//!
//! Tests for schema deserialization, template rendering, and validation.

use crate::webhook::models::{render_template, WebhookEntry};
use serde_json::json;

#[test]
fn test_webhook_entry_from_dict_defaults() {
    let data = json!({
        "id": "min",
        "title": "Min",
        "mode": "wake",
        "prompt_template": "go"
    });
    let entry: WebhookEntry = serde_json::from_value(data).unwrap();
    assert!(entry.enabled);
    assert_eq!(entry.task_folder, None);
    assert_eq!(entry.trigger_count, 0);
    assert_eq!(entry.last_triggered_at, None);
    assert_eq!(entry.last_error, None);
    assert_eq!(entry.description, "");
}

#[test]
fn test_webhook_entry_to_dict_includes_auth_fields() {
    let entry = WebhookEntry {
        id: "email-notify".to_string(),
        title: "Neue Emails".to_string(),
        description: "".to_string(),
        mode: "wake".to_string(),
        prompt_template: "".to_string(),
        enabled: true,
        created_at: "".to_string(),
        task_folder: None,
        auth_mode: "bearer".to_string(),
        token: "my-token".to_string(),
        hmac_secret: "".to_string(),
        hmac_header: "".to_string(),
        hmac_algorithm: "sha256".to_string(),
        hmac_encoding: "hex".to_string(),
        hmac_sig_prefix: "sha256=".to_string(),
        hmac_sig_regex: "".to_string(),
        hmac_payload_prefix_regex: "".to_string(),
        provider: None,
        model: None,
        reasoning_effort: None,
        cli_parameters: vec![],
        quiet_start: None,
        quiet_end: None,
        dependency: None,
        trigger_count: 0,
        last_triggered_at: None,
        last_error: None,
    };
    let val = serde_json::to_value(&entry).unwrap();
    assert_eq!(val.get("auth_mode").unwrap().as_str().unwrap(), "bearer");
    assert_eq!(val.get("token").unwrap().as_str().unwrap(), "my-token");
}

#[test]
fn test_basic_template_rendering() {
    let result = render_template(
        "Email von {{from}}: {{subject}}",
        &json!({"from": "alice@example.com", "subject": "Hello"}),
    );
    assert_eq!(result, "Email von alice@example.com: Hello");
}

#[test]
fn test_missing_key_renders_placeholder() {
    let result = render_template("{{name}} sent {{message}}", &json!({"name": "Bob"}));
    assert_eq!(result, "Bob sent {{?message}}");
}

#[test]
fn test_no_placeholders() {
    let result = render_template("plain text", &json!({"key": "val"}));
    assert_eq!(result, "plain text");
}

#[test]
fn test_none_value_treated_as_missing() {
    let result = render_template("{{x}}", &json!({"x": null}));
    assert_eq!(result, "{{?x}}");
}
