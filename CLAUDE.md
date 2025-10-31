# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Cargo Crusader is a tool for Rust crate authors to evaluate the impact of API changes on downstream users before publishing to crates.io. It downloads reverse dependencies (crates that depend on your crate), builds them against both the published version and your work-in-progress version, and reports differences.

**SECURITY WARNING**: This program executes arbitrary untrusted code from the internet. It should be run in a sandboxed environment.

## Building and Testing

```bash
# Build the project
cargo build --release

# Run the tool (from within a crate directory you want to test)
cargo-crusader

# The binary will be at target/release/cargo-crusader
```

## Recent Modernization (2025)

This codebase was recently updated from 7+ year old dependencies to modern Rust 2021 edition:
- **Edition**: Updated to Rust 2021
- **HTTP client**: Replaced `curl` 0.4 with `ureq` 2.10 (avoids old OpenSSL issues)
- **JSON/serialization**: Replaced deprecated `rustc-serialize` with `serde` 1.0 and `serde_json` 1.0
- **Temp files**: Updated `tempdir` 0.3 to `tempfile` 3.8
- **TOML parsing**: Updated from `toml` 0.1 to 0.8 with new API
- Other dependencies updated to modern versions (semver, log, env_logger, threadpool, etc.)

## Core Architecture

### Execution Flow (src/main.rs)

1. **Configuration** (`get_config`): Reads the local Cargo.toml to identify the crate name and sets up override paths
2. **Reverse Dependency Discovery** (`get_rev_deps`): Queries crates.io API for all crates that depend on the current crate (paginated, 100 per page)
3. **Parallel Testing** (`run_test`): Uses a ThreadPool (currently hardcoded to 1 thread at line 57) to test each reverse dependency
4. **Test Execution** (`run_test_local`): For each reverse dependency:
   - Resolves the latest version number
   - Downloads and caches the .crate file to `./.crusader/crate-cache/`
   - Builds against the published (baseline) version
   - If baseline passes, builds against the local work-in-progress (override)
   - Compares results
5. **Report Generation** (`export_report`): Creates `crusader-report.html` with color-coded results

### Test Result States

- **passed**: Built successfully both before and after upgrade
- **regressed**: Built before but failed after upgrade (exit code -2)
- **broken**: Failed to build even against the published version
- **error**: Internal Crusader error during testing

### Dependency Override Mechanism

The tool uses Cargo's `paths` override feature to substitute dependencies:
- Creates a `.cargo/config` file in the temporary build directory
- Sets `paths = ["/path/to/local/wip"]` to override the dependency
- This works because Cargo searches for `.cargo/config` based on CWD, not `--manifest-path`

### Caching

Downloaded crates are cached in `./.crusader/crate-cache/<crate-name>/<crate-name>-<version>.crate` to avoid re-downloading on subsequent runs.

### Test Crates

The `test-crates/` directory contains local test crates for development:
- `crusadertest1`: A minimal test crate
- `crusadertest2`: Depends on crusadertest1 (demonstrates dependency chain)

These can be used to test Crusader's functionality without hitting crates.io.

## Key Limitations and TODOs

- Thread pool is hardcoded to 1 thread (line 57)
- No semver compatibility checking before override (TODO at line 374)
- Unpacking uses external `tar` command, not a Rust library
- HTML report uses inline styles (lines 640-650)
