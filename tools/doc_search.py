#!/usr/bin/env python3
"""
Rust codebase API symbol and documentation search utility.
Parses Rust source files directly to find struct, enum, trait, mod, and fn definitions
along with their Rustdoc comments, providing a fast symbol search for LLM agents.
"""

import os
import re
import sys
import json
import argparse

def parse_args():
    parser = argparse.ArgumentParser(description="Search Rust codebase symbols and doc comments.")
    parser.add_argument("query", nargs="?", default="", help="Search query (substring, case-insensitive)")
    parser.add_argument("--path", default=os.path.abspath(os.path.join(os.path.dirname(__file__), "..")),
                        help="Path to the Rust project root (defaults to parent directory of the script)")
    parser.add_argument("--json", action="store_true", help="Output results in JSON format")
    return parser.parse_args()

def extract_items(project_root):
    src_dir = os.path.join(project_root, "src")
    if not os.path.isdir(src_dir):
        return []

    # Regex patterns for matching Rust symbols
    # Capture pub/crate modifiers, keyword, name
    symbol_pat = re.compile(
        r'^\s*(?:pub\s+)?(?:async\s+)?(struct|enum|trait|union|type|fn)\s+([a-zA-Z0-9_]+)'
    )
    mod_pat = re.compile(r'^\s*(?:pub\s+)?mod\s+([a-zA-Z0-9_]+);')

    items = []

    for root, _, files in os.walk(src_dir):
        for file in files:
            if not file.endswith(".rs"):
                continue
            
            abs_path = os.path.abspath(os.path.join(root, file))
            
            try:
                with open(abs_path, 'r', encoding='utf-8') as f:
                    lines = f.readlines()
            except Exception as e:
                continue

            current_docs = []
            file_docs = []
            
            # First pass: collect file-level comments //!
            for line in lines:
                trimmed = line.strip()
                if trimmed.startswith("//!"):
                    doc_line = trimmed[3:].strip()
                    file_docs.append(doc_line)

            # Record file level doc as an item
            if file_docs:
                items.append({
                    "type": "file",
                    "name": file,
                    "file": abs_path,
                    "line": 1,
                    "docs": "\n".join(file_docs),
                    "signature": f"file: {file}"
                })

            for idx, line in enumerate(lines):
                trimmed = line.strip()

                if not trimmed:
                    continue

                if trimmed.startswith("///"):
                    doc_line = trimmed[3:].strip()
                    current_docs.append(doc_line)
                    continue
                
                if trimmed.startswith("//!") or trimmed.startswith("//"):
                    # skip module docs and regular comments
                    continue

                # Check for symbol match
                sym_match = symbol_pat.match(line)
                if sym_match:
                    item_type = sym_match.group(1)
                    item_name = sym_match.group(2)
                    signature = line.strip().rstrip('{').rstrip(';')
                    
                    items.append({
                        "type": item_type,
                        "name": item_name,
                        "file": abs_path,
                        "line": idx + 1,
                        "docs": "\n".join(current_docs),
                        "signature": signature
                    })
                    current_docs = []
                    continue

                mod_match = mod_pat.match(line)
                if mod_match:
                    mod_name = mod_match.group(1)
                    items.append({
                        "type": "mod",
                        "name": mod_name,
                        "file": abs_path,
                        "line": idx + 1,
                        "docs": "\n".join(current_docs),
                        "signature": f"mod {mod_name};"
                    })
                    current_docs = []
                    continue

                # If it's code and not matched, clear current docs accumulator
                # but handle multiline function signatures or attributes
                if not trimmed.startswith("#[") and not trimmed.startswith("]"):
                    current_docs = []

    return items

def main():
    args = parse_args()
    
    if not os.path.isdir(args.path):
        print(f"Error: Project path '{args.path}' does not exist.", file=sys.stderr)
        sys.exit(1)

    items = extract_items(args.path)
    
    query = args.query.strip().lower()
    results = []

    for item in items:
        # Match query in symbol name, signature, docs, or filename
        name_match = query in item["name"].lower()
        sig_match = query in item["signature"].lower()
        docs_match = query in item["docs"].lower()
        file_match = query in os.path.basename(item["file"]).lower()

        if name_match or sig_match or docs_match or file_match:
            results.append(item)

    if args.json:
        print(json.dumps(results, indent=2))
        return

    # Print Markdown output
    if not results:
        print(f"No symbols found matching '{args.query}'.")
        return

    print(f"## Search Results for '{args.query}' ({len(results)} matches)\n")
    for res in results:
        rel_path = os.path.relpath(res["file"], args.path)
        file_link = f"[{rel_path}:L{res['line']}](file://{res['file']}#L{res['line']})"
        
        print(f"### `{res['type']}` **{res['name']}**")
        print(f"- **Defined in**: {file_link}")
        print(f"- **Signature**: `{res['signature']}`")
        if res["docs"]:
            # Format docs nicely
            doc_lines = "\n".join([f"> {l}" for l in res["docs"].split("\n")])
            print(f"- **Documentation**:\n{doc_lines}")
        print()

if __name__ == "__main__":
    main()
