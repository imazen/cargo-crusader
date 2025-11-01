# Cargo Crusader - Technical Specification

## Project Overview

Cargo Crusader is a tool for Rust crate authors to evaluate the impact of API changes on downstream users before publishing to crates.io. It downloads reverse dependencies, builds them against both published and work-in-progress versions, and reports differences.

**Security Warning**: This program executes arbitrary untrusted code from the internet. Run in sandboxed environments only.

## Architecture

### Execution Flow

1. **Configuration** - Read local Cargo.toml, extract crate name and version, capture git metadata
2. **Dependency Discovery** - Query crates.io API for reverse dependencies (or use CLI-specified crates)
3. **Parallel Testing** - Use ThreadPool to test each dependent × version combination
4. **Result Aggregation** - Collect and classify results (PASSED/REGRESSED/BROKEN/SKIPPED/ERROR)
5. **Report Generation** - Create console table, HTML report, and AI-focused markdown report

### Test Flow (Per Dependent × Version)

#### Current Implementation (2-step, legacy)
1. **Baseline Build**: cargo build against published version
2. **Override Build**: cargo build with local WIP override

#### Target Implementation (3-step ICT)
1. **I (Install)**: `cargo fetch` - Download dependencies
2. **C (Check)**: `cargo check` - Fast compilation check
3. **T (Test)**: `cargo test` - Run test suite

**Early stopping**: If step I fails, skip C and T. If C fails, skip T.

### Result States

- **PASSED**: All executed steps succeed
- **REGRESSED**: Baseline passes, override fails
- **BROKEN**: Baseline fails (pre-existing issue)
- **SKIPPED**: Version incompatibility (only without --test-versions)
- **ERROR**: Internal Crusader error

### Override Mechanism

#### Current (File-based)
Creates `.cargo/config` with:
```toml
paths = ["/path/to/local/wip"]
```

#### Target (Command-line)
Use `cargo --config`:
```bash
cargo build --config 'patch.crates-io.rgb.path="/path/to/rgb-0.3.0"'
```

**Benefits of --config approach**:
- No file I/O
- No cleanup needed
- No conflicts with existing configs
- Atomic per-command
- Enables clean multi-version testing

## CLI Interface

### Arguments

```
cargo-crusader [OPTIONS]

OPTIONS:
  -p, --path <PATH>
      Path to crate to test (directory or Cargo.toml file)
      Priority: CLI arg > CRUSADER_MANIFEST env var > "./Cargo.toml"

  --top-dependents <N>
      Test top N reverse dependencies by download count [default: 5]

  --dependents <CRATE[:VERSION]>...
      Explicitly test these crates from crates.io
      Supports version pinning: "image:0.25.8"
      Without version: fetches latest
      Examples: --dependents image:0.25.8 serde lodepng:3.10.5

  --dependent-paths <PATH>...
      Test local crates at these paths

  --test-versions <VERSION>...
      Test against specific versions of the base crate
      When used with --path, automatically includes "this" (WIP)
      Without --path, warns about missing WIP version
      Examples: --test-versions 0.3.0 4.1.1

  -j, --jobs <N>
      Number of parallel test jobs [default: 1]
      Parallelizes among dependents, not within

  --output <PATH>
      HTML report output path [default: crusader-report.html]

  --staging-dir <PATH>
      Directory for staging unpacked crates
      Enables caching across runs (source + build artifacts)
      [default: .crusader/staging]

  --no-check
      Skip cargo check (only run tests)

  --no-test
      Skip cargo test (only run check)

  --keep-tmp
      Keep temporary build directories for debugging

  --json
      Output results as JSON
```

### Validation Rules

- Cannot specify both `--no-check` and `--no-test`
- Must specify at least one of: `--top-dependents`, `--dependents`, or `--dependent-paths`
- `--jobs` must be at least 1

## Data Structures

### Core Types

```rust
// Crate name (String alias)
type RevDepName = String;

// Reverse dependency with version info
struct RevDep {
    name: RevDepName,
    vers: Version,                    // Dependent's version
    resolved_version: Option<String>, // Version requirement for base crate
}

// Configuration for test run
struct Config {
    crate_name: String,
    version: String,
    git_hash: Option<String>,    // Short git hash (7 chars)
    is_dirty: bool,              // Uncommitted changes
    staging_dir: PathBuf,
    base_override: CrateOverride,
    next_override: CrateOverride,
    limit: Option<usize>,
}

// Test result for one dependent
struct TestResult {
    rev_dep: RevDep,
    data: TestResultData,
}

enum TestResultData {
    Passed(FourStepResult),
    Regressed(FourStepResult),
    Broken(FourStepResult),
    Skipped(String),
    Error(Error),
}

// Current 4-step result structure
struct FourStepResult {
    baseline_check: CompileResult,
    baseline_test: Option<CompileResult>,
    override_check: Option<CompileResult>,
    override_test: Option<CompileResult>,
}

// Compilation step result
struct CompileResult {
    step: CompileStep,
    success: bool,
    stdout: String,
    stderr: String,
    duration: Duration,
    diagnostics: Vec<Diagnostic>,
}

enum CompileStep {
    Fetch,  // cargo fetch
    Check,  // cargo check
    Test,   // cargo test
}
```

### Target Multi-Version Structures

```rust
// Result for testing one dependent against multiple versions
struct MultiVersionTestResult {
    rev_dep: RevDep,
    version_results: Vec<VersionTestResult>,
}

// Result for one version
struct VersionTestResult {
    version_label: String,         // "0.3.0", "this"
    version_source: VersionSource,
    result: ThreeStepResult,
}

enum VersionSource {
    Published(String),  // Version from crates.io
    Local(PathBuf),     // Local WIP path
}

// 3-step ICT result
struct ThreeStepResult {
    fetch: CompileResult,              // cargo fetch
    check: Option<CompileResult>,      // cargo check (if fetch succeeded)
    test: Option<CompileResult>,       // cargo test (if check succeeded)
}
```

## Caching Strategy

### Source Caching
- **Location**: `--staging-dir` (default: `.crusader/staging`)
- **Structure**: `{staging-dir}/{crate-name}-{version}/`
- **Contents**: Unpacked .crate file (tar.gz extracted)
- **Behavior**: Check if directory exists before unpacking

### Build Artifact Caching
- **Location**: Same as source (`{staging-dir}/{crate-name}-{version}/target/`)
- **Performance**: 10x speedup (14.3s → 1.4s on second run)
- **Size**: ~770MB for typical test run
- **Persistence**: Survives across runs until manually cleared

### Crate Download Caching
- **Location**: `./.crusader/crate-cache/{crate-name}/{crate-name}-{version}.crate`
- **Purpose**: Avoid re-downloading from crates.io
- **Format**: Original .crate file (tar.gz)

## Version Resolution

### For Dependents (--dependents)

**Without version pin** (`--dependents image`):
1. Query crates.io API for crate metadata
2. Extract all version strings
3. Parse as semver versions
4. Sort and select highest

**With version pin** (`--dependents image:0.25.8`):
1. Parse version string
2. Validate with semver parser
3. Use directly (skip crates.io query)

### For Base Crate Dependency Requirements

Uses `cargo metadata` to extract exact version requirement:
```bash
cd {staging-dir}/{dependent-name}-{version}
cargo metadata --format-version=1
# Parse JSON, find base crate in dependencies
# Extract "req" field (e.g., "^0.8.48")
```

**Fallback**: If extraction fails, shows "?" in reports.

### For Git Metadata

```bash
# Short hash (7 chars)
git rev-parse --short HEAD

# Dirty flag (uncommitted changes)
git status --porcelain  # Empty output = clean
```

## Report Formats

### Console Table (Current)

```
==============================================================================================================
Testing 1 reverse dependencies of rgb
  this = 0.8.91 4cc3e60* (your work-in-progress version)
==============================================================================================================

┌────────────┬──────────────────────────┬──────────────────┬────────────────┬──────────┐
│   Status   │        Dependent         │    Depends On    │    Testing     │ Duration │
├────────────┼──────────────────────────┼──────────────────┼────────────────┼──────────┤
│  ✓ PASSED  │image 0.25.8              │^0.8.48 ✓✓        │this ✓✓         │     27.0s│
└────────────┴──────────────────────────┴──────────────────┴────────────────┴──────────┘

Legend: First ✓/✗ = check, Second ✓/✗ = test

Summary:
  ✓ Passed:    1
  ✗ Regressed: 0
  ⚠ Broken:    0
  ⊘ Skipped:   0
  ⚡ Error:     0
```

### Console Table (Target with --test-versions)

```
====================================================================================
Testing 2 reverse dependencies against 3 versions of rgb
  Versions: 0.8.48 (baseline), 0.3.0, this (0.8.91 4cc3e60*)

Legend: I=Install (cargo fetch), C=Check (cargo check), T=Test (cargo test)
====================================================================================
┌────────────┬──────────────────────────┬──────────────┬─────┬──────────┐
│   Status   │        Dependent         │  rgb Version │ ICT │ Duration │
├────────────┼──────────────────────────┼──────────────┼─────┼──────────┤
│  ✗ REGRESS │image 0.25.8              │0.3.0         │✓✗✓  │     18.2s│
│  ✓ PASSED  │image 0.25.8              │0.8.48        │✓✓✓  │     27.0s│
│  ✓ PASSED  │image 0.25.8              │this          │✓✓✓  │     27.0s│
│  ✓ PASSED  │lodepng 3.10.5            │0.8.48        │✓✓✓  │     15.3s│
│  ✓ PASSED  │lodepng 3.10.5            │0.3.0         │✓✓✓  │     15.1s│
│  ✓ PASSED  │lodepng 3.10.5            │this          │✓✓✓  │     15.2s│
└────────────┴──────────────────────────┴──────────────┴─────┴──────────┘

Sorting: Worst status first (REGRESSED > BROKEN > ERROR > PASSED > SKIPPED)
```

### HTML Report

- **Summary statistics**: Visual cards with counts
- **Summary table**: Crate | Version | Depends On | Result
- **Detailed results**: Expandable sections with full stdout/stderr
- **Styling**: Inline CSS, color-coded statuses
- **Navigation**: Anchor links from summary to details

### Markdown Report (AI-focused)

- **Optimized for LLM analysis**
- **Regression section first** (most important)
- **Structured error details**: JSON diagnostics when available
- **Concise passing section**: Just names
- **Actionable format**: Clear distinction between "needs fix" and "FYI"

## Error Extraction

### JSON Diagnostics (Modern)

When cargo is run with `--message-format=json`:
```rust
struct Diagnostic {
    level: DiagnosticLevel,  // Error, Warning, Help, Note
    message: String,
    code: Option<String>,    // E0308, etc.
    rendered: String,        // Formatted output
    primary_span: Option<Span>,
}

struct Span {
    file_name: String,
    line: usize,
    column: usize,
    label: Option<String>,
}
```

### Fallback (Legacy)

Parse stderr for error patterns when JSON not available.

## Test Infrastructure

### Test Fixtures (`test-crates/integration-fixtures/`)

**base-crate-v1** (published baseline):
```rust
pub fn new_api() -> i32 { 1 }
pub fn old_api() -> i32 { 2 }  // Will be removed in v2
```

**base-crate-v2** (WIP with breaking change):
```rust
pub fn new_api() -> i32 { 1 }
// old_api() removed - breaking change
```

**Dependents**:
- `dependent-passing`: Uses only `new_api()` → PASSED
- `dependent-regressed`: Uses `old_api()` → REGRESSED
- `dependent-broken`: Has type error → BROKEN
- `dependent-test-passing`: Tests pass with both → PASSED
- `dependent-test-failing`: Tests use `old_api()` → REGRESSED

### Test Coverage

**Unit tests**: 29 passing
- CLI argument parsing and validation
- Compile step enums
- Report formatting
- Error extraction

**Integration tests**: 6 passing (API)
- Reverse dependency pagination
- Version resolution
- Top dependents retrieval

**Offline integration**: 17 passing
- Fixture correctness
- Staging directory behavior
- Cargo metadata parsing
- Dependency extraction

**Total**: 52 tests

## Implementation Status

### ✅ Completed (Phase 1-4)
- Test fixtures
- CLI infrastructure (--path, --dependents, --test-versions)
- crate:version syntax parsing
- Compile module with 4-step testing
- API module with pagination
- Enhanced console table with version display
- Git version tracking
- Persistent caching (source + artifacts)
- HTML and markdown reports
- Dependency version resolution via cargo metadata

### 🚧 In Progress
- CompileStep::Fetch added (not yet used)
- --test-versions CLI arg added (not yet functional)

### ⏳ Planned (Phase 5-6)
- Refactor to use `--config` instead of `.cargo/config` files
- Implement 3-step ICT testing flow
- Multi-version testing loop
- Per-version result structures
- Updated console table (one row per version)
- Updated HTML/markdown reports for multi-version
- Live integration tests against real crates.io

### 💭 Future Enhancements
- --json output format
- Parallel version testing (within dependent)
- Smart error summarization
- CI/CD templates
- Docker image with sandboxing

## Performance Characteristics

### Without Caching
- **First run**: ~14.3s for 1 dependent
- **Dominated by**: Compilation time
- **Network**: 1-2s for dependency fetching

### With Caching
- **Subsequent runs**: ~1.4s (10x faster)
- **Cached**: Source files + build artifacts (target/)
- **Size**: ~770MB per run

### Parallelization
- **Current**: Among dependents only (--jobs N)
- **Thread pool**: Configurable parallelism
- **Future**: Could parallelize versions within dependent

## Security Considerations

⚠️ **CRITICAL**: This tool executes arbitrary code from crates.io

### Mitigation Strategies

1. **Sandboxing**: Run in Docker/VM/containers
2. **Network isolation**: Limit egress except crates.io
3. **Resource limits**: CPU, memory, disk quotas
4. **Timeout**: Per-crate compilation limits
5. **Audit logs**: Track what was executed
6. **Non-root**: Never run as root

### Docker Usage

```bash
docker run --rm -v $(pwd):/work crusader/cargo-crusader \
  --path /work \
  --top-dependents 5
```

## API Integration

### crates.io API

Uses `crates_io_api` crate (v0.12):

```rust
// Get reverse dependencies (paginated)
client.crate_reverse_dependencies("rgb", 1, 100)

// Get crate metadata
client.get_crate("image")

// Download crate file
client.download_crate("image", "0.25.8")
```

**Rate limits**: Respect crates.io limits, use pagination

## File Structure

```
cargo-crusader/
├── src/
│   ├── main.rs           # Entry point, test orchestration
│   ├── cli.rs            # CLI argument parsing
│   ├── api.rs            # crates.io API client
│   ├── compile.rs        # Compilation logic
│   ├── report.rs         # Report generation
│   └── error_extract.rs  # Error parsing
├── tests/
│   ├── api_integration_test.rs
│   ├── cli_parsing_test.rs
│   └── offline_integration.rs
├── test-crates/
│   └── integration-fixtures/
│       ├── base-crate-v1/
│       ├── base-crate-v2/
│       ├── dependent-passing/
│       ├── dependent-regressed/
│       ├── dependent-broken/
│       ├── dependent-test-passing/
│       └── dependent-test-failing/
├── .crusader/
│   ├── crate-cache/      # Downloaded .crate files
│   └── staging/          # Unpacked sources + build artifacts
├── Cargo.toml
├── README.md
├── SPEC.md               # This file
├── EXAMPLES.md           # Usage examples
├── CLAUDE.md             # AI assistant guidance
└── PLAN.md               # Implementation roadmap
```

## Dependencies

```toml
[dependencies]
clap = { version = "4.5", features = ["derive"] }
semver = "1.0"
toml = "0.8"
log = "0.4"
env_logger = "0.11"
ureq = "2.10"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
threadpool = "1.8"
num_cpus = "1.16"
tempfile = "3.8"
lazy_static = "1.4"
term = "0.7"
crates_io_api = "0.12"
```

## Build Requirements

- **Rust**: 2021 edition (1.56+)
- **Platform**: Linux, macOS, Windows (WSL2 tested)
- **Network**: Internet access for crates.io
- **Disk**: ~1GB free for caching

## Exit Codes

- `0`: Success, no regressions
- `-2`: Regressions detected
- Other: Internal error

## Environment Variables

- `CRUSADER_MANIFEST`: Path to Cargo.toml (fallback if --path not specified)
- `RUST_LOG`: Control logging level (debug, info, warn, error)
- `CRUSADER_LIMIT`: Deprecated, use `--top-dependents` instead
