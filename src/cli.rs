use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "cargo-crusader")]
#[command(about = "Test the downstream impact of crate changes before publishing")]
#[command(version)]
pub struct CliArgs {
    /// Path to the crate to test (directory or Cargo.toml file)
    #[arg(long, short = 'p', value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Test top N reverse dependencies by download count
    #[arg(long, default_value = "5")]
    pub top_dependents: usize,

    /// Explicitly test these crates from crates.io (supports "name:version" syntax)
    /// Examples: "image", "image:0.25.8"
    #[arg(long, value_name = "CRATE[:VERSION]")]
    pub dependents: Vec<String>,

    /// Test local crates at these paths
    #[arg(long, value_name = "PATH")]
    pub dependent_paths: Vec<PathBuf>,

    /// Test against specific versions of the base crate (e.g., "0.3.0 4.1.1")
    /// When specified with --path, includes "this" (WIP version) automatically
    #[arg(long, value_name = "VERSION")]
    pub test_versions: Vec<String>,

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
            path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            test_versions: vec![],
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: true,
            no_test: true,
            json: false,
        };
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_zero_jobs_fails() {
        let args = CliArgs {
            path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            test_versions: vec![],
            jobs: 0,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            json: false,
        };
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_valid_config_succeeds() {
        let args = CliArgs {
            path: None,
            top_dependents: 5,
            dependents: vec![],
            dependent_paths: vec![],
            test_versions: vec![],
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            json: false,
        };
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_is_offline_mode() {
        let args = CliArgs {
            path: None,
            top_dependents: 0,
            dependents: vec![],
            dependent_paths: vec![PathBuf::from("/tmp/crate")],
            test_versions: vec![],
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            json: false,
        };
        assert!(args.is_offline_mode());
    }

    #[test]
    fn test_not_offline_mode_with_dependents() {
        let args = CliArgs {
            path: None,
            top_dependents: 0,
            dependents: vec!["serde".to_string()],
            dependent_paths: vec![],
            test_versions: vec![],
            jobs: 1,
            output: PathBuf::from("report.html"),
            staging_dir: PathBuf::from(".crusader/staging"),
            no_check: false,
            no_test: false,
            json: false,
        };
        assert!(!args.is_offline_mode());
    }
}
