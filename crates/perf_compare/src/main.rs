//! Run the file-picker performance gate by benchmarking two commits on one machine.
//!
//! The gate builds the perf-gate test binary once for the current checkout and
//! once for the commit it is based on, measures them in alternating rounds, and
//! reports the ratio between the two. Comparing two builds measured back to back
//! keeps the verdict independent of how fast the runner happens to be, which an
//! absolute duration threshold cannot manage on shared CI hardware.

// This is a batch tool whose whole job is to wait on builds and benchmark runs,
// so the editor's ban on unbounded waits does not apply to it.
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The perf-gate test that writes one measurement per invocation.
const MEASUREMENT_TEST: &str = "dialogs::file_picker::tests::test_scan_git_perf_gate";

/// How many times each build is measured when `--rounds` is not given.
const DEFAULT_ROUNDS: u32 = 3;

/// One parsed command line.
struct Arguments {
    max_ratio: f64,
    rounds: u32,
}

/// Where each build's per-round measurements are collected.
struct MeasurementDirectories {
    baseline: PathBuf,
    candidate: PathBuf,
}

/// The best result each build reached for one metric.
struct MetricComparison {
    metric: String,
    baseline_milliseconds: f64,
    candidate_milliseconds: f64,
}

impl MetricComparison {
    /// Return the candidate's cost relative to the baseline; 1.0 means unchanged.
    fn ratio(&self) -> f64 {
        self.candidate_milliseconds / self.baseline_milliseconds
    }

    /// Return whether this metric regressed.
    ///
    /// True when the candidate is slower than the baseline by more than
    /// `max_ratio`, which fails the gate. False when the candidate is within
    /// budget, which includes every improvement.
    fn is_regression(&self, max_ratio: f64) -> bool {
        self.ratio() > max_ratio
    }
}

/// Restore the checkout that was current when the gate started.
struct CheckoutGuard {
    original_reference: String,
}

impl CheckoutGuard {
    /// Remember the current branch, or the current commit when detached.
    fn new() -> Result<Self, String> {
        let original_reference = capture("git", &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .or_else(|_| capture("git", &["rev-parse", "HEAD"]))?;
        Ok(Self { original_reference })
    }

    /// Check out `commit`, leaving the guard responsible for going back.
    fn switch_to(&self, commit: &str) -> Result<(), String> {
        capture("git", &["checkout", "--detach", "--quiet", commit]).map(|_| ())
    }
}

impl Drop for CheckoutGuard {
    fn drop(&mut self) {
        // A gate that leaves the tree on the baseline commit would be far more
        // disruptive than the failure that got it there.
        let restored = capture("git", &["checkout", "--quiet", &self.original_reference]);
        if let Err(message) = restored {
            eprintln!(
                "perf_compare: could not restore {}: {message}",
                self.original_reference
            );
        }
    }
}

/// Run one command and return its trimmed standard output.
fn capture(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("run {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Parse `--max-ratio` and the optional `--rounds` from the command line.
fn parse_arguments(mut raw: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut max_ratio = None;
    let mut rounds = DEFAULT_ROUNDS;

    while let Some(flag) = raw.next() {
        // Every flag this tool accepts takes exactly one value.
        let value = raw
            .next()
            .ok_or_else(|| format!("missing value after {flag}"))?;
        match flag.as_str() {
            "--max-ratio" => {
                max_ratio = Some(
                    value
                        .parse::<f64>()
                        .map_err(|error| format!("invalid --max-ratio {value}: {error}"))?,
                );
            }
            "--rounds" => {
                rounds = value
                    .parse::<u32>()
                    .map_err(|error| format!("invalid --rounds {value}: {error}"))?;
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }

    if rounds == 0 {
        return Err("--rounds must be at least 1".to_string());
    }
    Ok(Arguments {
        max_ratio: max_ratio.ok_or_else(|| "missing --max-ratio".to_string())?,
        rounds,
    })
}

/// Return the commit to compare against, or None when there is no parent.
///
/// On a pull request the comparison point is the merge-base with the target
/// branch; on a push it is the parent commit.
fn resolve_baseline_commit() -> Result<Option<String>, String> {
    let base_reference = std::env::var("GITHUB_BASE_REF").unwrap_or_default();
    if base_reference.is_empty() {
        return Ok(capture("git", &["rev-parse", "--verify", "HEAD^"]).ok());
    }
    // Checking out a pull request head leaves no remote-tracking ref for the
    // branch it merges into, so the target branch is fetched on demand.
    capture("git", &["fetch", "--quiet", "origin", &base_reference])?;
    capture("git", &["merge-base", "FETCH_HEAD", "HEAD"]).map(Some)
}

/// Build the perf-gate test binary for the current checkout.
fn build_test_binary() -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args([
            "test",
            "--release",
            "--features",
            "perf-gates",
            "--bin",
            "ordex",
            "--no-run",
            "--message-format=json",
        ])
        .output()
        .map_err(|error| format!("run cargo test: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo test --no-run failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    // Ask cargo which executable holds the unit tests rather than guessing the
    // hash it appends to the file name.
    let mut executable = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(message) = json::parse(line) else {
            continue;
        };
        let is_test_binary = message["reason"] == "compiler-artifact"
            && message["target"]["name"] == "ordex"
            && message["profile"]["test"] == true;
        if is_test_binary {
            executable = message["executable"].as_str().map(PathBuf::from);
        }
    }
    executable.ok_or_else(|| "cargo did not report a test executable".to_string())
}

/// Copy `source` to `destination` so the next build cannot overwrite it.
fn stash_binary(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|error| {
        format!(
            "copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

/// Prepare an empty directory at `path`, discarding results from earlier runs.
fn reset_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|error| format!("clear {}: {error}", path.display()))?;
    }
    fs::create_dir_all(path).map_err(|error| format!("create {}: {error}", path.display()))
}

/// Measure both builds, alternating between them on every round.
fn measure_rounds(
    baseline_binary: &Path,
    candidate_binary: &Path,
    directories: &MeasurementDirectories,
    rounds: u32,
) -> Result<(), String> {
    for round in 1..=rounds {
        // Alternating means a runner that slows down partway through the job
        // penalizes both sides rather than only the one measured last.
        for (binary, directory) in [
            (baseline_binary, &directories.baseline),
            (candidate_binary, &directories.candidate),
        ] {
            let output_path = directory.join(format!("round-{round}.json"));
            let status = Command::new(binary)
                .args([MEASUREMENT_TEST, "--exact"])
                .env("ORDEX_PERF_OUTPUT", &output_path)
                .status()
                .map_err(|error| format!("run {}: {error}", binary.display()))?;
            if !status.success() {
                return Err(format!(
                    "{} reported a failing measurement",
                    binary.display()
                ));
            }
        }
    }
    Ok(())
}

/// Read every round in `directory`, keeping the lowest value seen per metric.
fn read_best_measurements(directory: &Path) -> Result<BTreeMap<String, f64>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("read {}: {error}", directory.display()))?;
    let mut best: BTreeMap<String, f64> = BTreeMap::new();

    for entry in entries {
        let path = entry
            .map_err(|error| format!("read {}: {error}", directory.display()))?
            .path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let parsed =
            json::parse(&contents).map_err(|error| format!("parse {}: {error}", path.display()))?;

        for (metric, value) in parsed.entries() {
            let milliseconds = value
                .as_f64()
                .ok_or_else(|| format!("{} holds a non-numeric {metric}", path.display()))?;
            // Contention only ever adds time, so the fastest round is the
            // closest estimate of what this build actually costs.
            best.entry(metric.to_string())
                .and_modify(|lowest| *lowest = f64::min(*lowest, milliseconds))
                .or_insert(milliseconds);
        }
    }
    Ok(best)
}

/// Pair every baseline metric with the candidate result for the same metric.
fn compare_measurements(
    baseline: &BTreeMap<String, f64>,
    candidate: &BTreeMap<String, f64>,
) -> Result<Vec<MetricComparison>, String> {
    baseline
        .iter()
        .map(|(metric, baseline_milliseconds)| {
            let candidate_milliseconds = candidate
                .get(metric)
                .ok_or_else(|| format!("candidate is missing metric {metric}"))?;
            // A zero baseline would make every ratio meaningless, and it can
            // only mean the measurement itself went wrong.
            if *baseline_milliseconds <= 0.0 {
                return Err(format!(
                    "baseline metric {metric} is not a positive duration"
                ));
            }
            Ok(MetricComparison {
                metric: metric.clone(),
                baseline_milliseconds: *baseline_milliseconds,
                candidate_milliseconds: *candidate_milliseconds,
            })
        })
        .collect()
}

/// Print one row per metric and return how many of them regressed.
fn report(comparisons: &[MetricComparison], max_ratio: f64) -> usize {
    println!(
        "{:<40} {:>12} {:>12} {:>8}  verdict",
        "metric", "baseline", "candidate", "ratio"
    );
    let mut regressions = 0;
    for comparison in comparisons {
        let regressed = comparison.is_regression(max_ratio);
        regressions += usize::from(regressed);
        println!(
            "{:<40} {:>9.2} ms {:>9.2} ms {:>8.3}  {}",
            comparison.metric,
            comparison.baseline_milliseconds,
            comparison.candidate_milliseconds,
            comparison.ratio(),
            if regressed { "REGRESSED" } else { "ok" }
        );
    }
    regressions
}

/// Build, measure and compare both commits, returning the number of regressions.
///
/// Returns zero when there is nothing to compare, which happens on a root commit
/// or when the baseline predates the perf gate.
fn run_gate(arguments: &Arguments) -> Result<usize, String> {
    // Switching commits would silently discard uncommitted work.
    if !capture("git", &["status", "--porcelain"])?.is_empty() {
        return Err("working tree has uncommitted changes; commit or stash first".to_string());
    }

    let Some(baseline_commit) = resolve_baseline_commit()? else {
        println!("No parent commit to compare against; skipping the perf gate.");
        return Ok(0);
    };
    println!("Comparing against {baseline_commit}");

    let directories = MeasurementDirectories {
        baseline: PathBuf::from("target/perf-gate/baseline"),
        candidate: PathBuf::from("target/perf-gate/candidate"),
    };
    reset_directory(&directories.baseline)?;
    reset_directory(&directories.candidate)?;

    let baseline_binary = PathBuf::from("target/perf-gate/baseline-binary");
    let candidate_binary = PathBuf::from("target/perf-gate/candidate-binary");

    // The scope bounds the detour onto the baseline commit: leaving it restores
    // the original checkout, so the candidate is always built from where the
    // gate started.
    let baseline_built = {
        let guard = CheckoutGuard::new()?;
        guard.switch_to(&baseline_commit)?;
        // A baseline that predates the perf gate cannot build it, which leaves
        // nothing to compare rather than being a failure.
        build_test_binary()
            .and_then(|built| stash_binary(&built, &baseline_binary))
            .is_ok()
    };

    let built = build_test_binary()?;
    stash_binary(&built, &candidate_binary)?;

    if !baseline_built {
        println!("Baseline commit cannot build the perf gate; skipping the comparison.");
        return Ok(0);
    }

    measure_rounds(
        &baseline_binary,
        &candidate_binary,
        &directories,
        arguments.rounds,
    )?;

    let baseline = read_best_measurements(&directories.baseline)?;
    if baseline.is_empty() {
        // The gate exists on the baseline under a different name, so the two
        // sides cannot be lined up.
        println!("Baseline produced no measurements; skipping the comparison.");
        return Ok(0);
    }
    let candidate = read_best_measurements(&directories.candidate)?;
    if candidate.is_empty() {
        return Err("candidate produced no measurements".to_string());
    }

    let comparisons = compare_measurements(&baseline, &candidate)?;
    let regressions = report(&comparisons, arguments.max_ratio);
    if regressions == 0 {
        println!(
            "\nall {} metric(s) within {:.2}x",
            comparisons.len(),
            arguments.max_ratio
        );
    } else {
        eprintln!(
            "\n{regressions} metric(s) slower than {:.2}x",
            arguments.max_ratio
        );
    }
    Ok(regressions)
}

/// Run the perf gate and fail when any metric regressed.
fn main() -> ExitCode {
    let arguments = match parse_arguments(std::env::args().skip(1)) {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("perf_compare: {message}");
            eprintln!("usage: perf_compare --max-ratio <float> [--rounds <count>]");
            return ExitCode::FAILURE;
        }
    };

    match run_gate(&arguments) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("perf_compare: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one measurement map from name and millisecond pairs.
    fn measurements(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
        entries
            .iter()
            .map(|(metric, milliseconds)| ((*metric).to_string(), *milliseconds))
            .collect()
    }

    #[test]
    fn test_ratio_reports_candidate_cost_relative_to_baseline() {
        let comparison = MetricComparison {
            metric: "scan".to_string(),
            baseline_milliseconds: 400.0,
            candidate_milliseconds: 500.0,
        };
        assert!((comparison.ratio() - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn test_regression_is_reported_only_beyond_the_budget() {
        let comparison = MetricComparison {
            metric: "scan".to_string(),
            baseline_milliseconds: 400.0,
            candidate_milliseconds: 500.0,
        };
        assert!(comparison.is_regression(1.20));
        assert!(!comparison.is_regression(1.25));
    }

    #[test]
    fn test_improvement_is_never_a_regression() {
        let comparison = MetricComparison {
            metric: "scan".to_string(),
            baseline_milliseconds: 400.0,
            candidate_milliseconds: 200.0,
        };
        assert!(!comparison.is_regression(1.00));
    }

    #[test]
    fn test_comparison_pairs_metrics_by_name() {
        let baseline = measurements(&[("scan", 400.0), ("filter", 10.0)]);
        let candidate = measurements(&[("filter", 12.0), ("scan", 420.0)]);
        let comparisons = compare_measurements(&baseline, &candidate).expect("compare");
        let names: Vec<&str> = comparisons
            .iter()
            .map(|comparison| comparison.metric.as_str())
            .collect();
        assert_eq!(names, ["filter", "scan"]);
        assert!((comparisons[1].ratio() - 1.05).abs() < 1e-9);
    }

    #[test]
    fn test_comparison_rejects_a_metric_the_candidate_never_reported() {
        let baseline = measurements(&[("scan", 400.0)]);
        let candidate = measurements(&[("filter", 12.0)]);
        assert!(compare_measurements(&baseline, &candidate).is_err());
    }

    #[test]
    fn test_comparison_rejects_a_zero_baseline() {
        let baseline = measurements(&[("scan", 0.0)]);
        let candidate = measurements(&[("scan", 12.0)]);
        assert!(compare_measurements(&baseline, &candidate).is_err());
    }

    #[test]
    fn test_rounds_default_when_the_flag_is_absent() {
        let arguments =
            parse_arguments(["--max-ratio", "1.2"].iter().map(|value| value.to_string()))
                .expect("parse");
        assert_eq!(arguments.rounds, DEFAULT_ROUNDS);
    }

    #[test]
    fn test_zero_rounds_is_rejected() {
        let raw = ["--max-ratio", "1.2", "--rounds", "0"];
        assert!(parse_arguments(raw.iter().map(|value| value.to_string())).is_err());
    }

    #[test]
    fn test_missing_max_ratio_is_rejected() {
        let raw = ["--rounds", "3"];
        assert!(parse_arguments(raw.iter().map(|value| value.to_string())).is_err());
    }
}
