//! # Workspace Paths Resolution
//!
//! ## Overview
//! SINGLE SOURCE OF TRUTH for all workspace and configuration paths in tuner.
//! Resolves canonical directories for profiles, sessions, cron tasks, webhooks, logs, and rule templates.
//!
//! ## Search Tags
//! #paths

use std::path::PathBuf;

/// Immutable paths resolved for the workspace layout.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DuctorPaths {
    /// Tuner home.
    pub tuner_home: PathBuf,
    /// Home defaults.
    pub home_defaults: PathBuf,
    /// Framework root.
    pub framework_root: PathBuf,
    /// Profile.
    pub profile: Option<String>,
}

impl DuctorPaths {
    /// Create a new DuctorPaths instance.
    pub fn new(tuner_home: PathBuf, home_defaults: PathBuf, framework_root: PathBuf, profile: Option<String>) -> Self {
        Self {
            tuner_home,
            home_defaults,
            framework_root,
            profile,
        }
    }

    /// Profile home directory: `~/.tuner/profiles/<profile_name>/` or fallback to `~/.tuner/`
    pub fn profile_home(&self) -> PathBuf {
        if let Some(ref p) = self.profile {
            self.tuner_home.join("profiles").join(p)
        } else {
            self.tuner_home.clone()
        }
    }

    /// Resolve a profile-specific path.
    fn profile_join(&self, filename: &str) -> PathBuf {
        self.profile_home().join(filename)
    }

    /// User agent workspace directory
    pub fn workspace(&self) -> PathBuf {
        self.profile_join("workspace")
    }

    /// Dedicated remote development workspace directory: `~/.tuner/profiles/<profile_name>/remote_workspace`
    pub fn remote_workspace(&self) -> PathBuf {
        self.profile_join("remote_workspace")
    }

    /// Configuration directory: `~/.tuner/config`
    pub fn config_dir(&self) -> PathBuf {
        self.tuner_home.join("config")
    }

    /// Configuration path: `~/.tuner/config/config.json`
    pub fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.json")
    }

    /// Sessions path
    pub fn sessions_path(&self) -> PathBuf {
        self.profile_join("sessions.json")
    }

    /// Review sessions path
    pub fn review_sessions_path(&self) -> PathBuf {
        self.profile_join("review_sessions.json")
    }

    /// Encrypted credential vault directory: `~/.tuner/profiles/<profile_name>/vault`
    pub fn vault_dir(&self) -> PathBuf {
        self.profile_join("vault")
    }

    /// Cron jobs path
    pub fn cron_jobs_path(&self) -> PathBuf {
        self.profile_join("cron_jobs.json")
    }

    /// Webhooks path
    pub fn webhooks_path(&self) -> PathBuf {
        self.profile_join("webhooks.json")
    }

    /// Logs directory: `~/.tuner/logs`
    pub fn logs_dir(&self) -> PathBuf {
        self.tuner_home.join("logs")
    }

    /// Cron tasks directory: `~/.tuner/workspace/cron_tasks`
    pub fn cron_tasks_dir(&self) -> PathBuf {
        self.workspace().join("cron_tasks")
    }

    /// Tools directory: `~/.tuner/workspace/tools`
    pub fn tools_dir(&self) -> PathBuf {
        self.workspace().join("tools")
    }

    /// Output to user directory: `~/.tuner/workspace/output_to_user`
    pub fn output_to_user_dir(&self) -> PathBuf {
        self.workspace().join("output_to_user")
    }

    /// Telegram files directory: `~/.tuner/workspace/telegram_files`
    pub fn telegram_files_dir(&self) -> PathBuf {
        self.workspace().join("telegram_files")
    }

    /// User skills directory: `~/.tuner/workspace/skills`
    pub fn skills_dir(&self) -> PathBuf {
        self.workspace().join("skills")
    }

    /// Profile-isolated Antigravity native skills directory: `~/.tuner/profiles/<profile_name>/workspace/.agents/skills`
    pub fn agents_skills_dir(&self) -> PathBuf {
        self.workspace().join(".agents").join("skills")
    }

    /// Package-bundled skills directory: `{home_defaults}/workspace/skills`
    pub fn bundled_skills_dir(&self) -> PathBuf {
        self.home_defaults.join("workspace").join("skills")
    }

    /// User-managed environment file: `~/.tuner/.env`
    pub fn env_file(&self) -> PathBuf {
        self.tuner_home.join(".env")
    }

    /// Main memory path: `~/.tuner/workspace/.agents/rules/mainmemory.md`
    pub fn mainmemory_path(&self) -> PathBuf {
        self.workspace().join(".agents").join("rules").join("mainmemory.md")
    }

    /// Config example path: `{framework_root}/config.example.json` or fallback.
    pub fn config_example_path(&self) -> PathBuf {
        let repo_path = self.framework_root.join("config.example.json");
        if repo_path.is_file() {
            repo_path
        } else {
            self.home_defaults.join("_config_example.json")
        }
    }
}

/// Resolves DuctorPaths from explicit values, environment variables, or defaults.
pub fn resolve_paths(
    tuner_home: Option<PathBuf>,
    framework_root: Option<PathBuf>,
    home_defaults: Option<PathBuf>,
    profile: Option<String>,
) -> DuctorPaths {
    let home = tuner_home
        .or_else(|| std::env::var("TUNER_HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(base).join(".tuner")
        });

    let root = framework_root
        .or_else(|| std::env::var("TUNER_FRAMEWORK_ROOT").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

    let defaults = home_defaults
        .or_else(|| std::env::var("TUNER_HOME_DEFAULTS").ok().map(PathBuf::from))
        .unwrap_or_else(|| root.join("_home_defaults"));
    let defaults = std::fs::canonicalize(&defaults).unwrap_or(defaults);

    DuctorPaths::new(home, defaults, root, profile)
}

/// Extracts `(profile_name, vault_dir)` when `working_dir` points to `<tuner_home>/profiles/<profile>/workspace`.
pub fn extract_profile_from_working_dir(working_dir: &std::path::Path) -> Option<(String, PathBuf)> {
    let profile_home = working_dir.parent()?;
    let profiles_dir = profile_home.parent()?;
    if profiles_dir.file_name().and_then(|s| s.to_str()) == Some("profiles") {
        let profile = profile_home.file_name()?.to_str()?.to_string();
        if !profile.is_empty() {
            return Some((profile, profile_home.join("vault")));
        }
    }
    None
}
