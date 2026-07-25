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

def cmd_get_logs(workspace_dir):
    workspace = Path(workspace_dir).resolve()
    memory_file = workspace / "memory_system" / "MAINMEMORY.md"
    
    cutoff = find_last_consolidated_time(memory_file)
    if not cutoff:
        # Default to 24 hours ago
        cutoff = get_kst_now() - timedelta(days=1)
        print(f"No valid last consolidation timestamp found. Using default cutoff (24h ago): {cutoff.strftime('%Y-%m-%d %H:%M:%S KST')}", file=sys.stderr)
    else:
        print(f"Using cutoff timestamp from MAINMEMORY.md: {cutoff.strftime('%Y-%m-%d %H:%M:%S KST')}", file=sys.stderr)

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
        try:
            with open(history_file, "r", encoding="utf-8") as f:
                for line in f:
                    if not line.strip():
                        continue
                    data = json.loads(line)
                    ts_str = data.get("timestamp")
                    if not ts_str:
                        continue
                    # Parse ISO format timestamp
                    try:
                        ts = datetime.fromisoformat(ts_str.replace("Z", "+00:00"))
                        if ts >= cutoff:
                            recent_messages.append((ts, data.get("sender", "unknown"), data.get("text", "")))
                    except Exception:
                        pass
        except Exception as e:
            print(f"Warning: Failed to read {history_file}: {e}", file=sys.stderr)

    # Sort messages chronologically
    recent_messages.sort(key=lambda x: x[0])

    print(f"--- LOGS SINCE {cutoff.strftime('%Y-%m-%d %H:%M:%S KST')} ---")
    for ts, sender, text in recent_messages:
        # Convert timestamp to KST for display
        ts_kst = ts.astimezone(timezone(timedelta(hours=9)))
        print(f"[{ts_kst.strftime('%Y-%m-%d %H:%M:%S KST')}] {sender}: {text}")

def cmd_save_memory(workspace_dir, content_input):
    workspace = Path(workspace_dir).resolve()
    memory_file = workspace / "memory_system" / "MAINMEMORY.md"

    # Enforce strict path safety
    if not memory_file.resolve().parent.is_relative_to(workspace):
        print("Path safety violation: memory file path is outside workspace.", file=sys.stderr)
        sys.exit(1)

    # Validation Checks (Linting)
    lines = content_input.splitlines()
    line_count = len(lines)
    if line_count > 120:
        print(f"Error: MAINMEMORY.md exceeds the 120-line compaction limit (current lines: {line_count}). Please compress it.", file=sys.stderr)
        sys.exit(1)

    # Verify Title
    has_title = any(line.strip().startswith("# Main Memory") for line in lines)
    if not has_title:
        print("Error: Missing required title '# Main Memory'.", file=sys.stderr)
        sys.exit(1)

    # Verify Headings
    required_headings = [
        "## About the User",
        "## Core System Architecture & Roles",
        "## Decisions & Preferences"
    ]
    
    content_str = "\n".join(lines)
    for h in required_headings:
        if h not in content_str:
            print(f"Error: Missing required section heading '{h}'.", file=sys.stderr)
            sys.exit(1)

    # Clean existing timestamps if any at the bottom to avoid duplicates
    cleaned_content = re.sub(r"\n*마지막 정리 일시:[^\n\r]*", "", content_str).strip()

    # Append new timestamp
    now_kst = get_kst_now()
    timestamp_str = f"마지막 정리 일시: {now_kst.strftime('%Y-%m-%d %H:%M')} KST"
    final_content = f"{cleaned_content}\n\n{timestamp_str}\n"

    # Save file
    memory_file.parent.mkdir(parents=True, exist_ok=True)
    memory_file.write_text(final_content, encoding="utf-8")
    print(f"Successfully updated MAINMEMORY.md. (Lines: {len(final_content.splitlines())})")

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

    if cmd == "get-logs":
        cmd_get_logs(workspace_dir)
    elif cmd == "save-memory":
        # Read content from stdin to avoid command line limits
        content_input = sys.stdin.read()
        cmd_save_memory(workspace_dir, content_input)
    else:
        print(f"Unknown command: {cmd}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
