use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::env;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use log::debug;
use crate::error_extract::{Diagnostic, parse_cargo_json};

/// The type of compilation step being performed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileStep {
    /// cargo fetch - download dependencies
    Fetch,
    /// cargo check - fast compilation check without code generation
    Check,
    /// cargo test - full test suite execution
    Test,
}

impl CompileStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompileStep::Fetch => "fetch",
            CompileStep::Check => "check",
            CompileStep::Test => "test",
        }
    }

    pub fn cargo_subcommand(&self) -> &'static str {
        match self {
            CompileStep::Fetch => "fetch",
            CompileStep::Check => "check",
            CompileStep::Test => "test",
        }
    }
}

/// Result of a compilation step
#[derive(Debug, Clone)]
pub struct CompileResult {
    pub step: CompileStep,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub diagnostics: Vec<Diagnostic>,
}

impl CompileResult {
    pub fn failed(&self) -> bool {
        !self.success
    }
}

/// Verify that the correct version of a dependency is being used
/// Returns the actual version found, or None if not found
fn verify_dependency_version(
    crate_path: &Path,
    dep_name: &str,
) -> Option<String> {
    debug!("Verifying {} version in {:?}", dep_name, crate_path);

    // Try using cargo metadata which works better with path dependencies
    // Don't use --no-deps because we need to see resolved dependencies
    let output = Command::new("cargo")
        .args(&["metadata", "--format-version=1"])
        .current_dir(crate_path)
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&stdout) {
            // Check resolve.nodes for the dependency
            if let Some(resolve) = metadata.get("resolve") {
                if let Some(nodes) = resolve.get("nodes").and_then(|n| n.as_array()) {
                    for node in nodes {
                        if let Some(deps) = node.get("deps").and_then(|d| d.as_array()) {
                            for dep in deps {
                                if let Some(name) = dep.get("name").and_then(|n| n.as_str()) {
                                    if name == dep_name {
                                        if let Some(pkg) = dep.get("pkg").and_then(|p| p.as_str()) {
                                            // pkg format: "rgb 0.8.52 (path+file://...)" or "rgb 0.8.52 (registry+...)"
                                            let parts: Vec<&str> = pkg.split_whitespace().collect();
                                            if parts.len() >= 2 {
                                                let version = parts[1].to_string();
                                                debug!("Found {} version: {}", dep_name, version);
                                                return Some(version);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    debug!("Could not verify {} version", dep_name);
    None
}

/// Compile a crate at the given path with an optional dependency override
///
/// # Arguments
/// * `crate_path` - Path to the crate to compile
/// * `step` - Whether to run fetch, check, or test
/// * `override_spec` - Optional override specification (crate_name, override_path)
/// Modify Cargo.toml to use a specific version or path override
fn modify_cargo_toml_dependency(
    crate_path: &Path,
    dep_name: &str,
    override_path: &Path,
) -> Result<(), String> {
    use std::io::{Read, Write};

    // Convert to absolute path
    let override_path = if override_path.is_absolute() {
        override_path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|e| format!("Failed to get current dir: {}", e))?
            .join(override_path)
    };

    let cargo_toml_path = crate_path.join("Cargo.toml");
    let mut content = String::new();

    // Read original Cargo.toml
    let mut file = fs::File::open(&cargo_toml_path)
        .map_err(|e| format!("Failed to open Cargo.toml: {}", e))?;
    file.read_to_string(&mut content)
        .map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;
    drop(file);

    // Parse as TOML
    let mut doc: toml_edit::Document = content.parse()
        .map_err(|e| format!("Failed to parse Cargo.toml: {}", e))?;

    // Update dependency in all sections
    let sections = vec!["dependencies", "dev-dependencies", "build-dependencies"];

    for section in sections {
        if let Some(deps) = doc.get_mut(section).and_then(|s| s.as_table_mut()) {
            if let Some(dep) = deps.get_mut(dep_name) {
                debug!("Updating {} in [{}] to path {:?}", dep_name, section, override_path);

                // Replace with path override
                let mut new_dep = toml_edit::InlineTable::new();
                new_dep.insert("path", override_path.display().to_string().into());
                *dep = toml_edit::Item::Value(toml_edit::Value::InlineTable(new_dep));
            }
        }
    }

    // Write back
    let mut file = fs::File::create(&cargo_toml_path)
        .map_err(|e| format!("Failed to create Cargo.toml: {}", e))?;
    file.write_all(doc.to_string().as_bytes())
        .map_err(|e| format!("Failed to write Cargo.toml: {}", e))?;

    debug!("Modified Cargo.toml to use path override: {}", override_path.display());
    Ok(())
}

pub fn compile_crate(
    crate_path: &Path,
    step: CompileStep,
    override_spec: Option<(&str, &Path)>,
) -> Result<CompileResult, String> {
    debug!("compiling {:?} with step {:?}", crate_path, step);

    // Run the cargo command with JSON output for better error extraction
    let start = Instant::now();
    let mut cmd = Command::new("cargo");
    cmd.arg(step.cargo_subcommand());

    // Add --message-format=json for check and test (not fetch)
    if step != CompileStep::Fetch {
        cmd.arg("--message-format=json");
    }

    // If override is provided, use --config flag instead of creating .cargo/config file
    if let Some((crate_name, override_path)) = override_spec {
        // Convert to absolute path if needed
        let override_path = if override_path.is_absolute() {
            override_path.to_path_buf()
        } else {
            env::current_dir()
                .map_err(|e| format!("Failed to get current dir: {}", e))?
                .join(override_path)
        };

        let config_str = format!(
            "patch.crates-io.{}.path=\"{}\"",
            crate_name,
            override_path.display()
        );
        cmd.arg("--config").arg(&config_str);
        debug!("using --config: {}", config_str);
    }

    cmd.current_dir(crate_path);

    debug!("running cargo: {:?}", cmd);
    let output = cmd.output()
        .map_err(|e| format!("Failed to execute cargo: {}", e))?;

    let duration = start.elapsed();
    let success = output.status.success();

    debug!("result: {:?}, duration: {:?}", success, duration);

    // Parse stdout for JSON messages (cargo writes JSON to stdout)
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // Parse diagnostics from JSON output (only for check/test, not fetch)
    let diagnostics = if step != CompileStep::Fetch {
        parse_cargo_json(&stdout)
    } else {
        Vec::new()
    };

    debug!("parsed {} diagnostics", diagnostics.len());

    Ok(CompileResult {
        step,
        success,
        stdout,
        stderr,
        duration,
        diagnostics,
    })
}

/// Emit a .cargo/config file to override a dependency with a local path
fn emit_cargo_override_path(source_dir: &Path, override_path: &Path) -> Result<(), String> {
    debug!("overriding cargo path in {:?} with {:?}", source_dir, override_path);

    // Convert to absolute path if needed
    let override_path = if override_path.is_absolute() {
        override_path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|e| format!("Failed to get current dir: {}", e))?
            .join(override_path)
    };

    let cargo_dir = source_dir.join(".cargo");
    fs::create_dir_all(&cargo_dir)
        .map_err(|e| format!("Failed to create .cargo dir: {}", e))?;

    let config_path = cargo_dir.join("config.toml");
    let mut file = File::create(&config_path)
        .map_err(|e| format!("Failed to create config.toml: {}", e))?;

    let config_content = format!(
        r#"[patch.crates-io]
# This is a temporary override for cargo-crusader testing
# Any crate at this path will override the published version
paths = ["{}"]
"#,
        override_path.display()
    );

    file.write_all(config_content.as_bytes())
        .map_err(|e| format!("Failed to write config: {}", e))?;
    file.flush()
        .map_err(|e| format!("Failed to flush config: {}", e))?;

    Ok(())
}

/// Source of a version being tested
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSource {
    /// Published version from crates.io
    Published(String),
    /// Local work-in-progress version ("this")
    Local(PathBuf),
}

impl VersionSource {
    pub fn label(&self) -> String {
        match self {
            VersionSource::Published(v) => v.clone(),
            VersionSource::Local(_) => "this".to_string(),
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self, VersionSource::Local(_))
    }
}

/// Three-step ICT (Install/Check/Test) result for a single version
#[derive(Debug, Clone)]
pub struct ThreeStepResult {
    /// Install step (cargo fetch) - always runs
    pub fetch: CompileResult,
    /// Check step (cargo check) - only if fetch succeeds
    pub check: Option<CompileResult>,
    /// Test step (cargo test) - only if check succeeds
    pub test: Option<CompileResult>,
    /// Actual version resolved (from cargo tree), if verification succeeded
    pub actual_version: Option<String>,
    /// Expected version being tested
    pub expected_version: Option<String>,
}

impl ThreeStepResult {
    /// Determine if all executed steps succeeded
    pub fn is_success(&self) -> bool {
        if !self.fetch.success {
            return false;
        }
        if let Some(ref check) = self.check {
            if !check.success {
                return false;
            }
        }
        if let Some(ref test) = self.test {
            if !test.success {
                return false;
            }
        }
        true
    }

    /// Get the first failed step, if any
    pub fn first_failure(&self) -> Option<&CompileResult> {
        if !self.fetch.success {
            return Some(&self.fetch);
        }
        if let Some(ref check) = self.check {
            if !check.success {
                return Some(check);
            }
        }
        if let Some(ref test) = self.test {
            if !test.success {
                return Some(test);
            }
        }
        None
    }

    /// Format ICT marks for display (e.g., "✓✓✓", "✓✗-", "✗--")
    /// Shows cumulative failure: after first failure, show dashes
    pub fn format_ict_marks(&self) -> String {
        let fetch_mark = if self.fetch.success { "✓" } else { "✗" };

        if !self.fetch.success {
            return format!("{}--", fetch_mark);
        }

        let check_mark = match &self.check {
            Some(c) if c.success => "✓",
            Some(_) => "✗",
            None => "-",
        };

        if matches!(&self.check, Some(c) if !c.success) {
            return format!("{}{}-", fetch_mark, check_mark);
        }

        let test_mark = match &self.test {
            Some(t) if t.success => "✓",
            Some(_) => "✗",
            None => "-",
        };

        format!("{}{}{}", fetch_mark, check_mark, test_mark)
    }
}

/// Result of testing a dependent against a single version
#[derive(Debug, Clone)]
pub struct VersionTestResult {
    pub version_source: VersionSource,
    pub result: ThreeStepResult,
}

/// Four-step test result for a dependent crate
#[derive(Debug, Clone)]
pub struct FourStepResult {
    pub baseline_check: CompileResult,
    pub baseline_test: Option<CompileResult>,
    pub override_check: Option<CompileResult>,
    pub override_test: Option<CompileResult>,
}

impl FourStepResult {
    /// Determine if this represents a regression
    pub fn is_regressed(&self) -> bool {
        // Baseline must pass
        if !self.baseline_check.success {
            return false;
        }
        if let Some(ref test) = self.baseline_test {
            if !test.success {
                return false;
            }
        }

        // Override must have at least one failure
        if let Some(ref check) = self.override_check {
            if !check.success {
                return true;
            }
        }
        if let Some(ref test) = self.override_test {
            if !test.success {
                return true;
            }
        }

        false
    }

    /// Determine if this represents a passing result
    pub fn is_passed(&self) -> bool {
        // All executed steps must pass
        if !self.baseline_check.success {
            return false;
        }
        if let Some(ref test) = self.baseline_test {
            if !test.success {
                return false;
            }
        }
        if let Some(ref check) = self.override_check {
            if !check.success {
                return false;
            }
        }
        if let Some(ref test) = self.override_test {
            if !test.success {
                return false;
            }
        }

        true
    }

    /// Determine if this represents a broken baseline
    pub fn is_broken(&self) -> bool {
        if !self.baseline_check.success {
            return true;
        }
        if let Some(ref test) = self.baseline_test {
            if !test.success {
                return true;
            }
        }
        false
    }
}

/// Run three-step ICT (Install/Check/Test) test with early stopping
///
/// # Arguments
/// * `crate_path` - Path to the dependent crate
/// * `base_crate_name` - Name of the crate being overridden (e.g., "rgb")
/// * `override_path` - Optional path to override a dependency (None for published baseline)
/// * `skip_check` - Skip cargo check step
/// * `skip_test` - Skip cargo test step
///
/// # Returns
/// ThreeStepResult with cumulative early stopping:
/// - Fetch always runs
/// - Check only runs if fetch succeeds (and !skip_check)
/// - Test only runs if check succeeds (and !skip_test)
pub fn run_three_step_ict(
    crate_path: &Path,
    base_crate_name: &str,
    override_path: Option<&Path>,
    skip_check: bool,
    skip_test: bool,
    expected_version: Option<String>,
) -> Result<ThreeStepResult, String> {
    debug!("running three-step ICT for {:?}", crate_path);

    // Setup: Delete Cargo.lock, backup and modify Cargo.toml if override is provided
    let backup_path = if let Some(override_path) = override_path {
        let lock_file = crate_path.join("Cargo.lock");
        if lock_file.exists() {
            debug!("Deleting Cargo.lock to force dependency resolution");
            fs::remove_file(&lock_file)
                .map_err(|e| format!("Failed to remove Cargo.lock: {}", e))?;
        }

        // Backup Cargo.toml
        let cargo_toml = crate_path.join("Cargo.toml");
        let backup = crate_path.join(".Cargo.toml.backup");
        fs::copy(&cargo_toml, &backup)
            .map_err(|e| format!("Failed to backup Cargo.toml: {}", e))?;

        // Modify Cargo.toml to use path override
        modify_cargo_toml_dependency(crate_path, base_crate_name, override_path)?;

        Some(backup)
    } else {
        None
    };

    // Build override_spec for --config flag (now not used since we modify Cargo.toml directly)
    let override_spec = None; // Don't use --config flag anymore

    // Step 1: Fetch (always runs)
    let fetch = compile_crate(crate_path, CompileStep::Fetch, override_spec)?;

    // Verify the actual version after fetch
    let actual_version = if fetch.success {
        verify_dependency_version(crate_path, base_crate_name)
    } else {
        None
    };

    if fetch.failed() {
        // Fetch failed - stop here with dashes for remaining steps
        return Ok(ThreeStepResult {
            fetch,
            check: None,
            test: None,
            actual_version,
            expected_version,
        });
    }

    // Step 2: Check (only if fetch succeeded and not skipped)
    let check = if !skip_check {
        let result = compile_crate(crate_path, CompileStep::Check, override_spec)?;
        if result.failed() {
            // Check failed - stop here with dash for test
            return Ok(ThreeStepResult {
                fetch,
                check: Some(result),
                test: None,
                actual_version: actual_version.clone(),
                expected_version,
            });
        }
        Some(result)
    } else {
        None
    };

    // Step 3: Test (only if check succeeded or was skipped, and not skip_test)
    let test = if !skip_test {
        let should_run = match &check {
            Some(c) => c.success,
            None => true, // check was skipped, proceed
        };

        if should_run {
            Some(compile_crate(crate_path, CompileStep::Test, override_spec)?)
        } else {
            None
        }
    } else {
        None
    };

    // Cleanup: Restore Cargo.toml from backup if we modified it
    if let Some(backup) = backup_path {
        let cargo_toml = crate_path.join("Cargo.toml");
        fs::copy(&backup, &cargo_toml).ok(); // Ignore errors
        fs::remove_file(&backup).ok(); // Clean up backup
        debug!("Restored Cargo.toml from backup");
    }

    Ok(ThreeStepResult {
        fetch,
        check,
        test,
        actual_version,
        expected_version,
    })
}

/// Run all four build steps: baseline check, baseline test, override check, override test
/// (Legacy function for backwards compatibility - new code should use run_three_step_ict)
///
/// # Arguments
/// * `crate_path` - Path to the dependent crate
/// * `base_crate_name` - Name of the crate being overridden
/// * `baseline_path` - Path to baseline version (or None for published)
/// * `override_path` - Path to work-in-progress version
/// * `skip_check` - Skip cargo check steps
/// * `skip_test` - Skip cargo test steps
pub fn run_four_step_test(
    crate_path: &Path,
    base_crate_name: &str,
    baseline_path: Option<&Path>,
    override_path: &Path,
    skip_check: bool,
    skip_test: bool,
) -> Result<FourStepResult, String> {
    debug!("running four-step test for {:?}", crate_path);

    let baseline_spec = baseline_path.map(|p| (base_crate_name, p));
    let override_spec = Some((base_crate_name, override_path));

    // Step 1: Baseline check
    let baseline_check = if !skip_check {
        compile_crate(crate_path, CompileStep::Check, baseline_spec)?
    } else {
        // If skipping check, create a dummy success result
        CompileResult {
            step: CompileStep::Check,
            success: true,
            stdout: String::new(),
            stderr: "(skipped)".to_string(),
            duration: Duration::from_secs(0),
            diagnostics: Vec::new(),
        }
    };

    if baseline_check.failed() {
        // Baseline check failed - this is BROKEN, don't continue
        return Ok(FourStepResult {
            baseline_check,
            baseline_test: None,
            override_check: None,
            override_test: None,
        });
    }

    // Step 2: Baseline test
    let baseline_test = if !skip_test {
        let result = compile_crate(crate_path, CompileStep::Test, baseline_spec)?;
        if result.failed() {
            // Baseline test failed - this is BROKEN, don't continue
            return Ok(FourStepResult {
                baseline_check,
                baseline_test: Some(result),
                override_check: None,
                override_test: None,
            });
        }
        Some(result)
    } else {
        None
    };

    // Step 3: Override check
    let override_check = if !skip_check {
        Some(compile_crate(crate_path, CompileStep::Check, override_spec)?)
    } else {
        None
    };

    // Step 4: Override test (only if check passed or was skipped)
    let override_test = if !skip_test {
        let should_run_test = match &override_check {
            Some(check) => check.success,
            None => true, // check was skipped
        };

        if should_run_test {
            Some(compile_crate(crate_path, CompileStep::Test, override_spec)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(FourStepResult {
        baseline_check,
        baseline_test,
        override_check,
        override_test,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_step_as_str() {
        assert_eq!(CompileStep::Check.as_str(), "check");
        assert_eq!(CompileStep::Test.as_str(), "test");
    }

    #[test]
    fn test_compile_step_cargo_subcommand() {
        assert_eq!(CompileStep::Check.cargo_subcommand(), "check");
        assert_eq!(CompileStep::Test.cargo_subcommand(), "test");
    }

    #[test]
    fn test_compile_result_failed() {
        let result = CompileResult {
            step: CompileStep::Check,
            success: false,
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
            diagnostics: Vec::new(),
        };
        assert!(result.failed());

        let result = CompileResult {
            step: CompileStep::Check,
            success: true,
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
            diagnostics: Vec::new(),
        };
        assert!(!result.failed());
    }

    #[test]
    fn test_four_step_result_is_broken() {
        let broken = FourStepResult {
            baseline_check: CompileResult {
                step: CompileStep::Check,
                success: false,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(1),
                diagnostics: Vec::new(),
            },
            baseline_test: None,
            override_check: None,
            override_test: None,
        };
        assert!(broken.is_broken());
        assert!(!broken.is_passed());
        assert!(!broken.is_regressed());
    }

    #[test]
    fn test_four_step_result_is_passed() {
        let passed = FourStepResult {
            baseline_check: CompileResult {
                step: CompileStep::Check,
                success: true,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(1),
                diagnostics: Vec::new(),
            },
            baseline_test: Some(CompileResult {
                step: CompileStep::Test,
                success: true,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(2),
                diagnostics: Vec::new(),
            }),
            override_check: Some(CompileResult {
                step: CompileStep::Check,
                success: true,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(1),
                diagnostics: Vec::new(),
            }),
            override_test: Some(CompileResult {
                step: CompileStep::Test,
                success: true,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(2),
                diagnostics: Vec::new(),
            }),
        };
        assert!(!passed.is_broken());
        assert!(passed.is_passed());
        assert!(!passed.is_regressed());
    }

    #[test]
    fn test_four_step_result_is_regressed() {
        let regressed = FourStepResult {
            baseline_check: CompileResult {
                step: CompileStep::Check,
                success: true,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(1),
                diagnostics: Vec::new(),
            },
            baseline_test: Some(CompileResult {
                step: CompileStep::Test,
                success: true,
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(2),
                diagnostics: Vec::new(),
            }),
            override_check: Some(CompileResult {
                step: CompileStep::Check,
                success: false, // Failed!
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::from_secs(1),
                diagnostics: Vec::new(),
            }),
            override_test: None,
        };
        assert!(!regressed.is_broken());
        assert!(!regressed.is_passed());
        assert!(regressed.is_regressed());
    }
}
