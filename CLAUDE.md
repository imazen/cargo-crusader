# CLAUDE.md

AI assistant guidance for working with this codebase.

## Project

Cargo Crusader tests downstream impact of Rust crate changes by building reverse dependencies against both published and work-in-progress versions.

**⚠️ SECURITY**: Executes arbitrary code from crates.io. Use sandboxed environments.

## Quick Commands

```bash
cargo build --release
cargo test
./target/release/cargo-crusader --path ~/rust-rgb --top-dependents 1
```

## Key Files

- `src/main.rs` - Orchestration, testing flow
- `src/cli.rs` - Argument parsing (clap)
- `src/api.rs` - crates.io API (paginated, 100/page)
- `src/compile.rs` - Compilation logic
- `src/report.rs` - HTML/markdown generation

## Architecture

1. Read Cargo.toml → extract crate name/version
2. Query crates.io for reverse dependencies
3. ThreadPool tests each dependent in parallel
4. For each dependent:
   - Download/cache `.crate` file
   - Build against published version (baseline)
   - Build against local WIP version (override)
   - Compare results
5. Generate HTML/markdown reports

## Override Mechanism

**Current**: Creates `.cargo/config` with `paths = ["/path/to/wip"]`
**Phase 5 Target**: Use `cargo --config 'patch.crates-io.{crate}.path="..."'`

## Test States

- **PASSED**: Works with both baseline and override
- **REGRESSED**: Works with baseline, fails with override
- **BROKEN**: Fails with baseline (pre-existing issue)
- **ERROR**: Internal Crusader error

## Caching

- `.crusader/staging/{crate}-{version}/` - Unpacked sources + build artifacts
- `.crusader/crate-cache/` - Downloaded .crate files
- Provides 10x speedup on reruns

## Next Steps

See [PLAN.md](PLAN.md) for Phase 5+ roadmap:
- Multi-version testing with `--test-versions`
- 3-step ICT flow (Install/Check/Test)
- Per-version console table rows
- Live crates.io integration tests
