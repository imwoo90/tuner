//! # Markdown Table Formatter and Parser
//!
//! ## Overview
//! Detects Markdown pipe-tables and converts them into mobile-friendly structured
//! card/list blocks with native bold headers, bulleted attributes, and preserved inline formatting.
//!
//! ## Search Tags
//! #table, #markdown-table, #mobile-card-layout

use regex::Regex;

const ESCAPED_PIPE_SENTINEL: &str = "\x00PIPE\x00";

fn parse_table_row(line: &str) -> Vec<String> {
    let pre = line.trim().replace(r"\|", ESCAPED_PIPE_SENTINEL);
    let after_prefix = pre.strip_prefix('|').unwrap_or(&pre);
    let stripped = after_prefix.strip_suffix('|').unwrap_or(after_prefix);
    stripped
        .split('|')
        .map(|cell| cell.replace(ESCAPED_PIPE_SENTINEL, "|").trim().to_string())
        .collect()
}

fn is_separator_row(line: &str) -> bool {
    if !line.contains('|') {
        return false;
    }
    let Ok(re) = Regex::new(r"^\s*\|?[\s:]*-+[\s:]*(\|[\s:]*-+[\s:]*)*\|?\s*$") else {
        return false;
    };
    re.is_match(line)
}

fn split_cell_br(cell: &str) -> Vec<String> {
    let Ok(br_re) = Regex::new(r"(?i)<br\s*/?>") else {
        let t = cell.trim();
        return if t.is_empty() { Vec::new() } else { vec![t.to_string()] };
    };
    br_re
        .split(cell)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn strip_bold_markers(s: &str) -> String {
    let joined = split_cell_br(s).join(" ");
    joined.replace("**", "").trim().to_string()
}

fn strip_leading_bullet(s: &str) -> &str {
    let trimmed = s.trim();
    for prefix in ["• ", "- ", "* "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_start();
        }
    }
    trimmed
}

fn normalize_sub_bullet(s: &str) -> String {
    let trimmed = s.trim();
    for prefix in ["- ", "* "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return format!("• {}", rest.trim_start());
        }
    }
    trimmed.to_string()
}

fn format_attribute_cell(hdr: &str, cell: &str) -> Option<String> {
    let sub_lines = split_cell_br(cell);
    if sub_lines.is_empty() {
        return None;
    }

    if sub_lines.len() == 1 {
        let val = strip_leading_bullet(&sub_lines[0]);
        if val.is_empty() {
            return None;
        }
        return if hdr.is_empty() {
            Some(format!("• {}", val))
        } else {
            Some(format!("• **{}**: {}", hdr, val))
        };
    }

    let norm: Vec<String> = sub_lines.iter().map(|s| normalize_sub_bullet(s)).collect();
    if hdr.is_empty() {
        let lines: Vec<String> = norm
            .into_iter()
            .map(|l| if l.starts_with("• ") { l } else { format!("• {}", l) })
            .collect();
        return Some(lines.join("\n"));
    }

    if norm[0].starts_with("• ") {
        let indented: Vec<String> = norm.into_iter().map(|l| format!("  {}", l)).collect();
        Some(format!("• **{}**:\n{}", hdr, indented.join("\n")))
    } else {
        let mut out = format!("• **{}**: {}", hdr, norm[0]);
        for sub in norm.iter().skip(1) {
            out.push_str(&format!("\n  {}", sub));
        }
        Some(out)
    }
}

fn format_data_row(headers: &[String], row: &[String], n_cols: usize) -> Option<String> {
    if n_cols == 0 {
        return None;
    }
    if n_cols == 1 {
        let c0 = split_cell_br(&row[0]).join(" ");
        let val = strip_leading_bullet(&c0);
        return (!val.is_empty()).then(|| format!("• {}", val));
    }

    let mut card_lines = Vec::new();
    let clean_c0 = strip_bold_markers(&row[0]);
    if !clean_c0.is_empty() {
        card_lines.push(format!("▸ **{}**", clean_c0));
    }

    for c in 1..n_cols {
        let hdr = strip_bold_markers(&headers[c]);
        if let Some(attr) = format_attribute_cell(&hdr, &row[c]) {
            card_lines.push(attr);
        }
    }

    (!card_lines.is_empty()).then(|| card_lines.join("\n"))
}

fn format_table(lines: &[String]) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        if is_separator_row(line) {
            continue;
        }
        rows.push(parse_table_row(line));
    }

    if rows.is_empty() {
        return lines.join("\n");
    }

    let n_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for row in &mut rows {
        while row.len() < n_cols {
            row.push(String::new());
        }
    }

    if rows.len() == 1 {
        let cells: Vec<String> = rows[0]
            .iter()
            .map(|c| strip_bold_markers(c))
            .filter(|c| !c.is_empty())
            .map(|c| format!("**{}**", c))
            .collect();
        return cells.join(" · ");
    }

    let headers = &rows[0];
    let cards: Vec<String> = rows
        .iter()
        .skip(1)
        .filter_map(|row| format_data_row(headers, row, n_cols))
        .collect();

    cards.join("\n\n")
}

/// Converts Markdown pipe-tables in `src` into structured card/list blocks.
pub fn convert_tables(src: &str) -> String {
    let Ok(check_table_line_re) = Regex::new(r"\|.*\|") else {
        return src.to_string();
    };

    let mut out_lines = Vec::new();
    let mut table_buf: Vec<String> = Vec::new();

    let flush = |table_buf: &mut Vec<String>, out_lines: &mut Vec<String>| {
        if table_buf.len() >= 2 && table_buf.iter().any(|l| is_separator_row(l)) {
            out_lines.push(format_table(table_buf));
        } else {
            out_lines.append(table_buf);
        }
        table_buf.clear();
    };

    for line in src.split('\n') {
        if line.contains('|') && check_table_line_re.is_match(line.trim()) {
            table_buf.push(line.to_string());
        } else {
            if !table_buf.is_empty() {
                flush(&mut table_buf, &mut out_lines);
            }
            out_lines.push(line.to_string());
        }
    }
    if !table_buf.is_empty() {
        flush(&mut table_buf, &mut out_lines);
    }

    out_lines.join("\n")
}
