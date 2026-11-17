//! # Web AskQuestion PTY Keystroke & Step Helpers (`api_ask_helpers.rs`)
//!
//! ## Overview
//! Translates Web Dashboard single-select, multi-select, write-in, step-back (`prev`),
//! and skip submissions into provider PTY keystrokes and synchronized [`AskState`] mutations.
//!
//! ## Search Tags
//! #ask-keystrokes, #multi-select-helper, #pty-input

use super::api_ask::AskAnswerPayload;
use super::WebAppCtx;
use crate::cli::antigravity::session::AskState;
use crate::messenger::telegram::ask_helpers::find_write_in_index;
use crate::messenger::telegram::ask_process::clear_previous_write_in_if_any;
use crate::messenger::telegram::multi_select::get_bitmap_for_question;
use crate::session::data::SessionData;

pub(crate) fn build_multi_keystrokes_from_indices(
    options: &[String],
    selected_indices: &[usize],
    prev_ans: &str,
) -> (String, Vec<String>) {
    let mut prev_set = Vec::new();
    if !prev_ans.is_empty() {
        let parts: Vec<&str> = prev_ans.split(", ").map(str::trim).collect();
        for (i, opt) in options.iter().enumerate() {
            if parts.contains(&opt.as_str()) {
                prev_set.push(i);
            }
        }
    }
    let mut toggled = Vec::new();
    let mut labels = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        let is_sel = selected_indices.contains(&i);
        if is_sel {
            labels.push(opt.clone());
        }
        if is_sel != prev_set.contains(&i) {
            toggled.push(i);
        }
    }
    toggled.sort_unstable();
    let mut ks = String::new();
    let mut last = 0;
    for idx in toggled {
        ks.push_str(&"j".repeat(idx - last));
        ks.push(' ');
        last = idx;
    }
    ks.push('\r');
    (ks, labels)
}

async fn send_agy_writein_keystrokes(
    ctx: &WebAppCtx,
    sid: &str,
    q: &crate::cli::AskQuestionData,
    prev_text: &str,
    txt: &str,
) {
    let w_idx = find_write_in_index(&q.options);
    let _ = ctx.cli.sessions.write_to_session(sid, &format!("{}", w_idx + 1)).await;
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    if !prev_text.is_empty() {
        let bs = "\x7F".repeat(prev_text.chars().count());
        let _ = ctx.cli.sessions.write_to_session(sid, &bs).await;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }
    let _ = ctx.cli.sessions.write_to_session(sid, &format!("{txt}\r")).await;
    if q.is_multi_select {
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        let _ = ctx.cli.sessions.write_to_session(sid, "\r").await;
    }
}

async fn apply_single_or_writein(
    ctx: &WebAppCtx,
    is_agy: bool,
    sid: &str,
    state: &mut AskState,
    q: &crate::cli::AskQuestionData,
    payload: &AskAnswerPayload,
) -> Result<String, &'static str> {
    let ans = if let Some(&idx) = payload.selected_indices.first() {
        let label = q.options.get(idx).cloned().ok_or("Option index out of bounds")?;
        if is_agy {
            clear_previous_write_in_if_any(sid, &ctx.cli, state, true).await;
            let _ = ctx.cli.sessions.write_to_session(sid, &format!("{}", idx + 1)).await;
        }
        label
    } else if let Some(txt) = payload.answer.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let matched_idx = (!q.is_multi_select && !state.waiting_for_write_in)
            .then(|| crate::messenger::telegram::formatting::find_best_option(txt, &q.options))
            .flatten()
            .filter(|&i| {
                let low = q.options[i].to_lowercase();
                !low.contains("write-in") && !q.options[i].contains("직접 입력")
            });
        if let Some(idx) = matched_idx {
            if is_agy {
                clear_previous_write_in_if_any(sid, &ctx.cli, state, true).await;
                let _ = ctx.cli.sessions.write_to_session(sid, &format!("{}", idx + 1)).await;
            }
            q.options[idx].clone()
        } else {
            if is_agy {
                let prev = state.answers.get(state.current_index).map(String::as_str).unwrap_or("");
                send_agy_writein_keystrokes(ctx, sid, q, prev, txt).await;
            }
            txt.to_string()
        }
    } else {
        return Err("Missing selected_indices or answer");
    };
    if state.current_index < state.answers.len() {
        state.answers[state.current_index] = ans.clone();
    }
    Ok(ans)
}

async fn apply_ask_prev(
    ctx: &WebAppCtx,
    is_agy: bool,
    sid: &str,
    state: &mut AskState,
) -> Result<String, &'static str> {
    if state.current_index == 0 {
        return Err("Already at first question");
    }
    if is_agy {
        let _ = ctx.cli.sessions.write_to_session(sid, "\x1B[D").await;
    }
    state.current_index -= 1;
    let pq = &state.questions[state.current_index];
    state.current_bitmap = get_bitmap_for_question(pq, &state.answers, state.current_index);
    state.waiting_for_write_in = false;
    Ok("prev".to_string())
}

async fn apply_ask_skip(
    ctx: &WebAppCtx,
    is_agy: bool,
    sid: &str,
    state: &mut AskState,
) -> Result<String, &'static str> {
    if is_agy {
        clear_previous_write_in_if_any(sid, &ctx.cli, state, true).await;
        let _ = ctx.cli.sessions.write_to_session(sid, "\x1B").await;
    }
    if state.current_index < state.answers.len() {
        state.answers[state.current_index] = "User Skipped".to_string();
    }
    Ok("User Skipped".to_string())
}

async fn apply_ask_submit(
    ctx: &WebAppCtx,
    is_agy: bool,
    sid: &str,
    state: &mut AskState,
    q: &crate::cli::AskQuestionData,
    selected_indices: &[usize],
) -> Result<String, &'static str> {
    let prev = state.answers.get(state.current_index).map(String::as_str).unwrap_or("");
    let (ks, opts) = build_multi_keystrokes_from_indices(&q.options, selected_indices, prev);
    let joined = if opts.is_empty() { "None".to_string() } else { opts.join(", ") };
    if is_agy {
        clear_previous_write_in_if_any(sid, &ctx.cli, state, true).await;
        let _ = ctx.cli.sessions.write_to_session(sid, &ks).await;
    }
    if state.current_index < state.answers.len() {
        state.answers[state.current_index] = joined.clone();
    }
    Ok(joined)
}

async fn is_ask_tui_prompt_rendered(
    ctx: &WebAppCtx,
    sid: &str,
    q_idx: usize,
    q_total: usize,
) -> bool {
    let slice = {
        let holders = ctx.cli.sessions.holders.lock().await;
        let Some(h) = holders.get(sid) else {
            return true;
        };
        let off = h.turn_start_offset;
        let raw = h.output.lock().await;
        let start = off.min(raw.len()).max(raw.len().saturating_sub(65_536));
        raw[start..].to_vec()
    };
    let clean = crate::cli::antigravity::pty_spawner::strip_ansi(&String::from_utf8_lossy(&slice));
    let marker = format!("Question {}/{}:", q_idx + 1, q_total);
    clean.contains(&marker)
        || (q_idx == 0 && clean.contains("Question") && (clean.contains("esc Skip") || clean.contains("Navigate ·")))
}

async fn wait_for_agy_ask_tui_ready(ctx: &WebAppCtx, sid: &str, q_idx: usize, q_total: usize) {
    if cfg!(test) || sid.starts_with("mock-") || !ctx.cli.sessions.is_active(sid).await {
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(4500);
    while std::time::Instant::now() < deadline {
        if is_ask_tui_prompt_rendered(ctx, sid, q_idx, q_total).await {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }
}

pub(crate) async fn apply_ask_step_input(
    ctx: &WebAppCtx,
    sess: &SessionData,
    sid: &str,
    state: &mut AskState,
    payload: &AskAnswerPayload,
) -> Result<String, &'static str> {
    let q = state.questions.get(state.current_index).cloned().ok_or("Invalid question index")?;
    let is_agy = SessionData::effective_provider(&sess.provider, &ctx.config.provider) == "antigravity";
    if is_agy {
        wait_for_agy_ask_tui_ready(ctx, sid, state.current_index, state.questions.len()).await;
    }
    let res = match payload.action.as_str() {
        "prev" => apply_ask_prev(ctx, is_agy, sid, state).await,
        "skip" => apply_ask_skip(ctx, is_agy, sid, state).await,
        "submit" => apply_ask_submit(ctx, is_agy, sid, state, &q, &payload.selected_indices).await,
        _ => apply_single_or_writein(ctx, is_agy, sid, state, &q, payload).await,
    };
    if is_agy && !cfg!(test) && res.is_ok() {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    res
}
