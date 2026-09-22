---
name: memory-system
description: Provides deterministic tools and instructions to safely read recent profile conversation logs and consolidate them into MAINMEMORY.md without cross-profile contamination.
---

# memory-system Skill

This skill is designed to manage the compaction and consolidation of the profile's long-term factual memory ([MAINMEMORY.md](memory_system/MAINMEMORY.md)) in a highly structured, isolated, and verified manner.

## Key Capabilities

1. **Profile Isolation (Harnessing)**: The underlying script `memory_tool.py` guarantees that no files outside the current active profile workspace can be accessed.
2. **Deterministic Verification (Linting)**: Ensures the memory format preserves required headings (`## About the User`, `## Core System Architecture & Roles`, `## Decisions & Preferences`) and remains under the 120-line limit for token efficiency.
3. **Automated Timestamping**: Automatically updates the last consolidation time in KST.
4. **Rules vs Memory Scope Enforcement**: Strictly enforces that behavioral guidelines, negative constraints, and positive operational standards remain in Antigravity Rules Context (`AGENTS.md` / `.agents/rules/constraints.md`), keeping `MAINMEMORY.md` dedicated purely to durable factual knowledge.

## Helper Script Usage

The helper script is located at:
`skills/memory-system/scripts/memory_tool.py`

### 1. Retrieve Recent Logs
To get new messages added to the conversation history since the last consolidation timestamp, run:
```bash
python3 skills/memory-system/scripts/memory_tool.py get-logs --workspace <workspace_root_path>
```
*Note*: This will strictly scan `<workspace_root_path>/brain/` for `telegram_history.jsonl` files and output logs in chronological order.

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

When running the `memory-consolidator` cron job, follow these steps:
1. Locate the current workspace root path.
2. Run `get-logs` to retrieve the recent chat history.
3. If no new messages are found, only run `save-memory` with the existing `MAINMEMORY.md` content to update the consolidation timestamp.
4. If new messages exist, analyze them to extract any **durable factual knowledge** (facts about user identity, family, finance/assets, vehicle setup, education, long-term personal preferences).
   - **CRITICAL**: Do NOT merge operational behavioral guidelines, negative constraints, positive constraints, or coding/tool rules into `MAINMEMORY.md`. Such operational rules belong in Antigravity Rules Context (`AGENTS.md` / `.agents/rules/constraints.md`).
5. Merge the new durable facts into the existing sections of `MAINMEMORY.md`, ensuring old or temporary logs are discarded to keep the file strictly under the 120-line compaction limit.
6. Run `save-memory` with the updated markdown content.
