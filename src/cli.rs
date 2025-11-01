use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "cargo-crusader")]
#[command(about = "Test the downstream impact of crate changes before publishing")]
#[command(version)]
pub struct CliArgs {
    /// Path to the crate to test (directory or Cargo.toml file)
    #[arg(long, short = 'p', value_name = "PATH")]
    pub manifest_path: Option<PathBuf>,

    /// Test top N reverse dependencies by download count
    #[arg(long, default_value = "5")]
    pub top_dependents: usize,

    /// Explicitly test these crates from crates.io
    #[arg(long, value_name = "CRATE")]
    pub dependents: Vec<String>,

    /// Test local crates at these paths
    #[arg(long, value_name = "PATH")]
    pub dependent_paths: Vec<PathBuf>,

    /// Git reference for baseline (tag/commit/branch)
    #[arg(long, value_name = "REF")]
    pub baseline: Option<String>,

    /// Use local path as baseline instead of published version
    #[arg(long, value_name = "PATH")]
    pub baseline_path: Option<PathBuf>,

    /// Number of parallel test jobs
    #[arg(long, short = 'j', default_value = "1")]
    pub jobs: usize,

    /// HTML report output path
    #[arg(long, default_value = "crusader-report.html")]
    pub output: PathBuf,

    /// Directory for staging unpacked crates (enables caching across runs)
    #[arg(long, default_value = ".crusader/staging")]
    pub staging_dir: PathBuf,

    /// Skip cargo check (only run tests)
    #[arg(long)]
    pub no_check: bool,

    /// Skip cargo test (only run check)
    #[arg(long)]
    pub no_test: bool,

    /// Keep temporary build directories for debugging
    #[arg(long)]
    pub keep_tmp: bool,

    /// Output results as JSON
    #[arg(long)]
    pub json: bool,
}

impl CliArgs {
    /// Parse command-line arguments
    pub fn parse_args() -> Self {
        CliArgs::parse()
    }

    /// Validate argument combinations
    pub fn validate(&self) -> Result<(), String> {
        // Can't skip both check and test
        if self.no_check && self.no_test {
            return Err("Cannot specify both --no-check and --no-test".to_string());
        }

        // Can't specify both baseline options
        if self.baseline.is_some() && self.baseline_path.is_some() {
            return Err("Cannot specify both --baseline and --baseline-path".to_string());
        }

        // Need at least one of: top_dependents, dependents, or dependent_paths
        if self.top_dependents == 0
            && self.dependents.is_empty()
            && self.dependent_paths.is_empty() {
            return Err("Must specify at least one of: --top-dependents, --dependents, or --dependent-paths".to_string());
        }

        // Validate jobs >= 1
        if self.jobs == 0 {
            return Err("--jobs must be at least 1".to_string());
        }

        Ok(())
    }

    /// Check if we're testing local paths only (no network required)
    pub fn is_offline_mode(&self) -> bool {
        self.dependents.is_empty()
            && self.top_dependents == 0
            && !self.dependent_paths.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_both_no_flags_fails() {
        let args = CliArgs {
            manifest_path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            baseline: None,
            baseline_path: None,
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: true,
            no_test: true,
            keep_tmp: false,
            json: false,
        };
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_both_baseline_options_fails() {
        let args = CliArgs {
            manifest_path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            baseline: Some("v1.0.0".to_string()),
            baseline_path: Some(PathBuf::from("/tmp/baseline")),
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            keep_tmp: false,
            json: false,
        };
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_zero_jobs_fails() {
        let args = CliArgs {
            manifest_path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            baseline: None,
            baseline_path: None,
            jobs: 0,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            keep_tmp: false,
            json: false,
        };
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_valid_config_succeeds() {
        let args = CliArgs {
            manifest_path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            baseline: None,
            baseline_path: None,
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            keep_tmp: false,
            json: false,
        };
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_is_offline_mode() {
        let args = CliArgs {
            manifest_path: None,
            top_dependents: 0,
            dependents: vec![],
            dependent_paths: vec![PathBuf::from("/tmp/crate")],
            baseline: None,
            baseline_path: None,
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            keep_tmp: false,
            json: false,
        };
        assert!(args.is_offline_mode());
    }

    #[test]
    fn test_not_offline_mode_with_dependents() {
        let args = CliArgs {
            manifest_path: None,
            top_dependents: 0,
            dependents: vec!["serde".to_string()],
            dependent_paths: vec![],
            baseline: None,
            baseline_path: None,
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            keep_tmp: false,
            json: false,
        };
        assert!(!args.is_offline_mode());
    }
}
