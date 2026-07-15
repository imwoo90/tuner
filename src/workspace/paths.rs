//! # Workspace Paths Resolution
//!
//! SINGLE SOURCE OF TRUTH for all workspace and configuration paths in tuner.

use std::path::PathBuf;

/// Immutable paths resolved for the workspace layout.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DuctorPaths {
    pub tuner_home: PathBuf,
    pub home_defaults: PathBuf,
    pub framework_root: PathBuf,
}

impl DuctorPaths {
    /// Create a new DuctorPaths instance.
    pub fn new(tuner_home: PathBuf, home_defaults: PathBuf, framework_root: PathBuf) -> Self {
        Self {
            tuner_home,
            home_defaults,
            framework_root,
        }
    }

    /// User agent workspace directory: `~/.tuner/workspace`
    pub fn workspace(&self) -> PathBuf {
        self.tuner_home.join("workspace")
    }

    /// Configuration directory: `~/.tuner/config`
    pub fn config_dir(&self) -> PathBuf {
        self.tuner_home.join("config")
    }

    /// Configuration path: `~/.tuner/config/config.json`
    pub fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.json")
    }

    /// Sessions path: `~/.tuner/sessions.json`
    pub fn sessions_path(&self) -> PathBuf {
        self.tuner_home.join("sessions.json")
    }

    /// Cron jobs path: `~/.tuner/cron_jobs.json`
    pub fn cron_jobs_path(&self) -> PathBuf {
        self.tuner_home.join("cron_jobs.json")
    }

    /// Webhooks path: `~/.tuner/webhooks.json`
    pub fn webhooks_path(&self) -> PathBuf {
        self.tuner_home.join("webhooks.json")
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

    /// Matrix files directory: `~/.tuner/workspace/matrix_files`
    pub fn matrix_files_dir(&self) -> PathBuf {
        self.workspace().join("matrix_files")
    }

    /// API files directory: `~/.tuner/workspace/api_files`
    pub fn api_files_dir(&self) -> PathBuf {
        self.workspace().join("api_files")
    }

    /// Memory system directory: `~/.tuner/workspace/memory_system`
    pub fn memory_system_dir(&self) -> PathBuf {
        self.workspace().join("memory_system")
    }

    /// User skills directory: `~/.tuner/workspace/skills`
    pub fn skills_dir(&self) -> PathBuf {
        self.workspace().join("skills")
    }

    /// Package-bundled skills directory: `{home_defaults}/workspace/skills`
    pub fn bundled_skills_dir(&self) -> PathBuf {
        self.home_defaults.join("workspace").join("skills")
    }

    /// Tasks directory: `~/.tuner/workspace/tasks`
    pub fn tasks_dir(&self) -> PathBuf {
        self.workspace().join("tasks")
    }

    /// Tasks registry path: `~/.tuner/tasks.json`
    pub fn tasks_registry_path(&self) -> PathBuf {
        self.tuner_home.join("tasks.json")
    }

    /// Chat activity path: `~/.tuner/chat_activity.json`
    pub fn chat_activity_path(&self) -> PathBuf {
        self.tuner_home.join("chat_activity.json")
    }

    /// Named sessions path: `~/.tuner/named_sessions.json`
    pub fn named_sessions_path(&self) -> PathBuf {
        self.tuner_home.join("named_sessions.json")
    }

    /// Startup state path: `~/.tuner/startup_state.json`
    pub fn startup_state_path(&self) -> PathBuf {
        self.tuner_home.join("startup_state.json")
    }

    /// Inflight turns path: `~/.tuner/inflight_turns.json`
    pub fn inflight_turns_path(&self) -> PathBuf {
        self.tuner_home.join("inflight_turns.json")
    }

    /// User-managed environment file: `~/.tuner/.env`
    pub fn env_file(&self) -> PathBuf {
        self.tuner_home.join(".env")
    }

    /// Main memory path: `~/.tuner/workspace/memory_system/MAINMEMORY.md`
    pub fn mainmemory_path(&self) -> PathBuf {
        self.memory_system_dir().join("MAINMEMORY.md")
    }

    /// Join notification path: `~/.tuner/workspace/JOIN_NOTIFICATION.md`
    pub fn join_notification_path(&self) -> PathBuf {
        self.workspace().join("JOIN_NOTIFICATION.md")
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

    /// Dockerfile sandbox path: `{framework_root}/Dockerfile.sandbox` or fallback.
    pub fn dockerfile_sandbox_path(&self) -> PathBuf {
        let repo_path = self.framework_root.join("Dockerfile.sandbox");
        if repo_path.is_file() {
            repo_path
        } else {
            self.home_defaults.join("_Dockerfile.sandbox")
        }
    }
}

/// Resolves DuctorPaths from explicit values, environment variables, or defaults.
pub fn resolve_paths(
    tuner_home: Option<PathBuf>,
    framework_root: Option<PathBuf>,
    home_defaults: Option<PathBuf>,
) -> DuctorPaths {
    let home = tuner_home
        .or_else(|| std::env::var("TUNER_HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let base = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
            PathBuf::from(base).join(".tuner")
        });

    let root = framework_root
        .or_else(|| std::env::var("TUNER_FRAMEWORK_ROOT").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

    let defaults = home_defaults
        .or_else(|| std::env::var("TUNER_HOME_DEFAULTS").ok().map(PathBuf::from))
        .unwrap_or_else(|| root.join("_home_defaults"));

    DuctorPaths::new(home, defaults, root)
}
