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
  - `src/workspace`: workspace initialization, rules selection (CLAUDE.md, GEMINI.md, AGENTS.md), and skill sync.
  - `src/background`: background task manager, timeout handler, and CLI runner.
  - `src/security`: path safety checks, allowed roots boundaries, and response/content safety filters.
  - `src/bus`: event routing and adapters.
  - `src/webhook`: webhook receiver and API server (Axum).
  - `src/tasks`: task registry, state tracker, and host execution DAG runner.
  - `src/i18n`: internationalization system with TOML loaders.
  - `src/messenger/matrix`: matrix client and transport layer.

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | Phase 1 (M1-M8) | Initial core migration (CLI, Session, Telegram, Heartbeat, Cron, Cleanup) | none | DONE |
| 2 | Phase 2 (M9-M12) | Specialized features (Workspace rules, Background, Security, Bus) | M1 | DONE |
| 3 | M13 (R1) | Webhook & API Servers | M1, M2 | DONE |
| 4 | M14 (R2) | DAG Tasks & Registry (Tasks Module) | M1, M2 | DONE |
| 5 | M15 (R3) | i18n Localization Module | none | IN_PROGRESS |
| 6 | M16 (R4) | Matrix Messenger & Client | M1, M2 | PLANNED |
| 7 | M17 (R5) | Integration Test Suite Parity | M13-M16 | PLANNED |

## Interface Contracts
- Standardize on `anyhow::Result` across Rust modules.
- Configurations defined in `src/config.rs`.
- `tuner` logical size constraints in `AGENT.md` are compile-checked.
- API and Webhook endpoints must expose tokio-compatible web services under standard ports.
- Host executor maps sandbox execution requests to host execution runner.
- i18n supports English and Korean TOML translation files.
- Matrix client handles connection sync and dispatching using Matrix protocol.

## Code Layout
- Production modules: `src/` (e.g., `src/webhook.rs`, `src/tasks/`, etc.)
- Test modules: `src/<module>_tests.rs` or subfolders (e.g., `src/webhook_tests.rs`).
