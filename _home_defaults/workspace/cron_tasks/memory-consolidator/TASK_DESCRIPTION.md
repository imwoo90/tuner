# Memory Consolidator

## Goal

Consolidate, compress, and update the long-term memory file `MAINMEMORY.md` of the Wootuner system using the `memory-system` skill to maintain memory consistency and relevance in a secure and isolated manner.

## Assignment

1. **Read the memory-system Skill Instructions**:
   - Refer to the skill documentation in [SKILL.md](skills/builtin/memory-system/SKILL.md) to understand its capabilities and usage.
2. **Collect Recent Conversation Logs**:
   - Retrieve new chat history logs since the last consolidation by running the `get-logs` subcommand:
     ```bash
     python3 skills/builtin/memory-system/scripts/memory_tool.py get-logs --workspace .
     ```
   - Analyze the output. If no new logs are returned, proceed directly to step 5 to update the consolidation timestamp.
3. **Understand Current Memory**:
   - Read the current [MAINMEMORY.md](memory_system/MAINMEMORY.md) file to review existing user details, system architecture, and preferences.
4. **Extract and Merge Durable Facts**:
   - Analyze the new logs to extract any **durable facts** (long-term facts about the user, family, system configurations, or preferences).
   - Merge these facts into the appropriate sections of `MAINMEMORY.md`.
   - Keep the file compact by removing duplicates, obsolete history, or temporary debugging logs.
5. **Save and Validate via memory-system**:
   - Write the proposed memory content into a temporary file, then pipe it to the `save-memory` subcommand:
     ```bash
     cat <proposed_new_memory_file> | python3 skills/builtin/memory-system/scripts/memory_tool.py save-memory --workspace .
     ```
   - **Note**: The tool will validate the presence of required H2 headers, the `# Main Memory` title, enforce the 120-line compaction limit, and automatically append the current KST consolidation timestamp. If it fails, compress and refine the content and try again.

## Output

After successfully running the tools, output a summary of the results:
- If updates were made: Provide a brief summary of added/modified items along with a markdown diff of the changes.
- If no updates were made: Output: "No new durable information was found compared to existing memory. Only updated the last consolidation timestamp."
