# Project: wooductor

## Architecture
- `wooductor` is a Rust-based agent supervisor and automation runtime (replaces the Python `ductor_for_agy` service).
- Module boundaries:
  - `src/cli/antigravity`: wraps `agy` CLI, parses events, error handling, model discovery, and trust checks.
  - `src/session`: manages session keys, data persistence, and lifecycle.
  - `src/telegram`: command parser, message replies, markdown/HTML formatting, and scheduling selectors.
  - `src/heartbeat`: periodic status telemetry, logging, and quiet hours.
  - `src/cron`: cron job scheduler and manager.
  - `src/cleanup`: stale temp files, old logs, and process cleanup.

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | cli/antigravity provider | Core provider client, events parsing, and discovery | none | DONE |
| 2 | PTY session lifecycle | Process spawning, pty control, process lifecycle | M1 | DONE |
| 3 | Session state persistence | Named session manager, storage, state files | M2 | DONE |
| 4 | Telegram bot formatting | Text formatters, HTML escaping, formatting tests | none | IN_PROGRESS |
| 5 | Telegram commands & replies | Commands (/new, /reset, etc.), reply helper | M4, M3 | PLANNED |
| 6 | Heartbeat scheduler | Quiet hours check, heartbeat task, observer | none | PLANNED |
| 7 | Cron task manager | Job parser, job executor, state store | none | PLANNED |
| 8 | File cleanup task | Log rotater, tmp file wiper, filesystem observer | none | PLANNED |

## Interface Contracts
- Standardize on `anyhow::Result` across Rust modules.
- Configurations defined in `src/config.rs`.
- `SessionKey` uses transport prefix schema (e.g. `tg:`, `api:`).
- `tuner` logical size constraints in `AGENT.md` are compile-checked.
