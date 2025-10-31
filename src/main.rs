// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

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
    report_results(run());
}

fn run() -> Result<Vec<TestResult>, Error> {
    let config = get_config()?;

    // Find all the crates on crates.io the depend on ours
    let rev_deps = get_rev_deps(&config.crate_name, config.limit)?;

    // Run all the tests in a thread pool and create a list of result
    // receivers.
    let mut result_rxs = Vec::new();
    let ref mut pool = ThreadPool::new(1);
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

    let source_name = get_crate_name(&manifest)?;
    Ok(Config {
        crate_name: source_name,
        base_override: CrateOverride::Default,
        next_override: CrateOverride::Source(manifest),
        limit,
    })
}

fn get_crate_name(manifest_path: &Path) -> Result<String, Error> {
    let toml_str = load_string(manifest_path)?;
    let value: toml::Value = toml::from_str(&toml_str)?;

    match value.get("package") {
        Some(toml::Value::Table(t)) => {
            match t.get("name") {
                Some(toml::Value::String(s)) => {
                    Ok(s.clone())
                }
                _ => {
                    Err(Error::ManifestName)
                }
            }
        }
        _ => {
            Err(Error::ManifestName)
        }
    }
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
    Passed(CompileResult, CompileResult),
    Regressed(CompileResult, CompileResult),
    Broken(CompileResult),
    Error(Error),
}

impl TestResult {
    fn passed(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Passed(r1, r2)
        }
    }

    fn regressed(rev_dep: RevDep, r1: CompileResult, r2: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Regressed(r1, r2)
        }
    }

    fn broken(rev_dep: RevDep, r: CompileResult) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Broken(r)
        }
    }

    fn error(rev_dep: RevDep, e: Error) -> TestResult {
        TestResult {
            rev_dep: rev_dep,
            data: TestResultData::Error(e)
        }
    }
    
    fn quick_str(&self) -> &'static str {
        match self.data {
            TestResultData::Passed(..) => "passed",
            TestResultData::Regressed(..) => "regressed",
            TestResultData::Broken(_) => "broken",
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

    // TODO: Decide whether the version of our crate requested by the
    // rev dep is semver-compatible with the in-development version.
    
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


#[derive(Debug, Clone)]
struct CompileResult {
    stdout: String,
    stderr: String,
    success: bool
}

impl CompileResult {
    fn failed(&self) -> bool {
        !self.success
    }
}

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
    let mut cmd = Command::new("cargo");
    let cmd = cmd.arg("build")
        .current_dir(source_dir);
    debug!("running cargo: {:?}", cmd);
    let r = cmd.output()?;

    let success = r.status.success();

    debug!("result: {:?}", success);

    Ok(CompileResult {
        stdout: (String::from_utf8(r.stdout)?),
        stderr: (String::from_utf8(r.stderr)?),
        success: success
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
        print_color(&format!("{}", result.quick_str()), result.into());
        println!("");
    });
}

fn report_results(res: Result<Vec<TestResult>, Error>) {
    match res {
        Ok(r) => {
            match export_report(r) {
                Ok((summary, report_path)) => report_success(summary, report_path),
                Err(e) => {
                    report_error(e)
                }
            }
        }
        Err(e) => {
            report_error(e)
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

fn export_report(mut results: Vec<TestResult>) -> Result<(Summary, PathBuf), Error> {
    let path = PathBuf::from("./crusader-report.html");
    let s = summarize_results(&results);

    results.sort_by(|a, b| {
        a.rev_dep.name.cmp(&b.rev_dep.name)
    });

    let ref mut file = File::create(&path)?;
    (writeln!(file, "<!DOCTYPE html>")?);

    (writeln!(file, "<head>")?);
    writeln!(file, "{}", r"

<style>
.passed { color: green; }
.regressed { color: red; }
.broken { color: yellow; }
.error { color: magenta; }
.stdout, .stderr, .test-exception-output { white-space: pre; }
</style>

")?;
    (writeln!(file, "</head>")?);

    (writeln!(file, "<body>")?);

    // Print the summary table
    (writeln!(file, "<h1>Summary</h1>")?);
    (writeln!(file, "<table>")?);
    (writeln!(file, "<tr><th>Crate</th><th>Version</th><th>Result</th></tr>")?);
    for result in &results {
        (writeln!(file, "<tr>")?);
        (writeln!(file, "<td>")?);
        (writeln!(file, "<a href='#{}'>", result.html_anchor()))?;
        (writeln!(file, "{}", result.rev_dep.name))?;
        (writeln!(file, "</a>"))?;
        (writeln!(file, "</td>"))?;
        (writeln!(file, "<td>{}</td>", result.rev_dep.vers))?;
        (writeln!(file, "<td class='{}'>{}</td>", result.html_class(), result.quick_str()))?;
        (writeln!(file, "</tr>")?);
    }
    (writeln!(file, "</table>")?);

    (writeln!(file, "<h1>Details</h1>")?);
    for result in results {
        (writeln!(file, "<div class='complete-result'>")?);
        (writeln!(file, "<a name='{}'></a>", result.html_anchor()))?;
        (writeln!(file, "<h2>"))?;
        (writeln!(file, "<span>{} {}</span>", result.rev_dep.name, result.rev_dep.vers))?;
        (writeln!(file, "<span class='{}'>{}</span>", result.html_class(), result.quick_str()))?;
        (writeln!(file, "</h2>")?);
        match result.data {
            TestResultData::Passed(r1, r2) |
            TestResultData::Regressed(r1, r2) => {
                (export_compile_result(file, "before", r1)?);
                (export_compile_result(file, "after", r2)?);
            }
            TestResultData::Broken(r) => {
                (export_compile_result(file, "before", r)?);
            }
            TestResultData::Error(e) => {
                (export_error(file, e)?);
            }
        }
        (writeln!(file, "</div>")?);
    }
    
    (writeln!(file, "</body>")?);

    Ok((s, path))
}

fn export_compile_result(file: &mut File, label: &str, r: CompileResult) -> Result<(), Error> {
    let stdout = sanitize(&r.stdout);
    let stderr = sanitize(&r.stderr);
    (writeln!(file, "<h3>{}</h3>", label)?);
    (writeln!(file, "<div class='stdout'>\n{}\n</div>", stdout)?);
    (writeln!(file, "<div class='stderr'>\n{}\n</div>", stderr)?);

    Ok(())
}

fn export_error(file: &mut File, e: Error) -> Result<(), Error> {
    let err = sanitize(&format!("{}", e));
    (writeln!(file, "<h3>{}</h3>", "errors")?);
    (writeln!(file, "<div class='test-exception-output'>\n{}\n</div>", err)?);

    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars().flat_map(|c| {
        match c {
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '&' => "&amp;".chars().collect(),
            _ => vec![c]
        }
    }).collect()
}

enum ReportResult { Passed, Regressed, Broken, Error }

impl Into<term::color::Color> for ReportResult {
    fn into(self) -> term::color::Color {
        match self {
            ReportResult::Passed => term::color::BRIGHT_GREEN,
            ReportResult::Regressed => term::color::BRIGHT_RED,
            ReportResult::Broken => term::color::BRIGHT_YELLOW,
            ReportResult::Error => term::color::BRIGHT_MAGENTA,
        }
    }
}

impl<'a> Into<term::color::Color> for &'a TestResult {
    fn into(self) -> term::color::Color {
        match self.data {
            TestResultData::Passed(..) => ReportResult::Passed,
            TestResultData::Regressed(..) => ReportResult::Regressed,
            TestResultData::Broken(_) => ReportResult::Broken,
            TestResultData::Error(_) => ReportResult::Error,
        }.into()
    }
}

fn report_success(s: Summary, p: PathBuf) {
    println!("");
    print!("passed: ");
    print_color(&format!("{}", s.passed), ReportResult::Passed.into());
    println!("");
    print!("regressed: ");
    print_color(&format!("{}", s.regressed), ReportResult::Regressed.into());
    println!("");
    print!("broken: ");
    print_color(&format!("{}", s.broken), ReportResult::Broken.into());
    println!("");
    print!("error: ");
    print_color(&format!("{}", s.error), ReportResult::Error.into());
    println!("");

    println!("");
    println!("full report: {}", p.to_str().unwrap());
    println!("");
    
    if s.regressed > 0 { std::process::exit(-2) }
}

#[derive(Default)]
struct Summary {
    broken: usize,
    regressed: usize,
    passed: usize,
    error: usize
}

fn summarize_results(results: &[TestResult]) -> Summary {
    let mut sum = Summary::default();

    for result in results {
        match result.data {
            TestResultData::Broken(..) => sum.broken += 1,
            TestResultData::Regressed(..) => sum.regressed += 1,
            TestResultData::Passed(..) => sum.passed += 1,
            TestResultData::Error(..) => sum.error += 1,
        }
    }

    return sum;
}

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
    ProcessError(String)
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
            Error::ProcessError(ref s) => write!(f, "process error: {}", s)
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
