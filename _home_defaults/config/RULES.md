# Config Directory

Runtime config lives here: `config.json`.
Edit only when the user asks for behavior changes.

## Safe Edit Workflow

1. Change only requested keys.
2. Preserve unrelated values and structure.
3. Never expose secrets (`telegram_token`) in chat output.
4. Keep valid JSON.
5. Trigger a restart via `/restart` or `touch ~/.tuner/restart-requested` when changing core runtime settings (`telegram_token`, `heartbeat`, `cleanup`).

## Important Key Groups

### Model and Provider

- `provider`: `antigravity`
- `model`: default model id
    - `gemini-4-hydrogen` (effort: `high` | `xhigh` | `medium` | `low`) - Fast, state-of-the-art agentic model (recommended default)
    - `gemini-pro-orchestrator` (effortless) - Hybrid Carbon planner + Hydrogen High executor for complex reasoning & orchestration
    - `gemini-pro-orchestrator-argon` (effortless) - Hybrid Argon planner + Hydrogen High executor
    - `gemini-4-carbon` / `gemini-4-argon` (effort: `high` | `medium` | `low`) - Direct frontier reasoning models
    - `claude-opus-5-5` / `claude-sonnet-5-5`
- `effort`: reasoning effort level (`low`, `medium`, `high`, `xhigh`) for compatible models
- `permission_mode`: CLI permission behavior (`bypassPermissions` or standard)

### Time and Scheduling

- `user_timezone`: IANA timezone string (for example `Asia/Seoul` or `Europe/Berlin`)
- `daily_reset_hour`: session reset boundary (in `user_timezone`)
- `daily_reset_enabled`: `true`/`false`
- `idle_timeout_minutes`: minutes of inactivity before session reset (default `1440`)
- `max_session_messages`: optional message cap per session before rotation
- `heartbeat.quiet_start`, `heartbeat.quiet_end`: quiet hours (in `user_timezone`)
- `cleanup.check_hour`: daily cleanup hour (0-23)

If `user_timezone` is empty, runtime falls back to UTC.
For user-facing schedules, set `user_timezone` explicitly.

### Cleanup

- `cleanup.enabled`: `true`/`false`
- `cleanup.media_files_days`: retention days for `telegram_files/` (default `30`)
- `cleanup.output_to_user_days`: retention days for `output_to_user/` (default `30`)
- `cleanup.check_hour`: daily cleanup hour (default `3`)

### CLI Parameters

- `cli_parameters.antigravity`: List of extra CLI flags for Antigravity main agent

These parameters are appended to every CLI invocation for the Antigravity provider.

**Example:**
```json
{
  "cli_parameters": {
    "antigravity": ["--sandbox"]
  }
}
```

### Language

- `language`: UI language for bot messages — `en`, `de`, `nl`, `es`, `fr`, `id`, `pt`, `ru`, `ko`

### Heartbeat

- `heartbeat.enabled`: `true`/`false` — master switch
- `heartbeat.interval_minutes`: global tick interval for private chats (default `30`)
- `heartbeat.quiet_start`, `heartbeat.quiet_end`: quiet hours (0-23, in `user_timezone`)
- `heartbeat.ack_token`: token the agent replies with when nothing to report (default `HEARTBEAT_OK`)

### Access Control & Profiles

- `allowed_user_ids`: list of authorized Telegram user IDs
- `allowed_group_ids`: list of authorized Telegram group IDs
- `telegram_token`: Telegram Bot API token
- `profiles`: list of isolated profile descriptors (`[{"name": "default"}]`)
