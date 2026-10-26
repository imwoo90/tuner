"""Shared helpers for cron tool scripts."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

# Ensure tools directory is in sys.path
_TOOLS_DIR = Path(__file__).resolve().parent.parent
if str(_TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(_TOOLS_DIR))

from _tool_shared import (
    available_ids,
    find_by_id,
    load_collection_or_default,
    load_collection_strict,
    sanitize_name,
    save_collection,
)

# Re-export so existing tool scripts keep working with ``from _shared import sanitize_name``
sanitize_name = sanitize_name

_WORKSPACE_DIR = _TOOLS_DIR.parent
_PROFILE_DIR = _WORKSPACE_DIR.parent

if (
    _PROFILE_DIR.parent.name == "profiles"
    or (_PROFILE_DIR / "cron_jobs.json").exists()
    or (_PROFILE_DIR / "config").exists()
):
    TUNER_HOME = _PROFILE_DIR
else:
    TUNER_HOME = Path(os.environ.get("TUNER_HOME", "~/.tuner")).expanduser()

CONFIG_PATH = TUNER_HOME / "config" / "config.json"
GLOBAL_CONFIG_PATH = (
    _PROFILE_DIR.parent.parent / "config" / "config.json"
    if _PROFILE_DIR.parent.name == "profiles"
    else CONFIG_PATH
)
JOBS_PATH = TUNER_HOME / "cron_jobs.json"
CRON_TASKS_DIR = _WORKSPACE_DIR / "cron_tasks"


def read_user_timezone() -> str:
    """Read user_timezone from profile or global config.json. Returns empty string if not set."""
    for path in (CONFIG_PATH, GLOBAL_CONFIG_PATH):
        if not path.exists():
            continue
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            tz = str(data.get("user_timezone", "")).strip()
            if tz:
                return tz
        except (json.JSONDecodeError, OSError):
            continue
    return ""


def render_cron_task_agents_md(name: str) -> str:
    """Render fixed AGENTS.md content for a cron task folder."""
    return f"""\
# Your Mission

You are an **automated agent**. You run on a schedule with NO human interaction.
Complete your task autonomously and update memory when done.

## Workflow

1. **Read** `{name}_MEMORY.md` first -- it contains context from previous runs.
2. **Read** the whole `TASK_DESCRIPTION.md`!
3. **Follow the assignment** in `TASK_DESCRIPTION.md`!
4. Perform the task conscientiously!
5. **Update** `{name}_MEMORY.md` with the current date/time and what you did.

## Rules

- Stay focused on this task only. Do not deviate.
- Do not modify unrelated project files outside the scope of `TASK_DESCRIPTION.md`.
- Check whether all scripts are already present in the script folder! (IF they are needed!)
- Use `.venv` for Python dependencies if needed.

## Important

You provide the final answer after the task is completed in a pleasant, concise,
and well-formatted manner.
"""


def load_jobs_or_default(jobs_path: Path) -> dict[str, Any]:
    """Load cron jobs JSON or return an empty payload if missing/corrupt."""
    return load_collection_or_default(jobs_path, "jobs")


def load_jobs_strict(jobs_path: Path) -> dict[str, Any]:
    """Load cron jobs JSON and raise on malformed structure."""
    return load_collection_strict(jobs_path, "jobs")


def find_job_by_id_or_task_folder(jobs: list[dict[str, Any]], job_id: str) -> dict[str, Any] | None:
    """Find a job by exact id, then by task_folder."""
    exact = find_by_id(jobs, job_id)
    if exact:
        return exact
    return next((j for j in jobs if j.get("task_folder") == job_id), None)


def available_job_ids(jobs: list[dict[str, Any]]) -> list[str]:
    """Return all job IDs for diagnostics."""
    return available_ids(jobs)


def safe_task_dir(task_folder: str) -> Path:
    """Resolve and validate a cron task folder path under CRON_TASKS_DIR."""
    folder_path = (CRON_TASKS_DIR / task_folder).resolve()
    if not folder_path.is_relative_to(CRON_TASKS_DIR.resolve()):
        msg = f"Path traversal blocked: {task_folder!r} resolves outside cron_tasks"
        raise ValueError(msg)
    return folder_path


def save_jobs(jobs_path: Path, data: dict[str, Any]) -> None:
    """Persist cron jobs JSON with stable formatting."""
    save_collection(jobs_path, data)
