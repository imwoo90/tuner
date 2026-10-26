# Memory Consolidator

## Goal

Consolidate, compress, and update the profile's long-term declarative memory (`MAINMEMORY.md`) and autonomously manage the lifecycle of profile-isolated on-demand skills (`skills/<skill-name>/SKILL.md` and `skills/archive/`) using the `memory-system` skill without cross-profile contamination.

## Assignment

1. **Read the memory-system Skill Instructions**:
   - Refer to the skill documentation in [SKILL.md](skills/builtin/memory-system/SKILL.md) to understand its capabilities and usage.
2. **Collect Recent Conversation Logs**:
   - Retrieve new chat history logs since the last consolidation by running the `get-logs` subcommand:
     ```bash
     python3 skills/builtin/memory-system/scripts/memory_tool.py get-logs --workspace .
     ```
   - Analyze the output. If no new logs are returned, proceed directly to step 6 to update the consolidation timestamp.
3. **Understand Current Memory & Profile Skill Inventory**:
   - Read the current [.agents/rules/mainmemory.md](.agents/rules/mainmemory.md) (or symlink [MAINMEMORY.md](memory_system/MAINMEMORY.md)) file to review existing user details, system architecture, and preferences.
   - Inspect both active skills (`ls skills/`) and dormant skills (`ls skills/archive/ 2>/dev/null || true`) to understand the current procedural skill landscape in this profile.
4. **Phase 1 — Extract and Merge Durable Facts (Declarative Memory)**:
   - Use your semantic judgment to extract any **durable factual knowledge** (long-term facts about the user, family, personal finance/assets, vehicle setup, or personal preferences).
   - **CRITICAL SCOPE BOUNDARY**: Do NOT merge operational behavioral rules, negative constraints ("절대 금지"), positive constraints ("필수 준수"), tool routing rules, or coding/deployment policies into `mainmemory.md`. Such operational directives belong permanently in Antigravity Rules Context (`AGENTS.md` / `.agents/rules/constraints.md`).
   - Merge new durable facts into the appropriate sections of `mainmemory.md`, strictly staying under the 120-line limit.
5. **Phase 2 — AI-Judged Profile-Isolated Auto-Skillification & Lifecycle Management (Procedural Memory)**:
   - **A. Creation, Refinement & Revival**:
     - Use your **semantic and engineering judgment** (not a mechanical frequency counter) to evaluate whether the recent conversations contain **reusable procedural knowledge, recurring workflows, hard-won debugging/automation recipes, or domain-specific pipelines** worth preserving as a profile skill.
     - First check `skills/archive/`: if a relevant dormant skill already exists in `skills/archive/<name>`, revive it (`mv skills/archive/<name> skills/<name>`) and update its `SKILL.md` (`last_relevant_at: YYYY-MM-DD`).
     - Prefer refining/updating an existing active skill in `skills/<name>/SKILL.md` if the workflow naturally extends an existing domain skill.
     - When creating a new auto-consolidated skill, author `skills/<skill-name>/SKILL.md` with YAML frontmatter (`name`, `description`, `origin: auto-consolidated`, `created_at: YYYY-MM-DD`, `last_relevant_at: YYYY-MM-DD`) and link `.agents/skills/<skill-name>` -> `realpath(skills/<skill-name>)`.
   - **B. Consolidation (Merge) & Dormant Decay (`skills/archive/`)**:
     - **Protected Core Skills (NEVER Archive or Delete)**: Never move or archive any skill containing a `.git` directory (e.g., `eda-factory`, `cad-factory`, `hw-procurement`), bundled skills (`memory-system`, `skill-creator`), or hand-crafted core skills that do not have `origin: auto-consolidated` (e.g., `docu-pipeline`).
     - **Merge Fragmented Auto-Skills**: If 2+ `origin: auto-consolidated` skills cover overlapping or shallow parts of the same domain, merge their content into one cohesive domain skill and move the superseded skill directory into `skills/archive/<old-name>` (and `rm -f .agents/skills/<old-name>`).
     - **Dormant Transition for Unused/Obsolete Auto-Skills**: If an `origin: auto-consolidated` skill was tied to a completed short-term event, a deprecated workflow, or has remained unused/irrelevant for a prolonged period (e.g., 30+ days since `last_relevant_at` with no ongoing project relevance), move it to `skills/archive/<skill-name>` and remove `.agents/skills/<skill-name>` instead of deleting it (`rm -rf`), so `<skills>` stays lean while keeping the skill recoverable on disk.
6. **Save and Validate via memory-system**:
   - Write the proposed memory content into a temporary file, then pipe it to the `save-memory` subcommand:
     ```bash
     cat <proposed_new_memory_file> | python3 skills/builtin/memory-system/scripts/memory_tool.py save-memory --workspace .
     ```
   - **Note**: The tool will validate the presence of required H2 headers, the `# Main Memory` title, enforce the 120-line compaction limit, and automatically append the current KST consolidation timestamp. If it fails, compress and refine the content and try again.

## Output

After successfully running the tools, output a summary of the results:
- If factual memory or profile skills were updated: Provide a brief summary of added/modified facts (with a markdown diff of `MAINMEMORY.md`) and list any newly created, updated, merged, revived, or archived (`skills/archive/`) profile skills along with your reasoning.
- If no updates were made: Output: "No new durable information or reusable skill workflow was found compared to existing memory. Only updated the last consolidation timestamp."
