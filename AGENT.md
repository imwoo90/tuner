# Tuner Code & Documentation Guide (AGENT.md)

This file defines the coding guidelines and architectural standards for the `tuner` project. This project follows the **"Code as Documentation"** philosophy, optimized for seamless collaboration between human developers and AI Agents.

---

## 1. Core Philosophy: Code as Documentation

To prevent documentation drift, the source code itself serves as the single source of truth for both implementation and documentation.

1.  **`mod.rs` = `index.md`**:
    *   Using Rust's module hierarchy, each directory's `mod.rs` acts as the `index.md` (table of contents) for that module.
    *   Use module-level doc comments (`//!`) at the top of `mod.rs` to write high-level architectural summaries, dataflows, and design intents in Markdown.
2.  **Compiler-Verified Documentation**:
    *   Do not write documentation that cannot be compiled. Use standard triple-slash (`///`) doc comments for structs and functions, and always include executable examples (`doctests`).
    *   During `cargo test`, the compiler compiles and executes all doctests, ensuring the documentation never becomes obsolete.
3.  **Compile-Checked Intra-Doc Links**:
    *   Use Rust's native intra-doc link syntax (e.g., `[`[`MyStruct`](file://...)`]`) to reference other types or modules. Rustdoc will verify these links at build time, preventing dead links or hallucinations.

---

## 2. LLM-Agent Constraints (Logical Size Rules)

To keep information highly dense, modular, and friendly to LLM context windows, the following strict limits are enforced at compile time via `build.rs`:

1.  **File-Level Documentation Header (Min 100 Characters)**:
    *   Every non-test production `.rs` file must begin with a file-level documentation comment (`//!`) of **at least 100 characters** outlining its purpose, responsibility, and dependencies.
2.  **File Logical Code Limit (Max 10,000 Characters)**:
    *   The total character count of active executable code lines (excluding comments, doc comments, and empty lines) must be **under 10,000 characters** (approx. 200–300 lines of SLOC).
    *   If a file exceeds this limit, refactor by splitting responsibilities into submodules.
3.  **File Documentation Limit (Max 4,000 Characters)**:
    *   The total character count of documentation comments (`//`, `///`, `//!`, `/* */`) must be **under 4,000 characters** (approx. 50–80 lines).
    *   This forces descriptions to be concise and prevents bloat that reduces the LLM's signal-to-noise ratio.
4.  **Function Physical Limit (Max 2,000 Characters)**:
    *   A single function (including its signature, body, comments, and braces) must be **under 2,000 characters** (approx. 40–50 physical lines).
    *   This ensures each function physically fits on a single screen without scrolling, encouraging single responsibility.

---

## 3. Development Workflow (TDD)

When writing new features, always follow these TDD cycles:
1.  **Write Tests First**: Create a failing unit test or document test (`doctest`) describing the desired behavior.
2.  **Write Implementation & Docs**: Implement the code alongside clean, concise documentation comments.
3.  **Compile & Verify**: Run `cargo test` and `cargo doc` to verify the code passes, the linter complies with limits, and the documentation compiles cleanly without warnings or dead links.

---

## 4. AI Agent Navigation Guide (LLM Wiki)

For AI Agents traversing this repository:
1.  **Entry Points**: Start with `AGENT.md` (this file) and `PROJECT.md` to understand the system architecture, goals, and compilation rules.
2.  **Module Indexing**: Every directory is a Rust module with a `mod.rs` file acting as the `index.md`. Read the module-level documentation (`//!`) at the top of `mod.rs` to understand the architecture, data flow, and submodules.
3.  **Graph Traversal**: Follow the intra-doc links (e.g. `[SessionManager]`) or references in code comments to jump between components. Rustdoc verifies these links compile-time, ensuring a reliable knowledge graph.
4.  **Verification**: Execute `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` to verify that all relationships resolve successfully.
