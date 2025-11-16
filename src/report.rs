/// Report generation module - Clean rewrite for OfferedRow streaming
///
/// Provides console table output, HTML, and markdown reports

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use crate::{OfferedRow, DependencyRef, OfferedVersion, TestExecution, TestCommand, CommandType, CommandResult, CrateFailure, TransitiveTest, VersionSource};
use term::color::Color;

//
// Rendering Model Types
//

/// Status icon for the Offered column
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusIcon {
    Passed,     // ✓
    Failed,     // ✗
    Skipped,    // ⊘
}

impl StatusIcon {
    pub fn as_str(&self) -> &'static str {
        match self {
            StatusIcon::Passed => "✓",
            StatusIcon::Failed => "✗",
            StatusIcon::Skipped => "⊘",
        }
    }
}

/// Resolution marker showing how cargo resolved the version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Exact,      // = (cargo resolved to exact offered version)
    Upgraded,   // ↑ (cargo upgraded within semver range)
    Mismatch,   // ≠ (forced or semver incompatible)
}

impl Resolution {
    pub fn as_str(&self) -> &'static str {
        match self {
            Resolution::Exact => "=",
            Resolution::Upgraded => "↑",
            Resolution::Mismatch => "≠",
        }
    }
}

/// Content of the "Offered" cell - type-safe rendering model
#[derive(Debug, Clone, PartialEq)]
pub enum OfferedCell {
    /// Baseline test: "- baseline"
    Baseline,

    /// Tested version with status
    Tested {
        icon: StatusIcon,
        resolution: Resolution,
        version: String,
        forced: bool,  // adds [≠→!] suffix if true
    },
}

impl OfferedCell {
    /// Convert OfferedRow to OfferedCell (business logic → rendering model)
    pub fn from_offered_row(row: &OfferedRow) -> Self {
        if row.offered.is_none() {
            return OfferedCell::Baseline;
        }

        let offered = row.offered.as_ref().unwrap();
        let overall_passed = row.test.commands.iter().all(|cmd| cmd.result.passed);

        // Determine status icon
        let icon = match (row.baseline_passed, overall_passed) {
            (Some(true), true) => StatusIcon::Passed,   // PASSED
            (Some(true), false) => StatusIcon::Failed,  // REGRESSED
            (Some(false), _) => StatusIcon::Failed,     // BROKEN (baseline failed)
            (None, true) => StatusIcon::Passed,         // PASSED (no baseline)
            (None, false) => StatusIcon::Failed,        // FAILED (no baseline)
        };

        // Determine resolution marker
        let resolution = if offered.forced {
            Resolution::Mismatch  // Forced versions always show ≠
        } else if row.primary.used_offered_version {
            Resolution::Exact     // Cargo chose exactly what we offered
        } else {
            Resolution::Upgraded  // Cargo upgraded to something else
        };

        OfferedCell::Tested {
            icon,
            resolution,
            version: offered.version.clone(),
            forced: offered.forced,
        }
    }

    /// Format the cell content for display
    pub fn format(&self) -> String {
        match self {
            OfferedCell::Baseline => "- baseline".to_string(),
            OfferedCell::Tested { icon, resolution, version, forced } => {
                let mut result = format!(
                    "{} {}{}",
                    icon.as_str(),
                    resolution.as_str(),
                    version
                );
                if *forced {
                    result.push_str(" [≠→!]");
                }
                result
            }
        }
    }
}

//
// Console Table Rendering
//

// Column widths for the 5-column table
const W_OFFERED: usize = 20;
const W_SPEC: usize = 10;
const W_RESOLVED: usize = 17;
const W_DEPENDENT: usize = 21;
const W_RESULT: usize = 21;

/// Print table header
pub fn print_table_header(crate_name: &str, display_version: &str, total_deps: usize) {
    println!("\n{}", "=".repeat(99));
    println!("Testing {} reverse dependencies of {}", total_deps, crate_name);
    println!("  this = {} (your work-in-progress version)", display_version);
    println!("{}", "=".repeat(99));
    println!();
    println!("Legend: I=Install (cargo fetch), C=Check (cargo check), T=Test (cargo test)");
    println!("Icons: ✓=passed ✗=failed ⊘=skipped -=baseline  📦=crates.io 📁=local 🔀=git");
    println!();

    // Print table header
    println!("┌{:─<width1$}┬{:─<width2$}┬{:─<width3$}┬{:─<width4$}┬{:─<width5$}┐",
             "", "", "", "", "",
             width1 = W_OFFERED, width2 = W_SPEC, width3 = W_RESOLVED,
             width4 = W_DEPENDENT, width5 = W_RESULT);
    println!("│{:^width1$}│{:^width2$}│{:^width3$}│{:^width4$}│{:^width5$}│",
             "Offered", "Spec", "Resolved", "Dependent", "Result         Time",
             width1 = W_OFFERED, width2 = W_SPEC, width3 = W_RESOLVED,
             width4 = W_DEPENDENT, width5 = W_RESULT);
    println!("├{:─<width1$}┼{:─<width2$}┼{:─<width3$}┼{:─<width4$}┼{:─<width5$}┤",
             "", "", "", "", "",
             width1 = W_OFFERED, width2 = W_SPEC, width3 = W_RESOLVED,
             width4 = W_DEPENDENT, width5 = W_RESULT);
}

/// Print separator line between dependents
pub fn print_separator_line() {
    println!("├{:─<width1$}┼{:─<width2$}┼{:─<width3$}┼{:─<width4$}┼{:─<width5$}┤",
             "", "", "", "", "",
             width1 = W_OFFERED, width2 = W_SPEC, width3 = W_RESOLVED,
             width4 = W_DEPENDENT, width5 = W_RESULT);
}

/// Print table footer
pub fn print_table_footer() {
    println!("└{:─<width1$}┴{:─<width2$}┴{:─<width3$}┴{:─<width4$}┴{:─<width5$}┘",
             "", "", "", "", "",
             width1 = W_OFFERED, width2 = W_SPEC, width3 = W_RESOLVED,
             width4 = W_DEPENDENT, width5 = W_RESULT);
}

/// Print an OfferedRow using the standard table format
pub fn print_offered_row(row: &OfferedRow, is_last_in_group: bool) {
    // Convert OfferedRow to column strings
    let (offered_str, spec_str, resolved_str, dependent_str, result_str, time_str, color, error_details, multi_version_rows) = format_offered_row(row);

    // Print main row
    let offered_display = truncate_with_padding(&offered_str, W_OFFERED - 2);
    let spec_display = truncate_with_padding(&spec_str, W_SPEC - 2);
    let resolved_display = truncate_with_padding(&resolved_str, W_RESOLVED - 2);
    let dependent_display = truncate_with_padding(&dependent_str, W_DEPENDENT - 2);
    let result_display = format!("{:>12} {:>5}", result_str, time_str);
    let result_display = truncate_with_padding(&result_display, W_RESULT - 2);

    // Print main row with color
    if let Some(ref mut t) = term::stdout() {
        let _ = t.fg(color);
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
    if !error_details.is_empty() {
        let error_text_width = 99 - 1 - W_OFFERED - 1 - 1 - 1 - 1;
        let corner1_width = W_SPEC;
        let corner2_width = W_DEPENDENT;
        let padding_width = 52 - corner1_width - corner2_width;

        println!("│{:w_offered$}├{:─<corner1$}┘{:padding$}└{:─<corner2$}┘{:w_result$}│",
                 "", "", "", "", "",
                 w_offered = W_OFFERED, corner1 = corner1_width,
                 padding = padding_width, corner2 = corner2_width, w_result = W_RESULT);

        for error_line in &error_details {
            let truncated = truncate_with_padding(error_line, error_text_width);
            println!("│{:w_offered$}│ {} │",
                     "", truncated,
                     w_offered = W_OFFERED);
        }

        if !is_last_in_group {
            println!("│{:w_offered$}├{:─<w_spec$}┬{:─<w_resolved$}┬{:─<w_dependent$}┬{:─<w_result$}┤",
                     "", "", "", "", "",
                     w_offered = W_OFFERED, w_spec = W_SPEC, w_resolved = W_RESOLVED,
                     w_dependent = W_DEPENDENT, w_result = W_RESULT);
        }
    }

    // Print multi-version rows with ├─ prefixes (if any)
    if !multi_version_rows.is_empty() {
        for (i, (spec, resolved, dependent)) in multi_version_rows.iter().enumerate() {
            let spec_display = format!("├─ {}", spec);
            let spec_display = truncate_with_padding(&spec_display, W_SPEC - 2);
            let resolved_display = format!("├─ {}", resolved);
            let resolved_display = truncate_with_padding(&resolved_display, W_RESOLVED - 2);
            let dependent_display = format!("├─ {}", dependent);
            let dependent_display = truncate_with_padding(&dependent_display, W_DEPENDENT - 2);

            println!("│{:w$}│ {} │ {} │ {} │{:w_result$}│",
                     "", spec_display, resolved_display, dependent_display, "",
                     w = W_OFFERED, w_result = W_RESULT);
        }
    }
}

//
// OfferedRow to renderable format conversion
//

/// Convert OfferedRow to renderable row data
/// Returns: (offered_str, spec_str, resolved_str, dependent_str, result_str, time_str, color, error_details, multi_version_rows)
fn format_offered_row(row: &OfferedRow) -> (String, String, String, String, String, String, Color, Vec<String>, Vec<(String, String, String)>) {
    // Format Offered column using type-safe OfferedCell
    let offered_cell = OfferedCell::from_offered_row(row);
    let offered_str = offered_cell.format();

    // Format Spec column
    let spec_str = if let Some(ref offered) = row.offered {
        if offered.forced {
            format!("→ ={}", offered.version)
        } else {
            row.primary.spec.clone()
        }
    } else {
        row.primary.spec.clone()
    };

    // Format Resolved column
    let source_icon = match row.primary.resolved_source {
        VersionSource::CratesIo => "📦",
        VersionSource::Local => "📁",
        VersionSource::Git => "🔀",
    };
    let resolved_str = format!("{} {}", row.primary.resolved_version, source_icon);

    // Format Dependent column
    let dependent_str = format!("{} {}", row.primary.dependent_name, row.primary.dependent_version);

    // Format Result column
    let overall_passed = row.test.commands.iter().all(|cmd| cmd.result.passed);
    let result_status = match (row.baseline_passed, overall_passed) {
        (Some(true), true) => "PASSED",
        (Some(true), false) => "REGRESSED",
        (Some(false), _) => "BROKEN",
        (None, true) => "PASSED",
        (None, false) => "FAILED",
    };

    // Format ICT marks
    let mut ict_marks = String::new();
    for cmd in &row.test.commands {
        match cmd.command {
            CommandType::Fetch => ict_marks.push(if cmd.result.passed { '✓' } else { '✗' }),
            CommandType::Check => ict_marks.push(if cmd.result.passed { '✓' } else { '✗' }),
            CommandType::Test => ict_marks.push(if cmd.result.passed { '✓' } else { '✗' }),
        }
    }
    // Pad to 3 chars with '-' for skipped steps
    while ict_marks.len() < 3 {
        ict_marks.push('-');
    }

    let result_str = format!("{} {}", result_status, ict_marks);

    // Calculate total time
    let total_time: f64 = row.test.commands.iter()
        .map(|cmd| cmd.result.duration)
        .sum();
    let time_str = format!("{:.1}s", total_time);

    // Determine color
    let color = match (row.baseline_passed, overall_passed) {
        (Some(true), true) => term::color::BRIGHT_GREEN,
        (Some(true), false) => term::color::BRIGHT_RED,
        (Some(false), _) => term::color::BRIGHT_YELLOW,
        (None, true) => term::color::BRIGHT_GREEN,
        (None, false) => term::color::BRIGHT_RED,
    };

    // Extract error details
    let mut error_details = Vec::new();
    for cmd in &row.test.commands {
        if !cmd.result.passed {
            let cmd_name = match cmd.command {
                CommandType::Fetch => "fetch",
                CommandType::Check => "check",
                CommandType::Test => "test",
            };
            for failure in &cmd.result.failures {
                error_details.push(format!("cargo {} failed on {}", cmd_name, failure.crate_name));
                // Add error message if not empty
                if !failure.error_message.is_empty() {
                    let lines: Vec<&str> = failure.error_message.lines().take(5).collect();
                    for line in lines {
                        error_details.push(format!("  • {}", line));
                    }
                }
            }
        }
    }

    // Format transitive dependency rows (multi-version rows)
    let mut multi_version_rows = Vec::new();
    for transitive in &row.transitive {
        let source_icon = match transitive.dependency.resolved_source {
            VersionSource::CratesIo => "📦",
            VersionSource::Local => "📁",
            VersionSource::Git => "🔀",
        };
        multi_version_rows.push((
            transitive.dependency.spec.clone(),
            format!("{} {}", transitive.dependency.resolved_version, source_icon),
            format!("{} {}", transitive.dependency.dependent_name, transitive.dependency.dependent_version),
        ));
    }

    (offered_str, spec_str, resolved_str, dependent_str, result_str, time_str, color, error_details, multi_version_rows)
}

//
// Text formatting utilities
//

/// Truncate string to fit width, adding "..." if truncated
fn truncate_str(s: &str, max_width: usize) -> String {
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
fn display_width(s: &str) -> usize {
    s.chars().map(|c| {
        if c.is_ascii() {
            1
        } else {
            // Approximate: most emoji are 2 cells wide
            match c {
                '✓' | '✗' | '⊘' | '↑' | '≠' | '→' | '=' => 1,
                '📦' | '📁' | '🔀' => 2,
                _ => 1,
            }
        }
    }).sum()
}

/// Truncate and pad string to exact width
fn truncate_with_padding(s: &str, width: usize) -> String {
    let display_w = display_width(s);

    if display_w > width {
        // Truncate
        let mut result = String::new();
        let mut current_width = 0;
        let mut chars: Vec<char> = s.chars().collect();

        // Reserve space for "..."
        let target_width = if width >= 3 { width - 3 } else { width };

        for c in chars.iter() {
            let c_width = if c.is_ascii() { 1 } else {
                match c {
                    '✓' | '✗' | '⊘' | '↑' | '≠' | '→' | '=' => 1,
                    '📦' | '📁' | '🔀' => 2,
                    _ => 1,
                }
            };

            if current_width + c_width > target_width {
                break;
            }

            result.push(*c);
            current_width += c_width;
        }

        if width >= 3 {
            result.push_str("...");
            current_width += 3;
        }

        // Pad if needed
        if current_width < width {
            result.push_str(&" ".repeat(width - current_width));
        }

        result
    } else {
        // Pad with spaces to reach the width
        let padding = width - display_w;
        format!("{}{}", s, " ".repeat(padding))
    }
}

//
// Summary and statistics
//

pub struct TestSummary {
    pub passed: usize,
    pub regressed: usize,
    pub broken: usize,
    pub total: usize,
}

/// Calculate summary statistics from OfferedRows
pub fn summarize_offered_rows(rows: &[OfferedRow]) -> TestSummary {
    let mut passed = 0;
    let mut regressed = 0;
    let mut broken = 0;

    for row in rows {
        // Only count non-baseline rows
        if row.offered.is_some() {
            let overall_passed = row.test.commands.iter().all(|cmd| cmd.result.passed);

            match (row.baseline_passed, overall_passed) {
                (Some(true), true) => passed += 1,      // PASSED
                (Some(true), false) => regressed += 1,  // REGRESSED
                (Some(false), _) => broken += 1,        // BROKEN
                (None, true) => passed += 1,            // PASSED (no baseline)
                (None, false) => broken += 1,           // FAILED (no baseline)
            }
        }
    }

    TestSummary {
        passed,
        regressed,
        broken,
        total: passed + regressed + broken,
    }
}

/// Print summary statistics
pub fn print_summary(summary: &TestSummary) {
    println!("\nSummary:");
    println!("  ✓ Passed:    {}", summary.passed);
    println!("  ✗ Regressed: {}", summary.regressed);
    println!("  ⚠ Broken:    {}", summary.broken);
    println!("  ━━━━━━━━━━━━━");
    println!("  Total:       {}", summary.total);
    println!();
}

//
// HTML and Markdown report generation (simplified)
//

/// Generate HTML report from OfferedRows
pub fn generate_html_report(rows: &[OfferedRow], crate_name: &str, display_version: &str, output_path: &PathBuf) -> std::io::Result<()> {
    let mut file = File::create(output_path)?;

    writeln!(file, "<!DOCTYPE html>")?;
    writeln!(file, "<html><head><meta charset='UTF-8'>")?;
    writeln!(file, "<title>Cargo Crusader Report - {}</title>", crate_name)?;
    writeln!(file, "<style>")?;
    writeln!(file, "body {{ font-family: monospace; margin: 20px; }}")?;
    writeln!(file, "table {{ border-collapse: collapse; width: 100%; }}")?;
    writeln!(file, "th, td {{ border: 1px solid #ccc; padding: 8px; text-align: left; }}")?;
    writeln!(file, ".passed {{ color: green; }}")?;
    writeln!(file, ".regressed {{ color: red; }}")?;
    writeln!(file, ".broken {{ color: orange; }}")?;
    writeln!(file, "</style></head><body>")?;
    writeln!(file, "<h1>Cargo Crusader Report</h1>")?;
    writeln!(file, "<p>Crate: <strong>{}</strong> ({})</p>", crate_name, display_version)?;
    writeln!(file, "<table><thead><tr>")?;
    writeln!(file, "<th>Offered</th><th>Spec</th><th>Resolved</th><th>Dependent</th><th>Result</th>")?;
    writeln!(file, "</tr></thead><tbody>")?;

    for row in rows {
        let (offered, spec, resolved, dependent, result, time, _, _, _) = format_offered_row(row);
        let class = if row.offered.is_some() {
            let overall_passed = row.test.commands.iter().all(|cmd| cmd.result.passed);
            match (row.baseline_passed, overall_passed) {
                (Some(true), true) => "passed",
                (Some(true), false) => "regressed",
                _ => "broken",
            }
        } else {
            ""
        };

        writeln!(file, "<tr class='{}'><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{} {}</td></tr>",
                 class, sanitize(&offered), sanitize(&spec), sanitize(&resolved),
                 sanitize(&dependent), sanitize(&result), sanitize(&time))?;
    }

    writeln!(file, "</tbody></table>")?;

    let summary = summarize_offered_rows(rows);
    writeln!(file, "<h2>Summary</h2>")?;
    writeln!(file, "<p>Passed: {}, Regressed: {}, Broken: {}</p>",
             summary.passed, summary.regressed, summary.broken)?;

    writeln!(file, "</body></html>")?;
    Ok(())
}

/// Generate Markdown report from OfferedRows
pub fn generate_markdown_report(rows: &[OfferedRow], crate_name: &str, display_version: &str, output_path: &PathBuf) -> std::io::Result<()> {
    let mut file = File::create(output_path)?;

    writeln!(file, "# Cargo Crusader Report\n")?;
    writeln!(file, "**Crate**: {} ({})\n", crate_name, display_version)?;
    writeln!(file, "## Test Results\n")?;
    writeln!(file, "| Offered | Spec | Resolved | Dependent | Result |")?;
    writeln!(file, "|---------|------|----------|-----------|--------|")?;

    for row in rows {
        let (offered, spec, resolved, dependent, result, time, _, _, _) = format_offered_row(row);
        writeln!(file, "| {} | {} | {} | {} | {} {} |",
                 offered, spec, resolved, dependent, result, time)?;
    }

    let summary = summarize_offered_rows(rows);
    writeln!(file, "\n## Summary\n")?;
    writeln!(file, "- ✓ Passed: {}", summary.passed)?;
    writeln!(file, "- ✗ Regressed: {}", summary.regressed)?;
    writeln!(file, "- ⚠ Broken: {}", summary.broken)?;
    writeln!(file, "- **Total**: {}", summary.total)?;

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

//
// Temporary compatibility stubs for old API (TO BE REMOVED)
//

/// Stub for old API - needs migration to OfferedRow
pub fn print_immediate_failure(_result: &crate::TestResult) {
    // TODO: Migrate to OfferedRow-based error printing
    eprintln!("Warning: print_immediate_failure not yet migrated to OfferedRow");
}

/// Stub for old API - needs migration to OfferedRow
pub fn print_console_table_v2(_results: &[crate::TestResult], _crate_name: &str, _display_version: &str) {
    // TODO: Migrate to OfferedRow streaming
    println!("Warning: print_console_table_v2 not yet migrated to OfferedRow");
    println!("Use: print_table_header(), print_offered_row(), print_table_footer()");
}

/// Compatibility wrapper for old API
pub fn export_markdown_report(rows: &[crate::TestResult], output_path: &PathBuf, crate_name: &str, display_version: &str) -> std::io::Result<()> {
    // TODO: Convert TestResult to OfferedRow, then call generate_markdown_report
    eprintln!("Warning: export_markdown_report needs TestResult -> OfferedRow conversion");
    Ok(())
}

/// Compatibility wrapper for old API
pub fn export_html_report(rows: Vec<crate::TestResult>, output_path: &PathBuf, crate_name: &str, display_version: &str) -> std::io::Result<TestSummary> {
    // TODO: Convert TestResult to OfferedRow, then call generate_html_report
    eprintln!("Warning: export_html_report needs TestResult -> OfferedRow conversion");
    Ok(TestSummary { passed: 0, regressed: 0, broken: 0, total: 0 })
}
