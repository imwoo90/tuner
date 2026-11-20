# ⚡ tuner

[![Rust Compile & Test](https://github.com/imwoo90/tuner/actions/workflows/rust.yml/badge.svg)](https://github.com/imwoo90/tuner/actions)
[![Releases](https://img.shields.io/github/v/release/imwoo90/tuner?label=release&color=brightgreen)](https://github.com/imwoo90/tuner/releases)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

**Put your local AI coding CLIs (`Antigravity` · `Claude Code` · `OpenAI Codex`) in your pocket via a Responsive Web / PWA Dashboard & Telegram.**

`tuner` is a single-binary, pure-Rust agent orchestrator that turns your workstation, home server, or Docker container into a persistent 24/7 engineering studio accessible from your phone or desktop. Instead of paying extra per-token API bills on stateless cloud wrappers, `tuner` drives your **existing authenticated CLI subscriptions** directly against your local Git repositories, toolchains, and hardware.

```mermaid
graph LR
    WEB["🖥️ Web UI / PWA Dashboard"] <-->|"Real-Time WS · Built-in Browser"| T["⚡ tuner (Pure Rust Daemon)"]
    TG["📱 Telegram (Mirrored Topics)"] <-->|"Live CoT · Inline Buttons · Web Viewer"| T
    T <-->|"Virtual PTY + JSONL"| AGY["🚀 Google Antigravity (agy)"]
    T <-->|"Warm stream-json"| CL["🧠 Claude Code (claude)"]
    T <-->|"Warm JSON-RPC 2.0"| CX["⚙️ OpenAI Codex (codex)"]
```

---

## ✨ Why `tuner`?

- 🖥️ **Responsive Web UI / PWA Dashboard & Zero-State Onboarding (`/dashboard`)**
  Launch a full desktop & mobile PWA workspace on `:8743/dashboard` (with or without Telegram!). On a fresh Docker container, `tuner` automatically prints a First-Run Magic Link, installs `agy` / `claude` / `codex` in one click, completes OAuth sign-in inside an embedded **Built-in Browser** (auto-harvesting callback codes via CDP), syncs Telegram topics bidirectionally, and lets you connect a Telegram bot live with zero restart.
- 🔄 **3 Engines, 1 Workspace (`/provider`)**
  Switch seamlessly between **Google Antigravity (`agy`)**, **Claude Code (`claude`)**, and **OpenAI Codex (`codex`)** per Telegram forum topic—or prefix a single message with `@opus`, `@sonnet`, `@astra`, `@sol`, or `@argon`. `tuner` automatically bridges your `AGENTS.md`, rules, and skills across all three CLIs.
- ⚡ **`0ms` Warm-Process Pool & Soft `/stop`**
  Every active topic keeps its own warm CLI process in memory (`0ms` cold-start from turn 2 onward, auto-reclaimed after 24h idle). Hitting `/stop` sends a soft turn interrupt (`ESC` / `control_request: interrupt` / `turn/interrupt`) without killing your warm session.
- 💭 **Real-Time Expandable Reasoning & Interactive Buttons**
  Watch `💭 Thinking Process` and `🛠️ Tool Calls` stream live inside collapsible Telegram `<blockquote expandable>` blocks. When an agent asks a multiple-choice question (`ask_question` / `AskUserQuestion` / `request_user_input`), answer it directly by tapping Telegram inline buttons.
- 🔍 **Mobile-Ready Web File & 3D Viewer (`/review`)**
  Whenever an agent references or generates files (`file://`), `tuner` attaches a one-tap **`🔍 Open File Viewer`** button backed by an ephemeral Cloudflare Quick Tunnel—supporting code diffs, Markdown, PDFs, image galleries, ZIP archives, and interactive **3D CAD (`.step` / `.stl` / `.glb`)** previews.
- 🔐 **Zero-Knowledge Credential Vault & HITL Browser Auth (`/vault`)**
  Store site logins per profile in an encrypted vault (`XSalsa20-Poly1305`). Agents run authenticated tools via `tuner --vault exec <site> -- <cmd>` (auto-redacting `***REDACTED***` from stdout/stderr), or hand off CAPTCHA/2FA/OAuth gates to your phone via `tuner --vault auth <site>`—streaming a temporary one-time remote browser with 1-tap server-side Vault autofill and reactive agent wakeup on save.
- ⏰ **Autonomous Cron & Background Task Wakeup (`/cron`)**
  Schedule recurring agent workflows or let long-running builds/subagents finish asynchronously and wake the session automatically when done.

---

## 🚀 Quick Start (60 Seconds)

### Option A — 1-Command Official Docker (`linux/amd64` & `linux/arm64`)
Run on any x86_64 server, Apple Silicon Mac, or ARM64 mini-PC with zero host dependencies:
```bash
docker run -d --name tuner --restart unless-stopped \
  -p 8743:8743 \
  -v ./tuner-data:/home/tuner/.tuner \
  -v ./gemini-data:/home/tuner/.gemini \
  ghcr.io/imwoo90/tuner:latest

# Open the First-Run Setup Magic Link printed in the container logs:
docker logs tuner | grep "/dashboard/magic/"
```
Inside `:8743/dashboard`, install `agy` / `claude` / `codex` in one click, sign in via the **Built-in Browser**, grab a free `GEMINI_API_KEY`, and optionally hot-connect a Telegram Bot Token.

### Option B — Native Linux Host (`systemd`)
1. Authenticate at least one supported Agent CLI (or install later via `:8743/dashboard`):
   - **Google Antigravity**: `agy` (verified with `v1.5.12`)
   - **Anthropic Claude Code**: `claude` (verified with `v2.1.197`)
   - **OpenAI Codex**: `codex` (verified with `v0.167.0`)
2. Install `tuner` (`x86_64` & `arm64`):
```bash
curl -fsSL https://raw.githubusercontent.com/imwoo90/tuner/main/install.sh | bash
```

### 3. Connect Telegram & Launch Service
1. Message [@BotFather](https://t.me/BotFather) on Telegram (`/newbot`) to get your **Bot Token**.
2. Message [@userinfobot](https://t.me/userinfobot) to get your numeric **Telegram User ID**.
3. Run the interactive setup wizard (configures your profile and registers the `systemd` daemon):
```bash
~/.tuner/bin/tuner --setup
```

> 💡 **Using Forum Topics (Recommended)**: Create a Telegram Group, enable **Topics**, invite your bot, and promote it to **Administrator** (`Manage Topics` + message reading). Each forum topic becomes an independent, persistent agent session!

---

## 🎮 Telegram Command Cheat Sheet

### Core Session & Engine Controls
| Command | What it does |
|---|---|
| `/provider` | Switch the current topic's CLI engine (`antigravity` · `claude` · `codex`) and update the `/` menu. |
| `/model` | Live-query the active CLI and switch models via inline buttons (or type `@opus`, `@sonnet`, `@astra`, `@argon`). |
| `/effort` | Adjust reasoning effort (`minimal` · `low` · `medium` · `high` · `xhigh` · `max`). |
| `/stop` | Soft-interrupt the current turn while keeping the warm CLI process alive. |
| `/new` | Archive a handover summary of the current conversation and start a fresh session. |
| `/compact` | Compact conversation history to free up the context window. |
| `/status` · `/usage` | Inspect active provider/model/PID state, pending attachments, and remaining model quotas. |

### Workspace, Security & Remote Tools
| Command | What it does |
|---|---|
| `/vault` | Open the encrypted per-profile Credential Vault Web UI via an ephemeral Quick Tunnel. |
| `/dashboard` | Issue a single-use Magic Link and open the responsive Web UI / PWA Dashboard (`:8743/dashboard`, also available without Telegram via `tuner --dashboard [--set-pin <pin>]`). |
| `/remote` | Manage remote development access (`Antigravity` Web IDE tunnel, `Tailscale`, `SSH`). |
| `/cron` | Inspect, toggle, or immediately trigger scheduled background tasks (`⚡ Run Now`). |
| `/memory` · `/lang` | View durable workspace memory (`MAINMEMORY.md`) or switch session language. |
| `/upgrade` · `/restart` | Self-upgrade from [GitHub Releases](https://github.com/imwoo90/tuner/releases) or cleanly restart the daemon. |

### Native CLI Workflow Commands (Auto-switched per `/provider`)
| Active Provider | Available Workflow Slash Commands |
|---|---|
| **Antigravity (`agy`)** | `/plan` · `/grill_me` · `/goal` · `/teamwork` · `/boost` · `/learn` |
| **Claude Code (`claude`)** | `/review` · `/security_review` · `/verify` · `/debug` · `/simplify` · `/deep_research` · `/code_review` · `/goal` · `/init` |
| **OpenAI Codex (`codex`)** | `/review` · `/init` |

---

## 🏛️ Architecture & Documentation

- 📖 **[ARCHITECTURE.md](ARCHITECTURE.md)** — Deep-dive specification covering the 2-Tier Master-Worker supervision tree, Virtual PTY + `transcript_full.jsonl` invariant polling, `ClaudeSessionPool` (`stream-json`), `CodexSessionPool` (`app-server --stdio` JSON-RPC 2.0), deadlock-free `LockPool`, and `XSalsa20-Poly1305` Shadow Plane security.
- 📦 **[GitHub Releases](https://github.com/imwoo90/tuner/releases)** — Full version history, CLI compatibility notes, and prebuilt Linux binaries.

<details>
<summary><b>🛠️ Local Development & Testing (Click to expand)</b></summary>

```bash
# Run compiler architecture linter, unit/integration tests, Clippy, and Rustdoc checks
cargo check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

# Deploy local changes to the isolated Docker sandbox (tuner-sandbox)
./scripts/dev_deploy.sh --restart
```
</details>

---

## 📄 License
Licensed under the [Apache License 2.0](LICENSE).
