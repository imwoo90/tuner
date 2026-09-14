"""Shared helpers for webhook tool scripts."""

from __future__ import annotations

import json
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

if (_PROFILE_DIR / "webhooks.json").exists() or (_PROFILE_DIR / "config").exists():
    TUNER_HOME = _PROFILE_DIR
else:
    TUNER_HOME = Path(os.environ.get("TUNER_HOME", "~/.tuner")).expanduser()

HOOKS_PATH = TUNER_HOME / "webhooks.json"
CONFIG_PATH = TUNER_HOME / "config" / "config.json"
CRON_TASKS_DIR = _WORKSPACE_DIR / "cron_tasks"


def load_hooks_or_default(hooks_path: Path) -> dict[str, Any]:
    """Load webhooks JSON or return an empty payload if missing/corrupt."""
    return load_collection_or_default(hooks_path, "hooks")


def load_hooks_strict(hooks_path: Path) -> dict[str, Any]:
    """Load webhooks JSON and raise on malformed structure."""
    return load_collection_strict(hooks_path, "hooks")


def save_hooks(hooks_path: Path, data: dict[str, Any]) -> None:
    """Persist webhooks JSON with stable formatting."""
    save_collection(hooks_path, data)


def available_hook_ids(hooks: list[dict[str, Any]]) -> list[str]:
    """Return all hook IDs for diagnostics."""
    return available_ids(hooks)


def find_hook(hooks: list[dict[str, Any]], hook_id: str) -> dict[str, Any] | None:
    """Find a hook dict by ID."""
    return find_by_id(hooks, hook_id)


def load_webhook_config() -> dict[str, Any]:
    """Load the webhooks section from config.json."""
    if not CONFIG_PATH.exists():
        return {}
    try:
        data = json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
        return data.get("webhooks", {})
    except (json.JSONDecodeError, TypeError):
        return {}
