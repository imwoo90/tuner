//! # Workspace Paths Resolution
//!
//! SINGLE SOURCE OF TRUTH for all workspace and configuration paths in wooductor.

use std::path::PathBuf;

/// Immutable paths resolved for the workspace layout.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DuctorPaths {
    pub ductor_home: PathBuf,
    pub home_defaults: PathBuf,
    pub framework_root: PathBuf,
}

impl DuctorPaths {
    /// Create a new DuctorPaths instance.
    pub fn new(ductor_home: PathBuf, home_defaults: PathBuf, framework_root: PathBuf) -> Self {
        Self {
            ductor_home,
            home_defaults,
            framework_root,
        }
    }

    /// User agent workspace directory: `~/.ductor/workspace`
    pub fn workspace(&self) -> PathBuf {
        self.ductor_home.join("workspace")
    }

    /// Configuration directory: `~/.ductor/config`
    pub fn config_dir(&self) -> PathBuf {
        self.ductor_home.join("config")
    }

    /// Configuration path: `~/.ductor/config/config.json`
    pub fn config_path(&self) -> PathBuf {
        self.config_dir().join("config.json")
    }

    /// Sessions path: `~/.ductor/sessions.json`
    pub fn sessions_path(&self) -> PathBuf {
        self.ductor_home.join("sessions.json")
    }

    /// Cron jobs path: `~/.ductor/cron_jobs.json`
    pub fn cron_jobs_path(&self) -> PathBuf {
        self.ductor_home.join("cron_jobs.json")
    }

    /// Webhooks path: `~/.ductor/webhooks.json`
    pub fn webhooks_path(&self) -> PathBuf {
        self.ductor_home.join("webhooks.json")
    }

    /// Logs directory: `~/.ductor/logs`
    pub fn logs_dir(&self) -> PathBuf {
        self.ductor_home.join("logs")
    }

    /// Cron tasks directory: `~/.ductor/workspace/cron_tasks`
    pub fn cron_tasks_dir(&self) -> PathBuf {
        self.workspace().join("cron_tasks")
    }

    /// Tools directory: `~/.ductor/workspace/tools`
    pub fn tools_dir(&self) -> PathBuf {
        self.workspace().join("tools")
    }

    /// Output to user directory: `~/.ductor/workspace/output_to_user`
    pub fn output_to_user_dir(&self) -> PathBuf {
        self.workspace().join("output_to_user")
    }

    /// Telegram files directory: `~/.ductor/workspace/telegram_files`
    pub fn telegram_files_dir(&self) -> PathBuf {
        self.workspace().join("telegram_files")
    }

    /// Matrix files directory: `~/.ductor/workspace/matrix_files`
    pub fn matrix_files_dir(&self) -> PathBuf {
        self.workspace().join("matrix_files")
    }

    /// API files directory: `~/.ductor/workspace/api_files`
    pub fn api_files_dir(&self) -> PathBuf {
        self.workspace().join("api_files")
    }

    /// Memory system directory: `~/.ductor/workspace/memory_system`
    pub fn memory_system_dir(&self) -> PathBuf {
        self.workspace().join("memory_system")
    }

    /// User skills directory: `~/.ductor/workspace/skills`
    pub fn skills_dir(&self) -> PathBuf {
        self.workspace().join("skills")
    }

    /// Package-bundled skills directory: `{home_defaults}/workspace/skills`
    pub fn bundled_skills_dir(&self) -> PathBuf {
        self.home_defaults.join("workspace").join("skills")
    }

    /// Tasks directory: `~/.ductor/workspace/tasks`
    pub fn tasks_dir(&self) -> PathBuf {
        self.workspace().join("tasks")
    }

    /// Tasks registry path: `~/.ductor/tasks.json`
    pub fn tasks_registry_path(&self) -> PathBuf {
        self.ductor_home.join("tasks.json")
    }

    /// Chat activity path: `~/.ductor/chat_activity.json`
    pub fn chat_activity_path(&self) -> PathBuf {
        self.ductor_home.join("chat_activity.json")
    }

    /// Named sessions path: `~/.ductor/named_sessions.json`
    pub fn named_sessions_path(&self) -> PathBuf {
        self.ductor_home.join("named_sessions.json")
    }

    /// Startup state path: `~/.ductor/startup_state.json`
    pub fn startup_state_path(&self) -> PathBuf {
        self.ductor_home.join("startup_state.json")
    }

    /// Inflight turns path: `~/.ductor/inflight_turns.json`
    pub fn inflight_turns_path(&self) -> PathBuf {
        self.ductor_home.join("inflight_turns.json")
    }

    /// User-managed environment file: `~/.ductor/.env`
    pub fn env_file(&self) -> PathBuf {
        self.ductor_home.join(".env")
    }

    /// Main memory path: `~/.ductor/workspace/memory_system/MAINMEMORY.md`
    pub fn mainmemory_path(&self) -> PathBuf {
        self.memory_system_dir().join("MAINMEMORY.md")
    }

    /// Join notification path: `~/.ductor/workspace/JOIN_NOTIFICATION.md`
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
    ductor_home: Option<PathBuf>,
    framework_root: Option<PathBuf>,
    home_defaults: Option<PathBuf>,
) -> DuctorPaths {
    let home = ductor_home
        .or_else(|| std::env::var("DUCTOR_HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let base = std::env::var("HOME").unwrap_or_else(|_| "/home/wimvm".to_string());
            PathBuf::from(base).join(".ductor")
        });

    let root = framework_root
        .or_else(|| std::env::var("DUCTOR_FRAMEWORK_ROOT").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));

    let defaults = home_defaults
        .or_else(|| std::env::var("DUCTOR_HOME_DEFAULTS").ok().map(PathBuf::from))
        .unwrap_or_else(|| root.join("_home_defaults"));

    DuctorPaths::new(home, defaults, root)
}
