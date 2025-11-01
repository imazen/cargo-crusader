// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

mod api;
mod cli;
mod compile;
mod error_extract;
mod report;

use semver::Version;
use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{PathBuf, Path};
use std::process::Command;
use std::string::FromUtf8Error;
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender, Receiver, RecvError};
use std::time::Duration;
use threadpool::ThreadPool;
use tempfile::TempDir;
use crates_io_api::SyncClient;

use lazy_static::lazy_static;
use log::debug;

const USER_AGENT: &str = "cargo-crusader/0.1.1 (https://github.com/brson/cargo-crusader)";

lazy_static! {
    static ref CRATES_IO_CLIENT: SyncClient = {
        SyncClient::new(USER_AGENT, Duration::from_millis(1000))
            .expect("Failed to create crates.io API client")
    };
}

fn main() {
    env_logger::init();

    // Parse CLI arguments
    let args = cli::CliArgs::parse_args();

    // Validate arguments
    if let Err(e) = args.validate() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    // Get config
    let config = match get_config() {
        Ok(c) => c,
        Err(e) => {
            report_error(e);
            return;
        }
    };

    // Run tests and report results
    let results = run(args.clone(), config.clone());
    report_results(results, &args, &config);
}

fn run(args: cli::CliArgs, config: Config) -> Result<Vec<TestResult>, Error> {

    // Determine which dependents to test
    let rev_deps = if !args.dependent_paths.is_empty() {
        // Local paths mode - convert to rev dep names
        args.dependent_paths
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| Error::InvalidPath(p.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else if !args.dependents.is_empty() {
        // Explicit crate names from crates.io
        args.dependents.clone()
    } else {
        // Top N by downloads
        let api_deps = api::get_top_dependents(&config.crate_name, args.top_dependents)
            .map_err(|e| Error::CratesIoApiError(e))?;
        api_deps.into_iter().map(|d| d.name).collect()
    };

    status(&format!(
        "testing {} reverse dependencies of {} v{}",
        rev_deps.len(),
        config.crate_name,
        config.version
    ));

    // Run all the tests in a thread pool and create a list of result
    // receivers.
    let mut result_rxs = Vec::new();
    let ref mut pool = ThreadPool::new(args.jobs);
    for rev_dep in rev_deps {
        let result = run_test(pool, config.clone(), rev_dep);
        result_rxs.push(result);
    }

    // Now wait for all the results and return them.
    let total = result_rxs.len();
    let results = result_rxs.into_iter().enumerate().map(|(i, r)| {
        let r = r.recv();
        report_quick_result(i + 1, total, &r);
        r
    });
    let results = results.collect::<Vec<_>>();

    Ok(results)
}

#[derive(Clone)]
struct Config {
    crate_name: String,
    version: String,
    base_override: CrateOverride,
    next_override: CrateOverride,
    limit: Option<usize>,
}

#[derive(Clone)]
enum CrateOverride {
    Default,
    Source(PathBuf)
}

fn get_config() -> Result<Config, Error> {
    let manifest = env::var("CRUSADER_MANIFEST");
    let manifest = manifest.unwrap_or_else(|_| "./Cargo.toml".to_string());
    let manifest = PathBuf::from(manifest);
    debug!("Using manifest {:?}", manifest);

    let limit = env::var("CRUSADER_LIMIT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok());

    let (crate_name, version) = get_crate_info(&manifest)?;
    Ok(Config {
        crate_name,
        version,
        base_override: CrateOverride::Default,
        next_override: CrateOverride::Source(manifest),
        limit,
    })
}

fn get_crate_info(manifest_path: &Path) -> Result<(String, String), Error> {
    let toml_str = load_string(manifest_path)?;
    let value: toml::Value = toml::from_str(&toml_str)?;

    match value.get("package") {
        Some(toml::Value::Table(t)) => {
            let name = match t.get("name") {
                Some(toml::Value::String(s)) => s.clone(),
                _ => return Err(Error::ManifestName),
            };

            let version = match t.get("version") {
                Some(toml::Value::String(s)) => s.clone(),
                _ => "0.0.0".to_string(), // Default if no version
            };

            Ok((name, version))
        }
        _ => Err(Error::ManifestName),
    }
}

// Legacy function for compatibility
fn get_crate_name(manifest_path: &Path) -> Result<String, Error> {
    get_crate_info(manifest_path).map(|(name, _)| name)
}

fn load_string(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path)?;
    let mut s = String::new();
    (file.read_to_string(&mut s)?);
    Ok(s)
}

type RevDepName = String;

fn crate_url(krate: &str, call: Option<&str>) -> String {
    crate_url_with_parms(krate, call, &[])
}

fn crate_url_with_parms(krate: &str, call: Option<&str>, parms: &[(&str, &str)]) -> String {
    let url = format!("https://crates.io/api/v1/crates/{}", krate);
    let s = match call {
        Some(c) => format!("{}/{}", url, c),
        None => url
    };

    if !parms.is_empty() {
        let parms: Vec<String> = parms.iter().map(|&(k, v)| format!("{}={}", k, v)).collect();
        let parms: String = parms.join("&");
        format!("{}?{}", s, parms)
    } else {
        s
    }
}

fn get_rev_deps(crate_name: &str, limit: Option<usize>) -> Result<Vec<RevDepName>, Error> {
    status(&format!("downloading reverse deps for {}", crate_name));

    let deps = CRATES_IO_CLIENT.crate_reverse_dependencies(crate_name)
        .map_err(|e| Error::CratesIoApiError(e.to_string()))?;

    let mut all_deps: Vec<String> = deps.dependencies
        .into_iter()
        .map(|d| d.dependency.crate_id)
        .collect();

    // Apply limit if specified
    if let Some(lim) = limit {
        all_deps.truncate(lim);
    }

    status(&format!("{} reverse deps", all_deps.len()));

    Ok(all_deps)
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, Error> {
    let resp = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .call()?;
    let len = resp.header("Content-Length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let mut data: Vec<u8> = Vec::with_capacity(len);
    resp.into_reader().read_to_end(&mut data)?;
    Ok(data)
}

#[derive(Debug, Clone)]
struct RevDep {
    name: RevDepName,
    vers: Version
}

#[derive(Debug)]
struct TestResult {
    rev_dep: RevDep,
    data: TestResultData
}

#[derive(Debug)]
enum TestResultData {
    Passed(compile::FourStepResult),
    Regressed(compile::FourStepResult),
    Broken(compile::FourStepResult),
    Skipped(String), // Skipped with reason (e.g., version incompatibility)
    Error(Error),
}

impl TestResult {
    fn from_four_step(rev_dep: RevDep, result: compile::FourStepResult) -> TestResult {
        let data = if result.is_broken() {
            TestResultData::Broken(result)
        } else if result.is_regressed() {
            TestResultData::Regressed(result)
        } else {
            TestResultData::Passed(result)
        };

        TestResult { rev_dep, data }
    }

    // Legacy constructors for backwards compatibility during migration
    fn passed(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        // Convert old-style to new FourStepResult
        let four_step = compile::FourStepResult {
            baseline_check: r1.clone(),
            baseline_test: Some(r1),
            override_check: Some(r2.clone()),
            override_test: Some(r2),
        };
        TestResult {
            rev_dep,
            data: TestResultData::Passed(four_step)
        }
    }

    fn regressed(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        let four_step = compile::FourStepResult {
            baseline_check: r1.clone(),
            baseline_test: Some(r1),
            override_check: Some(r2.clone()),
            override_test: Some(r2),
        };
        TestResult {
            rev_dep,
            data: TestResultData::Regressed(four_step)
        }
    }

    fn broken(rev_dep: RevDep, r: CompileResult) -> TestResult {
        let four_step = compile::FourStepResult {
            baseline_check: r,
            baseline_test: None,
            override_check: None,
            override_test: None,
        };
        TestResult {
            rev_dep,
            data: TestResultData::Broken(four_step)
        }
    }

    fn skipped(rev_dep: RevDep, reason: String) -> TestResult {
        TestResult {
            rev_dep,
            data: TestResultData::Skipped(reason)
        }
    }

    fn error(rev_dep: RevDep, e: Error) -> TestResult {
        TestResult {
            rev_dep,
            data: TestResultData::Error(e)
        }
    }

    fn quick_str(&self) -> &'static str {
        match self.data {
            TestResultData::Passed(..) => "passed",
            TestResultData::Regressed(..) => "regressed",
            TestResultData::Broken(_) => "broken",
            TestResultData::Skipped(_) => "skipped",
            TestResultData::Error(_) => "error"
        }
    }

    fn html_class(&self) -> &'static str {
        self.quick_str()
    }

    fn html_anchor(&self) -> String {
        sanitize_link(&format!("{}-{}", self.rev_dep.name, self.rev_dep.vers))
    }
}

fn sanitize_link(s: &str) -> String {
    s.chars().map(|c| {
        let c = c.to_lowercase().collect::<Vec<_>>()[0];
        if c != '-' && (c < 'a' || c > 'z')
            && (c < '0' || c > '9') {
            '_'
        } else {
            c
        }
    }).collect()
}

struct TestResultReceiver {
    rev_dep: RevDepName,
    rx: Receiver<TestResult>
}

impl TestResultReceiver {
    fn recv(self) -> TestResult {
        match self.rx.recv() {
            Ok(r) => r,
            Err(e) => {
                let r = RevDep {
                    name: self.rev_dep,
                    vers: Version::parse("0.0.0").unwrap()
                };
                TestResult::error(r, Error::from(e))
            }
        }
    }
}

fn new_result_receiver(rev_dep: RevDepName) -> (Sender<TestResult>, TestResultReceiver) {
    let (tx, rx) = mpsc::channel();

    let fut = TestResultReceiver {
        rev_dep: rev_dep,
        rx: rx
    };

    (tx, fut)
}

fn run_test(pool: &mut ThreadPool,
            config: Config,
            rev_dep: RevDepName) -> TestResultReceiver {
    let (result_tx, result_rx) = new_result_receiver(rev_dep.clone());
    pool.execute(move || {
        let res = run_test_local(&config, rev_dep);
        result_tx.send(res).unwrap();
    });

    return result_rx;
}

fn run_test_local(config: &Config, rev_dep: RevDepName) -> TestResult {

    status(&format!("testing crate {}", rev_dep));

    // First, figure get the most recent version number
    let rev_dep = match resolve_rev_dep_version(rev_dep.clone()) {
        Ok(r) => r,
        Err(e) => {
            let rev_dep = RevDep {
                name: rev_dep,
                vers: Version::parse("0.0.0").unwrap()
            };
            return TestResult::error(rev_dep, e);
        }
    };

    // Check if the dependent's version requirement is compatible with our WIP version
    match check_version_compatibility(&rev_dep, &config) {
        Ok(true) => {
            // Compatible, continue testing
        }
        Ok(false) => {
            // Incompatible, skip testing
            let reason = format!(
                "Dependent requires version incompatible with {} v{}",
                config.crate_name, config.version
            );
            return TestResult::skipped(rev_dep, reason);
        }
        Err(e) => {
            debug!("Failed to check version compatibility: {}, testing anyway", e);
            // Continue testing if we can't determine compatibility
        }
    }

    let base_result = match compile_with_custom_dep(&rev_dep, &config.base_override) {
        Ok(r) => r,
        Err(e) => return TestResult::error(rev_dep, e)
    };

    if base_result.failed() {
        return TestResult::broken(rev_dep, base_result);
    }
    let next_result = match compile_with_custom_dep(&rev_dep, &config.next_override) {
        Ok(r) => r,
        Err(e) => return TestResult::error(rev_dep, e)
    };

    if next_result.failed() {
        TestResult::regressed(rev_dep, base_result, next_result)
    } else {
        TestResult::passed(rev_dep, base_result, next_result)
    }
}

fn check_version_compatibility(rev_dep: &RevDep, config: &Config) -> Result<bool, Error> {
    debug!("checking version compatibility for {} {}", rev_dep.name, rev_dep.vers);

    // Download and cache the dependent's .crate file
    let crate_handle = get_crate_handle(rev_dep)?;

    // Create temp directory to extract Cargo.toml
    let temp_dir = TempDir::new()?;
    let extract_dir = temp_dir.path().join("extracted");
    fs::create_dir(&extract_dir)?;

    // Extract just the Cargo.toml
    let mut cmd = Command::new("tar");
    let cmd = cmd
        .arg("xzf")
        .arg(&crate_handle.0)
        .arg("--strip-components=1")
        .arg("-C")
        .arg(&extract_dir)
        .arg("--wildcards")
        .arg("*/Cargo.toml");

    let output = cmd.output()?;
    if !output.status.success() {
        return Err(Error::ProcessError("Failed to extract Cargo.toml".to_string()));
    }

    // Read and parse Cargo.toml
    let toml_path = extract_dir.join("Cargo.toml");
    let toml_str = load_string(&toml_path)?;
    let value: toml::Value = toml::from_str(&toml_str)?;

    // Look for our crate in dependencies
    let our_crate = &config.crate_name;
    let wip_version = Version::parse(&config.version)?;

    // Check [dependencies]
    if let Some(deps) = value.get("dependencies").and_then(|v| v.as_table()) {
        if let Some(req) = deps.get(our_crate) {
            return check_requirement(req, &wip_version);
        }
    }

    // Check [dev-dependencies]
    if let Some(deps) = value.get("dev-dependencies").and_then(|v| v.as_table()) {
        if let Some(req) = deps.get(our_crate) {
            return check_requirement(req, &wip_version);
        }
    }

    // Check [build-dependencies]
    if let Some(deps) = value.get("build-dependencies").and_then(|v| v.as_table()) {
        if let Some(req) = deps.get(our_crate) {
            return check_requirement(req, &wip_version);
        }
    }

    // Crate not found in dependencies (shouldn't happen for reverse deps)
    debug!("Warning: {} not found in {}'s dependencies", our_crate, rev_dep.name);
    Ok(true) // Test anyway
}

fn check_requirement(req: &toml::Value, wip_version: &Version) -> Result<bool, Error> {
    use semver::VersionReq;

    let req_str = match req {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => {
            // Handle { version = "1.0", features = [...] } format
            t.get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("*")
                .to_string()
        }
        _ => "*".to_string(),
    };

    debug!("Checking if version {} satisfies requirement '{}'", wip_version, req_str);

    let version_req = VersionReq::parse(&req_str)
        .map_err(|e| Error::SemverError(e))?;

    Ok(version_req.matches(wip_version))
}

fn resolve_rev_dep_version(name: RevDepName) -> Result<RevDep, Error> {
    debug!("resolving current version for {}", name);

    let krate = CRATES_IO_CLIENT.get_crate(&name)
        .map_err(|e| Error::CratesIoApiError(e.to_string()))?;

    // Pull out the version numbers and sort them
    let versions = krate.versions.iter()
        .filter_map(|r| Version::parse(&r.num).ok());
    let mut versions = versions.collect::<Vec<_>>();
    versions.sort();

    versions.pop().map(|v| {
        RevDep {
            name: name,
            vers: v
        }
    }).ok_or(Error::NoCrateVersions)
}


// CompileResult is now in compile module
type CompileResult = compile::CompileResult;

fn compile_with_custom_dep(rev_dep: &RevDep, krate: &CrateOverride) -> Result<CompileResult, Error> {
    let ref crate_handle = get_crate_handle(rev_dep)?;
    let temp_dir = TempDir::new()?;
    let ref source_dir = temp_dir.path().join("source");
    (fs::create_dir(source_dir)?);
    (crate_handle.unpack_source_to(source_dir)?);

    match *krate {
        CrateOverride::Default => (),
        CrateOverride::Source(ref path) => {
            // Emit a .cargo/config file to override the project's
            // dependency on *our* project with the WIP.
            (emit_cargo_override_path(source_dir, path)?);
        }
    }

    // NB: The way cargo searches for .cargo/config, which we use to
    // override dependencies, depends on the CWD, and is not affacted
    // by the --manifest-path flag, so this is changing directories.
    let start = std::time::Instant::now();
    let mut cmd = Command::new("cargo");
    let cmd = cmd.arg("build")
        .current_dir(source_dir);
    debug!("running cargo: {:?}", cmd);
    let r = cmd.output()?;

    let duration = start.elapsed();
    let success = r.status.success();

    debug!("result: {:?}", success);

    Ok(CompileResult {
        step: compile::CompileStep::Check, // Legacy: using Check for old build command
        success,
        stdout: (String::from_utf8(r.stdout)?),
        stderr: (String::from_utf8(r.stderr)?),
        duration,
        diagnostics: Vec::new(), // Legacy path doesn't use JSON parsing
    })
}

struct CrateHandle(PathBuf);

fn get_crate_handle(rev_dep: &RevDep) -> Result<CrateHandle, Error> {
    let cache_path = Path::new("./.crusader/crate-cache");
    let ref crate_dir = cache_path.join(&rev_dep.name);
    (fs::create_dir_all(crate_dir)?);
    let crate_file = crate_dir.join(format!("{}-{}.crate", rev_dep.name, rev_dep.vers));
    // FIXME: Path::exists() is unstable so just opening the file
    let crate_file_exists = File::open(&crate_file).is_ok();
    if !crate_file_exists {
        let url = crate_url(&rev_dep.name,
                            Some(&format!("{}/download", rev_dep.vers)));
        let body = http_get_bytes(&url)?;
        // FIXME: Should move this into place atomically
        let mut file = File::create(&crate_file)?;
        (file.write_all(&body)?);
        (file.flush()?);
    }

    return Ok(CrateHandle(crate_file));
}

impl CrateHandle {
    fn unpack_source_to(&self, path: &Path) -> Result<(), Error> {
        debug!("unpackng {:?} to {:?}", self.0, path);
        let mut cmd = Command::new("tar");
        let cmd = cmd
            .arg("xzf")
            .arg(self.0.to_str().unwrap().to_owned())
            .arg("--strip-components=1")
            .arg("-C")
            .arg(path.to_str().unwrap().to_owned());
        let r = cmd.output()?;
        if r.status.success() {
            Ok(())
        } else {
            // FIXME: Want to put r in this value but
            // process::Output doesn't implement Debug
            let s = String::from_utf8_lossy(&r.stderr).into_owned();
            Err(Error::ProcessError(s))
        }
    }
}

fn emit_cargo_override_path(source_dir: &Path, override_path: &Path) -> Result<(), Error> {
    debug!("overriding cargo path in {:?} with {:?}", source_dir, override_path);

    assert!(override_path.ends_with("Cargo.toml"));
    let override_path = override_path.parent().unwrap();

    // Since cargo is going to be run with --manifest-path to change
    // directories a relative path is not going to make sense.
    let override_path = if override_path.is_absolute() {
        override_path.to_path_buf()
    } else {
        (env::current_dir()?).join(override_path)
    };
    let ref cargo_dir = source_dir.join(".cargo");
    (fs::create_dir_all(cargo_dir)?);
    let ref config_path = cargo_dir.join("config");
    let mut file = File::create(config_path)?;
    let s = format!(r#"paths = ["{}"]"#, override_path.to_str().unwrap());
    file.write_all(s.as_bytes())?;
    file.flush()?;

    Ok(())
}

fn status_lock<F>(f: F) where F: FnOnce() -> () {
   lazy_static! {
        static ref LOCK: Mutex<()> = Mutex::new(());
    }
    let _guard = LOCK.lock();
    f();
}

fn print_status_header() {
    print!("crusader: ");
}

fn print_color(s: &str, fg: term::color::Color) {
    if !really_print_color(s, fg) {
        print!("{}", s);
    }

    fn really_print_color(s: &str,
                          fg: term::color::Color) -> bool {
        if let Some(ref mut t) = term::stdout() {
            if t.fg(fg).is_err() { return false }
            let _ = t.attr(term::Attr::Bold);
            if write!(t, "{}", s).is_err() { return false }
            let _ = t.reset();
        }

        true
    }
}

fn status(s: &str) {
    status_lock(|| {
        print_status_header();
        println!("{}", s);
    });
}

fn report_quick_result(current_num: usize, total: usize, result: &TestResult) {
    status_lock(|| {
        print_status_header();
        print!("result {} of {}, {} {}: ",
               current_num,
               total,
               result.rev_dep.name,
               result.rev_dep.vers
               );
        let color = match result.data {
            TestResultData::Passed(..) => term::color::BRIGHT_GREEN,
            TestResultData::Regressed(..) => term::color::BRIGHT_RED,
            TestResultData::Broken(_) => term::color::BRIGHT_YELLOW,
            TestResultData::Skipped(_) => term::color::BRIGHT_CYAN,
            TestResultData::Error(_) => term::color::BRIGHT_MAGENTA,
        };
        print_color(&format!("{}", result.quick_str()), color);
        println!("");

        // Print detailed error output immediately for failures
        if matches!(result.data, TestResultData::Regressed(_) | TestResultData::Broken(_) | TestResultData::Error(_)) {
            report::print_immediate_failure(result);
        }
    });
}

fn report_results(res: Result<Vec<TestResult>, Error>, args: &cli::CliArgs, config: &Config) {
    match res {
        Ok(results) => {
            // Print console table
            report::print_console_table(&results, &config.crate_name, &config.version);

            // Generate markdown analysis report
            let markdown_path = args.output.with_extension("").with_extension("md")
                .file_name()
                .and_then(|f| f.to_str())
                .map(|f| f.replace(".html", "-analysis"))
                .map(|f| PathBuf::from(format!("{}.md", f)))
                .unwrap_or_else(|| PathBuf::from("crusader-analysis.md"));

            match report::export_markdown_report(&results, &markdown_path, &config.crate_name, &config.version) {
                Ok(_) => {
                    println!("Markdown report: {}", markdown_path.display());
                }
                Err(e) => {
                    eprintln!("Warning: Failed to generate markdown report: {}", e);
                }
            }

            // Generate HTML report
            match report::export_html_report(results, &args.output) {
                Ok(summary) => {
                    println!("HTML report: {}", args.output.display());
                    println!();

                    // Exit with error if there were regressions
                    if summary.regressed > 0 {
                        std::process::exit(-2);
                    }
                }
                Err(e) => {
                    report_error(e);
                }
            }
        }
        Err(e) => {
            report_error(e);
        }
    }
}

fn report_error(e: Error) {
    println!("");
    print_color("error", term::color::BRIGHT_RED);
    println!(": {}", e);
    println!("");

    std::process::exit(-1);
}

// Report generation functions moved to src/report.rs

#[derive(Debug)]
enum Error {
    ManifestName,
    SemverError(semver::Error),
    TomlError(toml::de::Error),
    IoError(io::Error),
    UreqError(Box<ureq::Error>),
    CratesIoApiError(String),
    RecvError(RecvError),
    NoCrateVersions,
    FromUtf8Error(FromUtf8Error),
    ProcessError(String),
    InvalidPath(PathBuf),
}

macro_rules! convert_error {
    ($from:ty, $to:ident) => (
        impl From<$from> for Error {
            fn from(e: $from) -> Error {
                Error::$to(e)
            }
        }
    )
}

convert_error!(semver::Error, SemverError);
convert_error!(io::Error, IoError);
convert_error!(toml::de::Error, TomlError);
convert_error!(RecvError, RecvError);
convert_error!(FromUtf8Error, FromUtf8Error);

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Error {
        Error::UreqError(Box::new(e))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match *self {
            Error::ManifestName => write!(f, "error extracting crate name from manifest"),
            Error::SemverError(ref e) => write!(f, "semver error: {}", e),
            Error::TomlError(ref e) => write!(f, "TOML parse error: {}", e),
            Error::IoError(ref e) => write!(f, "IO error: {}", e),
            Error::UreqError(ref e) => write!(f, "HTTP error: {}", e),
            Error::CratesIoApiError(ref e) => write!(f, "crates.io API error: {}", e),
            Error::RecvError(ref e) => write!(f, "receive error: {}", e),
            Error::NoCrateVersions => write!(f, "crate has no published versions"),
            Error::FromUtf8Error(ref e) => write!(f, "UTF-8 conversion error: {}", e),
            Error::ProcessError(ref s) => write!(f, "process error: {}", s),
            Error::InvalidPath(ref p) => write!(f, "invalid path: {}", p.display())
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match *self {
            Error::SemverError(ref e) => Some(e),
            Error::TomlError(ref e) => Some(e),
            Error::IoError(ref e) => Some(e),
            Error::UreqError(ref e) => Some(e.as_ref()),
            Error::RecvError(ref e) => Some(e),
            Error::FromUtf8Error(ref e) => Some(e),
            _ => None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn test_check_requirement_string_exact_version() {
        let req = toml::Value::String("0.2.0".to_string());
        let version = Version::parse("0.2.0").unwrap();

        assert!(check_requirement(&req, &version).unwrap());
    }

    #[test]
    fn test_check_requirement_string_caret() {
        let req = toml::Value::String("^0.1.0".to_string());
        let version_compatible = Version::parse("0.1.5").unwrap();
        let version_incompatible = Version::parse("0.2.0").unwrap();

        assert!(check_requirement(&req, &version_compatible).unwrap());
        assert!(!check_requirement(&req, &version_incompatible).unwrap());
    }

    #[test]
    fn test_check_requirement_string_tilde() {
        let req = toml::Value::String("~0.1.0".to_string());
        let version_compatible = Version::parse("0.1.9").unwrap();
        let version_incompatible = Version::parse("0.2.0").unwrap();

        assert!(check_requirement(&req, &version_compatible).unwrap());
        assert!(!check_requirement(&req, &version_incompatible).unwrap());
    }

    #[test]
    fn test_check_requirement_wildcard() {
        let req = toml::Value::String("*".to_string());
        let version = Version::parse("999.999.999").unwrap();

        assert!(check_requirement(&req, &version).unwrap());
    }

    #[test]
    fn test_check_requirement_table_with_version() {
        use toml::map::Map;

        let mut table = Map::new();
        table.insert("version".to_string(), toml::Value::String("^0.1.0".to_string()));
        table.insert("features".to_string(), toml::Value::Array(vec![]));
        let req = toml::Value::Table(table);

        let version_compatible = Version::parse("0.1.5").unwrap();
        let version_incompatible = Version::parse("0.2.0").unwrap();

        assert!(check_requirement(&req, &version_compatible).unwrap());
        assert!(!check_requirement(&req, &version_incompatible).unwrap());
    }

    #[test]
    fn test_check_requirement_table_without_version() {
        use toml::map::Map;

        let mut table = Map::new();
        table.insert("path".to_string(), toml::Value::String("../local".to_string()));
        let req = toml::Value::Table(table);

        // Table without version field should default to "*" (wildcard)
        let version = Version::parse("999.999.999").unwrap();
        assert!(check_requirement(&req, &version).unwrap());
    }

    #[test]
    fn test_check_requirement_gte_operator() {
        let req = toml::Value::String(">=0.1.0".to_string());
        let version_compatible = Version::parse("0.2.0").unwrap();
        let version_incompatible = Version::parse("0.0.9").unwrap();

        assert!(check_requirement(&req, &version_compatible).unwrap());
        assert!(!check_requirement(&req, &version_incompatible).unwrap());
    }

    #[test]
    fn test_check_requirement_complex_range() {
        let req = toml::Value::String(">=0.1.0, <0.3.0".to_string());
        let version_compatible1 = Version::parse("0.1.5").unwrap();
        let version_compatible2 = Version::parse("0.2.9").unwrap();
        let version_incompatible = Version::parse("0.3.0").unwrap();

        assert!(check_requirement(&req, &version_compatible1).unwrap());
        assert!(check_requirement(&req, &version_compatible2).unwrap());
        assert!(!check_requirement(&req, &version_incompatible).unwrap());
    }
}
