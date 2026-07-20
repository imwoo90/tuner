//! # Path Traversal Sandbox Validator
//!
//! ## Overview
//! Inspects path targets, resolving links to verify they lie inside authorized roots.
//!
//! ## Collaboration Graph
//! - Called by Webhook file servers and Cron managers before reading/writing files.
//!
//! ## Search Tags
//! #directory-sandbox, #link-resolution, #security-bounds

use std::path::{Path, PathBuf};
use std::fs;

/// Path validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathValidationError {
    NullByte(String),
    ControlCharacters(String),
    OutsideAllowedRoots(PathBuf, Vec<PathBuf>),
    IoError(String),
}

impl std::fmt::Display for PathValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathValidationError::NullByte(raw) => write!(f, "Path contains null byte: {:?}", raw),
            PathValidationError::ControlCharacters(raw) => write!(f, "Path contains control characters: {:?}", raw),
            PathValidationError::OutsideAllowedRoots(resolved, allowed) => {
                write!(f, "Path {:?} is outside allowed roots: {:?}", resolved, allowed)
            }
            PathValidationError::IoError(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

impl std::error::Error for PathValidationError {}

/// Helper function to resolve a path (handling symlinks, relative segments) even for non-existent paths.
pub fn resolve_path(path: &Path) -> std::io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut existing_ancestor = path;
            let mut remaining = Vec::new();

            while let Some(parent) = existing_ancestor.parent() {
                if existing_ancestor.exists() && !existing_ancestor.as_os_str().is_empty() {
                    break;
                }
                if let Some(name) = existing_ancestor.file_name() {
                    remaining.push(name);
                }
                existing_ancestor = parent;
            }

            let mut base = if existing_ancestor.exists() && !existing_ancestor.as_os_str().is_empty() {
                fs::canonicalize(existing_ancestor)?
            } else {
                fs::canonicalize(".")?
            };

            for comp in remaining.into_iter().rev() {
                let comp_str = comp.to_string_lossy();
                if comp_str == "." {
                    // Skip
                } else if comp_str == ".." {
                    base.pop();
                } else {
                    base.push(comp);
                }
            }
            Ok(base)
        }
        Err(e) => Err(e),
    }
}

/// Resolve and validate a file path against allowed root directories.
pub fn validate_file_path<P: AsRef<Path>>(
    path: P,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, PathValidationError> {
    let path = path.as_ref();
    let raw = path.to_string_lossy().into_owned();

    if raw.contains('\0') {
        return Err(PathValidationError::NullByte(raw));
    }

    if raw.chars().any(|c| (c as u32) < 32 && c != '\n') {
        return Err(PathValidationError::ControlCharacters(raw));
    }

    let resolved = resolve_path(path).map_err(|e| PathValidationError::IoError(e.to_string()))?;

    for root in allowed_roots {
        if let Ok(resolved_root) = resolve_path(root) {
            if resolved.starts_with(&resolved_root) {
                return Ok(resolved);
            }
        }
    }

    Err(PathValidationError::OutsideAllowedRoots(
        resolved,
        allowed_roots.to_vec(),
    ))
}

/// Non-throwing version of validate_file_path.
pub fn is_path_safe<P: AsRef<Path>>(
    path: P,
    allowed_roots: &[PathBuf],
) -> bool {
    validate_file_path(path, allowed_roots).is_ok()
}

