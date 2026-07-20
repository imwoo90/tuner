//! # Matrix Connection Credentials Manager
//!
//! Handles encryption keys, access tokens, homeserver URLs, and room association caches for the
//! Matrix interface.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use serde::{Serialize, Deserialize};
use matrix_sdk::{
    Client, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession,
    ruma::UserId,
    AuthSession,
};

#[derive(Serialize, Deserialize, Debug)]
pub struct MatrixCredentials {
    pub user_id: String,
    pub device_id: String,
    pub access_token: String,
}

async fn restore_from_file(client: &Client, creds_file: &Path) -> Result<(), String> {
    if creds_file.exists() {
        let content = fs::read_to_string(creds_file).map_err(|e| e.to_string())?;
        let creds = serde_json::from_str::<MatrixCredentials>(&content).map_err(|e| e.to_string())?;
        let user_id = UserId::parse(&creds.user_id).map_err(|e| e.to_string())?;
        let session = MatrixSession {
            meta: SessionMeta {
                user_id,
                device_id: creds.device_id.into(),
            },
            tokens: SessionTokens {
                access_token: creds.access_token,
                refresh_token: None,
            },
        };
        client.restore_session(session).await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("Credentials file does not exist".to_string())
}

async fn restore_from_config(
    client: &Client,
    creds_file: &Path,
    config_user_id: &str,
    token: &str,
    dev_id: &str,
) -> Result<(), String> {
    let user_id = UserId::parse(config_user_id).map_err(|e| e.to_string())?;
    let session = MatrixSession {
        meta: SessionMeta {
            user_id: user_id.clone(),
            device_id: dev_id.to_string().into(),
        },
        tokens: SessionTokens {
            access_token: token.to_string(),
            refresh_token: None,
        },
    };
    client.restore_session(session).await.map_err(|e| e.to_string())?;
    save_credentials(creds_file, config_user_id, dev_id, token)?;
    Ok(())
}

pub async fn login_or_restore(
    client: &Client,
    store_path: &Path,
    config_user_id: &str,
    config_access_token: Option<&str>,
    config_device_id: Option<&str>,
    config_password: Option<&str>,
) -> Result<(), String> {
    let creds_file = store_path.join("credentials.json");

    // 1. Try saved credentials from previous session
    if restore_from_file(client, &creds_file).await.is_ok() {
        return Ok(());
    }

    // 2. Try config access_token + device_id
    if let (Some(token), Some(dev_id)) = (config_access_token, config_device_id) {
        if restore_from_config(client, &creds_file, config_user_id, token, dev_id).await.is_ok() {
            return Ok(());
        }
    }

    // 3. First login with password
    let password = config_password.ok_or_else(|| {
        "Matrix AUTH FAILED: No access_token, device_id, or password configured.".to_string()
    })?;

    client
        .matrix_auth()
        .login_username(config_user_id, password)
        .initial_device_display_name("ductor")
        .send()

        .await
        .map_err(|e| e.to_string())?;

    if let Some(AuthSession::Matrix(session)) = client.session() {
        save_credentials(
            &creds_file,
            session.meta.user_id.as_str(),
            session.meta.device_id.as_str(),
            session.tokens.access_token.as_str(),
        )?;
    } else {
        return Err("Matrix AUTH FAILED: Could not retrieve session after password login.".to_string());
    }

    Ok(())
}

fn save_credentials(
    path: &Path,
    user_id: &str,
    device_id: &str,
    access_token: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = MatrixCredentials {
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        access_token: access_token.to_string(),
    };
    let content = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_save_credentials_file_mode() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("creds.json");
        save_credentials(&path, "@user:localhost", "DEV1", "TOKEN1").unwrap();

        assert!(path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::metadata(&path).unwrap();
            let mode = meta.mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
