#!/usr/bin/env python3
import sys
import os
import json
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path

def get_kst_now():
    # UTC to KST (UTC+9)
    return datetime.now(timezone.utc) + timedelta(hours=9)

def parse_kst_timestamp(ts_str):
    # YYYY-MM-DD HH:MM KST
    m = re.match(r"(\d{4})-(\d{2})-(\d{2})\s+(\d{2}):(\d{2})", ts_str.strip())
    if not m:
        return None
    year, month, day, hour, minute = map(int, m.groups())
    # Create datetime in KST (using simple timezone offset of +9)
    kst_tz = timezone(timedelta(hours=9))
    return datetime(year, month, day, hour, minute, tzinfo=kst_tz)

def find_last_consolidated_time(memory_file):
    if not memory_file.exists():
        return None
    content = memory_file.read_text(encoding="utf-8")
    m = re.search(r"마지막 정리 일시:\s*([^\n\r]+)", content)
    if m:
        ts = parse_kst_timestamp(m.group(1))
        if ts:
            return ts
    return None

def cmd_get_logs(workspace_dir, filter_topic_id=None):
    workspace = Path(workspace_dir).resolve()
    memory_file = workspace / ".agents" / "rules" / "mainmemory.md"
    if not memory_file.exists():
        memory_file = workspace / "memory_system" / "MAINMEMORY.md"
    
    cutoff = find_last_consolidated_time(memory_file)
    if not cutoff:
        # Default to 24 hours ago
        cutoff = get_kst_now() - timedelta(days=1)
        print(f"No valid last consolidation timestamp found. Using default cutoff (24h ago): {cutoff.strftime('%Y-%m-%d %H:%M:%S KST')}", file=sys.stderr)
    else:
        print(f"Using cutoff timestamp from mainmemory.md: {cutoff.strftime('%Y-%m-%d %H:%M:%S KST')}", file=sys.stderr)

    brain_dir = workspace / "brain"
    if not brain_dir.is_dir():
        print(f"No brain directory found at {brain_dir}", file=sys.stderr)
        return

    # Strict path verification to prevent directory traversal
    if not brain_dir.resolve().is_relative_to(workspace):
        print("Path safety violation: brain folder resolves outside workspace.", file=sys.stderr)
        sys.exit(1)

    recent_messages = []
    # Search only inside workspace/brain/*/telegram_history.jsonl
    for history_file in brain_dir.glob("*/telegram_history.jsonl"):
        # Double check path containment
        if not history_file.resolve().is_relative_to(brain_dir.resolve()):
            continue

        parent_topic_id = None
        topic_meta_file = history_file.parent / "topic.json"
        if topic_meta_file.exists():
            try:
                meta = json.loads(topic_meta_file.read_text(encoding="utf-8"))
                parent_topic_id = meta.get("topic_id")
            except Exception:
                pass

        if filter_topic_id is not None and parent_topic_id is not None and parent_topic_id != filter_topic_id:
            continue

        try:
            with open(history_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    data = json.loads(line)
                    ts_str = data.get("timestamp")
                    if not ts_str:
                        continue
                    entry_topic_id = data.get("topic_id", parent_topic_id)
                    if filter_topic_id is not None and entry_topic_id != filter_topic_id:
                        continue
                    # Parse ISO format timestamp
                    try:
                        ts = datetime.fromisoformat(ts_str.replace("Z", "+00:00"))
                        if ts >= cutoff:
                            topic_label = f" [Topic: {data.get('topic_name') or entry_topic_id}]" if (data.get('topic_name') or entry_topic_id) else ""
                            recent_messages.append((ts, data.get("sender", "unknown") + topic_label, data.get("text", "")))
                    except Exception:
                        pass
        except Exception as e:
            print(f"Warning: Failed to read {history_file}: {e}", file=sys.stderr)

    # Sort messages chronologically
    recent_messages.sort(key=lambda x: x[0])

    topic_suffix = f" (Filtered by Topic ID: {filter_topic_id})" if filter_topic_id is not None else ""
    print(f"--- LOGS SINCE {cutoff.strftime('%Y-%m-%d %H:%M:%S KST')}{topic_suffix} ---")
    for ts, sender, text in recent_messages:
        # Convert timestamp to KST for display
        ts_kst = ts.astimezone(timezone(timedelta(hours=9)))
        print(f"[{ts_kst.strftime('%Y-%m-%d %H:%M:%S KST')}] {sender}: {text}")

def cmd_save_memory(workspace_dir, content_input):
    workspace = Path(workspace_dir).resolve()
    rules_memory_file = workspace / ".agents" / "rules" / "mainmemory.md"

    # Enforce strict path safety
    if not rules_memory_file.resolve().parent.is_relative_to(workspace):
        print("Path safety violation: memory file path is outside workspace.", file=sys.stderr)
        sys.exit(1)

    # Strip existing frontmatter if provided in input
    clean_input = content_input.strip()
    if clean_input.startswith("---"):
        parts = clean_input.split("---", 2)
        if len(parts) >= 3:
            clean_input = parts[2].strip()

    # Validation Checks (Linting)
    lines = clean_input.splitlines()
    line_count = len(lines)
    if line_count > 120:
        print(f"Error: mainmemory.md exceeds the 120-line compaction limit (current lines: {line_count}). Please compress it.", file=sys.stderr)
        sys.exit(1)

    # Verify Title
    has_title = any(line.strip().startswith("# Main Memory") for line in lines)
    if not has_title:
        print("Error: Missing required title '# Main Memory'.", file=sys.stderr)
        sys.exit(1)

    content_str = "\n".join(lines)

    # Programmatic Scope Boundary: Prevent operational constraints from leaking into factual memory
    forbidden_rules_patterns = [
        (r"절대\s*금지", "절대 금지 (Negative Constraints)"),
        (r"필수\s*준수", "필수 준수 (Positive Constraints)"),
        (r"negative\s*constraints", "Negative Constraints"),
        (r"positive\s*constraints", "Positive Constraints"),
        (r"비서\s*명칭\s*:", "비서 명칭 (Assistant identity belongs in AGENTS.md rules)"),
        (r"비서\s*명칭은\s*반드시", "비서 명칭 표기 규칙 (Belongs in AGENTS.md rules)"),
        (r"접기\s*서식", "텔레그램 접기 서식 규칙 (Belongs in AGENTS.md rules)"),
    ]
    for pattern, label in forbidden_rules_patterns:
        if re.search(pattern, content_str, re.IGNORECASE):
            print(f"Error: Behavioral rules or constraints ('{label}') must not be stored in mainmemory.md. Keep operational constraints in Antigravity rules context (AGENTS.md / .agents/rules/constraints.md).", file=sys.stderr)
            sys.exit(1)

    # Verify Required Headings (robust to '&' vs 'and', and profile section variations)
    if not re.search(r"^##\s+About\s+the\s+User", content_str, re.MULTILINE | re.IGNORECASE):
        print("Error: Missing required section heading '## About the User'.", file=sys.stderr)
        sys.exit(1)

    if not re.search(r"^##\s+Decisions\s+(?:&|and)\s+Preferences", content_str, re.MULTILINE | re.IGNORECASE):
        print("Error: Missing required section heading '## Decisions & Preferences' (or '## Decisions and Preferences').", file=sys.stderr)
        sys.exit(1)

    has_arch = bool(re.search(r"^##\s+Core\s+System\s+Architecture\s+(?:&|and)\s+Roles", content_str, re.MULTILINE | re.IGNORECASE))
    has_facts = bool(re.search(r"^##\s+Learned\s+Facts", content_str, re.MULTILINE | re.IGNORECASE))
    if not (has_arch or has_facts):
        print("Error: Missing required intermediate section heading ('## Core System Architecture & Roles' or '## Learned Facts').", file=sys.stderr)
        sys.exit(1)

    # Clean existing timestamps if any at the bottom to avoid duplicates
    cleaned_content = re.sub(r"\n*마지막 정리 일시:[^\n\r]*", "", content_str).strip()

    # Append new timestamp
    now_kst = get_kst_now()
    timestamp_str = f"마지막 정리 일시: {now_kst.strftime('%Y-%m-%d %H:%M')} KST"
    final_content = f"{cleaned_content}\n\n{timestamp_str}\n"

    # Save to the single source of truth: .agents/rules/mainmemory.md
    rules_memory_file.parent.mkdir(parents=True, exist_ok=True)
    rules_header = (
        "---\n"
        "trigger: always_on\n"
        "description: \"Core factual memory about user, family, assets, vehicle, and preferences\"\n"
        "---\n"
    )
    rules_memory_file.write_text(f"{rules_header}{final_content}", encoding="utf-8")

    # Maintain backward compatibility: ensure memory_system/MAINMEMORY.md is a relative symlink
    legacy_dir = workspace / "memory_system"
    legacy_dir.mkdir(parents=True, exist_ok=True)
    legacy_symlink = legacy_dir / "MAINMEMORY.md"
    target_rel = Path("../.agents/rules/mainmemory.md")
    try:
        if legacy_symlink.is_symlink():
            if os.readlink(legacy_symlink) != str(target_rel):
                legacy_symlink.unlink()
                legacy_symlink.symlink_to(target_rel)
        elif legacy_symlink.exists():
            legacy_symlink.unlink()
            legacy_symlink.symlink_to(target_rel)
        else:
            legacy_symlink.symlink_to(target_rel)
    except Exception as e:
        print(f"Warning: Failed to update legacy symlink: {e}", file=sys.stderr)

    print(f"Successfully updated .agents/rules/mainmemory.md (symlink maintained at memory_system/MAINMEMORY.md). (Lines: {len(final_content.splitlines())})")

def main():
    if len(sys.argv) < 3:
        print("Usage: python3 memory_tool.py <command> --workspace <path> [options]", file=sys.stderr)
        sys.exit(1)

    cmd = sys.argv[1]
    
    workspace_dir = None
    for i in range(2, len(sys.argv)):
        if sys.argv[i] == "--workspace" and i + 1 < len(sys.argv):
            workspace_dir = sys.argv[i+1]
            break

    if not workspace_dir:
        print("Error: Missing --workspace parameter.", file=sys.stderr)
        sys.exit(1)

    topic_id = None
    for i in range(2, len(sys.argv)):
        if sys.argv[i] == "--topic-id" and i + 1 < len(sys.argv):
            try:
                topic_id = int(sys.argv[i+1])
            except ValueError:
                pass
            break

    if cmd == "get-logs":
        cmd_get_logs(workspace_dir, topic_id)
    elif cmd == "save-memory":
        # Read content from stdin to avoid command line limits
        content_input = sys.stdin.read()
        cmd_save_memory(workspace_dir, content_input)
    else:
        print(f"Unknown command: {cmd}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
