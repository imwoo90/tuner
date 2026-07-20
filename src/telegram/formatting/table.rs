//! # Markdown Table Formatter and Parser
//!
//! Extracts markdown pipe-tables, parses their columns and alignment rules, and formats them
//! into fixed-width monospace layouts wrapped in pre/code blocks for clean grid presentation.

//! 
//! ## Search Tags
//! #table

use regex::Regex;

fn parse_table_row(line: &str) -> Vec<String> {
    let stripped = line.trim().trim_start_matches('|').trim_end_matches('|');
    stripped.split('|').map(|cell| cell.trim().to_string()).collect()
}

fn is_separator_row(line: &str) -> bool {
    let re = Regex::new(r"^\s*\|?[\s:]*-{2,}[\s:]*(\|[\s:]*-{2,}[\s:]*)*\|?\s*$").unwrap();
    re.is_match(line)
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

    let mut widths = vec![0; n_cols];
    for col in 0..n_cols {
        let mut max_w = 0;
        for row in &rows {
            max_w = max_w.max(row[col].chars().count());
        }
        widths[col] = max_w;
    }

    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let mut cells = Vec::new();
        for (c, cell) in row.iter().enumerate() {
            let width = widths[c];
            let padding = width.saturating_sub(cell.chars().count());
            let padded_cell = format!("{}{}", cell, " ".repeat(padding));
            cells.push(padded_cell);
        }
        out.push(cells.join("  "));
        if i == 0 && rows.len() > 1 {
            let divider: Vec<String> = widths.iter().map(|&w| "─".repeat(w)).collect();
            out.push(divider.join("  "));
        }
    }

    out.join("\n")
}

pub fn extract_tables(src: &str, table_blocks: &mut Vec<String>) -> String {
    let mut out_lines = Vec::new();
    let mut table_buf = Vec::new();

    let flush = |table_buf: &mut Vec<String>, table_blocks: &mut Vec<String>, out_lines: &mut Vec<String>| {
        if table_buf.len() >= 2 {
            let idx = table_blocks.len();
            table_blocks.push(format_table(table_buf));
            out_lines.push(super::helpers::placeholder("TB", idx));
        } else {
            out_lines.extend(table_buf.clone());
        }
        table_buf.clear();
    };

    let check_table_line_re = Regex::new(r"\|.*\|").unwrap();
    for line in src.split('\n') {
        if line.contains('|') && check_table_line_re.is_match(line.trim()) {
            table_buf.push(line.to_string());
        } else {
            if !table_buf.is_empty() {
                let mut tb = table_buf.clone();
                flush(&mut tb, table_blocks, &mut out_lines);
                table_buf.clear();
            }
            out_lines.push(line.to_string());
        }
    }
    if !table_buf.is_empty() {
        let mut tb = table_buf.clone();
        flush(&mut tb, table_blocks, &mut out_lines);
    }

    out_lines.join("\n")
}
