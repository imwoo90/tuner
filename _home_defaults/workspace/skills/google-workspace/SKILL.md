---
name: google-workspace
description: Google AI Studio (GEMINI_API_KEY) and Google Workspace (Gmail, Calendar, Drive, Docs, Sheets, Tasks, Contacts, YouTube) integration via `scripts/google_cli.py` and Tuner Vault browser authentication. Activate when calling Gemini API models or managing Gmail, Calendar, Drive, Sheets, Docs, Tasks, Contacts, or YouTube.
---

# Google AI & Workspace Integration (`google-workspace`)

This bundled skill provides deterministic CLI access to:
1. **Google AI Studio (`GEMINI_API_KEY`)** — works immediately after 1-click setup in the Tuner Web Dashboard (`Engines` -> `Google AI Studio`).
2. **Google Workspace REST APIs** (`Gmail`, `Calendar`, `Drive`, `Sheets`, `Docs`, `Tasks`, `Contacts`, `YouTube`) — configured on demand via OAuth 2.0 Desktop Client JSON in the profile Vault directory.

---

## 1. Credential Paths (Profile-Isolated)

- **Gemini API Key (`$0` Free Tier supported)**:
  - Stored in `$TUNER_HOME/.env` (default `~/.tuner/.env`) as `GEMINI_API_KEY=...` (`chmod 0600`).
  - Users can issue and save a free key in one click via `:8743/dashboard` -> `Engines` -> **Get Free Key (Built-in Browser)**.
- **Google Workspace OAuth 2.0 (Optional, On-Demand)**:
  - OAuth Desktop Client JSON: `<profile_home>/vault/google_oauth_client.json` (`chmod 0600`)
  - OAuth Token JSON (auto-refreshing): `<profile_home>/vault/google_token.json` (`chmod 0600`)
  - If the user asks to connect Gmail/Calendar/Drive and `google_oauth_client.json` is missing:
    1. Guide them (or open the Built-in Browser via `tuner --vault auth google --url https://console.cloud.google.com/apis/credentials`) to create a **Desktop app** OAuth 2.0 Client ID and save the downloaded JSON to `<profile_home>/vault/google_oauth_client.json`.
    2. Run `python3 skills/builtin/google-workspace/scripts/google_cli.py auth url` to generate the consent URL, let the user approve via Built-in Browser or paste the callback URL into `auth exchange "<callback_url>"`.

---

## 2. CLI Usage (`scripts/google_cli.py`)

Resolve the script path from either `skills/google-workspace/scripts/google_cli.py` (if customized) or `skills/builtin/google-workspace/scripts/google_cli.py`:

### Gemini API (`GEMINI_API_KEY`)
```bash
# Check GEMINI_API_KEY & Workspace OAuth readiness
python3 .agents/skills/google-workspace/scripts/google_cli.py auth status

# List available Gemini / Gemma / TTS / Live models
python3 .agents/skills/google-workspace/scripts/google_cli.py gemini models

# Generate text with Gemini API
python3 .agents/skills/google-workspace/scripts/google_cli.py gemini generate "Summarize quantum tunneling in one sentence." --model gemini-2.5-flash
```

### Gmail, Calendar, Drive, Tasks, Sheets, Docs, Contacts & YouTube
```bash
# Gmail
python3 .agents/skills/google-workspace/scripts/google_cli.py gmail list --max 5 --query "is:unread"
python3 .agents/skills/google-workspace/scripts/google_cli.py gmail read <msg_id>
python3 .agents/skills/google-workspace/scripts/google_cli.py gmail draft --to "user@example.com" --subject "Hello" --body "Draft text"
python3 .agents/skills/google-workspace/scripts/google_cli.py gmail send --to "user@example.com" --subject "Hello" --body "Message text"

# Calendar & Tasks
python3 .agents/skills/google-workspace/scripts/google_cli.py calendar list --days 14
python3 .agents/skills/google-workspace/scripts/google_cli.py calendar add --summary "Sync" --start "2026-11-25T10:00:00Z" --end "2026-11-25T10:30:00Z"
python3 .agents/skills/google-workspace/scripts/google_cli.py tasks list
python3 .agents/skills/google-workspace/scripts/google_cli.py tasks add --title "Follow up" --due "2026-11-25"

# Drive, Sheets & Docs
python3 .agents/skills/google-workspace/scripts/google_cli.py drive list --max 10
python3 .agents/skills/google-workspace/scripts/google_cli.py drive search "report"
python3 .agents/skills/google-workspace/scripts/google_cli.py sheets create "Tracker"
python3 .agents/skills/google-workspace/scripts/google_cli.py sheets read <spreadsheet_id> "Sheet1!A1:D10"
python3 .agents/skills/google-workspace/scripts/google_cli.py docs create "Notes" --text "Meeting summary\n"
python3 .agents/skills/google-workspace/scripts/google_cli.py docs read <doc_id>

# Contacts & YouTube
python3 .agents/skills/google-workspace/scripts/google_cli.py contacts search "Alice"
python3 .agents/skills/google-workspace/scripts/google_cli.py youtube my-channel
```

## 3. Safety Rules
- Always ask for explicit user confirmation before sending external emails (`gmail send`) or deleting/modifying shared calendar events or files. Prefer `gmail draft` when unsure.
