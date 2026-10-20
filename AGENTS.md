# Tuner Agent Guide & Standards (AGENTS.md)

This file defines the project-specific coding guidelines, architectural overview, and verification standards for **Tuner**, an AI agent orchestrator and multi-profile Telegram bot daemon.

---

## 1. Architectural Baseline: Agent-Native Core
This project strictly adheres to the compiler-enforced **Agent-Native** architecture.

👉 **Mandatory Baseline Reading**: Consult [AGENT_NATIVE.md](file://AGENT_NATIVE.md) for the authoritative specification on:
- **Core Philosophy**: Living LLM-Wiki (`mod.rs` = `index.md`), compile-checked intra-doc links, and living doctests.
- **Compiler Hard Limits**: Rules 1-4 enforced via `build.rs` and `build_linter.rs` (file headers, logical code limits, inline test limits, function physical limits).
- **Anti-Code-Golfing Principle**: Decompose into cohesive submodules; never compress variable names or eliminate idiomatic whitespace.
- **Safety Lints**: `missing_docs = "deny"`, `clippy::unwrap_used = "deny"`, `clippy::expect_used = "deny"` in `Cargo.toml`.

---

## 2. System Architecture & Module Domains
Tuner is an asynchronous multi-profile daemon built with Rust, Tokio, Teloxide, and Axum. Its root crate index (`src/main.rs`) organizes the codebase into sixteen top-level domains:

1. **`src/cli/` (`src/cli/mod.rs`)**:
   - Agent CLI provider abstraction (`CliProvider`, `CliResponse`, `StreamEvent`) and the Google Antigravity PTY driver (`src/cli/antigravity/mod.rs`: `provider.rs`, `session.rs`, `pty_spawner.rs`, `polling.rs`, `completion.rs`, `log_parser.rs`, `log_helpers.rs`, `events.rs`, `discovery.rs`, `trust.rs`, `error_parser.rs`).
2. **`src/messenger/` (`src/messenger/mod.rs`)**:
   - Messaging frontend adapters anchored by the Telegram bot subsystem (`src/messenger/telegram/mod.rs`), including HTML formatting (`formatting/mod.rs`), streaming progress debouncing (`stream.rs`), idle background turn observation (`async_observer.rs`), interactive `ask_question` menus (`ask_process.rs`, `ask_callbacks.rs`, `ask_helpers.rs`, `multi_select.rs`), web file review viewer (`review.rs`, `review_server.rs`, `review_store.rs`, `review_callback.rs`), and slash commands (`commands.rs`, `commands_model.rs`, `commands_session.rs`, `commands_usage.rs`, `commands_remote.rs`, `commands_remote_hub.rs`, `commands_registry.rs`).
3. **`src/bus/` (`src/bus/mod.rs`)**:
   - Internal message bus (`bus.rs`), typed routing envelopes (`envelope.rs`), domain adapters (`adapters.rs`), per-topic concurrency locks (`lock_pool.rs`), and observer wiring (`observers_wire.rs`).
4. **`src/session/` (`src/session/mod.rs`)**:
   - Persistent chat/topic session lifecycle (`manager.rs`), session state models (`data.rs`), composite keys (`key.rs`), inactivity/daily reset freshness checks (`freshness.rs`), and named session aliases (`named.rs`).
5. **`src/workspace/` (`src/workspace/mod.rs`)**:
   - Profile path resolution (`paths.rs`), rule template selection (`rules.rs`, `rules_check.rs`), whitelist-based rule synchronization (`sync.rs`, `sync_helpers.rs`, `sync_layout.rs`), and skill symlink mounting (`skills.rs`, `skills_helpers.rs`).
6. **`src/webhook/` (`src/webhook/mod.rs`)**:
   - Axum HTTP/WebSocket ingress server (`server.rs`), HMAC/Bearer authentication (`auth.rs`), webhook registry (`manager.rs`, `models.rs`), wake file observer (`observer.rs`), and encrypted remote API endpoints (`src/webhook/api/mod.rs`: `crypto.rs`, `handshake.rs`, `websocket.rs`, `session_loop.rs`, `files.rs`, `server.rs`).
7. **`src/cron/` (`src/cron/mod.rs`) & `src/heartbeat/` (`src/heartbeat/mod.rs`)**:
   - Timezone-aware periodic job scheduling (`cron/manager.rs`, `cron/scheduler.rs`) and quiet-hour-aware liveness heartbeats (`heartbeat/quiet.rs`, `heartbeat/scheduler.rs`).
8. **`src/background/` (`src/background/mod.rs`) & `src/cleanup/` (`src/cleanup/mod.rs`)**:
   - Asynchronous background CLI task execution (`background/models.rs`, `background/observer.rs`, `background/test_utils.rs`) and retention-based storage cleanup (`cleanup/observer.rs`).
9. **`src/config.rs`, `src/security/` (`src/security/mod.rs`), `src/i18n/` (`src/i18n/mod.rs`), `src/supervisor.rs`, `src/setup.rs`, & `src/upgrade.rs`**:
   - JSON configuration (`config.rs`), path/credential sandboxing (`security/paths.rs`, `security/content.rs`), localization (`i18n/loader.rs`, `i18n/check.rs`), process supervision (`supervisor.rs`), initial setup (`setup.rs`), and GitHub release self-upgrade (`upgrade.rs`).

---

## 3. Verification Protocol
Always verify code changes before completing any task:

```bash
# 1. Check compiler linter rules (Agent-Native constraints)
cargo check

# 2. Run unit tests, integration tests, and doctests
cargo test --all-targets

# 3. Verify Clippy lints and zero unwrap/expect in production code
cargo clippy --all-targets -- -D warnings

# 4. Verify doc link integrity and eliminate broken references
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

---

## 4. AI Agent Navigation Guide
1. **Entry Points**: Start with `AGENTS.md` (this file), `AGENT_NATIVE.md`, and `src/main.rs` to grasp the project architecture and top-level module catalog.
2. **Module Indexing (`mod.rs` = `index.md`)**: Treat `src/main.rs` and every `mod.rs` as a directory's Living LLM-Wiki index. Read its `//!` header (`## Overview` and `## Submodules`)—which explicitly specifies whether each child is a leaf file (`foo.rs`) or directory index (`foo/mod.rs`)—before opening child files.
3. **Graph Traversal via Rustdoc Links**: Follow compile-checked intra-doc links (`[Type]`) and `#tag` search tags to traverse dependencies deterministically without brute-force filesystem searches.
4. **Pre-commit Verification**: Never consider a task complete without passing all 4 steps in the Verification Protocol.
