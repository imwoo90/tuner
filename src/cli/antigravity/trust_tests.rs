//! # Antigravity Workspace Trust Tests
//!
//! This module validates settings.json manipulation to auto-trust workspace directories.

#[cfg(test)]
mod tests {
    use crate::cli::antigravity::trust::trust_workspace_in_settings;
    use std::fs;

    #[test]
    fn test_trust_workspace_nonexistent_parent() {
        let temp = tempfile::tempdir().unwrap();
        // Since .gemini/antigravity-cli parent does not exist inside temp dir, it should not create anything
        let home_dir = temp.path().to_path_buf();
        let workspace = home_dir.join("my_workspace");

        trust_workspace_in_settings(&workspace, Some(home_dir.clone()));

        let settings_path = home_dir.join(".gemini").join("antigravity-cli").join("settings.json");
        assert!(!settings_path.exists());
    }

    #[test]
    fn test_trust_workspace_creates_settings_json() {
        let temp = tempfile::tempdir().unwrap();
        let home_dir = temp.path().to_path_buf();
        
        // Create the parent directory first
        let parent = home_dir.join(".gemini").join("antigravity-cli");
        fs::create_dir_all(&parent).unwrap();

        let workspace = temp.path().join("my_workspace");
        fs::create_dir_all(&workspace).unwrap();

        trust_workspace_in_settings(&workspace, Some(home_dir.clone()));

        let settings_path = parent.join("settings.json");
        assert!(settings_path.exists());

        let content = fs::read_to_string(settings_path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();
        
        let workspaces = data.get("trustedWorkspaces").unwrap().as_array().unwrap();
        assert_eq!(workspaces.len(), 1);
        
        let expected_path = workspace.canonicalize().unwrap().to_string_lossy().to_string();
        assert_eq!(workspaces[0].as_str().unwrap(), expected_path);
    }

    #[test]
    fn test_trust_workspace_appends_to_existing() {
        let temp = tempfile::tempdir().unwrap();
        let home_dir = temp.path().to_path_buf();
        let parent = home_dir.join(".gemini").join("antigravity-cli");
        fs::create_dir_all(&parent).unwrap();

        let settings_path = parent.join("settings.json");
        let initial_json = r#"{"trustedWorkspaces": ["/some/other/path"]}"#;
        fs::write(&settings_path, initial_json).unwrap();

        let workspace = temp.path().join("my_workspace_2");
        fs::create_dir_all(&workspace).unwrap();

        trust_workspace_in_settings(&workspace, Some(home_dir.clone()));

        let content = fs::read_to_string(settings_path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();
        let workspaces = data.get("trustedWorkspaces").unwrap().as_array().unwrap();
        
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].as_str().unwrap(), "/some/other/path");
        
        let expected_path = workspace.canonicalize().unwrap().to_string_lossy().to_string();
        assert_eq!(workspaces[1].as_str().unwrap(), expected_path);
    }

    #[test]
    fn test_trust_workspace_avoids_duplicate() {
        let temp = tempfile::tempdir().unwrap();
        let home_dir = temp.path().to_path_buf();
        let parent = home_dir.join(".gemini").join("antigravity-cli");
        fs::create_dir_all(&parent).unwrap();

        let workspace = temp.path().join("my_workspace_3");
        fs::create_dir_all(&workspace).unwrap();
        let resolved_ws = workspace.canonicalize().unwrap().to_string_lossy().to_string();

        let settings_path = parent.join("settings.json");
        let initial_json = format!(r#"{{"trustedWorkspaces": ["{}"]}}"#, resolved_ws);
        fs::write(&settings_path, initial_json).unwrap();

        trust_workspace_in_settings(&workspace, Some(home_dir.clone()));

        let content = fs::read_to_string(settings_path).unwrap();
        let data: serde_json::Value = serde_json::from_str(&content).unwrap();
        let workspaces = data.get("trustedWorkspaces").unwrap().as_array().unwrap();
        
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].as_str().unwrap(), resolved_ws);
    }
}
