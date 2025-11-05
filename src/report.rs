/// Report generation module
///
/// Provides both HTML and console table output for test results

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use crate::{TestResult, TestResultData, CompileResult, Error, VersionTestOutcome, VersionStatus};
use crate::compile::{FourStepResult, VersionSource, ThreeStepResult};
use term::color::Color;

/// Status of a version test for the Offered column
#[derive(Debug, Clone, PartialEq)]
pub enum TestStatus {
    Passed,     // ✓
    Regressed,  // ✗
    Broken,     // ⚠
}

/// Natural resolution outcome (when respecting semver)
#[derive(Debug, Clone, PartialEq)]
pub enum NaturalResolution {
    Exact,      // = (cargo resolved to exact offered version)
    Upgraded,   // ↑ (cargo upgraded within semver range)
}

/// Data for rendering a single row in the table
#[derive(Debug, Clone)]
pub struct TableRow {
    pub offered: OfferedCell,
    pub spec: String,
    pub resolved: String,
    pub dependent: String,
    pub result: String,
    pub time: String,
    pub color: Color,
    pub error_details: Vec<String>,  // Additional error lines
    pub multi_version_rows: Vec<MultiVersionRow>,  // Additional version rows
}

/// Content of the "Offered" cell - encodes valid combinations only
#[derive(Debug, Clone)]
pub enum OfferedCell {
    /// Baseline test: "-" only
    Baseline,

    /// Skipped test with no version info: "⊘" only (for errors/whole-dependent skips)
    SkippedNoVersion,

    /// Skipped test with version: "⊘ ↑version" or "⊘ ≠version" (cargo resolved to different version)
    SkippedWithVersion {
        resolution: NaturalResolution,  // ↑ or ≠
        version: String,                // "0.8.48" etc
    },

    /// Natural resolution (respects semver): "✓ =version" or "✓ ↑version"
    Natural {
        status: TestStatus,             // ✓/✗/⚠
        resolution: NaturalResolution,  // = or ↑
        version: String,                // "this(0.8.91)" or "0.8.51"
    },

    /// Forced resolution (bypasses semver): "✓ ≠version [≠→!]"
    /// Resolution is always Mismatch (≠), displayed with [≠→!] suffix
    Forced {
        status: TestStatus,  // ✓/✗/⚠
        version: String,     // "this(0.8.91)" or "0.8.51"
    },
}

/// Additional row for multi-version resolution
#[derive(Debug, Clone)]
pub struct MultiVersionRow {
    pub spec: String,
    pub resolved: String,
    pub dependent: String,
}

impl OfferedCell {
    /// Format the offered cell content
    pub fn format(&self) -> String {
        match self {
            OfferedCell::Baseline => "-".to_string(),
            OfferedCell::SkippedNoVersion => "⊘".to_string(),
            OfferedCell::SkippedWithVersion { resolution, version } => {
                let resolution_symbol = match resolution {
                    NaturalResolution::Exact => "=",
                    NaturalResolution::Upgraded => "↑",
                };
                format!("⊘ {}{}", resolution_symbol, version)
            }
            OfferedCell::Natural { status, resolution, version } => {
                let status_icon = match status {
                    TestStatus::Passed => "✓",
                    TestStatus::Regressed => "✗",
                    TestStatus::Broken => "⚠",
                };

                let resolution_symbol = match resolution {
                    NaturalResolution::Exact => "=",
                    NaturalResolution::Upgraded => "↑",
                };

                format!("{} {}{}", status_icon, resolution_symbol, version)
            }
            OfferedCell::Forced { status, version } => {
                let status_icon = match status {
                    TestStatus::Passed => "✓",
                    TestStatus::Regressed => "✗",
                    TestStatus::Broken => "⚠",
                };

                // Forced always shows ≠ with [≠→!] suffix
                format!("{} ≠{} [≠→!]", status_icon, version)
            }
        }
    }
}

#[derive(Default, Debug)]
pub struct Summary {
    pub broken: usize,
    pub regressed: usize,
    pub passed: usize,
    pub skipped: usize,
    pub error: usize,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.broken + self.regressed + self.passed + self.skipped + self.error
    }
}

/// Summarize test results into counts
pub fn summarize_results(results: &[TestResult]) -> Summary {
    let mut sum = Summary::default();

    for result in results {
        match &result.data {
            TestResultData::Broken(..) => sum.broken += 1,
            TestResultData::Regressed(..) => sum.regressed += 1,
            TestResultData::Passed(..) => sum.passed += 1,
            TestResultData::Skipped(_) => sum.skipped += 1,
            TestResultData::Error(..) => sum.error += 1,
            TestResultData::MultiVersion(ref outcomes) => {
                // Count based on worst status across all versions
                // Baseline is the first version
                let baseline = outcomes.first();

                let mut has_regressed = false;
                let mut has_broken = false;
                let mut all_passed = true;

                for (idx, outcome) in outcomes.iter().enumerate() {
                    let is_baseline = idx == 0;
                    let status = if is_baseline {
                        if outcome.result.is_success() {
                            VersionStatus::Passed
                        } else {
                            VersionStatus::Broken
                        }
                    } else {
                        classify_version_outcome(outcome, baseline)
                    };

                    match status {
                        VersionStatus::Regressed => {
                            has_regressed = true;
                            all_passed = false;
                        }
                        VersionStatus::Broken => {
                            has_broken = true;
                            all_passed = false;
                        }
                        VersionStatus::Passed => {}
                    }
                }

                // Classify based on worst outcome
                if has_regressed {
                    sum.regressed += 1;
                } else if has_broken {
                    sum.broken += 1;
                } else if all_passed {
                    sum.passed += 1;
                }
            }
        }
    }

    sum
}

/// Print immediate failure details when a test fails
pub fn print_immediate_failure(result: &TestResult) {
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Print header with colored status
    if let Some(ref mut t) = term::stdout() {
        let _ = t.fg(term::color::BRIGHT_RED);
        let _ = t.attr(term::Attr::Bold);
        let _ = write!(t, "FAILURE: ");
        let _ = t.reset();
        let _ = writeln!(t, "{} {}", result.rev_dep.name, result.rev_dep.vers);
    } else {
        println!("FAILURE: {} {}", result.rev_dep.name, result.rev_dep.vers);
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    match &result.data {
        TestResultData::Regressed(four_step) => {
            println!("\nStatus: REGRESSED (compiled with baseline, failed with WIP version)\n");

            // Show which step failed
            if let Some(ref check) = four_step.override_check {
                if check.failed() {
                    print_step_failure("Override Check", check);
                }
            }
            if let Some(ref test) = four_step.override_test {
                if test.failed() {
                    print_step_failure("Override Test", test);
                }
            }
        }
        TestResultData::Broken(four_step) => {
            println!("\nStatus: BROKEN (already fails with published baseline version)\n");

            if four_step.baseline_check.failed() {
                print_step_failure("Baseline Check", &four_step.baseline_check);
            }
            if let Some(ref test) = four_step.baseline_test {
                if test.failed() {
                    print_step_failure("Baseline Test", test);
                }
            }
        }
        TestResultData::Error(e) => {
            println!("\nStatus: ERROR (internal crusader error)\n");
            println!("{}", e);
        }
        _ => {}
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();
}

/// Print details of a failed compilation step
fn print_step_failure(step_name: &str, result: &CompileResult) {
    println!("▶ {} failed after {:.1}s", step_name, result.duration.as_secs_f64());
    println!();

    // If we have parsed diagnostics, show them
    if !result.diagnostics.is_empty() {
        let error_count = result.diagnostics.iter()
            .filter(|d| d.level.is_error())
            .count();

        if error_count > 0 {
            println!("Compilation errors ({} total):", error_count);
            println!();

            // Print each error's rendered output
            for diag in result.diagnostics.iter().filter(|d| d.level.is_error()) {
                println!("{}", diag.rendered.trim());
                println!();
            }
        }
    } else {
        // Fallback: show stderr if no diagnostics available
        if !result.stderr.is_empty() {
            println!("Error output:");
            println!();

            // Show last 50 lines or all if less
            let lines: Vec<&str> = result.stderr.lines().collect();
            let start = if lines.len() > 50 {
                println!("... (showing last 50 lines of output) ...");
                println!();
                lines.len() - 50
            } else {
                0
            };

            for line in &lines[start..] {
                // Highlight error lines
                if line.starts_with("error") || line.contains("error[E") {
                    if let Some(ref mut t) = term::stdout() {
                        let _ = t.fg(term::color::BRIGHT_RED);
                        let _ = write!(t, "{}", line);
                        let _ = t.reset();
                        println!();
                    } else {
                        println!("{}", line);
                    }
                } else {
                    println!("{}", line);
                }
            }
        }
    }
}

/// Get status label and color for a test result
fn get_status_info(data: &TestResultData) -> (&'static str, Color) {
    match data {
        TestResultData::Passed(..) => ("✓ PASSED", term::color::BRIGHT_GREEN),
        TestResultData::Regressed(..) => ("✗ REGRESSED", term::color::BRIGHT_RED),
        TestResultData::Broken(..) => ("⚠ BROKEN", term::color::BRIGHT_YELLOW),
        TestResultData::Skipped(..) => ("⊘ SKIPPED", term::color::BRIGHT_CYAN),
        TestResultData::Error(..) => ("⚡ ERROR", term::color::BRIGHT_MAGENTA),
        TestResultData::MultiVersion(_) => ("✓ MULTI-VERSION", term::color::BRIGHT_GREEN), // TODO: Compute worst status
    }
}

/// Print a colored table row (legacy format)
#[allow(dead_code)]
fn print_colored_row(status: &str, name: &str, depends_on: &str, testing: &str,
                     duration: &str, color: Color) {
    // Print the row with coloring
    // Status (12) | Dependent (26) | Depends On (18) | Testing (16) | Duration (10)
    let row = format!("│{:^12}│{:<26}│{:<18}│{:<16}│{:>10}│",
                     status, name, depends_on, testing, duration);

    if let Some(ref mut t) = term::stdout() {
        let _ = t.fg(color);
        let _ = write!(t, "{}", row);
        let _ = t.reset();
        println!();
    } else {
        println!("{}", row);
    }
}

/// Print a colored table row with ICT column (Phase 5 format)
fn print_colored_row_ict(status: &str, name: &str, version: &str, ict: &str,
                         duration: &str, color: Color) {
    // Print the row with coloring
    // Status (12) | Dependent (26) | Version (14) | ICT (5) | Duration (10)
    let row = format!("│{:^12}│{:<26}│{:<14}│{:^5}│{:>10}│",
                     status, name, version, ict, duration);

    if let Some(ref mut t) = term::stdout() {
        let _ = t.fg(color);
        let _ = write!(t, "{}", row);
        let _ = t.reset();
        println!();
    } else {
        println!("{}", row);
    }
}

/// Classify a version outcome as PASSED, REGRESSED, or BROKEN
fn classify_version_outcome(outcome: &VersionTestOutcome, baseline: Option<&VersionTestOutcome>) -> VersionStatus {
    if outcome.result.is_success() {
        // All steps passed
        VersionStatus::Passed
    } else {
        // Failed - determine if REGRESSED or BROKEN
        if let Some(baseline) = baseline {
            if baseline.result.is_success() {
                // Baseline passed but this version failed → REGRESSED
                VersionStatus::Regressed
            } else {
                // Baseline also failed → BROKEN
                VersionStatus::Broken
            }
        } else {
            // No baseline to compare, or this IS the baseline → BROKEN
            VersionStatus::Broken
        }
    }
}

/// Build table rows from multi-version test results
fn build_table_rows(result: &TestResult, this_version: &str) -> Vec<TableRow> {
    let mut rows = Vec::new();

    let name_with_version = format!("{} {}", result.rev_dep.name, result.rev_dep.vers);

    match &result.data {
        TestResultData::MultiVersion(ref outcomes) => {
            // First outcome is always baseline
            let baseline = outcomes.first();

            for (idx, outcome) in outcomes.iter().enumerate() {
                let is_baseline = idx == 0;

                // Determine status
                let status = if is_baseline {
                    if outcome.result.is_success() {
                        VersionStatus::Passed
                    } else {
                        VersionStatus::Broken
                    }
                } else {
                    classify_version_outcome(outcome, baseline)
                };

                let offered_cell = if is_baseline {
                    OfferedCell::Baseline
                } else {
                    // Check if version was actually tested or skipped by cargo
                    let was_skipped = match (&outcome.result.expected_version, &outcome.result.actual_version) {
                        (Some(expected), Some(actual)) if expected != actual && !outcome.result.forced_version => {
                            // Cargo resolved to a different version than we offered (not forced)
                            true
                        }
                        _ => false
                    };

                    if was_skipped {
                        // Format version string
                        let version = format_version_with_this(&outcome.version_source, this_version);
                        // Determine resolution symbol (likely upgraded since cargo chose a different version)
                        let resolution = determine_natural_resolution(&outcome.result);
                        OfferedCell::SkippedWithVersion { resolution, version }
                    } else {
                        // Format version string
                        let version = format_version_with_this(&outcome.version_source, this_version);

                        let test_status = match status {
                            VersionStatus::Passed => TestStatus::Passed,
                            VersionStatus::Regressed => TestStatus::Regressed,
                            VersionStatus::Broken => TestStatus::Broken,
                        };

                        if outcome.result.forced_version {
                            // Forced version: always shows ≠ with [≠→!]
                            OfferedCell::Forced {
                                status: test_status,
                                version,
                            }
                        } else {
                            // Natural resolution: determine if exact or upgraded
                            let resolution = determine_natural_resolution(&outcome.result);

                            OfferedCell::Natural {
                                status: test_status,
                                resolution,
                                version,
                            }
                        }
                    }
                };

                // Format spec
                let spec = if outcome.result.forced_version {
                    // For forced versions, show → =version
                    outcome.result.expected_version.as_ref()
                        .map(|v| format!("→ ={}", v))
                        .unwrap_or_else(|| "→ (forced)".to_string())
                } else {
                    // For natural versions, show original requirement
                    outcome.result.original_requirement.as_ref()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "".to_string())
                };

                // Format resolved version
                let resolved = format_resolved_version(&outcome.result, &outcome.version_source);

                // Result summary (status + ICT marks)
                let result_status = match status {
                    VersionStatus::Passed => "PASSED",
                    VersionStatus::Regressed => "REGRESSED",
                    VersionStatus::Broken => "BROKEN",
                };
                let ict_marks = outcome.result.format_ict_marks();
                let result_str = format!("{} {}", result_status, ict_marks);

                // Time
                let time_str = format!("{:.1}s",
                    outcome.result.fetch.duration.as_secs_f64()
                    + outcome.result.check.as_ref().map(|c| c.duration.as_secs_f64()).unwrap_or(0.0)
                    + outcome.result.test.as_ref().map(|t| t.duration.as_secs_f64()).unwrap_or(0.0));

                // Color
                let color = match status {
                    VersionStatus::Passed => term::color::BRIGHT_GREEN,
                    VersionStatus::Regressed => term::color::BRIGHT_RED,
                    VersionStatus::Broken => term::color::BRIGHT_YELLOW,
                };

                // Error details (if failed)
                let error_details = if !outcome.result.is_success() {
                    extract_error_details(&outcome.result)
                } else {
                    Vec::new()
                };

                rows.push(TableRow {
                    offered: offered_cell,
                    spec,
                    resolved,
                    dependent: name_with_version.clone(),
                    result: result_str,
                    time: time_str,
                    color,
                    error_details,
                    multi_version_rows: Vec::new(),  // TODO: populate for actual multi-version in tree
                });
            }
        }
        TestResultData::Skipped(_) => {
            rows.push(TableRow {
                offered: OfferedCell::SkippedNoVersion,
                spec: "".to_string(),
                resolved: "".to_string(),
                dependent: name_with_version,
                result: "(skipped)".to_string(),
                time: "".to_string(),
                color: term::color::BRIGHT_CYAN,
                error_details: Vec::new(),
                multi_version_rows: Vec::new(),
            });
        }
        TestResultData::Error(_) => {
            rows.push(TableRow {
                offered: OfferedCell::SkippedNoVersion,
                spec: "".to_string(),
                resolved: "ERROR".to_string(),
                dependent: name_with_version,
                result: "ERROR".to_string(),
                time: "".to_string(),
                color: term::color::BRIGHT_MAGENTA,
                error_details: Vec::new(),
                multi_version_rows: Vec::new(),
            });
        }
        _ => {
            // All other result types (Broken, Passed, Regressed) have been migrated to MultiVersion
            // This branch should never be reached after migration
        }
    }

    rows
}

/// Determine natural resolution outcome (exact or upgraded)
/// Only called for non-forced versions
fn determine_natural_resolution(result: &ThreeStepResult) -> NaturalResolution {
    match (&result.expected_version, &result.actual_version) {
        (Some(expected), Some(actual)) => {
            if expected == actual {
                NaturalResolution::Exact
            } else {
                // Natural cargo resolution chose a different (newer) version
                NaturalResolution::Upgraded
            }
        }
        _ => NaturalResolution::Exact,  // Default if we can't verify
    }
}

/// Format version string, using "this(version)" for local versions
fn format_version_with_this(version_source: &VersionSource, this_version: &str) -> String {
    match version_source {
        VersionSource::Local(_) => format!("this({})", this_version),
        VersionSource::Published(v) => v.clone(),
    }
}

/// Format resolved version with source icon
fn format_resolved_version(result: &ThreeStepResult, version_source: &VersionSource) -> String {
    let version = result.actual_version.as_ref()
        .or(result.expected_version.as_ref())
        .map(|v| v.as_str())
        .unwrap_or("?");

    let icon = match version_source {
        VersionSource::Local(_) => "📁",
        VersionSource::Published(_) => "📦",
    };

    format!("{} {}", version, icon)
}

/// Extract error details from a failed test result
fn extract_error_details(result: &ThreeStepResult) -> Vec<String> {
    let mut details = Vec::new();

    if let Some(failed) = result.first_failure() {
        if !failed.diagnostics.is_empty() {
            let errors: Vec<_> = failed.diagnostics.iter()
                .filter(|d| d.level.is_error())
                .take(3)  // Limit to first 3 errors
                .collect();

            for diag in errors {
                let msg = if diag.message.len() > 70 {
                    format!("{}...", &diag.message[..67])
                } else {
                    diag.message.clone()
                };

                let error_line = if let Some(code) = &diag.code {
                    format!("• error[{}]: {}", code, msg)
                } else {
                    format!("• error: {}", msg)
                };

                details.push(error_line);

                // Add location if available
                if let Some(span) = &diag.primary_span {
                    details.push(format!("   --> {}:{}:{}", span.file_name, span.line, span.column));
                }
            }
        }
    }

    details
}

/// Print table header (for real-time printing mode)
pub fn print_table_header(crate_name: &str, display_version: &str, total_deps: usize) {
    println!("\n{}", "=".repeat(120));
    println!("Testing {} reverse dependencies of {}", total_deps, crate_name);
    println!("  this = {} (your work-in-progress version)", display_version);
    println!("{}", "=".repeat(120));
    println!();

    // Column widths
    let w_offered = 20;
    let w_spec = 10;
    let w_resolved = 17;
    let w_dependent = 25;
    let w_result = 21;

    // Print table header
    println!("┌{:─<w_offered$}┬{:─<w_spec$}┬{:─<w_resolved$}┬{:─<w_dependent$}┬{:─<w_result$}┐",
             "", "", "", "", "",
             w_offered=w_offered, w_spec=w_spec, w_resolved=w_resolved,
             w_dependent=w_dependent, w_result=w_result);

    println!("│{:^w_offered$}│{:^w_spec$}│{:^w_resolved$}│{:^w_dependent$}│{:^w_result$}│",
             "Offered", "Spec", "Resolved", "Dependent", "Result         Time",
             w_offered=w_offered, w_spec=w_spec, w_resolved=w_resolved,
             w_dependent=w_dependent, w_result=w_result);

    println!("├{:─<w_offered$}┼{:─<w_spec$}┼{:─<w_resolved$}┼{:─<w_dependent$}┼{:─<w_result$}┤",
             "", "", "", "", "",
             w_offered=w_offered, w_spec=w_spec, w_resolved=w_resolved,
             w_dependent=w_dependent, w_result=w_result);
}

/// Print a single test result block (for real-time printing)
pub fn print_result_block(result: &TestResult, display_version: &str) {
    let w_offered = 20;
    let w_spec = 10;
    let w_resolved = 17;
    let w_dependent = 25;
    let w_result = 21;

    let rows = build_table_rows(result, display_version);
    let num_rows = rows.len();

    for (idx, row) in rows.iter().enumerate() {
        let is_last = idx == num_rows - 1;
        print_table_row(&row, w_offered, w_spec, w_resolved, w_dependent, w_result, is_last);
    }
}

/// Print table footer
pub fn print_table_footer() {
    let w_offered = 20;
    let w_spec = 10;
    let w_resolved = 17;
    let w_dependent = 25;
    let w_result = 21;

    println!("└{:─<w_offered$}┴{:─<w_spec$}┴{:─<w_resolved$}┴{:─<w_dependent$}┴{:─<w_result$}┘",
             "", "", "", "", "",
             w_offered=w_offered, w_spec=w_spec, w_resolved=w_resolved,
             w_dependent=w_dependent, w_result=w_result);
    println!();
}

/// Print a horizontal separator line between dependents
pub fn print_separator_line() {
    let w_offered = 20;
    let w_spec = 10;
    let w_resolved = 17;
    let w_dependent = 25;
    let w_result = 21;

    println!("├{:─<w_offered$}┼{:─<w_spec$}┼{:─<w_resolved$}┼{:─<w_dependent$}┼{:─<w_result$}┤",
             "", "", "", "", "",
             w_offered=w_offered, w_spec=w_spec, w_resolved=w_resolved,
             w_dependent=w_dependent, w_result=w_result);
}

/// Print a console table showing all test results (new five-column format)
pub fn print_console_table_v2(results: &[TestResult], crate_name: &str, display_version: &str) {
    if results.is_empty() {
        println!("No reverse dependencies tested.");
        return;
    }

    // Print header
    print_table_header(crate_name, display_version, results.len());

    // Print each result with separators between dependents
    for (i, result) in results.iter().enumerate() {
        if i > 0 {
            // Print separator between each dependent
            print_separator_line();
        }
        print_result_block(result, display_version);
    }

    // Print footer
    print_table_footer();

    // Print summary
    let summary = summarize_results(results);
    println!("Summary:");
    println!("  ✓ Passed:    {}", summary.passed);
    println!("  ✗ Regressed: {}", summary.regressed);
    println!("  ⚠ Broken:    {}", summary.broken);
    println!("  ⊘ Skipped:   {}", summary.skipped);
    println!("  ⚡ Error:     {}", summary.error);
    println!("  ━━━━━━━━━━━━━");
    println!("  Total:       {}", summary.total());
    println!();
}

/// Print a single table row with optional error details and multi-version rows
fn print_table_row(row: &TableRow, w_offered: usize, w_spec: usize, w_resolved: usize, w_dependent: usize, w_result: usize, is_last: bool) {
    // Main row
    let offered_str = row.offered.format();
    let offered_display = truncate_with_padding(&offered_str, w_offered - 2);
    let spec_display = truncate_with_padding(&row.spec, w_spec - 2);
    let resolved_display = truncate_with_padding(&row.resolved, w_resolved - 2);
    let dependent_display = truncate_with_padding(&row.dependent, w_dependent - 2);
    let result_display = format!("{:>12} {:>5}", row.result, row.time);
    let result_display = truncate_with_padding(&result_display, w_result - 2);

    // Print main row with color
    // Note: strings are already padded by truncate_with_padding(), so we don't use width specifiers
    if let Some(ref mut t) = term::stdout() {
        let _ = t.fg(row.color);
        let _ = write!(t, "│ {} │", offered_display);
        let _ = write!(t, " {} │", spec_display);
        let _ = write!(t, " {} │", resolved_display);
        let _ = write!(t, " {} │", dependent_display);
        let _ = write!(t, " {} │", result_display);
        let _ = t.reset();
        println!();
    } else {
        println!("│ {} │ {} │ {} │ {} │ {} │",
                 offered_display, spec_display, resolved_display, dependent_display, result_display);
    }

    // Print error details with dropped-panel border (if any)
    if !row.error_details.is_empty() {
        // Error text row format: │{w_offered}│ {error_text} │
        // Total: 1 + w_offered + 1 + 1 + error_text + 1 + 1 = 99
        // So: error_text = 99 - 1 - w_offered - 1 - 1 - 1 - 1 = 74
        let error_text_width = 99 - 1 - w_offered - 1 - 1 - 1 - 1;

        // Top border with corners dropping to create panel
        // Format: │{w_offered}├{col2_dashes}┘{spaces}└{col4_dashes}┘{w_result}│
        // Middle section: 99 - 1 - w_offered - w_result - 1 = 56
        // Contains: ├ (1) + col2_dashes + ┘ (1) + spaces + └ (1) + col4_dashes + ┘ (1) = 56
        // So: col2_dashes + spaces + col4_dashes = 52
        let corner1_width = w_spec;  // 10
        let corner2_width = w_dependent;  // 25
        let padding_width = 52 - corner1_width - corner2_width;  // 17

        println!("│{:w_offered$}├{:─<corner1$}┘{:padding$}└{:─<corner2$}┘{:w_result$}│",
                 "", "", "", "", "",
                 w_offered = w_offered, corner1 = corner1_width,
                 padding = padding_width, corner2 = corner2_width, w_result = w_result);

        // Error detail lines - span columns 2-5, only outer borders
        for error_line in &row.error_details {
            let truncated = truncate_with_padding(error_line, error_text_width);
            println!("│{:w_offered$}│ {} │",
                     "", truncated,
                     w_offered = w_offered);
        }

        // Restore full border (only if there are more rows to follow)
        if !is_last {
            println!("│{:w_offered$}├{:─<w_spec$}┬{:─<w_resolved$}┬{:─<w_dependent$}┬{:─<w_result$}┤",
                     "", "", "", "", "",
                     w_offered = w_offered, w_spec = w_spec, w_resolved = w_resolved,
                     w_dependent = w_dependent, w_result = w_result);
        }
    }

    // Print multi-version rows with ├─ prefixes (if any)
    if !row.multi_version_rows.is_empty() {
        for mv_row in &row.multi_version_rows {
            let spec_display = format!("├─ {}", mv_row.spec);
            let spec_display = truncate_with_padding(&spec_display, w_spec - 2);
            let resolved_display = format!("├─ {}", mv_row.resolved);
            let resolved_display = truncate_with_padding(&resolved_display, w_resolved - 2);
            let dependent_display = format!("├─ {}", mv_row.dependent);
            let dependent_display = truncate_with_padding(&dependent_display, w_dependent - 2);

            println!("│{:w$}│ {} │ {} │ {} │{:w_result$}│",
                     "", spec_display, resolved_display, dependent_display, "",
                     w = w_offered, w_result = w_result);
        }
    }
}

/// Truncate string to fit width, adding "..." if truncated
/// Handles UTF-8 character boundaries correctly
fn truncate_str(s: &str, max_width: usize) -> String {
    // Count characters, not bytes, to handle UTF-8
    let char_count = s.chars().count();

    if char_count <= max_width {
        s.to_string()
    } else if max_width >= 3 {
        let truncate_at = max_width - 3;
        let truncated: String = s.chars().take(truncate_at).collect();
        format!("{}...", truncated)
    } else {
        let truncated: String = s.chars().take(max_width).collect();
        truncated
    }
}

/// Count the display width of a string, accounting for wide Unicode characters
/// Emojis like 📦 and 📁 take 2 columns, box-drawing is 1, other symbols vary
fn display_width(s: &str) -> usize {
    s.chars().map(|c| {
        match c {
            // Emojis that are definitely 2 columns wide (Wide in East Asian Width)
            '📦' | '📁' | '⚡' => 2,
            // Unicode symbols - 1 column in Western terminals (Neutral/Narrow)
            '✓' | '✗' | '⊘' | '⚠' => 1,
            // Box-drawing characters - 1 column (Ambiguous, but 1 in Western terminals)
            '━' | '─' | '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' => 1,
            // Most other characters are 1 column
            _ => {
                // Additional check for emoji ranges
                let code = c as u32;
                if (code >= 0x1F300 && code <= 0x1F9FF) || // Misc Symbols and Pictographs
                   (code >= 0x2600 && code <= 0x26FF) {    // Misc Symbols (includes ⚡)
                    // Most in this range are wide, but we've handled exceptions above
                    2
                } else {
                    1
                }
            }
        }
    }).sum()
}

/// Truncate and pad string to exact display width
/// Accounts for wide Unicode characters that take 2 columns
fn truncate_with_padding(s: &str, width: usize) -> String {
    let display_w = display_width(s);

    if display_w >= width {
        // Need to truncate - rebuild string char by char until we hit width
        let mut result = String::new();
        let mut current_width = 0;

        for c in s.chars() {
            let char_width = match c {
                '📦' | '📁' | '⚡' => 2,
                '✓' | '✗' | '⊘' | '⚠' => 1,
                '━' | '─' | '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' => 1,
                _ => {
                    let code = c as u32;
                    if (code >= 0x1F300 && code <= 0x1F9FF) || (code >= 0x2600 && code <= 0x26FF) {
                        2
                    } else {
                        1
                    }
                }
            };

            if current_width + char_width > width {
                break;
            }

            result.push(c);
            current_width += char_width;
        }

        // If we have room, add "..."
        if width >= 3 && current_width < width - 3 {
            result.push_str("...");
        }

        // Pad to exact width
        let final_width = display_width(&result);
        if final_width < width {
            result.push_str(&" ".repeat(width - final_width));
        }

        result
    } else {
        // Pad with spaces to reach the width
        let padding = width - display_w;
        format!("{}{}", s, " ".repeat(padding))
    }
}

/// Print a console table showing all test results (legacy format)
pub fn print_console_table(results: &[TestResult], crate_name: &str, display_version: &str) {
    println!("\n{}", "=".repeat(110));

    // Count total rows (including multi-version expansions) - currently unused but may be needed for pagination
    let mut _total_rows = 0;
    for result in results {
        if let TestResultData::MultiVersion(ref outcomes) = result.data {
            _total_rows += outcomes.len();
        } else {
            _total_rows += 1;
        }
    }

    println!("Testing {} reverse dependencies of {}", results.len(), crate_name);
    println!("  this = {} (your work-in-progress version)", display_version);
    println!("{}", "=".repeat(110));
    println!();

    if results.is_empty() {
        println!("No reverse dependencies tested.");
        return;
    }

    // Print table header with ICT column
    // Status (12) | Dependent (26) | Version (14) | ICT (5) | Duration (10)
    println!("┌{:─<12}┬{:─<26}┬{:─<14}┬{:─<5}┬{:─<10}┐",
             "", "", "", "", "");
    println!("│{:^12}│{:^26}│{:^14}│{:^5}│{:^10}│",
             "Status", "Dependent", "Version", "ICT", "Duration");
    println!("├{:─<12}┼{:─<26}┼{:─<14}┼{:─<5}┼{:─<10}┤",
             "", "", "", "", "");

    // Print each result
    for result in results {
        // Format name with version: "crate-name 1.2.3"
        let name_with_version = format!("{} {}", result.rev_dep.name, result.rev_dep.vers);
        let name = if name_with_version.len() > 24 {
            format!("{}...", &name_with_version[..21])
        } else {
            name_with_version
        };

        match &result.data {
            TestResultData::Passed(four_step) |
            TestResultData::Regressed(four_step) |
            TestResultData::Broken(four_step) => {
                let (status_label, color) = get_status_info(&result.data);
                let _depends_on = format_depends_on(&result.rev_dep.resolved_version, four_step);
                let _testing = format_testing(four_step);
                let duration = format_total_duration(four_step);

                // For legacy 4-step, show in old format (combine depends_on and testing)
                let version_col = "baseline";
                let ict_col = "✓✓"; // Simplified for legacy
                print_colored_row_ict(status_label, &name, version_col, ict_col, &duration, color);
            }
            TestResultData::Skipped(_) => {
                let (status_label, color) = get_status_info(&result.data);
                print_colored_row_ict(status_label, &name, "(incompatible)", "", "", color);
            }
            TestResultData::Error(_) => {
                let (status_label, color) = get_status_info(&result.data);
                print_colored_row_ict(status_label, &name, "ERROR", "", "", color);
            }
            TestResultData::MultiVersion(ref outcomes) => {
                // Print one row per version
                // Baseline is always the FIRST version (reordered in run_multi_version_test)
                let baseline = outcomes.first();

                for (idx, outcome) in outcomes.iter().enumerate() {
                    let mut version_label = outcome.version_source.label();
                    // Add asterisk if this version was forced (outside semver requirements)
                    if outcome.result.forced_version {
                        version_label = format!("{}*", version_label);
                    }
                    let ict_marks = outcome.result.format_ict_marks();
                    let duration = format!("{:.1}s", outcome.result.fetch.duration.as_secs_f64()
                        + outcome.result.check.as_ref().map(|c| c.duration.as_secs_f64()).unwrap_or(0.0)
                        + outcome.result.test.as_ref().map(|t| t.duration.as_secs_f64()).unwrap_or(0.0));

                    // Determine status for this version compared to baseline
                    let is_baseline = idx == 0;
                    let status = if is_baseline {
                        // First version IS the baseline
                        if outcome.result.is_success() {
                            VersionStatus::Passed
                        } else {
                            VersionStatus::Broken
                        }
                    } else {
                        // Compare to baseline
                        classify_version_outcome(outcome, baseline)
                    };

                    let (status_label, color) = match status {
                        VersionStatus::Passed => ("✓ PASSED", term::color::BRIGHT_GREEN),
                        VersionStatus::Regressed => ("✗ REGRESS", term::color::BRIGHT_RED),
                        VersionStatus::Broken => ("⚠ BROKEN", term::color::BRIGHT_YELLOW),
                    };

                    print_colored_row_ict(status_label, &name, &version_label, &ict_marks, &duration, color);

                    // Show version verification and diagnostics for failures
                    if !outcome.result.is_success() {
                        // Version mismatch warning
                        if let (Some(ref expected), Some(ref actual)) = (&outcome.result.expected_version, &outcome.result.actual_version) {
                            if expected != actual {
                                println!("│            │  ⚠️ VERSION MISMATCH: Expected {} but got {}",
                                    expected, actual);
                            }
                        }

                        // Print diagnostics
                        print_outcome_diagnostics(outcome);
                    }
                }
            }
        }
    }

    println!("└{:─<12}┴{:─<26}┴{:─<14}┴{:─<5}┴{:─<10}┘",
             "", "", "", "", "");
    println!();

    // Print summary
    let summary = summarize_results(results);
    println!("Summary:");
    println!("  ✓ Passed:    {}", summary.passed);
    println!("  ✗ Regressed: {}", summary.regressed);
    println!("  ⚠ Broken:    {}", summary.broken);
    println!("  ⊘ Skipped:   {}", summary.skipped);
    println!("  ⚡ Error:     {}", summary.error);
    println!("  ━━━━━━━━━━━━━");
    println!("  Total:       {}", summary.total());
    println!();
}

/// Format the "Depends On" column showing baseline version with checkmarks
fn format_depends_on(resolved_version: &Option<String>, four_step: &FourStepResult) -> String {
    let version = resolved_version.as_deref().unwrap_or("?");
    let check_mark = if four_step.baseline_check.success { "✓" } else { "✗" };
    let test_mark = four_step.baseline_test.as_ref()
        .map(|t| if t.success { "✓" } else { "✗" })
        .unwrap_or(" ");

    format!("{} {}{}", version, check_mark, test_mark)
}

/// Format the "Testing" column showing "this" with checkmarks
fn format_testing(four_step: &FourStepResult) -> String {
    let check_mark = four_step.override_check.as_ref()
        .map(|c| if c.success { "✓" } else { "✗" })
        .unwrap_or(" ");
    let test_mark = four_step.override_test.as_ref()
        .map(|t| if t.success { "✓" } else { "✗" })
        .unwrap_or(" ");

    format!("this {}{}", check_mark, test_mark)
}

/// Format total duration across all four steps
fn format_total_duration(four_step: &FourStepResult) -> String {
    let mut total = four_step.baseline_check.duration;
    if let Some(ref t) = four_step.baseline_test {
        total += t.duration;
    }
    if let Some(ref c) = four_step.override_check {
        total += c.duration;
    }
    if let Some(ref t) = four_step.override_test {
        total += t.duration;
    }
    format!("{:.1}s", total.as_secs_f64())
}

/// Format a compile step for console display
fn format_step(result: &CompileResult) -> String {
    let marker = if result.success { "✓" } else { "✗" };
    let duration = format!("{:.1}s", result.duration.as_secs_f64());
    format!("{} {}", marker, duration)
}

/// Print diagnostics for a failed version test outcome
fn print_outcome_diagnostics(outcome: &VersionTestOutcome) {
    use crate::error_extract::DiagnosticLevel;

    // Find the first failed step
    let failed_step = outcome.result.first_failure();

    if let Some(failed) = failed_step {
        // Only show diagnostics if we have them
        if !failed.diagnostics.is_empty() {
            let error_count = failed.diagnostics.iter()
                .filter(|d| matches!(d.level, DiagnosticLevel::Error))
                .count();

            if error_count > 0 {
                println!("│            │  └─ {} error(s):", error_count);

                // Show first 2 errors with their rendered output
                for diag in failed.diagnostics.iter()
                    .filter(|d| matches!(d.level, DiagnosticLevel::Error))
                    .take(2) {

                    // Truncate message if too long
                    let msg = if diag.message.len() > 80 {
                        format!("{}...", &diag.message[..77])
                    } else {
                        diag.message.clone()
                    };

                    println!("│            │     • {}", msg);
                }

                if error_count > 2 {
                    println!("│            │     ... and {} more error(s)", error_count - 2);
                }
            }
        } else {
            // No diagnostics, show stderr excerpt
            let stderr = &failed.stderr;
            if !stderr.is_empty() {
                let lines: Vec<&str> = stderr.lines().collect();
                let relevant_lines: Vec<&str> = lines.iter()
                    .filter(|l| l.contains("error") || l.contains("Error") || l.contains("failed"))
                    .take(3)
                    .copied()
                    .collect();

                if !relevant_lines.is_empty() {
                    println!("│            │  └─ Error output:");
                    for line in relevant_lines {
                        let truncated = if line.len() > 80 {
                            format!("{}...", &line[..77])
                        } else {
                            line.to_string()
                        };
                        println!("│            │     {}", truncated);
                    }
                }
            }
        }
    }
}

/// Export markdown analysis report for AI/LLM analysis
pub fn export_markdown_report(
    results: &[TestResult],
    output_path: &PathBuf,
    crate_name: &str,
    display_version: &str,
) -> Result<Summary, Error> {
    let summary = summarize_results(results);

    let mut file = File::create(output_path)?;

    // Title and summary
    writeln!(file, "# Cargo Crusader Analysis Report\n")?;
    writeln!(file, "**Testing**: {} {}\n", crate_name, display_version)?;

    writeln!(file, "## Summary Statistics\n")?;
    writeln!(file, "| Status | Count |")?;
    writeln!(file, "|--------|-------|")?;
    writeln!(file, "| ✅ Passed | {} |", summary.passed)?;
    writeln!(file, "| ❌ Regressed | {} |", summary.regressed)?;
    writeln!(file, "| ⚠️ Broken | {} |", summary.broken)?;
    writeln!(file, "| ⊘ Skipped | {} |", summary.skipped)?;
    writeln!(file, "| ⚡ Error | {} |", summary.error)?;
    writeln!(file, "| **Total** | **{}** |\n", summary.total())?;

    // Regressions section (most important)
    if summary.regressed > 0 {
        writeln!(file, "## ⚠️ Regressions (Breaking Changes)\n")?;
        writeln!(file, "These crates compiled successfully with the published version but fail with your WIP changes.")?;
        writeln!(file, "**Action required**: These are breaking changes that need attention.\n")?;

        for result in results.iter().filter(|r| matches!(r.data, TestResultData::Regressed(_))) {
            export_regression_markdown(&mut file, result)?;
        }
    }

    // Broken crates section (pre-existing issues)
    if summary.broken > 0 {
        writeln!(file, "## 🔧 Broken Crates (Pre-existing Issues)\n")?;
        writeln!(file, "These crates already fail to compile with the published baseline version.")?;
        writeln!(file, "**No action needed**: These issues exist independently of your changes.\n")?;

        for result in results.iter().filter(|r| matches!(r.data, TestResultData::Broken(_))) {
            export_broken_markdown(&mut file, result)?;
        }
    }

    // Skipped crates
    if summary.skipped > 0 {
        writeln!(file, "## ⊘ Skipped Crates (Version Incompatibility)\n")?;
        writeln!(file, "These crates were skipped because their version requirements are incompatible with your WIP version.\n")?;

        for result in results.iter().filter(|r| matches!(r.data, TestResultData::Skipped(_))) {
            if let TestResultData::Skipped(reason) = &result.data {
                writeln!(file, "### {} v{}", result.rev_dep.name, result.rev_dep.vers)?;
                writeln!(file, "**Reason**: {}\n", reason)?;
            }
        }
    }

    // Passed crates (brief summary)
    if summary.passed > 0 {
        writeln!(file, "## ✅ Passed Crates\n")?;
        writeln!(file, "The following {} crates compiled successfully with your changes:\n", summary.passed)?;

        let passed_names: Vec<String> = results.iter()
            .filter(|r| matches!(r.data, TestResultData::Passed(_)))
            .map(|r| format!("`{}` v{}", r.rev_dep.name, r.rev_dep.vers))
            .collect();

        // Show as bullet list
        for name in passed_names {
            writeln!(file, "- {}", name)?;
        }
        writeln!(file)?;
    }

    // Errors
    if summary.error > 0 {
        writeln!(file, "## ⚡ Errors\n")?;
        writeln!(file, "These crates encountered internal errors during testing:\n")?;

        for result in results.iter().filter(|r| matches!(r.data, TestResultData::Error(_))) {
            if let TestResultData::Error(e) = &result.data {
                writeln!(file, "### {} v{}", result.rev_dep.name, result.rev_dep.vers)?;
                writeln!(file, "```")?;
                writeln!(file, "{}", e)?;
                writeln!(file, "```\n")?;
            }
        }
    }

    Ok(summary)
}

/// Export a regression to markdown format
fn export_regression_markdown(file: &mut File, result: &TestResult) -> Result<(), Error> {
    writeln!(file, "### {} v{}\n", result.rev_dep.name, result.rev_dep.vers)?;
    writeln!(file, "**Status**: ❌ REGRESSED")?;
    writeln!(file, "**Crates.io**: https://crates.io/crates/{}", result.rev_dep.name)?;
    if let Some(ref resolved) = result.rev_dep.resolved_version {
        writeln!(file, "**Depends On**: {}\n", resolved)?;
    } else {
        writeln!(file)?;
    }

    if let TestResultData::Regressed(four_step) = &result.data {
        // Show step results
        writeln!(file, "#### Build Results\n")?;
        writeln!(file, "| Step | Baseline | Override |")?;
        writeln!(file, "|------|----------|----------|")?;

        let baseline_check_result = if four_step.baseline_check.success { "✅" } else { "❌" };
        let baseline_test_result = if let Some(ref t) = four_step.baseline_test {
            if t.success { "✅" } else { "❌" }
        } else { "⊘" };
        let override_check_result = if let Some(ref c) = four_step.override_check {
            if c.success { "✅" } else { "❌" }
        } else { "⊘" };
        let override_test_result = if let Some(ref t) = four_step.override_test {
            if t.success { "✅" } else { "❌" }
        } else { "⊘" };

        writeln!(file, "| Check | {} {:.1}s | {} {:.1}s |",
                 baseline_check_result,
                 four_step.baseline_check.duration.as_secs_f64(),
                 override_check_result,
                 four_step.override_check.as_ref().map(|c| c.duration.as_secs_f64()).unwrap_or(0.0))?;

        writeln!(file, "| Test | {} {:.1}s | {} {:.1}s |\n",
                 baseline_test_result,
                 four_step.baseline_test.as_ref().map(|t| t.duration.as_secs_f64()).unwrap_or(0.0),
                 override_test_result,
                 four_step.override_test.as_ref().map(|t| t.duration.as_secs_f64()).unwrap_or(0.0))?;

        // Show error details
        writeln!(file, "#### Error Details\n")?;

        let failed_step = if let Some(ref check) = four_step.override_check {
            if check.failed() {
                Some(("Override Check", check))
            } else {
                four_step.override_test.as_ref().map(|t| ("Override Test", t))
            }
        } else {
            None
        };

        if let Some((step_name, step_result)) = failed_step {
            writeln!(file, "**Failed Step**: {}\n", step_name)?;

            // Show diagnostics if available
            if !step_result.diagnostics.is_empty() {
                let errors: Vec<_> = step_result.diagnostics.iter()
                    .filter(|d| d.level.is_error())
                    .collect();

                for (i, diag) in errors.iter().enumerate() {
                    if i > 0 {
                        writeln!(file)?;
                    }

                    if let Some(code) = &diag.code {
                        writeln!(file, "**Error [{}]**: {}", code, diag.message)?;
                    } else {
                        writeln!(file, "**Error**: {}", diag.message)?;
                    }

                    if let Some(span) = &diag.primary_span {
                        writeln!(file, "- **Location**: `{}:{}:{}`", span.file_name, span.line, span.column)?;
                        if let Some(label) = &span.label {
                            writeln!(file, "- **Detail**: {}", label)?;
                        }
                    }

                    writeln!(file, "\n```")?;
                    writeln!(file, "{}", diag.rendered.trim())?;
                    writeln!(file, "```")?;
                }
            } else if !step_result.stderr.is_empty() {
                // Fallback: extract errors from stderr
                writeln!(file, "```")?;
                for line in step_result.stderr.lines().take(30) {
                    writeln!(file, "{}", line)?;
                }
                if step_result.stderr.lines().count() > 30 {
                    writeln!(file, "... (truncated)")?;
                }
                writeln!(file, "```")?;
            }
        }

        writeln!(file)?;
    }

    Ok(())
}

/// Export a broken crate to markdown format
fn export_broken_markdown(file: &mut File, result: &TestResult) -> Result<(), Error> {
    writeln!(file, "### {} v{}\n", result.rev_dep.name, result.rev_dep.vers)?;
    writeln!(file, "**Status**: ⚠️ BROKEN (pre-existing)")?;
    writeln!(file, "**Crates.io**: https://crates.io/crates/{}", result.rev_dep.name)?;
    if let Some(ref resolved) = result.rev_dep.resolved_version {
        writeln!(file, "**Depends On**: {}\n", resolved)?;
    } else {
        writeln!(file)?;
    }

    if let TestResultData::Broken(four_step) = &result.data {
        let failed_step = if four_step.baseline_check.failed() {
            Some(("Baseline Check", &four_step.baseline_check))
        } else {
            four_step.baseline_test.as_ref().map(|t| ("Baseline Test", t))
        };

        if let Some((step_name, step_result)) = failed_step {
            writeln!(file, "**Failed Step**: {}\n", step_name)?;

            // Brief error summary
            if !step_result.diagnostics.is_empty() {
                let error_count = step_result.diagnostics.iter()
                    .filter(|d| d.level.is_error())
                    .count();
                writeln!(file, "**Errors**: {} compilation error(s)\n", error_count)?;
            } else {
                writeln!(file, "**Note**: Failed with published baseline version\n")?;
            }
        }
    }

    Ok(())
}

/// Export HTML report to file
pub fn export_html_report(
    mut results: Vec<TestResult>,
    output_path: &PathBuf,
    crate_name: &str,
    display_version: &str,
) -> Result<Summary, Error> {
    let summary = summarize_results(&results);

    results.sort_by(|a, b| a.rev_dep.name.cmp(&b.rev_dep.name));

    let mut file = File::create(output_path)?;
    writeln!(file, "<!DOCTYPE html>")?;

    writeln!(file, "<head>")?;
    writeln!(file, "{}", r"
<style>
body { font-family: sans-serif; margin: 20px; }
table { border-collapse: collapse; width: 100%; margin: 20px 0; }
th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }
th { background-color: #f2f2f2; }
.passed { color: green; font-weight: bold; }
.regressed { color: red; font-weight: bold; }
.broken { color: orange; font-weight: bold; }
.skipped { color: gray; font-weight: bold; }
.error { color: magenta; font-weight: bold; }
.stdout, .stderr, .test-exception-output {
    white-space: pre;
    background: #f5f5f5;
    padding: 10px;
    border-radius: 4px;
    overflow-x: auto;
    font-family: monospace;
    font-size: 12px;
}
.complete-result { margin: 30px 0; border-top: 2px solid #333; padding-top: 20px; }
h3 { margin-top: 15px; color: #333; }
.summary-stats { display: flex; gap: 20px; margin: 20px 0; }
.stat { padding: 15px; border-radius: 8px; min-width: 100px; }
.stat-passed { background: #d4edda; color: #155724; }
.stat-regressed { background: #f8d7da; color: #721c24; }
.stat-broken { background: #fff3cd; color: #856404; }
.stat-skipped { background: #e2e3e5; color: #383d41; }
.stat-error { background: #f5c6cb; color: #721c24; }
.stat-number { font-size: 32px; font-weight: bold; }
.stat-label { font-size: 14px; }
</style>
")?;
    writeln!(file, "</head>")?;

    writeln!(file, "<body>")?;
    writeln!(file, "<h1>Cargo Crusader Report</h1>")?;
    writeln!(file, "<p><strong>Testing:</strong> {} {}</p>", sanitize(crate_name), sanitize(display_version))?;

    // Summary statistics
    writeln!(file, "<div class='summary-stats'>")?;
    writeln!(
        file,
        "<div class='stat stat-passed'><div class='stat-number'>{}</div><div class='stat-label'>Passed</div></div>",
        summary.passed
    )?;
    writeln!(
        file,
        "<div class='stat stat-regressed'><div class='stat-number'>{}</div><div class='stat-label'>Regressed</div></div>",
        summary.regressed
    )?;
    writeln!(
        file,
        "<div class='stat stat-broken'><div class='stat-number'>{}</div><div class='stat-label'>Broken</div></div>",
        summary.broken
    )?;
    writeln!(
        file,
        "<div class='stat stat-skipped'><div class='stat-number'>{}</div><div class='stat-label'>Skipped</div></div>",
        summary.skipped
    )?;
    writeln!(
        file,
        "<div class='stat stat-error'><div class='stat-number'>{}</div><div class='stat-label'>Error</div></div>",
        summary.error
    )?;
    writeln!(file, "</div>")?;

    // Summary table
    writeln!(file, "<h2>Summary</h2>")?;
    writeln!(file, "<table>")?;
    writeln!(
        file,
        "<tr><th>Crate</th><th>Version</th><th>Depends On</th><th>Result</th></tr>"
    )?;
    for result in &results {
        writeln!(file, "<tr>")?;
        writeln!(file, "<td>")?;
        writeln!(file, "<a href='#{}'>", result.html_anchor())?;
        writeln!(file, "{}", result.rev_dep.name)?;
        writeln!(file, "</a>")?;
        writeln!(file, "</td>")?;
        writeln!(file, "<td>{}</td>", result.rev_dep.vers)?;
        let depends_on = result.rev_dep.resolved_version.as_deref().unwrap_or("?");
        writeln!(file, "<td>{}</td>", sanitize(depends_on))?;
        writeln!(
            file,
            "<td class='{}'>{}</td>",
            result.html_class(),
            result.quick_str()
        )?;
        writeln!(file, "</tr>")?;
    }
    writeln!(file, "</table>")?;

    // Detailed results
    writeln!(file, "<h2>Details</h2>")?;
    for result in results {
        writeln!(file, "<div class='complete-result'>")?;
        writeln!(file, "<a name='{}'></a>", result.html_anchor())?;
        writeln!(file, "<h2>")?;
        writeln!(
            file,
            "<span>{} {}</span>",
            result.rev_dep.name, result.rev_dep.vers
        )?;
        writeln!(
            file,
            " <span class='{}'>{}</span>",
            result.html_class(),
            result.quick_str()
        )?;
        writeln!(file, "</h2>")?;

        match &result.data {
            TestResultData::Passed(four_step) | TestResultData::Regressed(four_step) => {
                export_compile_result(&mut file, "baseline check", &four_step.baseline_check)?;
                if let Some(ref test) = four_step.baseline_test {
                    export_compile_result(&mut file, "baseline test", test)?;
                }
                if let Some(ref check) = four_step.override_check {
                    export_compile_result(&mut file, "override check", check)?;
                }
                if let Some(ref test) = four_step.override_test {
                    export_compile_result(&mut file, "override test", test)?;
                }
            }
            TestResultData::Broken(four_step) => {
                export_compile_result(&mut file, "baseline check", &four_step.baseline_check)?;
                if let Some(ref test) = four_step.baseline_test {
                    export_compile_result(&mut file, "baseline test", test)?;
                }
            }
            TestResultData::Skipped(reason) => {
                writeln!(file, "<h3>Skipped</h3>")?;
                writeln!(file, "<p>Reason: {}</p>", sanitize(reason))?;
            }
            TestResultData::Error(e) => {
                export_error(&mut file, e)?;
            }
            TestResultData::MultiVersion(_) => {
                // TODO: Export multi-version results
                writeln!(file, "<h3>Multi-version testing (TODO)</h3>")?;
            }
        }
        writeln!(file, "</div>")?;
    }

    writeln!(file, "</body>")?;
    writeln!(file, "</html>")?;

    Ok(summary)
}

/// Export a single compile result to HTML
fn export_compile_result(
    file: &mut File,
    label: &str,
    r: &CompileResult,
) -> Result<(), Error> {
    let stdout = sanitize(&r.stdout);
    let stderr = sanitize(&r.stderr);
    let success_marker = if r.success { "✓" } else { "✗" };
    let duration_str = format!("{:.2}s", r.duration.as_secs_f64());
    writeln!(
        file,
        "<h3>{} {} ({})</h3>",
        label, success_marker, duration_str
    )?;
    writeln!(file, "<div class='stdout'>\n{}\n</div>", stdout)?;
    writeln!(file, "<div class='stderr'>\n{}\n</div>", stderr)?;

    Ok(())
}

/// Export an error to HTML
fn export_error(file: &mut File, e: &Error) -> Result<(), Error> {
    let err = sanitize(&format!("{}", e));
    writeln!(file, "<h3>{}</h3>", "errors")?;
    writeln!(
        file,
        "<div class='test-exception-output'>\n{}\n</div>",
        err
    )?;

    Ok(())
}

/// Sanitize HTML special characters
fn sanitize(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '&' => "&amp;".chars().collect(),
            _ => vec![c],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summary_default() {
        let summary = Summary::default();
        assert_eq!(summary.passed, 0);
        assert_eq!(summary.regressed, 0);
        assert_eq!(summary.broken, 0);
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.error, 0);
        assert_eq!(summary.total(), 0);
    }

    #[test]
    fn test_summary_total() {
        let summary = Summary {
            passed: 5,
            regressed: 2,
            broken: 1,
            skipped: 3,
            error: 1,
        };
        assert_eq!(summary.total(), 12);
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(sanitize("<script>"), "&lt;script&gt;");
        assert_eq!(sanitize("hello & goodbye"), "hello &amp; goodbye");
        assert_eq!(sanitize("normal text"), "normal text");
    }

    #[test]
    fn test_format_step() {
        use std::time::Duration;
        use crate::compile::{CompileStep, CompileResult};

        let success = CompileResult {
            step: CompileStep::Check,
            success: true,
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_secs(2),
            diagnostics: Vec::new(),
        };
        assert_eq!(format_step(&success), "✓ 2.0s");

        let failure = CompileResult {
            step: CompileStep::Check,
            success: false,
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_millis(1500),
            diagnostics: Vec::new(),
        };
        assert_eq!(format_step(&failure), "✗ 1.5s");
    }

    #[test]
    fn test_error_row_width_calculation() {
        // Column widths from the actual code
        let w_offered = 20;
        let w_spec = 10;
        let w_resolved = 17;
        let w_dependent = 25;
        let w_result = 21;

        // Expected total width of any row (from border line)
        // ┌{20}┬{10}┬{17}┬{25}┬{21}┐
        let expected_total = 1 + w_offered + 1 + w_spec + 1 + w_resolved + 1 + w_dependent + 1 + w_result + 1;
        assert_eq!(expected_total, 99, "Border line should be 99 chars");

        // Current buggy calculation from line 776
        let detail_width_buggy = w_spec + w_resolved + w_dependent + w_result + 6;
        println!("Buggy detail_width: {}", detail_width_buggy);
        // = 10 + 17 + 25 + 21 + 6 = 79

        // Current corner line calculation from lines 783-790
        let corner1_width = w_spec;  // 10
        let corner2_width = w_dependent;  // 25
        let padding_width = w_resolved + 2;  // 17 + 2 = 19

        // Line format: │{w_offered}├{corner1}┘{padding}└{corner2}┘{w_result}│
        let corner_line_width_buggy =
            1 + // left │
            w_offered + // 20 spaces
            1 + // ├
            corner1_width + 1 + // dashes + ┘ (10 + 1 = 11)
            padding_width + // spaces (19)
            1 + // └
            corner2_width + 1 + // dashes + ┘ (25 + 1 = 26)
            w_result + // 21 spaces
            1; // right │

        println!("Buggy corner line width: {}", corner_line_width_buggy);
        // = 1 + 20 + 1 + 11 + 19 + 1 + 26 + 21 + 1 = 101 (WRONG! Should be 99)

        assert_ne!(corner_line_width_buggy, 99, "Current calculation produces 101 chars instead of 99");

        // Correct calculation
        // The middle section (between col1 and col5) should be:
        // 99 - 1 (│) - 20 (col1) - 21 (col5) - 1 (│) = 56 chars
        let middle_section_width = 99 - 1 - w_offered - w_result - 1;
        println!("Middle section should be: {} chars", middle_section_width);
        assert_eq!(middle_section_width, 56);

        // The middle section contains:
        // ├{col2_dashes}┘{spaces}└{col4_dashes}┘
        // We need: 1 + col2_dashes + 1 + spaces + 1 + col4_dashes + 1 = 56
        // So: col2_dashes + spaces + col4_dashes = 56 - 4 = 52

        // Natural choice: col2_dashes = w_spec, col4_dashes = w_dependent
        // So: spaces = 52 - w_spec - w_dependent = 52 - 10 - 25 = 17
        let correct_padding = 52 - w_spec - w_dependent;
        println!("Correct padding width: {}", correct_padding);
        assert_eq!(correct_padding, 17);

        // Verify the corrected line
        let corner_line_width_correct =
            1 + // │
            w_offered + // 20
            1 + // ├
            w_spec + 1 + // 10 + 1 = 11
            correct_padding + // 17
            1 + // └
            w_dependent + 1 + // 25 + 1 = 26
            w_result + // 21
            1; // │
        println!("Correct corner line width: {}", corner_line_width_correct);
        assert_eq!(corner_line_width_correct, 99, "Corrected calculation should produce 99 chars");
    }

    #[test]
    fn test_error_text_row_width() {
        let w_offered = 20;
        let w_result = 21;

        // Error text row format: │{w_offered}│ {error_text} │
        // Total: 1 + 20 + 1 + 1 + error_text + 1 + 1 = 99
        // So: error_text = 99 - 1 - 20 - 1 - 1 - 1 - 1 = 74

        let error_text_width = 99 - 1 - w_offered - 1 - 1 - 1 - 1;
        println!("Error text width should be: {}", error_text_width);
        assert_eq!(error_text_width, 74);

        // Current buggy calculation claims detail_width = 79, minus 2 for padding = 77
        // That's 3 chars too many!
    }
}
