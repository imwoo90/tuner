//! # Voice Desk Secretary Tool Executor (`voice_tools.rs`)
//!
//! ## Overview
//! Executes front-desk orchestration tools (`list_topics`, `get_topic_context`,
//! `delegate_to_topic`, `answer_topic_question`, `stop_topic`, `switch_active_topic`)
//! directly against [`super::WebAppCtx`].
//!
//! ## Search Tags
//! #voice-secretary, #gemini-live-tools, #topic-delegation, #voice-ask

use super::api_ask::{process_validated_ask, AskAnswerPayload};
use super::api_history::{build_active_ask_view, collect_history_messages};
use super::api_sessions::{build_session_summary, CreateSessionPayload};
use super::events::emit_session_updated;
use super::turn_runner::execute_web_turn;
use super::voice_models::{
    apply_voice_spec_to_session, exec_configure_topic, exec_list_available_models,
    resolve_spoken_model_spec,
};
use super::WebAppCtx;
use crate::session::{data::SessionData, key::SessionKey};
use serde_json::{json, Value};
use std::sync::Arc;

async fn resolve_key_or_default(
    ctx: &WebAppCtx,
    explicit: Option<&str>,
    current_topic: Option<&str>,
    prefer_asking: bool,
) -> Option<SessionKey> {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty())
        && let Ok(k) = raw.parse::<SessionKey>()
    {
        return Some(k);
    }
    if prefer_asking {
        let all = ctx.sessions.list_all().await.unwrap_or_default();
        for s in &all {
            if super::api_sessions::check_busy_and_asking(ctx, s).await.1 {
                return Some(s.session_key());
            }
        }
    }
    if let Some(cur) = current_topic.map(str::trim).filter(|s| !s.is_empty())
        && let Ok(k) = cur.parse::<SessionKey>()
    {
        return Some(k);
    }
    ctx.sessions.list_all().await.ok().and_then(|mut v| {
        v.sort_by(|a, b| b.last_active.cmp(&a.last_active));
        v.into_iter().next().map(|s| s.session_key())
    })
}

async fn exec_list_topics(ctx: &WebAppCtx) -> Value {
    let mut all = ctx.sessions.list_all().await.unwrap_or_default();
    all.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    let mut items = Vec::new();
    for s in all.iter().take(20) {
        let sum = build_session_summary(ctx, s).await;
        items.push(json!({
            "session_key": sum.session_key,
            "title": sum.topic_name.unwrap_or_else(|| format!("{} #{:?}", sum.transport, sum.topic_id)),
            "transport": sum.transport,
            "provider": sum.provider,
            "model": sum.model,
            "busy": sum.busy,
            "asking": sum.asking,
            "last_active": sum.last_active
        }));
    }
    json!({ "count": items.len(), "topics": items })
}

async fn exec_get_context(ctx: &WebAppCtx, args: &Value, cur: Option<&str>) -> Value {
    let sk_arg = args.get("session_key").and_then(Value::as_str);
    let Some(key) = resolve_key_or_default(ctx, sk_arg, cur, false).await else {
        return json!({ "error": "No active topics exist yet." });
    };
    let Ok(Some(sess)) = ctx.sessions.get_active(&key).await else {
        return json!({ "error": "Topic not found." });
    };
    let prov = SessionData::effective_provider(&sess.provider, &ctx.config.provider);
    let sid = sess.get_session_id(prov);
    let max_n = args.get("max_messages").and_then(Value::as_u64).unwrap_or(6).clamp(1, 15) as usize;
    let recent: Vec<Value> = collect_history_messages(ctx, &sess, prov, &sid)
        .into_iter()
        .rev()
        .take(max_n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|m| {
            let txt: String = m["text"].as_str().unwrap_or("").chars().take(600).collect();
            json!({ "sender": m["sender"], "text": txt, "timestamp": m["timestamp"] })
        })
        .collect();
    let ask = build_active_ask_view(ctx, &sess, prov, &sid).await;
    let stream_tail = ctx
        .events
        .get_live_stream(&key.storage_key())
        .await
        .map(|s| s.chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect::<String>());
    let summary = build_session_summary(ctx, &sess).await;
    json!({ "session": summary, "active_ask": ask, "live_stream_tail": stream_tail, "recent_messages": recent })
}

fn opt_str<'a>(args: &'a Value, field: &str) -> Option<&'a str> {
    args.get(field).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

async fn create_voice_topic(
    ctx: &Arc<WebAppCtx>,
    title: &str,
    tr: Option<String>,
    raw_spec: (Option<&str>, Option<&str>, Option<&str>),
) -> Result<(SessionKey, String, String, Option<String>), String> {
    let (p, m, e) = resolve_spoken_model_spec(ctx, &ctx.config.provider, raw_spec.0, raw_spec.1, raw_spec.2).await?;
    let req = CreateSessionPayload {
        transport: tr,
        chat_id: None,
        silent_greeting: true,
        topic_name: Some(title.to_string()),
        provider: Some(p.clone()),
        model: Some(m.clone()),
        effort: e.clone(),
    };
    let sess = super::tg_topics::create_session_from_payload(ctx, req)
        .await
        .map_err(|_| "Failed to create topic".to_string())?;
    emit_session_updated(ctx, &sess, false, false).await;
    Ok((sess.session_key(), p, m, e))
}

async fn exec_delegate(ctx: &Arc<WebAppCtx>, args: &Value, cur: &mut Option<String>) -> (Value, Option<String>) {
    let Some(prompt) = opt_str(args, "prompt").map(str::to_string) else {
        return (json!({ "error": "prompt is required" }), None);
    };
    let tr = opt_str(args, "transport").map(str::to_string);
    let raw_spec = (opt_str(args, "provider"), opt_str(args, "model"), opt_str(args, "effort"));

    let resolved = if let Some(title) = opt_str(args, "create_new_topic_title") {
        create_voice_topic(ctx, title, tr, raw_spec).await
    } else if let Some(k) = resolve_key_or_default(ctx, opt_str(args, "session_key"), cur.as_deref(), false).await {
        apply_voice_spec_to_session(ctx, &k, raw_spec.0, raw_spec.1, raw_spec.2, None)
            .await
            .map(|(p, m, e)| (k, p, m, e))
    } else {
        let auto_title: String = prompt.chars().take(28).collect();
        create_voice_topic(ctx, &auto_title, Some("web".to_string()), raw_spec).await
    };
    let (target_key, eff_prov, eff_model, eff_effort) = match resolved {
        Ok(v) => v,
        Err(e) => return (json!({ "error": e }), None),
    };

    let sk = target_key.storage_key();
    *cur = Some(sk.clone());
    let ctx_bg = ctx.clone();
    tokio::spawn(async move {
        execute_web_turn(ctx_bg, target_key, prompt, Vec::new()).await;
    });
    let out = json!({
        "delegated": true,
        "session_key": sk,
        "provider": eff_prov,
        "model": eff_model,
        "effort": eff_effort
    });
    (out, Some(sk))
}

async fn exec_answer_ask(ctx: &Arc<WebAppCtx>, args: &Value, cur: Option<&str>) -> Value {
    let sk_arg = args.get("session_key").and_then(Value::as_str);
    let Some(key) = resolve_key_or_default(ctx, sk_arg, cur, true).await else {
        return json!({ "error": "No topic with an open AskQuestion found." });
    };
    let skip = args.get("skip").and_then(Value::as_bool).unwrap_or(false);
    let selected_indices = args.get("option_index").and_then(Value::as_u64).map(|i| vec![i as usize]).unwrap_or_default();
    let payload = AskAnswerPayload {
        session_key: key.storage_key(),
        action: if skip { "skip".to_string() } else { "answer".to_string() },
        answer: args.get("answer").and_then(Value::as_str).map(str::to_string),
        selected_indices,
    };
    let resp = process_validated_ask(ctx.clone(), key.clone(), payload).await;
    json!({ "ok": resp.status().is_success(), "session_key": key.storage_key() })
}

async fn exec_stop_topic(ctx: &Arc<WebAppCtx>, args: &Value, cur: Option<&str>) -> Value {
    let sk_arg = args.get("session_key").and_then(Value::as_str);
    let Some(key) = resolve_key_or_default(ctx, sk_arg, cur, false).await else {
        return json!({ "error": "No active topic found." });
    };
    let a = ctx.cli.sessions.interrupt(key.chat_id, key.topic_id).await;
    let b = crate::cli::claude::interrupt_session(key.chat_id, key.topic_id).await;
    let c = crate::cli::codex::interrupt_session(key.chat_id, key.topic_id).await;
    if let Ok(Some(sess)) = ctx.sessions.get_active(&key).await {
        emit_session_updated(ctx, &sess, false, false).await;
    }
    json!({ "ok": true, "session_key": key.storage_key(), "interrupted": a || b || c })
}

/// Executes a single Gemini Live tool call and returns `(tool_output_json, optional_focus_topic_key)`.
pub async fn execute_voice_tool(
    ctx: &Arc<WebAppCtx>,
    current_topic: &mut Option<String>,
    name: &str,
    args: &Value,
) -> (Value, Option<String>) {
    match name {
        "list_topics" => (exec_list_topics(ctx).await, None),
        "get_topic_context" => (exec_get_context(ctx, args, current_topic.as_deref()).await, None),
        "list_available_models" => (exec_list_available_models(ctx, args).await, None),
        "delegate_to_topic" => exec_delegate(ctx, args, current_topic).await,
        "configure_topic" => {
            let sk_arg = args.get("session_key").and_then(Value::as_str);
            match resolve_key_or_default(ctx, sk_arg, current_topic.as_deref(), false).await {
                Some(k) => (exec_configure_topic(ctx, &k, args).await, None),
                None => (json!({ "error": "No active topic found." }), None),
            }
        }
        "answer_topic_question" => (exec_answer_ask(ctx, args, current_topic.as_deref()).await, None),
        "stop_topic" => (exec_stop_topic(ctx, args, current_topic.as_deref()).await, None),
        "switch_active_topic" => {
            if let Some(sk) = args.get("session_key").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                *current_topic = Some(sk.to_string());
                (json!({ "ok": true, "focused": sk }), Some(sk.to_string()))
            } else {
                (json!({ "error": "session_key is required" }), None)
            }
        }
        other => (json!({ "error": format!("Unknown tool `{other}`") }), None),
    }
}
