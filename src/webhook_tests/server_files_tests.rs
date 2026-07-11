//! # Direct API Server Files Tests
//!
//! Verification of filename sanitization and tag parsing.

use crate::webhook::api::files::{is_image_path, parse_file_refs, sanitize_filename};

#[test]
fn test_sanitize_filename() {
    assert_eq!(sanitize_filename("valid_name.txt"), "valid_name.txt");
    assert_eq!(sanitize_filename("illegal/path\\chars.png"), "illegal_path_chars.png");
    assert_eq!(sanitize_filename("a".repeat(200).as_str()).len(), 120);
}

#[test]
fn test_is_image_path() {
    assert!(is_image_path("test.jpg"));
    assert!(is_image_path("dir/image.PNG"));
    assert!(!is_image_path("document.pdf"));
}

#[test]
fn test_parse_file_refs() {
    let text = "Here is a file: <file:/path/to/image.png> and another: <file:C:\\doc.txt>";
    let refs = parse_file_refs(text);
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].get("name").unwrap().as_str().unwrap(), "image.png");
    assert!(refs[0].get("is_image").unwrap().as_bool().unwrap());
    assert_eq!(refs[1].get("name").unwrap().as_str().unwrap(), "doc.txt");
    assert!(!refs[1].get("is_image").unwrap().as_bool().unwrap());
}
