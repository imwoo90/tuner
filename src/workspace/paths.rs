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
pub struct TunerPaths {
    /// Tuner home.
    pub tuner_home: PathBuf,
    /// Home defaults.
    pub home_defaults: PathBuf,
    /// Profile.
    pub profile: Option<String>,
}

impl TunerPaths {
    /// Create a new TunerPaths instance.
    pub fn new(tuner_home: PathBuf, home_defaults: PathBuf, profile: Option<String>) -> Self {
        Self {
            tuner_home,
            home_defaults,
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

    /// Cron tasks directory: `~/.tuner/workspace/cron_tasks`
    pub fn cron_tasks_dir(&self) -> PathBuf {
        self.workspace().join("cron_tasks")
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

    /// Config example path: `{home_defaults}/config/config.json` or fallback.
    pub fn config_example_path(&self) -> PathBuf {
        let default_cfg = self.home_defaults.join("config").join("config.json");
        if default_cfg.is_file() {
            default_cfg
        } else {
            self.home_defaults.join("_config_example.json")
        }
    }
}

/// Resolves TunerPaths from explicit values, environment variables, or defaults.
pub fn resolve_paths(
    tuner_home: Option<PathBuf>,
    framework_root: Option<PathBuf>,
    home_defaults: Option<PathBuf>,
    profile: Option<String>,
) -> TunerPaths {
    let home = tuner_home
        .or_else(|| std::env::var("TUNER_HOME").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(base).join(".tuner")
        });

    let explicit_root = framework_root
        .or_else(|| std::env::var("TUNER_FRAMEWORK_ROOT").ok().map(PathBuf::from));

    let defaults = home_defaults
        .or_else(|| std::env::var("TUNER_HOME_DEFAULTS").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            if let Some(root) = explicit_root {
                return root.join("_home_defaults");
            }
            let cwd_defaults = PathBuf::from(".").join("_home_defaults");
            if cwd_defaults.is_dir() {
                return cwd_defaults;
            }
            let bin_defaults = home.join("bin").join("_home_defaults");
            if bin_defaults.is_dir() {
                return bin_defaults;
            }
            if let Some(exe_defaults) = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("_home_defaults")))
                && exe_defaults.is_dir()
            {
                return exe_defaults;
            }
            cwd_defaults
        });
    let defaults = std::fs::canonicalize(&defaults).unwrap_or(defaults);

    TunerPaths::new(home, defaults, profile)
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
