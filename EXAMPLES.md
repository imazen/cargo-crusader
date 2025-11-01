# Cargo Crusader - Usage Examples

This document provides exhaustive examples of every major use case and CLI argument combination.

## Table of Contents
- [Basic Usage](#basic-usage)
- [Specifying Target Crate](#specifying-target-crate)
- [Selecting Dependents](#selecting-dependents)
- [Version Testing](#version-testing)
- [Performance Tuning](#performance-tuning)
- [Output Formats](#output-formats)
- [Advanced Scenarios](#advanced-scenarios)
- [Expected Output](#expected-output)
- [Troubleshooting](#troubleshooting)

---

## Basic Usage

### Test top 5 dependents (default)

```bash
cd /path/to/your/crate
cargo-crusader
```

**What happens**:
1. Reads `./Cargo.toml` to get crate name and version
2. Queries crates.io for top 5 reverse dependencies by downloads
3. Downloads and tests each dependent
4. Generates `crusader-report.html` and `crusader-report.md`

**Expected output**:
```
crusader: testing 5 reverse dependencies of rgb v0.8.91
crusader: result 1 of 5, image 0.25.8: passed
crusader: result 2 of 5, lodepng 3.10.5: passed
...

Testing 5 reverse dependencies of rgb
  this = 0.8.91 4cc3e60* (your work-in-progress version)

┌────────────┬──────────────────────────┬──────────────────┬────────────────┬──────────┐
│   Status   │        Dependent         │    Depends On    │    Testing     │ Duration │
├────────────┼──────────────────────────┼──────────────────┼────────────────┼──────────┤
│  ✓ PASSED  │image 0.25.8              │^0.8.48 ✓✓        │this ✓✓         │     27.0s│
│  ✓ PASSED  │lodepng 3.10.5            │^0.8.0 ✓✓         │this ✓✓         │     15.3s│
...
└────────────┴──────────────────────────┴──────────────────┴────────────────┴──────────┘

HTML report: crusader-report.html
Markdown report: crusader-report.md
```

### Test top 10 dependents

```bash
cargo-crusader --top-dependents 10
```

**Use case**: More comprehensive testing before major release.

---

## Specifying Target Crate

### Using --path with directory

```bash
cargo-crusader --path /home/user/rust-rgb
```

**What happens**:
- Looks for `/home/user/rust-rgb/Cargo.toml`
- Tests dependents against this WIP version

### Using --path with Cargo.toml

```bash
cargo-crusader --path /home/user/rust-rgb/Cargo.toml
```

**What happens**: Same as above (both forms supported)

### Using environment variable

```bash
export CRUSADER_MANIFEST=/home/user/rust-rgb/Cargo.toml
cargo-crusader
```

**Priority order**: `--path` > `CRUSADER_MANIFEST` > `./Cargo.toml`

### Testing from different directory

```bash
cd /tmp
cargo-crusader --path ~/projects/my-crate
```

**Use case**: Run tests from CI working directory.

---

## Selecting Dependents

### Test specific crates (latest versions)

```bash
cargo-crusader --dependents image serde tokio
```

**What happens**:
1. Queries crates.io for latest version of each crate
2. Downloads: `image` (latest), `serde` (latest), `tokio` (latest)
3. Tests against your WIP version

**Expected output**:
```
Testing 3 reverse dependencies of rgb
  Dependents: image (latest), serde (latest), tokio (latest)
```

### Test specific crate versions

```bash
cargo-crusader --dependents image:0.25.8 serde:1.0.0 tokio:1.35.1
```

**What happens**:
- Downloads exact versions specified
- No crates.io API query for version resolution

**Use case**: Reproduce specific regression or test against known versions.

### Mix pinned and latest versions

```bash
cargo-crusader --dependents image:0.25.8 serde lodepng:3.10.5
```

**What happens**:
- `image`: Uses version 0.25.8
- `serde`: Fetches latest version from crates.io
- `lodepng`: Uses version 3.10.5

### Test local dependent crates

```bash
cargo-crusader --dependent-paths \
  /home/user/my-app \
  /home/user/another-app
```

**What happens**:
- Tests local crates without downloading from crates.io
- Uses crate names from directory names
- Useful for testing internal projects

**Use case**: Test internal applications that depend on your library.

---

## Version Testing

### ⚠️ NOT YET IMPLEMENTED - Future Functionality

### Test against multiple base crate versions

```bash
cargo-crusader \
  --path ~/rust-rgb \
  --test-versions 0.8.48 0.8.40 0.8.0 \
  --dependents image lodepng
```

**What happens** (when implemented):
1. Tests `image` against: `rgb@0.8.48`, `rgb@0.8.40`, `rgb@0.8.0`, `rgb@this` (WIP)
2. Tests `lodepng` against: same 4 versions
3. Total: 8 test combinations (2 dependents × 4 versions)

**Expected output**:
```
Testing 2 reverse dependencies against 4 versions of rgb
  Versions: 0.8.48, 0.8.40, 0.8.0, this (0.8.91 4cc3e60*)

Legend: I=Install (cargo fetch), C=Check (cargo check), T=Test (cargo test)

┌────────────┬──────────────────────────┬──────────────┬─────┬──────────┐
│   Status   │        Dependent         │  rgb Version │ ICT │ Duration │
├────────────┼──────────────────────────┼──────────────┼─────┼──────────┤
│  ✗ REGRESS │image 0.25.8              │0.8.0         │✓✗✓  │     18.2s│
│  ✓ PASSED  │image 0.25.8              │0.8.40        │✓✓✓  │     27.0s│
│  ✓ PASSED  │image 0.25.8              │0.8.48        │✓✓✓  │     27.0s│
│  ✓ PASSED  │image 0.25.8              │this          │✓✓✓  │     27.0s│
│  ✓ PASSED  │lodepng 3.10.5            │0.8.0         │✓✓✓  │     15.1s│
│  ✓ PASSED  │lodepng 3.10.5            │0.8.40        │✓✓✓  │     15.2s│
│  ✓ PASSED  │lodepng 3.10.5            │0.8.48        │✓✓✓  │     15.3s│
│  ✓ PASSED  │lodepng 3.10.5            │this          │✓✓✓  │     15.2s│
└────────────┴──────────────────────────┴──────────────┴─────┴──────────┘
```

**Use case**: Identify when a regression was introduced across version history.

### Test versions without WIP

```bash
cargo-crusader --test-versions 0.8.48 0.8.40
# No --path specified
```

**Expected output**:
```
Warning: Only testing published versions (no --path specified for WIP version)

Testing 5 reverse dependencies against 2 versions of rgb
  Versions: 0.8.48, 0.8.40
```

**Use case**: Compare historical versions without local changes.

### Find breaking change introduction point

```bash
cargo-crusader \
  --dependents image:0.25.8 \
  --test-versions 0.8.0 0.8.10 0.8.20 0.8.30 0.8.40 0.8.48
```

**Use case**: Binary search to find which version introduced a regression.

---

## Performance Tuning

### Parallel testing (4 jobs)

```bash
cargo-crusader --jobs 4 --top-dependents 20
```

**What happens**:
- Creates thread pool with 4 workers
- Tests up to 4 dependents simultaneously
- CPU cores: Usually set to number of cores

**Performance**: ~4x faster on multi-core systems.

### Use persistent caching

```bash
# First run
cargo-crusader --staging-dir .crusader/staging

# Second run (10x faster)
cargo-crusader --staging-dir .crusader/staging
```

**What's cached**:
- Unpacked source files
- Compiled build artifacts (target/)
- Downloaded .crate files

**Disk usage**: ~770MB per run (build artifacts)

### Custom staging directory

```bash
cargo-crusader --staging-dir /tmp/crusader-cache
```

**Use case**:
- Use fast SSD for compilation
- Separate cache per branch
- CI environments with mounted volumes

### Skip tests for faster check-only

```bash
cargo-crusader --no-test --jobs 8
```

**What happens**:
- Only runs `cargo check` (fast compilation)
- Skips `cargo test` (test execution)
- Much faster: ~5x speedup

**Use case**: Quick smoke test during development.

### Skip check for test-only validation

```bash
cargo-crusader --no-check
```

**What happens**:
- Skips `cargo check`
- Only runs `cargo test`

**Use case**: When you know compilation works, validate test behavior.

---

## Output Formats

### Custom HTML output path

```bash
cargo-crusader --output reports/rgb-0.8.91.html
```

**Generated files**:
- `reports/rgb-0.8.91.html` (HTML report)
- `reports/rgb-0.8.91-analysis.md` (Markdown report)

### JSON output (for CI integration)

```bash
cargo-crusader --json > results.json
```

**Format** (when implemented):
```json
{
  "crate": "rgb",
  "version": "0.8.91",
  "git_hash": "4cc3e60",
  "is_dirty": true,
  "results": [
    {
      "dependent": "image",
      "version": "0.25.8",
      "status": "passed",
      "duration_ms": 27000
    }
  ],
  "summary": {
    "passed": 5,
    "regressed": 0,
    "broken": 0
  }
}
```

### Keep temp directories for debugging

```bash
cargo-crusader --keep-tmp
```

**What happens**:
- Doesn't clean up staging directories
- Inspect build artifacts manually
- Check exact cargo commands run

**Use case**: Debug why specific dependent fails.

---

## Advanced Scenarios

### Complete pre-release validation

```bash
cargo-crusader \
  --path . \
  --top-dependents 50 \
  --jobs 8 \
  --staging-dir /mnt/fast-ssd/crusader-cache \
  --output reports/pre-release-$(date +%Y%m%d).html
```

**Use case**: Comprehensive testing before publishing major version.

### CI Pipeline Integration

```bash
#!/bin/bash
set -e

# In GitHub Actions / GitLab CI
cargo-crusader \
  --path . \
  --top-dependents 10 \
  --jobs 4 \
  --output crusader-report.html

# Upload report as artifact
# Fail if regressions detected (exit code -2)
```

### Test specific regression report

```bash
# Reproduce reported issue
cargo-crusader \
  --dependents image:0.25.8 \
  --path . \
  --keep-tmp
```

### Compare two branches

```bash
# Test main branch
git checkout main
cargo-crusader --output reports/main.html

# Test feature branch
git checkout feature/new-api
cargo-crusader --output reports/feature.html

# Diff reports manually
```

### Matrix testing across rust versions

```bash
# Test with multiple rust toolchains
for toolchain in stable beta nightly; do
  rustup run $toolchain cargo-crusader \
    --output reports/$toolchain.html
done
```

### Offline testing (no network)

```bash
# First, populate cache with network
cargo-crusader --top-dependents 5

# Later, test offline
cargo-crusader \
  --dependent-paths .crusader/staging/image-0.25.8 \
  --dependent-paths .crusader/staging/lodepng-3.10.5
```

**Use case**: Test in air-gapped environments.

---

## Expected Output

### Successful run (no regressions)

```
crusader: testing 5 reverse dependencies of rgb v0.8.91
crusader: result 1 of 5, image 0.25.8: passed
crusader: result 2 of 5, lodepng 3.10.5: passed
crusader: result 3 of 5, gif 0.12.0: passed
crusader: result 4 of 5, png 0.17.10: passed
crusader: result 5 of 5, imageproc 0.23.0: passed

====================================================================================
Testing 5 reverse dependencies of rgb
  this = 0.8.91 4cc3e60* (your work-in-progress version)
====================================================================================

┌────────────┬──────────────────────────┬──────────────────┬────────────────┬──────────┐
│   Status   │        Dependent         │    Depends On    │    Testing     │ Duration │
├────────────┼──────────────────────────┼──────────────────┼────────────────┼──────────┤
│  ✓ PASSED  │gif 0.12.0                │^0.8 ✓✓           │this ✓✓         │     12.5s│
│  ✓ PASSED  │image 0.25.8              │^0.8.48 ✓✓        │this ✓✓         │     27.0s│
│  ✓ PASSED  │imageproc 0.23.0          │^0.8 ✓✓           │this ✓✓         │     45.2s│
│  ✓ PASSED  │lodepng 3.10.5            │^0.8.0 ✓✓         │this ✓✓         │     15.3s│
│  ✓ PASSED  │png 0.17.10               │^0.8 ✓✓           │this ✓✓         │     18.7s│
└────────────┴──────────────────────────┴──────────────────┴────────────────┴──────────┘

Summary:
  ✓ Passed:    5
  ✗ Regressed: 0
  ⚠ Broken:    0
  ⊘ Skipped:   0
  ⚡ Error:     0
  ━━━━━━━━━━━━━
  Total:       5

Markdown report: crusader-report.md
HTML report: crusader-report.html
```

**Exit code**: `0` (success)

### Run with regressions

```
crusader: testing 3 reverse dependencies of rgb v0.9.0
crusader: result 1 of 3, image 0.25.8: regressed
crusader: result 2 of 3, lodepng 3.10.5: passed
crusader: result 3 of 3, gif 0.12.0: broken

====================================================================================
Testing 3 reverse dependencies of rgb
  this = 0.9.0 a1b2c3d* (your work-in-progress version)
====================================================================================

┌────────────┬──────────────────────────┬──────────────────┬────────────────┬──────────┐
│   Status   │        Dependent         │    Depends On    │    Testing     │ Duration │
├────────────┼──────────────────────────┼──────────────────┼────────────────┼──────────┤
│  ✗ REGRESS │image 0.25.8              │^0.8.48 ✓✓        │this ✗✓         │     12.3s│
│  ⚠ BROKEN  │gif 0.12.0                │^0.8 ✗            │(not tested)    │      5.1s│
│  ✓ PASSED  │lodepng 3.10.5            │^0.8.0 ✓✓         │this ✓✓         │     15.3s│
└────────────┴──────────────────────────┴──────────────────┴────────────────┴──────────┘

Summary:
  ✓ Passed:    1
  ✗ Regressed: 1  ← Breaking changes detected!
  ⚠ Broken:    1
  ⊘ Skipped:   0
  ⚡ Error:     0
  ━━━━━━━━━━━━━
  Total:       3

Markdown report: crusader-report.md
HTML report: crusader-report.html
```

**Exit code**: `-2` (regressions detected)

### First run vs. cached run

**First run** (cold cache):
```
crusader: testing 1 reverse dependencies of rgb v0.8.91
crusader: downloading image-0.25.8.crate
crusader: unpacking image-0.25.8
crusader: building image-0.25.8 with baseline
    Compiling image v0.25.8
    Finished in 14.3s
crusader: building image-0.25.8 with override
    Compiling image v0.25.8
    Finished in 13.1s
crusader: result 1 of 1, image 0.25.8: passed

Total time: 27.4s
```

**Second run** (warm cache):
```
crusader: testing 1 reverse dependencies of rgb v0.8.91
crusader: using cached image-0.25.8
crusader: using cached staging dir for compilation
crusader: building image-0.25.8 with baseline
    Finished in 0.7s
crusader: using cached staging dir for compilation
crusader: building image-0.25.8 with override
    Finished in 0.7s
crusader: result 1 of 1, image 0.25.8: passed

Total time: 1.4s  (10x faster!)
```

---

## Troubleshooting

### Error: "Cannot specify both --no-check and --no-test"

```bash
cargo-crusader --no-check --no-test  # ❌ Invalid
```

**Fix**: Choose one or neither:
```bash
cargo-crusader --no-check   # ✓ Test only
cargo-crusader --no-test    # ✓ Check only
cargo-crusader              # ✓ Both
```

### Error: "Must specify at least one dependent source"

```bash
cargo-crusader --top-dependents 0  # ❌ Invalid
```

**Fix**:
```bash
cargo-crusader                      # ✓ Use default (5)
cargo-crusader --dependents image   # ✓ Explicit crates
cargo-crusader --dependent-paths /path  # ✓ Local paths
```

### Warning: "Only testing published versions"

```bash
cargo-crusader --test-versions 0.8.0 0.8.48
# Warning: No --path specified, WIP version not tested
```

**Fix**:
```bash
cargo-crusader --path . --test-versions 0.8.0 0.8.48
# Now includes "this" (WIP) automatically
```

### Crate download fails

```
Error: Failed to download image-0.25.8
Network error: Connection timeout
```

**Fix**:
1. Check internet connection
2. Verify crates.io is accessible
3. Check firewall/proxy settings
4. Try with fewer dependents first

### Compilation timeout

```
Error: Compilation timeout after 600s
Crate: some-large-project
```

**Fix**: Not yet configurable, but you can:
1. Skip slow crates
2. Use `--no-test` for faster check-only
3. Increase system resources

### Disk space exhausted

```
Error: No space left on device
Location: .crusader/staging/
```

**Fix**:
```bash
# Clear cache
rm -rf .crusader/

# Or use custom location
cargo-crusader --staging-dir /mnt/large-disk/crusader-cache
```

### Permission denied on staging dir

```
Error: Permission denied: .crusader/staging
```

**Fix**:
```bash
# Fix permissions
chmod -R u+w .crusader/

# Or use different location
cargo-crusader --staging-dir /tmp/crusader-$$
```

---

## Common Patterns

### Pre-commit hook

```bash
# .git/hooks/pre-push
#!/bin/bash
cargo-crusader --top-dependents 3 --no-test
if [ $? -ne 0 ]; then
  echo "Regressions detected! Push aborted."
  exit 1
fi
```

### GitHub Actions

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
        run: cargo install cargo-crusader

      - name: Test top 10 dependents
        run: cargo-crusader --top-dependents 10 --jobs 4

      - name: Upload report
        if: always()
        uses: actions/upload-artifact@v3
        with:
          name: crusader-report
          path: crusader-report.html
```

### Release checklist

```bash
# 1. Update version in Cargo.toml
vim Cargo.toml

# 2. Test top 50 dependents
cargo-crusader --top-dependents 50 --jobs 8

# 3. If passed, publish
cargo publish

# 4. Archive report
mv crusader-report.html reports/v$(cargo read-manifest | jq -r .version).html
```

---

## Performance Tips

1. **Use parallelism**: `--jobs N` (N = number of CPU cores)
2. **Enable caching**: Use `--staging-dir` consistently
3. **Skip tests during dev**: `--no-test` for 5x speedup
4. **Test fewer dependents**: Start with `--top-dependents 3`
5. **Use SSD**: Point `--staging-dir` to fast storage
6. **Incremental testing**: Test specific crates with `--dependents`

## Security Best Practices

1. **Always use sandboxing**: Docker, VMs, or containers
2. **Review dependents**: Check crate sources before testing
3. **Network isolation**: Limit egress to crates.io only
4. **Resource limits**: Set CPU/memory/disk quotas
5. **Non-root execution**: Never run as root
6. **Audit logs**: Track which crates were tested
7. **Clean cache regularly**: Remove old staging directories

---

## Quick Reference

| Scenario | Command |
|----------|---------|
| Basic test | `cargo-crusader` |
| Test 10 dependents | `cargo-crusader --top-dependents 10` |
| Test specific crate | `cargo-crusader --dependents image:0.25.8` |
| Fast check-only | `cargo-crusader --no-test --jobs 4` |
| With caching | `cargo-crusader --staging-dir .crusader/staging` |
| Custom path | `cargo-crusader --path ~/my-crate` |
| Parallel (4 jobs) | `cargo-crusader --jobs 4` |
| Multi-version (future) | `cargo-crusader --test-versions 0.8.0 0.8.48` |
| Debug mode | `RUST_LOG=debug cargo-crusader` |

---

For more information:
- **Full spec**: See [SPEC.md](SPEC.md)
- **Implementation details**: See [PLAN.md](PLAN.md)
- **Contributing**: See [README.md](README.md)
