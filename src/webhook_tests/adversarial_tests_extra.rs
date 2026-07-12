use crate::webhook::auth::validate_bearer_token;

#[test]
fn test_bearer_token_bypass_variations() {
    let token = "my-secret-token";

    // Benign matching case
    assert!(validate_bearer_token("Bearer my-secret-token", token));
    assert!(validate_bearer_token("bearer my-secret-token", token));
    assert!(validate_bearer_token("BEARER my-secret-token", token));

    // Malformed headers
    assert!(!validate_bearer_token("Bearer", token));
    assert!(!validate_bearer_token("Bearer ", token));
    assert!(!validate_bearer_token("Bearer  my-secret-token", token));
    assert!(!validate_bearer_token("Bearer my-secret-token ", token));
    assert!(!validate_bearer_token("Bearer my-secret-token extra", token));
    assert!(!validate_bearer_token("Bearer token", token));
    assert!(!validate_bearer_token("Basic my-secret-token", token));
    assert!(!validate_bearer_token("", token));
    assert!(!validate_bearer_token("my-secret-token", token));
    assert!(!validate_bearer_token("Bearer \x00my-secret-token", token));
}

#[test]
fn test_path_safety_adversarial_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    let allowed = tmp.path().join("allowed");
    std::fs::create_dir(&allowed).unwrap();
    let roots = vec![allowed.clone()];

    // 1. Classic directory traversal escaping roots
    let sneaky = allowed.join("..").join("outside.txt");
    let res = crate::security::paths::validate_file_path(&sneaky, &roots);
    assert!(res.is_err(), "Should block classic traversal escaping roots");

    // 2. Traversal using multiple double-dots
    let sneaky2 = allowed.join("..").join("..").join("etc").join("passwd");
    let res2 = crate::security::paths::validate_file_path(&sneaky2, &roots);
    assert!(res2.is_err(), "Should block multi-level traversal");

    // 3. Absolute path outside allowed roots
    let absolute = std::path::PathBuf::from("/etc/passwd");
    let res3 = crate::security::paths::validate_file_path(&absolute, &roots);
    assert!(res3.is_err(), "Should block absolute paths outside root");

    // 4. Null byte injection
    let null_byte_path = allowed.join("file\x00name.txt");
    let res4 = crate::security::paths::validate_file_path(&null_byte_path, &roots);
    assert!(res4.is_err(), "Should fail with NullByte error");

    // 5. Control characters
    let control_char_path = allowed.join("file\x01name.txt");
    let res5 = crate::security::paths::validate_file_path(&control_char_path, &roots);
    assert!(res5.is_err(), "Should fail with ControlCharacters error");
}
