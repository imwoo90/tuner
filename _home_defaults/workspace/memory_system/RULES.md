# Memory System

`MAINMEMORY.md` is long-term factual memory across sessions.

## Silence Is Mandatory

Never tell the user you are reading or writing memory.
Memory operations are invisible.

## Read First

At the start of new sessions (especially personal or ongoing work), read `MAINMEMORY.md` for factual user context.

## Scope Separation: Memory vs. Rules Context

- **Write to Rules (`AGENTS.md` / `.agents/rules/constraints.md`)**:
  - Behavioral directives, negative prohibitions ("절대 금지"), positive operational standards ("필수 준수"), tool/workflow rules, and coding/deployment policies.
  - Rules enjoy Antigravity CLI's dedicated 20,000-token rules budget (`defaultRulesBudget`), are injected on every turn, and are 100% immune to conversation compaction. Never place behavioral rules in `MAINMEMORY.md`.
- **Write to Memory (`MAINMEMORY.md`)**:
  - Durable personal facts about the user, family, assets, accounts, real-world context, and personal preferences (e.g., editor choices, philosophical interests).
  - Strictly capped at 120 lines by `memory_tool.py`.

## When to Write to MAINMEMORY.md

- Durable personal facts or personal preferences
- Family, asset, or vehicle updates
- User explicitly asks to remember a personal fact
- Long-term real-world context

## When Not to Write to MAINMEMORY.md

- Operational rules or behavioral constraints (put in `.agents/rules/constraints.md` instead)
- One-off throwaway requests
- Temporary debugging noise
- Facts already recorded

## Format Rules

- Keep entries short and actionable.
- Use consistent Markdown sections (`## About the User`, `## Core System Architecture & Roles`, `## Decisions & Preferences`).
- Never exceed the 120-line compaction limit.
- Merge duplicates and remove stale facts.

## Shared Knowledge (SHAREDMEMORY.md)

When you learn something relevant to ALL agents (server facts, infrastructure changes, shared conventions), update shared knowledge instead of only your own MAINMEMORY.md:

```bash
python3 tools/agent_tools/edit_shared_knowledge.py --append "New shared fact"
```

The Supervisor automatically syncs SHAREDMEMORY.md into every agent's MAINMEMORY.md.
Agent-specific knowledge (project details, personal context) stays in your own memory.

## Cleanup Rules

- If user says data is wrong or should be forgotten, remove/update immediately.
- Do not leave "deleted" markers; keep the file clean.
