# Webhook Tools

Scripts for managing incoming HTTP webhook endpoints.

## MANDATORY: Ask Before Creating cron_task Webhooks

**When creating a webhook in `cron_task` mode, you MUST ask:**

1. **Which model?**
   - `gemini-3.8-flash` (effort: `high` | `medium` | `low`) - Fast, state-of-the-art agentic model (recommended)
   - `gemini-4-argon` (effort: `high` | `medium` | `low`) - Frontier reasoning model
   - `claude-opus-5-5` - Claude frontier reasoning model
   - `claude-sonnet-5-5` - Claude reasoning model

2. **Should this webhook respect quiet hours?**
   - Ask: "Should this webhook skip execution during specific hours (e.g., at night)?"
   - If YES: Ask for start/end hours (e.g., "Don't run between 22:00-08:00")
   - Explain: "Quiet hours prevent webhooks from running during specified times"
   - Use `--quiet-start <hour>` and `--quiet-end <hour>` (0-23, supports wrap-around)

**Present these options and wait for the user's choice!**

For `wake` mode webhooks, these parameters are not applicable (uses current main session).

Do NOT suggest `--cli-parameters` proactively. Only mention it exists if the user asks.

## Mandatory Rules

1. Use webhook tool scripts for create/list/edit/remove/test/rotate.
2. Do not manually edit `webhooks.json` for normal operations.
3. Use exact hook IDs from `webhook_list.py` output.
4. Run tools with `python3`.

## Runtime Model (What Happens)

Endpoint pattern:

```text
POST /hooks/<hook-id>
```

Request validation order:
1. Check sliding-window rate limits.
2. Validate `Authorization: Bearer <token>` or HMAC signature header.
3. Dispatch asynchronously and return `202 Accepted` (skipping execution if `cron_task` is in quiet hours).

## Commands

- `python3 tools/webhook_tools/webhook_add.py --name "alert-handler" --title "Alert Handler" --description "Task description..." --mode cron_task --task-folder "alert-handler" --prompt-template "{{message}}" --model "gemini-3.8-flash" --effort "high"`
- `python3 tools/webhook_tools/webhook_list.py`
- `python3 tools/webhook_tools/webhook_edit.py "alert-handler" --disable`
- `python3 tools/webhook_tools/webhook_remove.py "alert-handler"`
- `python3 tools/webhook_tools/webhook_rotate_token.py "alert-handler"`
- `python3 tools/webhook_tools/webhook_test.py "alert-handler" --payload '{"message":"hello"}'`
