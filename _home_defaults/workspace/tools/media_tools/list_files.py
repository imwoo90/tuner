#!/usr/bin/env python3
"""List received media files with optional filtering.

Reads the auto-maintained _index.yaml in the media files directory.
Searches both telegram_files/ and matrix_files/.

Usage:
    python tools/media_tools/list_files.py
    python tools/media_tools/list_files.py --type image
    python tools/media_tools/list_files.py --date 2025-01-15
    python tools/media_tools/list_files.py --limit 5
"""
from __future__ import annotations

import argparse
import json
import mimetypes
from datetime import datetime, timezone
from pathlib import Path

_MEDIA_DIRS = ("telegram_files",)
_IGNORED_FILES = frozenset({"RULES.md", "AGENTS.md", "CLAUDE.md", "GEMINI.md"})


def _get_base_dirs() -> list[Path]:
    """Resolve all media file directories relative to workspace/tools/media_tools/."""
    workspace = Path(__file__).resolve().parent.parent.parent
    return [workspace / d for d in _MEDIA_DIRS if (workspace / d).is_dir()]


def _append_file_entry(f: Path, date_str: str, source: str, args: argparse.Namespace, results: list[dict]) -> None:
    if not f.is_file() or f.name.startswith(".") or f.name in _IGNORED_FILES:
        return
    if args.date and date_str != args.date:
        return
    mime, _ = mimetypes.guess_type(f.name)
    mime_str = mime or "application/octet-stream"
    if args.file_type and not mime_str.startswith(args.file_type):
        return
    try:
        size = f.stat().st_size
    except OSError:
        size = 0
    results.append({
        "date": date_str,
        "name": f.name,
        "type": mime_str,
        "size": size,
        "path": str(f),
        "source": source,
    })


def _scan_dir(base_dir: Path, args: argparse.Namespace) -> list[dict]:
    """Scan flat media files and legacy date subdirectories in a media directory."""
    results: list[dict] = []
    for entry in sorted(base_dir.iterdir(), key=lambda p: p.name, reverse=True):
        if entry.name.startswith("."):
            continue
        if entry.is_file():
            try:
                mtime = entry.stat().st_mtime
                date_str = datetime.fromtimestamp(mtime, tz=timezone.utc).strftime("%Y-%m-%d")
            except OSError:
                date_str = "unknown"
            _append_file_entry(entry, date_str, base_dir.name, args, results)
        elif entry.is_dir():
            date_str = entry.name
            for f in sorted(entry.iterdir(), key=lambda p: p.name):
                _append_file_entry(f, date_str, base_dir.name, args, results)
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description="List received media files")
    parser.add_argument("--type", dest="file_type", help="Filter by MIME prefix (image, audio, video, application)")
    parser.add_argument("--date", help="Filter by date (YYYY-MM-DD)")
    parser.add_argument("--limit", type=int, default=20, help="Max results")
    args = parser.parse_args()

    base_dirs = _get_base_dirs()
    if not base_dirs:
        print(json.dumps({"files": [], "total": 0, "note": "No media directories found. No files received yet."}))
        return

    all_results: list[dict] = []
    for base_dir in base_dirs:
        all_results.extend(_scan_dir(base_dir, args))

    # Sort by date descending, limit
    all_results.sort(key=lambda r: r["date"], reverse=True)
    all_results = all_results[: args.limit]

    print(json.dumps({
        "files": all_results,
        "total": len(all_results),
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
