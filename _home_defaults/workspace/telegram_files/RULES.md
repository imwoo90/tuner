# Telegram Files

Incoming Telegram media and documents are stored in this directory.

## Structure

```text
telegram_files/
  media_<chat_id>_<message_id>.<ext>
```

## Processing

Use tools from `tools/media_tools/`:

- images: inspect directly
- audio/voice: `transcribe_audio.py --file <path>`
- documents: `read_document.py --file <path>`
- video: `process_video.py --file <path>`
- metadata: `file_info.py --file <path>`
- listing: `list_files.py --type image --limit 10`

Tool scripts require absolute paths (`--file /absolute/path/...`).

## Rules

- Do not move/delete files unless user requested it.
- Auto-cleanup removes files older than `cleanup.media_files_days` (see `../config/RULES.md`).
  Cleanup is non-recursive (top-level date folders are not auto-pruned).

## Memory

When file processing reveals durable user patterns or preferences
(e.g., "always transcribe voice notes", preferred formats), update
`memory_system/MAINMEMORY.md` silently.
