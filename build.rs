use std::fs;
use std::path::Path;

fn main() {
    // Tell Cargo to rerun this build script if src/ changes
    println!("cargo:rerun-if-changed=src");

    let src_dir = Path::new("src");
    let mut docs = Vec::new();

    if let Err(e) = check_dir(src_dir, &mut docs) {
        // Output compilation error and fail
        eprintln!("{}", e);
        std::process::exit(1);
    }

    // Sort docs by module path for deterministic order
    docs.sort_by(|a, b| a.0.cmp(&b.0));

    // Generate llms.txt
    if let Err(e) = generate_llms_txt(&docs) {
        eprintln!("Failed to generate llms.txt: {}", e);
        std::process::exit(1);
    }
}

fn check_dir(dir: &Path, docs: &mut Vec<(String, String)>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            check_dir(&path, docs)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let doc_opt = check_file(&path)?;
            if let Some(doc_content) = doc_opt {
                let mod_name = path.to_string_lossy().to_string();
                docs.push((mod_name, doc_content));
            }
        }
    }
    Ok(())
}

fn check_file(path: &Path) -> Result<Option<String>, String> {
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

    if logical_code_chars > 10000 {
        return Err(format!(
            "error: File {:?} has {} logical code characters, which exceeds the limit of 10,000 characters (AGENT.md rule).",
            path,
            logical_code_chars
        ));
    }

    if doc_chars > 4000 {
        return Err(format!(
            "error: File {:?} has {} documentation/comment characters, which exceeds the limit of 4,000 characters (AGENT.md rule).",
            path,
            doc_chars
        ));
    }

    // 2. Check function character size (max 2,000 physical characters)
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

    let doc_content = if module_docs.is_empty() {
        None
    } else {
        Some(module_docs.join("\n"))
    };

    Ok(doc_content)
}

fn generate_llms_txt(docs: &[(String, String)]) -> Result<(), std::io::Error> {
    let mut content = String::new();
    content.push_str("# Tuner LLM Wiki (llms.txt)\n\n");
    content.push_str("This file is automatically generated by build.rs. Do not edit directly.\n");
    content.push_str("Use this file to gain high-level context of all modules in Tuner.\n\n");
    content.push_str("========================================================================\n\n");

    for (mod_path, doc) in docs {
        content.push_str(&format!("## Module: {}\n\n", mod_path));
        content.push_str(doc);
        content.push_str("\n\n------------------------------------------------------------------------\n\n");
    }

    fs::write("llms.txt", content)?;
    Ok(())
}
