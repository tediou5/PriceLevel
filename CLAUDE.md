# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this directory.

<!--## Crate-specific CLAUDE.md files
Always consult CLAUDE.md files in sub-crates. Instructions in local CLAUDE.md files override instructions
in this file when they are in conflict.-->

## CLAUDE-local.md files
CLAUDE-local.md files are not checked in, and are maintained by individuals. Consult them if they are present.

## Essential Development Commands

### Building and Installation

```bash
# Build a specific crate. Generally don't need to do release build.
cargo build

# Check code without building (preferred)
cargo check
```

### Testing

```bash
# Run Rust unittests
cargo nextest run --all-features # Preferred test runner
```

**Important Notes for Testing:**
- When compiling or running tests in this repository, set timeout limits to at least 10 minutes due to the large codebase size
- For faster iteration, use -p to select only the most relevant packages for testing. Use multiple `-p` flags if necessary, e.g. `cargo nextest run -p sui-types -p sui-core`
- Use `cargo nextest --lib` to run only library tests and skip integration tests for faster feedback
- Consult crate-specific CLAUDE.md files for instructions on which tests to run, when changing files in those crates

### Linting and Formatting

```bash
# Rust
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
```

## High-Level Architecture

### Critical Development Notes
1. **Testing Requirements**:
   - Always run tests before submitting changes
   - Framework changes require snapshot updates
2. **CRITICAL - Final Development Steps**:
   - **ALWAYS run `cargo clippy` after finishing development** to ensure code passes all linting checks
   - **NEVER disable or ignore tests** - all tests must pass and be enabled
   - **NEVER use `#[allow(dead_code)]`, `#[allow(unused)]`, or any other linting suppressions** - fix the underlying issues instead

### **Comment Writing Guidelines**

**Do NOT comment the obvious** - comments should not simply repeat what the code does.
**When to comment**:
- Non-obvious algorithms or business logic
- Temporary exclusions, timeouts, or thresholds and their reasoning
- Complex calculations where the "why" isn't immediately clear
- Subtle race conditions or threading considerations
- Assumptions about external state or preconditions

**When NOT to comment**:
- Simple variable assignments
- Standard library usage
- Self-descriptive function calls
- Basic control flow (if/for/while)
