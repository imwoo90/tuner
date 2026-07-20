//! # Content Safety and Password Filter
//!
//! ## Overview
//! Evaluates text payloads against block patterns to prevent leakage of credentials or keys.
//!
//! ## Collaboration Graph
//! - Inspects outgoing message envelopes before dispatching.
//!
//! ## Search Tags
//! #credential-filter, #regex-leak, #data-safety

use std::sync::OnceLock;
use regex::{Regex, RegexBuilder};

fn get_patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let raw = vec![
            (r"ignore\s+(all\s+)?(previous|prior|above)\s+(instructions?|prompts?)", "instruction_override"),
            (r"disregard\s+(all\s+)?(previous|prior|above)", "instruction_override"),
            (r"forget\s+(everything|all|your)\s+(instructions?|rules?)", "instruction_override"),
            (r"you\s+are\s+now\s+(a|an)\s+", "role_hijack"),
            (r"new\s+instructions?:", "role_hijack"),
            (r"system\s*:\s*prompt", "fake_system_prompt"),
            (r"<\|(?:im_start|im_end|system|endoftext)\|>", "special_token"),
            (r"\[INST\]|\[/INST\]|<<SYS>>|<</SYS>>", "llama_markers"),
            (r"(?:^|\n)\s*(?:Human|Assistant|System)\s*:", "anthropic_markers"),
            (r"GROUND_RULES|(?:AGENT_)?SOUL\.md|(?:AGENT_)?SYSTEM\.md|BOOTSTRAP\.md|(?:AGENT_)?IDENTITY\.md", "internal_file_ref"),
            (r"mem_add\.py|mem_edit\.py|mem_delete\.py|task_add\.py", "tool_injection"),
            (r"--system-prompt|--append-system-prompt|--permission-mode", "cli_flag_injection"),
            (r"<file:[^>]+>", "file_tag_injection"),
        ];
        raw.into_iter()
            .map(|(p, cat)| {
                let re = RegexBuilder::new(p)
                    .case_insensitive(true)
                    .build()
                    .unwrap();
                (re, cat)
            })
            .collect()
    })
}

/// Scan text for prompt injection patterns. Empty vector = clean.
pub fn detect_suspicious_patterns(text: &str) -> Vec<String> {
    let folded = fold_fullwidth(text);
    let mut matched = Vec::new();
    for (re, cat) in get_patterns() {
        if re.is_match(&folded) {
            matched.push((*cat).to_string());
        }
    }
    matched
}

/// Helper to fold fullwidth Unicode characters to standard ASCII equivalents.
pub fn fold_fullwidth(text: &str) -> String {
    text.chars().map(fold_fullwidth_char).collect()
}

/// Helper to fold a single character if it is a fullwidth equivalent.
pub fn fold_fullwidth_char(c: char) -> char {
    let code = c as u32;
    if (0xFF21..=0xFF3A).contains(&code) || (0xFF41..=0xFF5A).contains(&code) {
        char::from_u32(code - 0xFEE0).unwrap_or(c)
    } else if code == 0xFF1C {
        '<'
    } else if code == 0xFF1E {
        '>'
    } else {
        c
    }
}

