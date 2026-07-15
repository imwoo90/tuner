# Tools Directory

This is the navigation index for workspace tools.

## Global Rules

- Prefer these tool scripts over manual JSON/file surgery.
- Run with `python3`.
- Normal successful runs are JSON-oriented; tutorial/help output may be plain text.
- Open the matching subfolder `RULES.md` before non-trivial changes.

## Routing

- recurring tasks / schedules -> `cron_tools/RULES.md`
- incoming HTTP triggers -> `webhook_tools/RULES.md`
- file/media processing -> `media_tools/RULES.md`
- custom user scripts -> `user_tools/RULES.md`

## External API Secrets

External API keys are loaded from `~/.tuner/.env` and injected into all CLI subprocesses. Standard dotenv syntax:

```env
PPLX_API_KEY=sk-xxx
DEEPSEEK_API_KEY=sk-yyy
export MY_VAR="quoted value"
```

Existing environment variables are never overridden by `.env` values.

## Bot Restart

To restart the bot (e.g. after config changes or recovery):

```bash
touch ~/.tuner/restart-requested
```

The bot picks up this marker within seconds and restarts cleanly.

## Output and Memory

- Save user deliverables in `../output_to_user/`.
- Update `../memory_system/MAINMEMORY.md` silently for durable user facts/preferences.
