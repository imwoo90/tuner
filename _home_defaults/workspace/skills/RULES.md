# Skills Directory

This directory manages profile-isolated custom, auto-consolidated, and bundled skills for Tuner.

## Sync Topology (Profile-Isolated)

```text
<profile>/workspace/skills/<name>         -> <profile>/workspace/.agents/skills/<name> (Active)
<profile>/workspace/skills/archive/<name> -> Unmounted from .agents/skills/ (Dormant / 0-Token)
```

## Sync & Lifecycle Rules

- **Strict Profile Isolation**: Skills are synchronized exclusively into the active profile's `.agents/skills/` directory and are never exported to the host-global `~/.gemini/skills/` directory.
- **On-Demand Context Loading (Progressive Disclosure)**: Antigravity CLI injects only each active skill's YAML frontmatter (`name` and `description`) into `<skills>`. The full `SKILL.md` instructions are read on-demand by the agent only when relevant to the current task.
- **Dormant Archiving (`skills/archive/`)**: Moving an unused or superseded auto-generated skill into `skills/archive/<name>` automatically removes its symlink from `.agents/skills/<name>`, reducing its `<skills>` index footprint to 0 tokens while preserving all files on disk for future revival.
- **Protected Core Skills**: Skills containing a `.git` repository (e.g., `eda-factory`, `cad-factory`, `hw-procurement`), bundled skills (`memory-system`, `skill-creator`), or hand-crafted core skills without `origin: auto-consolidated` must never be auto-archived.
- Sync runs automatically on worker startup and every 30 seconds at runtime.

## Add or Update a Skill

Create a subdirectory containing `SKILL.md` with valid YAML frontmatter (`name` and `description`):

```text
skills/my-skill/SKILL.md
```

When `memory-consolidator` auto-generates a skill, include lifecycle metadata in the frontmatter:
```yaml
---
name: my-skill
description: Clear trigger description for when to use this skill.
origin: auto-consolidated
created_at: YYYY-MM-DD
last_relevant_at: YYYY-MM-DD
---
```

## Bundled Skills

Bundled framework skills (`memory-system`, `skill-creator`) live in `skills/builtin/` and are linked automatically.
To customize one for a specific profile, place a real directory of the same name in `skills/<name>/`.

## Do Not

- Do not edit hidden/internal directories.
- Do not place loose files directly in `skills/` root.
- Do not symlink skills from another profile's workspace.
