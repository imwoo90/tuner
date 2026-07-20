//! # Markdown Blockquote Parser and Converter
//!
//! Identifies blockquote markers (`>`) in Markdown output and groups consecutive blockquote lines
//! into clean HTML tag block structures acceptable by the Telegram Bot API client.

//! 
//! ## Search Tags
//! #blockquotes

pub fn convert_blockquotes(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut result = Vec::new();
    let mut quote_buf = Vec::new();
    let escaped_gt = "&gt; ";
    let escaped_gt_exp = "&gt;! ";
    let mut is_expandable = false;

    for line in lines {
        if line.starts_with(escaped_gt) || line.starts_with(escaped_gt_exp) {
            if line.starts_with(escaped_gt_exp) {
                is_expandable = true;
                quote_buf.push(&line[escaped_gt_exp.len()..]);
            } else {
                quote_buf.push(&line[escaped_gt.len()..]);
            }
        } else {
            if !quote_buf.is_empty() {
                let tag = if is_expandable {
                    "<blockquote expandable>"
                } else {
                    "<blockquote>"
                };
                result.push(format!("{}{}{}</blockquote>", tag, quote_buf.join("\n"), ""));
                quote_buf.clear();
                is_expandable = false;
            }
            result.push(line.to_string());
        }
    }

    if !quote_buf.is_empty() {
        let tag = if is_expandable {
            "<blockquote expandable>"
        } else {
            "<blockquote>"
        };
        result.push(format!("{}{}{}</blockquote>", tag, quote_buf.join("\n"), ""));
    }

    result.join("\n")
}
