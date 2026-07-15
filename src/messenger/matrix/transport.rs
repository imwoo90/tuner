//! Matrix delivery adapter for the MessageBus.
//!
//! Translates Envelope instances into Matrix messages.

use async_trait::async_trait;
use crate::bus::bus::Transport;
use crate::bus::envelope::{Envelope, Origin};

use crate::messenger::matrix::id_map::MatrixIdMap;
use crate::messenger::matrix::formatting::markdown_to_matrix_html;
use matrix_sdk::Client;
use matrix_sdk::ruma::RoomId;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use std::sync::Arc;

pub struct MatrixTransport {
    client: Client,
    id_map: Arc<MatrixIdMap>,
    allowed_rooms: Vec<String>,
}

impl MatrixTransport {
    pub fn new(client: Client, id_map: Arc<MatrixIdMap>, allowed_rooms: Vec<String>) -> Self {
        Self { client, id_map, allowed_rooms }
    }

    fn format_message(&self, env: &Envelope) -> String {
        match env.origin {
            Origin::Background => self.format_background(env),
            Origin::Cron => self.format_cron(env),
            Origin::Heartbeat => env.result_text.clone(),
            Origin::Interagent => self.format_interagent(env),
            Origin::TaskResult => self.format_task_result(env),
            Origin::TaskQuestion => self.format_task_question(env),
            Origin::WebhookWake => env.result_text.clone(),
            _ => env.result_text.clone(),
        }
    }

    fn format_background(&self, env: &Envelope) -> String {
        let elapsed = format!("{:.0}s", env.elapsed_seconds);
        if !env.session_name.is_empty() {
            if env.status == "aborted" {
                format!("**[{}] Cancelled**\n\n_{}_", env.session_name, env.prompt_preview)
            } else if env.is_error {
                let body = if env.result_text.len() > 2000 {
                    &env.result_text[..2000]
                } else if !env.result_text.is_empty() {
                    &env.result_text
                } else {
                    "_No output._"
                };
                format!("**[{}] Failed** ({})\n\n{}", env.session_name, elapsed, body)
            } else {
                let body = if !env.result_text.is_empty() { &env.result_text } else { "_No output._" };
                format!("**[{}] Complete** ({})\n\n{}", env.session_name, elapsed, body)
            }
        } else {
            let task_id = env.metadata.get("task_id").map(|s| s.as_str()).unwrap_or("?");
            if env.status == "aborted" {
                format!("**Background Task Cancelled**\n\nTask `{}` was cancelled.\nPrompt: _{}_", task_id, env.prompt_preview)
            } else if env.is_error {
                let body = if env.result_text.len() > 2000 {
                    &env.result_text[..2000]
                } else if !env.result_text.is_empty() {
                    &env.result_text
                } else {
                    "_No output._"
                };
                format!("**Background Task Failed** ({})\n\nTask `{}` failed ({}).\nPrompt: _{}_\n\n{}", elapsed, task_id, env.status, env.prompt_preview, body)
            } else {
                let body = if !env.result_text.is_empty() { &env.result_text } else { "_No output._" };
                format!("**Background Task Complete** ({})\n\n{}", elapsed, body)
            }
        }
    }

    fn format_cron(&self, env: &Envelope) -> String {
        let title = env.metadata.get("title").map(|s| s.as_str()).unwrap_or("?");
        let clean_result = sanitize_cron_result_text(&env.result_text);
        if !env.result_text.is_empty() && clean_result.is_empty() && env.status == "success" {
            return String::new();
        }
        if !clean_result.is_empty() {
            format!("**TASK: {}**\n\n{}", title, clean_result)
        } else {
            format!("**TASK: {}**\n\n_{}_", title, env.status)
        }
    }

    fn format_interagent(&self, env: &Envelope) -> String {
        if env.is_error {
            let session_info = if !env.session_name.is_empty() {
                format!("\nSession: `{}`", env.session_name)
            } else {
                "".to_string()
            };
            format!(
                "**Inter-Agent Request Failed**\n\nAgent: `{}`{}\nError: {}\nRequest: _{}_",
                env.metadata.get("recipient").map(|s| s.as_str()).unwrap_or("?"),
                session_info,
                env.metadata.get("error").map(|s| s.as_str()).unwrap_or("unknown"),
                env.prompt_preview
            )
        } else {
            let mut parts = Vec::new();
            if let Some(notice) = env.metadata.get("provider_switch_notice") {
                if !notice.is_empty() {
                    parts.push(format!("**Provider Switch Detected**\n\n{}", notice));
                }
            }
            if !env.result_text.is_empty() {
                parts.push(env.result_text.clone());
            }
            parts.join("\n\n")
        }
    }

    fn format_task_result(&self, env: &Envelope) -> String {
        let name = env.metadata.get("name").or_else(|| env.metadata.get("task_id")).map(|s| s.as_str()).unwrap_or("?");
        let mut parts = Vec::new();
        if env.status == "done" {
            let duration = format!("{:.0}s", env.elapsed_seconds);
            let detail = if !env.provider.is_empty() {
                format!("{}, {}/{}", duration, env.provider, env.model)
            } else {
                duration
            };
            parts.push(format!("**Task `{}` completed** ({})", name, detail));
        } else if env.status == "cancelled" {
            parts.push(format!("**Task `{}` cancelled**", name));
        } else if env.status == "failed" {
            parts.push(format!("**Task `{}` failed**\nReason: {}", name, env.metadata.get("error").map(|s| s.as_str()).unwrap_or("unknown")));
        }

        if env.needs_injection && !env.result_text.is_empty() {
            parts.push(env.result_text.clone());
        }
        parts.join("\n\n")
    }

    fn format_task_question(&self, env: &Envelope) -> String {
        let task_id = env.metadata.get("task_id").map(|s| s.as_str()).unwrap_or("?");
        let mut parts = vec![format!("**Task `{}` has a question:**\n{}", task_id, env.prompt)];
        if !env.result_text.is_empty() {
            parts.push(env.result_text.clone());
        }
        parts.join("\n\n")
    }

    async fn send_message(&self, room_id_str: &str, text: &str) -> Result<(), String> {
        let room_id = RoomId::parse(room_id_str).map_err(|e| e.to_string())?;
        let room = self.client.get_room(&room_id).ok_or_else(|| format!("Room {} not found", room_id_str))?;

        let (plain, html) = markdown_to_matrix_html(text);
        let content = RoomMessageEventContent::text_html(plain, html);
        room.send(content).await.map_err(|e| e.to_string())?;
        Ok(())
    }

}

fn sanitize_cron_result_text(result: &str) -> String {
    let mut clean_lines = Vec::new();
    for line in result.lines() {
        let normalized: String = line.split_whitespace().collect::<Vec<&str>>().join(" ").to_lowercase();
        if normalized.contains("message sent successfully") && normalized.contains("delivered to telegram") {
            continue;
        }
        clean_lines.push(line);
    }
    clean_lines.join("\n").trim().to_string()
}

#[async_trait]
impl Transport for MatrixTransport {
    fn transport_name(&self) -> &str {
        "mx"
    }

    async fn deliver(&self, envelope: &Envelope) -> Result<(), String> {
        let room_id_str = self.id_map.int_to_room(envelope.chat_id)
            .ok_or_else(|| format!("No room found for chat_id={}", envelope.chat_id))?;
        
        let text = self.format_message(envelope);
        if text.is_empty() {
            return Ok(());
        }

        self.send_message(&room_id_str, &text).await
    }

    async fn deliver_broadcast(&self, envelope: &Envelope) -> Result<(), String> {
        let text = match envelope.origin {
            Origin::WebhookCron => {
                let title = envelope.metadata.get("hook_title").map(|s| s.as_str()).unwrap_or("?");
                if !envelope.result_text.is_empty() {
                    format!("**WEBHOOK (CRON TASK): {}**\n\n{}", title, envelope.result_text)
                } else {
                    format!("**WEBHOOK (CRON TASK): {}**\n\n_{}_", title, envelope.status)
                }
            }
            _ => self.format_message(envelope),
        };

        if text.is_empty() {
            return Ok(());
        }

        for room_id_str in &self.allowed_rooms {
            let _ = self.send_message(room_id_str, &text).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_sanitize_cron_result() {
        let input = "delivered to telegram: message sent successfully\nActual cron result line";
        assert_eq!(sanitize_cron_result_text(input), "Actual cron result line");
    }
}
