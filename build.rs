use std::fs;
use std::path::Path;

fn main() {
    // Tell Cargo to rerun this build script if src/ changes
    println!("cargo:rerun-if-changed=src");

    let src_dir = Path::new("src");

    if let Err(e) = check_dir(src_dir) {
        // Output compilation error and fail
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn check_dir(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            check_dir(&path)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            check_file(&path)?;
        }
    }
    Ok(())
}

fn check_file(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content.lines().collect();

    let mut logical_code_chars = 0;
    let mut doc_chars = 0;
    let mut in_block_comment = false;
    let mut module_docs = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        let line_len = line.len() + 1; // Count raw line length + newline character

        if trimmed.is_empty() {
            continue;
        }

        if in_block_comment {
            doc_chars += line_len;
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }

        if trimmed.starts_with("/*") {
            doc_chars += line_len;
            if !trimmed.contains("*/") {
                in_block_comment = true;
            }
            continue;
        }

        if trimmed.starts_with("//") {
            doc_chars += line_len;
            // Check for module-level docs
            if trimmed.starts_with("//!") {
                let doc_line = if trimmed.starts_with("//! ") {
                    &trimmed[4..]
                } else {
                    &trimmed[3..]
                };
                module_docs.push(doc_line.to_string());
            }
            continue;
        }

        logical_code_chars += line_len;
    }

    // 1. Check logical code limit (max 10,000 characters)
    if logical_code_chars > 10000 {
        return Err(format!(
            "error: File {:?} has {} logical code characters, which exceeds the limit of 10,000 characters (AGENT.md rule).",
            path,
            logical_code_chars
        ));
    }

    // 2. Check documentation character limit (max 4,000 characters)
    if doc_chars > 4000 {
        return Err(format!(
            "error: File {:?} has {} documentation/comment characters, which exceeds the limit of 4,000 characters (AGENT.md rule).",
            path,
            doc_chars
        ));
    }

    // 3. Enforce File-Level Documentation (//! at least 100 characters) for production code
    let path_str = path.to_string_lossy();
    let is_test = path_str.contains("test");

    if !is_test {
        let module_doc_len = module_docs.iter().map(|line| line.trim().len()).sum::<usize>();
        if module_doc_len < 100 {
            return Err(format!(
                "error: File {:?} has a module/file level documentation comment (//!) of only {} characters, which is below the minimum required 100 characters. Every production source file must serve as a Wiki entry and document itself.",
                path,
                module_doc_len
            ));
        }

        let joined_docs = module_docs.join("\n");
        let has_search_tags = joined_docs.to_lowercase().contains("search tags");
        if !has_search_tags {
            return Err(format!(
                "error: File {:?} is missing a 'Search Tags' section in its module/file level documentation comment (//!). Every production source file must contain a 'Search Tags' header to support semantic search indexing.",
                path
            ));
        }

        let has_hashtag = joined_docs.contains('#');
        if !has_hashtag {
            return Err(format!(
                "error: File {:?} is missing hashtag references ('#') in its 'Search Tags' section. Every production source file must define at least one search tag (e.g. '#tag-name').",
                path
            ));
        }
    }

    // 4. Check function character size (max 2,000 physical characters)
    let mut in_fn = false;
    let mut seen_opening_brace = false;
    let mut fn_start_line = 0;
    let mut brace_depth = 0;
    let mut fn_chars = 0;
    let mut fn_name = String::new();

    for (idx, line) in lines.iter().enumerate() {
        let line_trimmed = line.trim();
        let line_len = line.len() + 1;

        // Detect function start
        if !in_fn
            && brace_depth == 0
            && (line_trimmed.starts_with("fn ")
                || line_trimmed.starts_with("pub fn ")
                || line_trimmed.starts_with("pub(crate) fn ")
                || line_trimmed.starts_with("async fn ")
                || line_trimmed.starts_with("pub async fn ")
                || line_trimmed.starts_with("pub(crate) async fn "))
        {
            in_fn = true;
            seen_opening_brace = false;
            fn_start_line = idx + 1;
            fn_chars = 0;
            fn_name = line_trimmed.to_string();
        }

        if in_fn {
            fn_chars += line_len;

            // Count braces on this line
            for c in line.chars() {
                if c == '{' {
                    brace_depth += 1;
                    seen_opening_brace = true;
                } else if c == '}' {
                    brace_depth -= 1;
                }
            }

            // If we have seen the opening brace and depth returns to 0, function ends.
            // Also, if the line ends with a semicolon and we haven't seen an opening brace yet (e.g. trait declaration),
            // it ends without an error.
            if (seen_opening_brace && brace_depth == 0)
                || (!seen_opening_brace && line_trimmed.ends_with(';'))
            {
                if seen_opening_brace && fn_chars > 2000 {
                    return Err(format!(
                        "error: Function at {:?}:{}: {} characters (exceeds the limit of 2,000 characters, AGENT.md rule).\nLine: {}",
                        path,
                        fn_start_line,
                        fn_chars,
                        fn_name
                    ));
                }
                in_fn = false;
            }
        }
    }

    Ok(())
}
