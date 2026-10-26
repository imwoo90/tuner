# Cron Tools (Antigravity Only)

Scripts for creating, editing, listing, and removing scheduled jobs.

## MANDATORY: Ask Before Creating Jobs

**When the user requests a new cron job, you MUST ask:**

1. **Which model?**
   - `gemini-3.8-flash` (effort: `high` | `medium` | `low`) - Fast, state-of-the-art agentic model (recommended)
   - `gemini-4-argon` (effort: `high` | `medium` | `low`) - Frontier reasoning model
   - `claude-opus-5-5` - Claude frontier reasoning model
   - `claude-sonnet-5-5` - Claude reasoning model

2. **Should this job respect quiet hours?**
   - Ask: "Should this job skip execution during specific hours (e.g., at night)?"
   - If YES: Ask for start/end hours (e.g., "Don't run between 22:00-08:00")
   - Explain: "Quiet hours prevent jobs from running during specified times"
   - Use `--quiet-start <hour>` and `--quiet-end <hour>` (0-23, supports wrap-around)

**Present these options and wait for the user's choice!**

Do NOT suggest `--cli-parameters` proactively. Only mention it exists if the user asks.

## Mandatory Rules

1. Use these scripts for cron lifecycle actions.
2. Do not manually edit `cron_jobs.json` for normal operations.
3. Do not manually delete `cron_tasks/` folders.
4. Run `cron_list.py` before `cron_remove.py` and use exact job IDs.

## Timezone (Critical)

All job schedules use the timezone defined in the profile or global config: `user_timezone`.
Verify the active time and offset before recommending cron schedules by running:
`python3 tools/cron_tools/cron_time.py`

## Commands

- `python3 tools/cron_tools/cron_add.py --name "task-name" --title "Task Title" --description "Task description..." --schedule "*/5 * * * *" --model "gemini-3.8-flash" --effort "high"`
- `python3 tools/cron_tools/cron_edit.py JOB_ID --schedule "0 9 * * *"`
- `python3 tools/cron_tools/cron_remove.py JOB_ID`
- `python3 tools/cron_tools/cron_list.py`
