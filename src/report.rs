/// Report generation module
///
/// Provides both HTML and console table output for test results

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use crate::{TestResult, TestResultData, CompileResult, Error};
use crate::error_extract::{Diagnostic, extract_error_summary};
use term::color::Color;

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
        match result.data {
            TestResultData::Broken(..) => sum.broken += 1,
            TestResultData::Regressed(..) => sum.regressed += 1,
            TestResultData::Passed(..) => sum.passed += 1,
            TestResultData::Skipped(_) => sum.skipped += 1,
            TestResultData::Error(..) => sum.error += 1,
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
    }
}

/// Print a colored table row
fn print_colored_row(status: &str, name: &str, base_check: &str, base_test: &str,
                     over_check: &str, over_test: &str, color: Color) {
    // Print the row with coloring
    let row = format!("│{:^12}│{:<28}│{:^13}│{:^12}│{:^13}│{:^13}│",
                     status, name, base_check, base_test, over_check, over_test);

    if let Some(ref mut t) = term::stdout() {
        let _ = t.fg(color);
        let _ = write!(t, "{}", row);
        let _ = t.reset();
        println!();
    } else {
        println!("{}", row);
    }
}

/// Print a console table showing all test results
pub fn print_console_table(results: &[TestResult], crate_name: &str, crate_version: &str) {
    println!("\n{}", "=".repeat(110));
    println!("Testing {} reverse dependencies of {} v{}", results.len(), crate_name, crate_version);
    println!("{}", "=".repeat(110));
    println!();

    if results.is_empty() {
        println!("No reverse dependencies tested.");
        return;
    }

    // Print table header
    println!("┌{:─<12}┬{:─<28}┬{:─<13}┬{:─<12}┬{:─<13}┬{:─<13}┐",
             "", "", "", "", "", "");
    println!("│{:^12}│{:^28}│{:^13}│{:^12}│{:^13}│{:^13}│",
             "Status", "Dependent", "Base Check", "Base Test", "Over Check", "Over Test");
    println!("├{:─<12}┼{:─<28}┼{:─<13}┼{:─<12}┼{:─<13}┼{:─<13}┤",
             "", "", "", "", "", "");

    // Print each result
    for result in results {
        let name = if result.rev_dep.name.len() > 26 {
            format!("{}...", &result.rev_dep.name[..23])
        } else {
            result.rev_dep.name.clone()
        };

        let (status_label, color) = get_status_info(&result.data);

        match &result.data {
            TestResultData::Passed(four_step) => {
                let base_check = format_step(&four_step.baseline_check);
                let base_test = four_step.baseline_test.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());
                let over_check = four_step.override_check.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());
                let over_test = four_step.override_test.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());

                print_colored_row(status_label, &name, &base_check, &base_test,
                                 &over_check, &over_test, color);
            }
            TestResultData::Regressed(four_step) => {
                let base_check = format_step(&four_step.baseline_check);
                let base_test = four_step.baseline_test.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());
                let over_check = four_step.override_check.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());
                let over_test = four_step.override_test.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());

                print_colored_row(status_label, &name, &base_check, &base_test,
                                 &over_check, &over_test, color);
            }
            TestResultData::Broken(four_step) => {
                let base_check = format_step(&four_step.baseline_check);
                let base_test = four_step.baseline_test.as_ref()
                    .map(format_step)
                    .unwrap_or_else(|| "(skipped)".to_string());

                print_colored_row(status_label, &name, &base_check, &base_test,
                                 "(skipped)", "(skipped)", color);
            }
            TestResultData::Skipped(_) => {
                print_colored_row(status_label, &name, "SKIPPED", "(incompatible)",
                                 "", "", color);
            }
            TestResultData::Error(_) => {
                print_colored_row(status_label, &name, "ERROR", "", "", "", color);
            }
        }
    }

    println!("└{:─<12}┴{:─<28}┴{:─<13}┴{:─<12}┴{:─<13}┴{:─<13}┘",
             "", "", "", "", "", "");
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

/// Format a compile step for console display
fn format_step(result: &CompileResult) -> String {
    let marker = if result.success { "✓" } else { "✗" };
    let duration = format!("{:.1}s", result.duration.as_secs_f64());
    format!("{} {}", marker, duration)
}

/// Export HTML report to file
pub fn export_html_report(
    mut results: Vec<TestResult>,
    output_path: &PathBuf,
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
        "<tr><th>Crate</th><th>Version</th><th>Result</th></tr>"
    )?;
    for result in &results {
        writeln!(file, "<tr>")?;
        writeln!(file, "<td>")?;
        writeln!(file, "<a href='#{}'>", result.html_anchor())?;
        writeln!(file, "{}", result.rev_dep.name)?;
        writeln!(file, "</a>")?;
        writeln!(file, "</td>")?;
        writeln!(file, "<td>{}</td>", result.rev_dep.vers)?;
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
}
