//! # Matrix Client Listener and Event Router
//!
//! Establishes long-polling sync loops with Matrix homeservers. Parses incoming events and dispatches
//! them to the central message bus.

use std::sync::Arc;
use std::path::Path;
use std::time::Duration;
use matrix_sdk::{Client, Room};
use matrix_sdk::ruma::events::room::member::{StrippedRoomMemberEvent, MembershipState};
use matrix_sdk::ruma::events::room::message::{OriginalSyncRoomMessageEvent, MessageType, RoomMessageEventContent};
use crate::config::CliConfig;
use crate::session::manager::SessionManager;
use crate::cli::antigravity::AntigravityCli;
use crate::cli::AgentProvider;
use crate::bus::bus::MessageBus;
use crate::messenger::matrix::id_map::MatrixIdMap;
use crate::messenger::matrix::message_queue::MatrixMessageQueue;
use crate::messenger::matrix::typing::MatrixTypingGuard;
use crate::messenger::matrix::credentials::login_or_restore;

#[derive(Clone)]
pub struct MatrixBot {
    client: Client,
    config: Arc<CliConfig>,
    sessions: Arc<SessionManager>,
    cli: Arc<AntigravityCli>,
    id_map: Arc<MatrixIdMap>,
    queue: Arc<MatrixMessageQueue>,
    bus: Arc<MessageBus>,
}

impl MatrixBot {
    pub async fn new(
        config: Arc<CliConfig>,
        sessions: Arc<SessionManager>,
        cli: Arc<AntigravityCli>,
        id_map: Arc<MatrixIdMap>,
        bus: Arc<MessageBus>,
    ) -> Result<Self, String> {
        let url = url::Url::parse(&config.matrix.homeserver).map_err(|e| e.to_string())?;
        let client = Client::new(url).await.map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            config,
            sessions,
            cli,
            id_map,
            queue: Arc::new(MatrixMessageQueue::new()),
            bus,
        })
    }

    async fn setup_event_handlers(&self) {
        self.client.add_event_handler(move |ev: StrippedRoomMemberEvent, room: Room| async move {
            if ev.content.membership == MembershipState::Invite {
                let _ = room.join().await;
            }
        });

        let bot2 = self.clone();
        self.client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room, client: Client| {
            let bot_clone = bot2.clone();
            async move {
                bot_clone.handle_message(ev, room, client).await;
            }
        });
    }

    fn start_sync_loop(&self, store_path: &Path) {
        let client_clone = self.client.clone();
        let token_file = store_path.join("next_batch");
        tokio::spawn(async move {
            let mut sync_settings = matrix_sdk::config::SyncSettings::default()
                .timeout(std::time::Duration::from_secs(30));

            if token_file.exists() {
                if let Ok(token) = std::fs::read_to_string(&token_file) {
                    sync_settings = sync_settings.token(token.trim().to_string());
                }
            }

            loop {
                match client_clone.sync_once(sync_settings.clone()).await {
                    Ok(response) => {
                        let next_batch = response.next_batch;
                        let _ = std::fs::write(&token_file, &next_batch);
                        sync_settings = sync_settings.token(next_batch);
                    }
                    Err(e) => {
                        eprintln!("Matrix Sync Error: {:?}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    pub async fn start(&self) -> Result<(), String> {
        let store_path = Path::new(&self.config.matrix.store_path);
        let user_id = &self.config.matrix.user_id;

        login_or_restore(
            &self.client,
            store_path,
            user_id,
            self.config.matrix.access_token.as_deref(),
            self.config.matrix.device_id.as_deref(),
            self.config.matrix.password.as_deref(),
        ).await?;

        self.setup_event_handlers().await;
        self.start_sync_loop(store_path);

        Ok(())
    }

    fn validate_message(&self, ev: &OriginalSyncRoomMessageEvent, room: &Room, client: &Client) -> Option<(String, String)> {
        let room_id = room.room_id().to_string();
        let self_user_id = client.user_id()?;

        if ev.sender == self_user_id {
            return None;
        }

        if self.queue.is_duplicate(ev.event_id.as_str()) {
            return None;
        }

        let text = match &ev.content.msgtype {
            MessageType::Text(text_content) => &text_content.body,
            _ => return None,
        };

        if !self.config.matrix.allowed_rooms.contains(&room_id) {
            return None;
        }
        if !self.config.matrix.allowed_users.contains(&ev.sender.to_string()) {
            return None;
        }

        let is_dm = room.name().is_none() && room.canonical_alias().is_none() && room.active_members_count() <= 2;
        let is_addressed = is_dm || text.contains(self_user_id.as_str());

        if !is_addressed {
            return None;
        }

        let prompt = text.replace(self_user_id.as_str(), "").trim().to_string();
        Some((prompt, room_id))
    }

    fn process_message_task(&self, chat_id: i64, room: Room, prompt: String) -> tokio::task::JoinHandle<()> {
        let bot_clone = self.clone();
        tokio::spawn(async move {
            let lock = bot_clone.bus.lock_pool().get_chat(chat_id);
            let _guard = lock.lock().await;

            let typing_guard = match MatrixTypingGuard::new(
                bot_clone.client.clone(),
                &room.room_id(),
                Duration::from_secs(3),
                Duration::from_secs(4),
            ).await {
                Ok(g) => Some(g),
                Err(_) => None,
            };

            let key = crate::session::key::SessionKey::matrix(chat_id);
            let default_model = bot_clone.config.model.as_deref().unwrap_or("antigravity-default");

            if let Ok((session, _)) = bot_clone.sessions.resolve_session(&key, &bot_clone.config.provider, default_model).await {

                let session_id = session.get_session_id(&bot_clone.config.provider);
                let opt_sid = if session_id.is_empty() { None } else { Some(&session_id[..]) };

                let res = bot_clone.cli.send(&prompt, opt_sid, false, bot_clone.config.working_dir.clone()).await;

                drop(typing_guard);

                match res {
                    Ok(resp) => {
                        let mut env = crate::bus::envelope::Envelope::new(crate::bus::envelope::Origin::User, chat_id);
                        env.result_text = resp.result;
                        env.transport = "mx".to_string();
                        let _ = bot_clone.bus.submit(&mut env).await;
                    }
                    Err(e) => {
                        let content = RoomMessageEventContent::text_plain(format!("Error: {}", e));
                        let _ = room.send(content).await;
                    }
                }
            }
        })
    }

    async fn handle_message(&self, ev: OriginalSyncRoomMessageEvent, room: Room, client: Client) {
        let (prompt, room_id) = match self.validate_message(&ev, &room, &client) {
            Some(res) => res,
            None => return,
        };

        let chat_id = self.id_map.room_to_int(&room_id);

        if prompt == "/stop" || prompt == "/abort" {
            self.queue.drain(chat_id);
            let content = RoomMessageEventContent::text_plain("Stopped processing.");
            let _ = room.send(content).await;
            return;
        }

        let task = self.process_message_task(chat_id, room, prompt);
        self.queue.track(chat_id, task);
    }
}
