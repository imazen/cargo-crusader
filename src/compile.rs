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

/// Compile a crate at the given path with an optional dependency override
///
/// # Arguments
/// * `crate_path` - Path to the crate to compile
/// * `step` - Whether to run check or test
/// * `override_path` - Optional path to override a dependency with
pub fn compile_crate(
    crate_path: &Path,
    step: CompileStep,
    override_path: Option<&Path>,
) -> Result<CompileResult, String> {
    debug!("compiling {:?} with step {:?}", crate_path, step);

    // If override is provided, set up .cargo/config
    if let Some(override_path) = override_path {
        emit_cargo_override_path(crate_path, override_path)
            .map_err(|e| format!("Failed to emit cargo override: {}", e))?;
    }

    // Run the cargo command with JSON output for better error extraction
    let start = Instant::now();
    let mut cmd = Command::new("cargo");
    cmd.arg(step.cargo_subcommand())
        .arg("--message-format=json")
        .current_dir(crate_path);

    debug!("running cargo: {:?}", cmd);
    let output = cmd.output()
        .map_err(|e| format!("Failed to execute cargo: {}", e))?;

    let duration = start.elapsed();
    let success = output.status.success();

    debug!("result: {:?}, duration: {:?}", success, duration);

    // Parse stdout for JSON messages (cargo writes JSON to stdout)
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // Parse diagnostics from JSON output
    let diagnostics = parse_cargo_json(&stdout);

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

/// Run all four build steps: baseline check, baseline test, override check, override test
///
/// # Arguments
/// * `crate_path` - Path to the dependent crate
/// * `baseline_path` - Path to baseline version (or None for published)
/// * `override_path` - Path to work-in-progress version
/// * `skip_check` - Skip cargo check steps
/// * `skip_test` - Skip cargo test steps
pub fn run_four_step_test(
    crate_path: &Path,
    baseline_path: Option<&Path>,
    override_path: &Path,
    skip_check: bool,
    skip_test: bool,
) -> Result<FourStepResult, String> {
    debug!("running four-step test for {:?}", crate_path);

    // Step 1: Baseline check
    let baseline_check = if !skip_check {
        compile_crate(crate_path, CompileStep::Check, baseline_path)?
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
        let result = compile_crate(crate_path, CompileStep::Test, baseline_path)?;
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
        Some(compile_crate(crate_path, CompileStep::Check, Some(override_path))?)
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
            Some(compile_crate(crate_path, CompileStep::Test, Some(override_path))?)
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
