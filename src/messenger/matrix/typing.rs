//! RAII MatrixTypingGuard keep-alive.

use std::time::Duration;
use tokio::task::JoinHandle;
use matrix_sdk::{Client, ruma::OwnedRoomId, ruma::RoomId};

pub struct MatrixTypingGuard {
    client: Client,
    room_id: OwnedRoomId,
    handle: JoinHandle<()>,
}

impl MatrixTypingGuard {
    pub async fn new(
        client: Client,
        room_id: &RoomId,
        interval: Duration,
        _timeout: Duration,
    ) -> Result<Self, anyhow::Error> {
        let room_id_owned = room_id.to_owned();
        
        if let Some(room) = client.get_room(&room_id_owned) {
            let _ = room.typing_notice(true).await;
        }

        let client_clone = client.clone();
        let room_id_clone = room_id_owned.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if let Some(room) = client_clone.get_room(&room_id_clone) {
                    let _ = room.typing_notice(true).await;
                }
            }
        });

        Ok(Self {
            client,
            room_id: room_id_owned,
            handle,
        })
    }
}

impl Drop for MatrixTypingGuard {
    fn drop(&mut self) {
        self.handle.abort();
        let client = self.client.clone();
        let room_id = self.room_id.clone();
        tokio::spawn(async move {
            if let Some(room) = client.get_room(&room_id) {
                let _ = room.typing_notice(false).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::room_id;

    #[tokio::test]
    async fn test_typing_guard_lifecycle() {
        let client = Client::builder()
            .homeserver_url("https://localhost:1234")
            .build()
            .await
            .unwrap();
        let r_id = room_id!("!test:localhost");
        
        let guard = MatrixTypingGuard::new(
            client,
            r_id,
            Duration::from_millis(50),
            Duration::from_secs(1),
        ).await;
        
        assert!(guard.is_ok());
        let g = guard.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(g);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

