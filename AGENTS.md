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
Tuner is an asynchronous multi-profile daemon built with Rust, Tokio, Teloxide, and Axum. Its root crate index (`src/main.rs`) organizes the codebase into fifteen top-level domains:

1. **`src/cli/` (`src/cli/mod.rs`)**:
   - Agent CLI provider abstraction (`CliProvider`, `CliResponse`, `StreamEvent`) and the Google Antigravity PTY driver (`src/cli/antigravity/mod.rs`: `provider.rs`, `session.rs`, `pty_spawner.rs`, `polling.rs`, `completion.rs`, `log_parser.rs`, `log_helpers.rs`, `events.rs`, `trust.rs`, `error_parser.rs`).
2. **`src/messenger/` (`src/messenger/mod.rs`)**:
   - Messaging frontend adapters anchored by the Telegram bot subsystem (`src/messenger/telegram/mod.rs`), including HTML formatting (`formatting/mod.rs`: `helpers.rs`, `splitting.rs`, `table.rs`), streaming progress debouncing (`stream.rs`), idle background turn observation (`async_observer.rs`), session handover (`handover.rs`, `handover_helpers.rs`), inbound/outbound attachments (`attachments.rs`, `attachments_dl.rs`), interactive `ask_question` menus (`ask_process.rs`, `ask_callbacks.rs`, `ask_helpers.rs`, `multi_select.rs`), responsive web file & 3D review viewer v2 (`review.rs`, `review_server.rs`, `review_store.rs`, `review_callback.rs`, `review_viewer.html`), credential vault Web UI (`vault.rs`, `vault_viewer.html`), and slash commands (`commands.rs`, `commands_model.rs`, `commands_session.rs`, `commands_usage.rs`, `commands_remote.rs`, `commands_remote_hub.rs`, `commands_registry.rs`).
3. **`src/bus/` (`src/bus/mod.rs`)**:
   - Internal message bus (`bus.rs`), typed routing envelopes (`envelope.rs`), domain adapters (`adapters.rs`), per-topic concurrency locks (`lock_pool.rs`), and observer wiring (`observers_wire.rs`).
4. **`src/session/` (`src/session/mod.rs`)**:
   - Persistent chat/topic session lifecycle (`manager.rs`), session state & handover models (`data.rs`), composite keys (`key.rs`), and inactivity/daily reset freshness checks (`session/mod.rs`).
5. **`src/workspace/` (`src/workspace/mod.rs`)**:
   - Profile path resolution (`paths.rs`), rule template selection (`rules.rs`), whitelist-based rule synchronization (`sync.rs`, `sync_helpers.rs`, `sync_layout.rs`), and bidirectional Antigravity skill symlink mounting (`skills.rs`, `skills_helpers.rs`).
6. **`src/webhook/` (`src/webhook/mod.rs`)**:
   - Axum HTTP ingress server (`server.rs`), constant-time HMAC/Bearer authentication (`auth.rs`), webhook registry (`manager.rs`, `models.rs`), and wake file observer (`observer.rs`).
7. **`src/cron/` (`src/cron/mod.rs`) & `src/heartbeat/` (`src/heartbeat/mod.rs`)**:
   - Timezone-aware periodic job scheduling (`cron/manager.rs`, `cron/scheduler.rs`) and quiet-hour-aware liveness heartbeats (`heartbeat/mod.rs`, `heartbeat/scheduler.rs`).
8. **`src/cleanup/` (`src/cleanup/mod.rs`)**:
   - Retention-based temporary file and process cleanup (`cleanup/observer.rs`).
9. **`src/security/` (`src/security/mod.rs`)**:
   - Path/credential sandboxing (`security/mod.rs`, `security/paths.rs`), per-profile `XSalsa20-Poly1305` encrypted credential vault (`security/vault.rs`), and Zero-Knowledge CLI secret injection & redaction (`security/vault_cli.rs`).
10. **`src/config.rs`, `src/i18n/` (`src/i18n/mod.rs`), `src/supervisor.rs`, `src/setup.rs`, & `src/upgrade.rs`**:
    - JSON configuration (`config.rs`), localization (`i18n/mod.rs`, `i18n/loader.rs`), process supervision (`supervisor.rs`), initial setup (`setup.rs`), and GitHub release self-upgrade (`upgrade.rs`).

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
