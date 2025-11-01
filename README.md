# Cargo Crusader

[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

> Test the downstream impact of crate changes before publishing to crates.io

Cargo Crusader helps Rust crate authors evaluate the impact of API changes on reverse dependencies before publishing. It downloads dependents, builds them against both the published version and your work-in-progress, and reports differences.

**Join the Cargo Crusade** and bring the [Theory of Responsible API Evolution][evo] to practice.

[evo]: https://github.com/rust-lang/rfcs/blob/master/text/1105-api-evolution.md

## ⚠️ Security Warning

**CRITICAL**: This program executes arbitrary untrusted code from crates.io. Always run in sandboxed environments (Docker, VMs, containers).

---

## Quick Start

### Installation

```bash
git clone https://github.com/brson/cargo-crusader
cd cargo-crusader
cargo build --release
export PATH=$PATH:$(pwd)/target/release/
```

### Basic Usage

```bash
cd /path/to/your/crate
cargo-crusader
```

**Output:**
```
Testing 5 reverse dependencies of rgb
  this = 0.8.91 4cc3e60* (your work-in-progress version)

┌────────────┬──────────────────────────┬──────────────────┬────────────────┬──────────┐
│   Status   │        Dependent         │    Depends On    │    Testing     │ Duration │
├────────────┼──────────────────────────┼──────────────────┼────────────────┼──────────┤
│  ✓ PASSED  │image 0.25.8              │^0.8.48 ✓✓        │this ✓✓         │     27.0s│
│  ✓ PASSED  │lodepng 3.10.5            │^0.8.0 ✓✓         │this ✓✓         │     15.3s│
└────────────┴──────────────────────────┴──────────────────┴────────────────┴──────────┘

Summary:
  ✓ Passed:    5
  ✗ Regressed: 0
  ⚠ Broken:    0

HTML report: crusader-report.html
Markdown report: crusader-report.md
```

---

## Features

### ✅ Currently Available

- **Automated Testing**: Test top N dependents or specific crates
- **Version Pinning**: Test specific versions with `crate:version` syntax
- **Parallel Execution**: Multi-threaded testing with `--jobs`
- **Persistent Caching**: 10x faster reruns with build artifact caching
- **Rich Reports**: HTML and AI-optimized markdown outputs
- **Git Integration**: Tracks commit hash and dirty status
- **Detailed Diagnostics**: JSON error parsing from cargo output

### 🚧 In Progress

- **Multi-Version Testing**: `--test-versions` to test against multiple base crate versions
- **3-Step ICT Flow**: Install (fetch) + Check + Test with early stopping

### 💭 Planned

- **Baseline Testing**: Test against git refs with `--baseline`
- **JSON Output**: Machine-readable results for CI integration
- **Docker Support**: Official sandboxed container images

---

## Common Commands

```bash
# Test top 10 dependents
cargo-crusader --top-dependents 10

# Test specific crates (latest versions)
cargo-crusader --dependents image serde tokio

# Test specific versions
cargo-crusader --dependents image:0.25.8 serde:1.0.0

# Parallel testing (4 jobs)
cargo-crusader --jobs 4 --top-dependents 20

# With persistent caching (10x faster reruns)
cargo-crusader --staging-dir .crusader/staging

# Custom path to your crate
cargo-crusader --path ~/my-crate

# Fast check-only (skip tests)
cargo-crusader --no-test --jobs 8

# Future: Multi-version testing
cargo-crusader --test-versions 0.8.0 0.8.48 --path .
```

---

## Result States

| Status | Description |
|--------|-------------|
| **✓ PASSED** | Compiled and tested successfully with both baseline and override |
| **✗ REGRESSED** | Worked with published version, fails with WIP changes |
| **⚠ BROKEN** | Already fails with published version (pre-existing issue) |
| **⊘ SKIPPED** | Version incompatibility (only without `--test-versions`) |
| **⚡ ERROR** | Internal Crusader error during testing |

---

## Reports

### Console Table

Shows colored status, dependent info, version requirements, and build results:
- **Depends On**: Version requirement with check/test marks (e.g., `^0.8.48 ✓✓`)
- **Testing**: WIP version with check/test marks (e.g., `this ✓✓`)
- **Duration**: Total time across all compilation steps

### HTML Report

- Visual summary cards with statistics
- Detailed compilation logs for each dependent
- Expandable error sections
- Color-coded statuses

### Markdown Report (AI-Optimized)

- **Regressions first** (most actionable)
- Structured error details with JSON diagnostics
- Concise passing section
- Ready for LLM analysis

---

## Performance

### Without Caching
- ~14.3s per dependent (first run)
- Full compilation from scratch

### With Caching (`--staging-dir`)
- ~1.4s per dependent (subsequent runs)
- **10x faster** with persistent cache
- ~770MB disk usage for build artifacts

### Parallelization
- Use `--jobs N` (N = CPU cores)
- ~4x speedup on 4-core systems
- Parallelizes among dependents, not within

---

## CLI Reference

For complete documentation, see **[EXAMPLES.md](EXAMPLES.md)**

### Primary Options

```
-p, --path <PATH>               Path to crate (dir or Cargo.toml)
--top-dependents <N>            Test top N by downloads [default: 5]
--dependents <CRATE[:VER]>...   Test specific crates (supports version pins)
--dependent-paths <PATH>...     Test local crates
-j, --jobs <N>                  Parallel jobs [default: 1]
--staging-dir <PATH>            Cache dir [default: .crusader/staging]
--output <PATH>                 HTML output [default: crusader-report.html]
--no-check                      Skip cargo check
--no-test                       Skip cargo test
--keep-tmp                      Keep temp dirs for debugging
```

### Future Options

```
--test-versions <VER>...        Test against multiple base crate versions
--baseline <REF>                Git ref for baseline comparison
--baseline-path <PATH>          Local path as baseline
--json                          JSON output format
```

---

## Architecture

### Execution Flow

1. **Configuration** - Read Cargo.toml, extract name/version, capture git state
2. **Discovery** - Query crates.io API (or use CLI-specified crates)
3. **Testing** - ThreadPool tests each dependent in parallel
4. **Classification** - Determine PASSED/REGRESSED/BROKEN/ERROR
5. **Reporting** - Generate console, HTML, and markdown reports

### Caching Strategy

- **Source cache**: `.crusader/staging/{crate}-{version}/` (unpacked sources)
- **Build artifacts**: Same location, includes `target/` directory
- **Downloads**: `.crusader/crate-cache/` (original .crate files)

### Override Mechanism

Currently uses `.cargo/config` with `paths = [...]` override.

**Planned**: Migrate to `cargo --config 'patch.crates-io.{crate}.path="..."'` for cleaner multi-version testing.

---

## Development

### Project Structure

```
cargo-crusader/
├── src/
│   ├── main.rs           # Entry point, orchestration
│   ├── cli.rs            # CLI parsing with clap
│   ├── api.rs            # crates.io API client
│   ├── compile.rs        # Compilation logic
│   ├── report.rs         # Report generation
│   └── error_extract.rs  # Error parsing
├── tests/
│   ├── api_integration_test.rs
│   ├── cli_parsing_test.rs
│   └── offline_integration.rs
├── test-crates/integration-fixtures/  # Test fixtures
├── SPEC.md               # Technical specification
├── EXAMPLES.md           # Exhaustive usage examples
├── PLAN.md               # Implementation roadmap
└── CLAUDE.md             # AI assistant guidance
```

### Testing

```bash
# Run all tests
cargo test

# Run with API tests (requires network)
cargo test -- --include-ignored

# Build release
cargo build --release

# Test against real crate
RUST_LOG=debug ./target/release/cargo-crusader --path ~/rust-rgb --top-dependents 1
```

### Test Fixtures

Located in `test-crates/integration-fixtures/`:
- **base-crate-v1**: Published baseline version
- **base-crate-v2**: WIP with breaking change (removed `old_api()`)
- **dependent-passing**: Uses only `new_api()` → PASSED
- **dependent-regressed**: Uses removed `old_api()` → REGRESSED
- **dependent-broken**: Has type error → BROKEN
- **dependent-test-***: Test-time regression scenarios

---

## Documentation

- **[SPEC.md](SPEC.md)** - Complete technical specification
- **[EXAMPLES.md](EXAMPLES.md)** - Exhaustive usage examples and patterns
- **[PLAN.md](PLAN.md)** - Implementation roadmap and TODOs
- **[CLAUDE.md](CLAUDE.md)** - Guidance for AI assistants

---

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Test Downstream Impact
on: [pull_request]

jobs:
  crusader:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Install cargo-crusader
        run: |
          git clone https://github.com/brson/cargo-crusader
          cd cargo-crusader
          cargo install --path .

      - name: Test top 10 dependents
        run: cargo-crusader --top-dependents 10 --jobs 4

      - name: Upload report
        if: always()
        uses: actions/upload-artifact@v3
        with:
          name: crusader-report
          path: crusader-report.html
```

### Docker (Recommended for Security)

```bash
docker run --rm -v $(pwd):/work crusader/cargo-crusader \
  --path /work \
  --top-dependents 5
```

---

## Modernization (2025)

This codebase was recently updated from 7-year-old dependencies:

**Updated**:
- Rust 2021 edition
- `ureq` 2.10 (replaced old `curl`)
- `serde` 1.0 (replaced deprecated `rustc-serialize`)
- `tempfile` 3.8 (replaced `tempdir` 0.3)
- `toml` 0.8 (updated from 0.1)
- `crates_io_api` 0.12 (new integration)
- All dependencies to modern versions

**Added**:
- `clap` 4.5 for robust CLI parsing
- Enhanced error diagnostics with JSON parsing
- Persistent caching infrastructure
- AI-optimized markdown reports
- Git version tracking

---

## Contributing

Contributions welcome! Areas for improvement:

1. **Multi-version testing**: Complete `--test-versions` implementation
2. **Refactor to --config**: Replace `.cargo/config` files
3. **3-step ICT flow**: Fetch + Check + Test with early stopping
4. **Live integration tests**: Test against real crates.io
5. **Docker images**: Official sandboxed containers
6. **JSON output**: Machine-readable format
7. **Baseline testing**: Git ref comparison

See **[PLAN.md](PLAN.md)** for detailed roadmap.

---

## Troubleshooting

### Common Issues

**Error: "Cannot specify both --no-check and --no-test"**
→ Choose one or neither

**Error: "Must specify at least one dependent source"**
→ Use `--top-dependents N`, `--dependents`, or `--dependent-paths`

**Disk space exhausted**
→ Clear cache: `rm -rf .crusader/` or use custom `--staging-dir`

**Compilation timeout**
→ Use `--no-test` for faster check-only runs

**Network errors**
→ Check internet connection and crates.io accessibility

See **[EXAMPLES.md](EXAMPLES.md)** for more troubleshooting tips.

---

## Security Best Practices

1. ✅ **Run in Docker/VM/containers** (isolated execution)
2. ✅ **Network isolation** (limit egress to crates.io only)
3. ✅ **Resource limits** (CPU, memory, disk quotas)
4. ✅ **Non-root execution** (never run as root)
5. ✅ **Audit logs** (track tested crates)
6. ✅ **Regular cache cleanup** (remove old artifacts)
7. ✅ **Review dependents** (check crate sources)

---

## Exit Codes

- `0` - Success, no regressions detected
- `-2` - Regressions detected (breaking changes found)
- Other - Internal error

---

## License

MIT/Apache-2.0

This is the official license of The Rust Project and The Cargo Crusade.

---

## History

Original author: Brian Anderson ([@brson](https://github.com/brson))

Modernized in 2025 with:
- Rust 2021 edition
- Modern dependencies
- Enhanced features (caching, parallel testing, rich reports)
- Comprehensive documentation

---

## Links

- **GitHub**: https://github.com/brson/cargo-crusader
- **crates.io**: https://crates.io (API integration)
- **Rust API Evolution RFC**: https://github.com/rust-lang/rfcs/blob/master/text/1105-api-evolution.md

---

**Ready to crusade?** Run `cargo-crusader` in your crate and ensure your changes don't break the ecosystem! 🛡️
