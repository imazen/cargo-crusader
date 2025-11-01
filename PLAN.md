# Cargo Crusader - Implementation Plan

This document tracks remaining work and next steps. For completed work, see commit history and HANDOFF.md.

---

## Current Status

**Phase 1-4**: ✅ Complete
- Test fixtures and offline testing
- CLI infrastructure (--path, --dependents, --test-versions)
- 4-step compilation testing
- API integration with pagination
- Enhanced console table with version display
- Persistent caching (10x speedup)
- HTML and markdown reports
- Git version tracking

**Test Coverage**: 52 tests passing

---

## Phase 5: Multi-Version Testing (Next)

**Goal**: Enable testing dependents against multiple versions of the base crate.

**Estimated Effort**: 3-4 hours

### Tasks

#### 5.1 Refactor to Use `--config` Instead of File-Based Override

**Current**: Creates `.cargo/config` files
**Target**: Use `cargo --config 'patch.crates-io.{crate}.path="..."'`

**Benefits**:
- No file I/O
- No cleanup needed
- Enables clean multi-version testing

**Files**: `src/main.rs:766` (compile_with_custom_dep)

#### 5.2 Implement 3-Step ICT Testing

**Steps**:
- I (Install): `cargo fetch` - Download dependencies
- C (Check): `cargo check` - Fast compilation
- T (Test): `cargo test` - Run test suite

**Early stopping**: If I fails, skip C and T. If C fails, skip T.

**Files**: New function in `src/compile.rs`

#### 5.3 Add Multi-Version Data Structures

```rust
struct VersionTestResult {
    version_label: String,      // "0.3.0" or "this"
    version_source: VersionSource,
    result: ThreeStepResult,
}

struct ThreeStepResult {
    fetch: CompileResult,
    check: Option<CompileResult>,
    test: Option<CompileResult>,
}
```

#### 5.4 Implement Multi-Version Testing Loop

**Baseline Inference Logic**:
- If `--path` specified → look for Cargo.toml in that directory
- If no `--path` → look for ./Cargo.toml
- If Cargo.toml found → use as baseline ("this")
- If not found + `--test-versions` → warn, test only specified versions
- If not found + no `--test-versions` → error

**Example behaviors**:
```bash
# Has baseline (Cargo.toml in --path)
cargo-crusader --path ~/my-crate --test-versions 0.8.0 0.8.48
# Tests: 0.8.0, 0.8.48, this (inferred from ~/my-crate/Cargo.toml)

# No baseline (no Cargo.toml)
cargo-crusader --test-versions 0.8.0 0.8.48
# Warning: No baseline found, testing only: 0.8.0, 0.8.48

# Baseline only (no --test-versions)
cargo-crusader --path ~/my-crate
# Tests: published baseline vs. this (current behavior)
```

#### 5.5 Update Console Table for Per-Version Rows

**Format**:
```
Legend: I=Install (cargo fetch), C=Check (cargo check), T=Test (cargo test)

┌────────────┬──────────────────────────┬──────────────┬─────┬──────────┐
│   Status   │        Dependent         │  Version     │ ICT │ Duration │
├────────────┼──────────────────────────┼──────────────┼─────┼──────────┤
│  ✗ REGRESS │image 0.25.8              │0.8.0         │✓✗✓  │     18.2s│
│  ✓ PASSED  │image 0.25.8              │0.8.48        │✓✓✓  │     27.0s│
│  ✓ PASSED  │image 0.25.8              │this          │✓✓✓  │     27.0s│
└────────────┴──────────────────────────┴──────────────┴─────┴──────────┘
```

**Sorting**: Worst status first (REGRESSED > BROKEN > ERROR > PASSED)

#### 5.6 Update HTML and Markdown Reports

- Add "Version" column to HTML summary table
- Expand details sections per version
- In markdown, group by dependent with version matrix

#### 5.7 Add Live Integration Tests

Create `tests/live_integration_test.rs` with `#[ignore]` tests against real crates.io.

---

## Phase 6: Polish and Production Readiness

### 6.1 Performance Optimizations

- [ ] Parallel version testing (within dependent)
- [ ] Smart dependency resolution (skip redundant fetches)
- [ ] Progress bar for long-running tests

### 6.2 User Experience Improvements

- [ ] Better error messages with suggestions
- [ ] Colorized diffs for regression details
- [ ] Summary at start showing test matrix
- [ ] Estimated time remaining

### 6.3 Output Formats

- [ ] JSON output format (`--json`)
- [ ] JUnit XML for CI integration
- [ ] TAP (Test Anything Protocol) format

### 6.4 Advanced Features

- [ ] Watch mode (`--watch`) for continuous testing
- [ ] Baseline from git tags/branches (fetch from repo)
- [ ] Test against multiple Rust toolchains
- [ ] Custom test commands (not just cargo test)

---

## Future Enhancements (Phase 7+)

### 7.1 Docker Integration

- [ ] Official Docker image
- [ ] Sandboxing by default
- [ ] Resource limits built-in
- [ ] Multi-architecture support (x86_64, arm64)

### 7.2 CI/CD Templates

- [ ] GitHub Actions workflow templates
- [ ] GitLab CI templates
- [ ] CircleCI orbs
- [ ] Pre-commit hook templates

### 7.3 Advanced Analysis

- [ ] API diff generation (show what changed)
- [ ] Semver compliance checking
- [ ] Deprecation warnings extraction
- [ ] Dependency graph visualization

### 7.4 Ecosystem Integration

- [ ] crates.io badge support
- [ ] docs.rs integration
- [ ] GitHub PR comments with results
- [ ] Slack/Discord notifications

---

## Non-Goals / Won't Implement

- ~~`--baseline` flag~~ (removed - baseline inferred from Cargo.toml presence)
- ~~`--baseline-path` flag~~ (removed - use --path instead)
- Testing upstream changes (out of scope)
- GUI interface (CLI-focused)
- Windows-specific features (WSL2 sufficient)

---

## Implementation Priority

**High Priority** (Phase 5):
1. Refactor to --config
2. 3-step ICT testing
3. Multi-version loop
4. Console table updates

**Medium Priority** (Phase 6):
- JSON output
- Better error messages
- Progress indicators

**Low Priority** (Phase 7+):
- Docker images
- CI templates
- Advanced analysis

---

## Testing Strategy

### For Phase 5:
1. **Unit tests**: Test individual functions (format_ict_marks, status classification)
2. **Offline integration**: Use test fixtures with multiple versions
3. **Live integration**: Test against real crates.io (with --ignore)
4. **Manual testing**: Test with real crates (rgb, serde, etc.)

### Test Checklist:
- [ ] All existing tests still pass
- [ ] New tests cover multi-version logic
- [ ] Console output renders correctly
- [ ] HTML report displays properly
- [ ] Markdown report is well-formatted
- [ ] No regression in performance
- [ ] Error messages are clear

---

## Documentation Updates Needed

After Phase 5 completion:

**EXAMPLES.md**:
- Remove "⚠️ NOT YET IMPLEMENTED" warnings
- Add real --test-versions examples with output
- Update troubleshooting section

**SPEC.md**:
- Move target structures to current implementation
- Update console output examples
- Document baseline inference logic

**README.md**:
- Move multi-version testing to "✅ Currently Available"
- Remove "Future:" prefixes

**HANDOFF.md**:
- Mark Phase 5 as completed
- Update to Phase 6 plan

---

## Breaking Changes Log

None planned for Phase 5. All changes are additive or internal refactoring.

**Deprecated** (will be removed):
- `CRUSADER_MANIFEST` env var (still works, but `--path` preferred)
- `CRUSADER_LIMIT` env var (never documented, use `--top-dependents`)

---

## Performance Targets

**Current**:
- ~1.4s per dependent (with caching)
- ~14.3s per dependent (cold cache)

**Target (Phase 5)**:
- ~0.5s per version (with caching, using --config)
- No regression in cold cache performance

**Target (Phase 6)**:
- Parallel version testing: N versions in ~1.5s (same as 1 version)
- Progress bar with ETA

---

## Quick Reference for Contributors

### Build and Test
```bash
cargo build --release
cargo test
./target/release/cargo-crusader --path ~/rust-rgb --top-dependents 1
```

### Key Files for Phase 5
- `src/main.rs:766` - compile_with_custom_dep() (needs --config refactor)
- `src/main.rs:79` - run() function (needs version loop)
- `src/compile.rs` - Add run_three_step_test()
- `src/report.rs:157` - Console table printing
- `src/cli.rs` - CLI arguments (already has --test-versions)

### Code Style
- Follow existing patterns
- Add debug logging for new features
- Update tests for new functionality
- Run `cargo clippy` before committing

---

## Questions for Decisions

None currently. Baseline inference logic is now clear and simple.

---

## Links

- **Detailed Phase 5 Plan**: See [HANDOFF.md](HANDOFF.md)
- **Technical Spec**: See [SPEC.md](SPEC.md)
- **Usage Examples**: See [EXAMPLES.md](EXAMPLES.md)
- **Project Overview**: See [README.md](README.md)
