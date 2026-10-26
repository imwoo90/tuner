---
name: memory-system
description: Provides deterministic tools and instructions to safely read recent profile conversation logs, consolidate durable facts into MAINMEMORY.md, and extract repeatable workflows into profile-isolated skills without cross-profile contamination.
---

# memory-system Skill

This skill manages the compaction and consolidation of the profile's long-term declarative memory ([MAINMEMORY.md](memory_system/MAINMEMORY.md)) and AI-judged procedural skill lifecycle (`skills/<skill-name>/SKILL.md` & `skills/archive/`) in a structured, profile-isolated, and verified manner.

## Key Capabilities

1. **Profile Isolation (Harnessing)**: `memory_tool.py` guarantees that no files outside the current active profile workspace can be accessed, and skills synced to `<workspace>/.agents/skills/` remain strictly isolated to the current profile.
2. **Deterministic Verification (Linting)**: Ensures `MAINMEMORY.md` preserves required headings (`## About the User`, `## Core System Architecture & Roles`, `## Decisions & Preferences`) and remains under the 120-line limit for token efficiency.
3. **Automated Timestamping**: Automatically updates the last consolidation time in KST.
4. **Three-Way Knowledge Routing & Non-Destructive Skill Decay**:
   - **Declarative Facts (`MAINMEMORY.md`)**: Durable facts about user identity, family, assets, vehicle, and preferences (always-on, <=120 lines).
   - **Operational Rules (`AGENTS.md` / `.agents/rules/constraints.md`)**: Negative/positive behavioral constraints and system constitution (always-on).
   - **Active Procedural Skills (`skills/<skill-name>/SKILL.md` -> `.agents/skills/<skill-name>`)**: Reusable multi-step workflows, domain pipelines, and verified script recipes judged by the AI agent as worth keeping active (loaded on-demand via progressive disclosure).
   - **Dormant Procedural Archive (`skills/archive/<skill-name>/`)**: Superseded or long-unused `origin: auto-consolidated` skills are moved to `skills/archive/` (unlinked from `.agents/skills/` so they consume 0 prompt tokens) rather than deleted, and can be revived whenever relevant again.

## Helper Script Usage

The helper script is located at:
`skills/memory-system/scripts/memory_tool.py`

### 1. Retrieve Recent Logs
To get new messages added to the conversation history since the last consolidation timestamp, run:
```bash
python3 skills/memory-system/scripts/memory_tool.py get-logs --workspace <workspace_root_path>
```
*Note*: This strictly scans `<workspace_root_path>/brain/` for `telegram_history.jsonl` files and outputs logs in chronological order.

To filter logs by a specific Telegram topic (useful for on-demand context recovery across rotated sessions):
```bash
python3 skills/memory-system/scripts/memory_tool.py get-logs --workspace <workspace_root_path> --topic-id <TOPIC_ID>
```

### 2. Save Consolidated Memory
To save the consolidated memory, pipe the proposed markdown text directly to the tool:
```bash
cat new_memory.md | python3 skills/memory-system/scripts/memory_tool.py save-memory --workspace <workspace_root_path>
```
*Note*: The tool will validate the content and fail if:
- The line count exceeds 120 lines.
- Required headings are missing.
- Title `# Main Memory` is missing.

## Memory Consolidator Cron-Job Workflow

When running the `memory-consolidator` cron job (executed by the `gemini-4-argon-high` agent), follow these steps:
1. Locate the current workspace root path.
2. Run `get-logs` to retrieve the recent chat history.
3. If no new messages are found, only run `save-memory` with the existing `MAINMEMORY.md` content to update the consolidation timestamp.
4. **Declarative Fact Extraction (AI Semantic Judgment)**: Analyze the new messages to extract any **durable factual knowledge** and merge it into `MAINMEMORY.md` (strictly under 120 lines; never put behavioral constraints in `MAINMEMORY.md`).
5. **Procedural Auto-Skillification & Lifecycle Management (AI Semantic Judgment)**:
   - **Create / Update / Revive**: Evaluate whether the conversation logs contain a **reusable workflow, recurring task procedure, or hard-won script/pipeline recipe**. Check `skills/archive/` first to revive a dormant skill if applicable, or update an existing skill in `skills/`. New auto-generated skills must include `origin: auto-consolidated`, `created_at: YYYY-MM-DD`, and `last_relevant_at: YYYY-MM-DD` in their YAML frontmatter, and be linked into `.agents/skills/<skill-name>`.
   - **Protect Core Skills**: Never archive or modify the structure of skills containing `.git` (`eda-factory`, `cad-factory`, `hw-procurement`), bundled skills (`memory-system`, `skill-creator`), or hand-crafted core skills without `origin: auto-consolidated` (`docu-pipeline`).
   - **Merge & Dormant Archive (`skills/archive/`)**: Merge fragmented `origin: auto-consolidated` skills of the same domain into one cohesive skill, and move superseded or long-unused (30+ days irrelevant) `origin: auto-consolidated` skills to `skills/archive/<skill-name>` (removing `.agents/skills/<skill-name>`).
6. Run `save-memory` with the updated `MAINMEMORY.md` markdown content.
