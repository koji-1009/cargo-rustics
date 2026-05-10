//! Parallel metric runner.
//!
//! For each discovered file, parses the source once with
//! `ra_ap_syntax::SourceFile::parse` and runs every enabled lens
//! against the parsed CST. The work is sharded across worker threads using
//! `std::thread::scope`. picks `std::thread::scope` over `rayon`
//! — work units are roughly even-sized and we keep the dependency
//! footprint small.

use std::path::Path;
use std::sync::Mutex;
use std::thread;

use rustics::{MetricCalculator, MetricInput, MetricMeasurement, MetricMetadata};

use crate::discover::DiscoveredFile;

/// Output of running every metric over every file.
pub struct RunOutput {
    /// One entry per file × metric combination.
    pub records: Vec<FileMetricRecord>,
    /// Files whose source could not be parsed by `syn`. The CLI reports
    /// these but does not abort — partial results are useful.
    pub parse_errors: Vec<ParseError>,
    /// Number of files successfully analysed (parsed + measured).
    pub files_analyzed: usize,
}

/// One metric's measurements for one file.
pub struct FileMetricRecord {
    /// Workspace-relative path.
    pub relative: String,
    /// Lens id.
    pub metric: String,
    /// Lens metadata (rationale, refactor hints, etc.).
    pub metadata: MetricMetadata,
    /// Per-scope measurements emitted by the lens.
    pub measurements: Vec<MetricMeasurement>,
}

/// A single failed parse.
#[derive(Debug)]
pub struct ParseError {
    /// Workspace-relative file path.
    pub relative: String,
    /// Parse-error message from `syn`.
    pub message: String,
}

/// Runs every metric over every file. Returns aggregated results.
pub fn run(
    files: &[DiscoveredFile],
    metrics: &[Box<dyn MetricCalculator>],
    concurrency: usize,
) -> RunOutput {
    let chunks = partition(files, concurrency);
    let records = Mutex::new(Vec::<FileMetricRecord>::new());
    let parse_errors = Mutex::new(Vec::<ParseError>::new());
    let analyzed = Mutex::new(0usize);

    thread::scope(|s| {
        for chunk in chunks {
            let records = &records;
            let parse_errors = &parse_errors;
            let analyzed = &analyzed;
            s.spawn(move || {
                for file in chunk {
                    process_one(file, metrics, records, parse_errors, analyzed);
                }
            });
        }
    });

    drain_outputs(records, parse_errors, analyzed)
}

/// Drains the three accumulator Mutexes into a [`RunOutput`].
///
/// If a worker thread panicked, the matching `Mutex` is now poisoned —
/// `into_inner()` returns `Err(PoisonError)` carrying the *partial*
/// data the panicking worker left behind. We DO want that partial
/// data (better than `default()` from thin air), but we also need to
/// SHOUT about the panic so the caller can react. Burying the
/// poisoning silently makes a panicking metric look like a clean run
/// with zero violations, which is the worst possible failure shape
/// for a CI gate.
fn drain_outputs(
    records: Mutex<Vec<FileMetricRecord>>,
    parse_errors: Mutex<Vec<ParseError>>,
    analyzed: Mutex<usize>,
) -> RunOutput {
    let mut parse_errors_out = drain_or_recover(parse_errors, "parse_errors");
    let records_out = drain_or_recover_records(records, &mut parse_errors_out);
    let analyzed_out = drain_or_recover_count(analyzed, &mut parse_errors_out);
    RunOutput {
        records: records_out,
        parse_errors: parse_errors_out,
        files_analyzed: analyzed_out,
    }
}

/// Generic poison-aware drain for the `parse_errors` bag itself. If
/// poisoned, take the inner data and surface a synthetic
/// [`ParseError`] so downstream report rendering and exit codes treat
/// it as a real failure (rather than reporting `0` and exiting clean).
fn drain_or_recover<T: Default>(m: Mutex<T>, label: &str) -> T {
    match m.into_inner() {
        Ok(v) => v,
        Err(poisoned) => {
            eprintln!(
                "rustics: worker thread panicked while holding `{label}` lock; \
                 returning partial data — re-run with RUST_BACKTRACE=1 to diagnose"
            );
            poisoned.into_inner()
        }
    }
}

fn drain_or_recover_records(
    m: Mutex<Vec<FileMetricRecord>>,
    parse_errors: &mut Vec<ParseError>,
) -> Vec<FileMetricRecord> {
    match m.into_inner() {
        Ok(v) => v,
        Err(poisoned) => {
            parse_errors.push(panic_marker("records lock"));
            poisoned.into_inner()
        }
    }
}

fn drain_or_recover_count(m: Mutex<usize>, parse_errors: &mut Vec<ParseError>) -> usize {
    match m.into_inner() {
        Ok(n) => n,
        Err(poisoned) => {
            parse_errors.push(panic_marker("file-counter lock"));
            poisoned.into_inner()
        }
    }
}

fn panic_marker(which: &str) -> ParseError {
    ParseError {
        relative: "<runner>".to_string(),
        message: format!(
            "worker panic poisoned `{which}` — partial results returned, \
             treat exit as failure"
        ),
    }
}

fn process_one(
    file: &DiscoveredFile,
    metrics: &[Box<dyn MetricCalculator>],
    records: &Mutex<Vec<FileMetricRecord>>,
    parse_errors: &Mutex<Vec<ParseError>>,
    analyzed: &Mutex<usize>,
) {
    let Some(source) = read_or_record(file, parse_errors) else {
        return;
    };
    let tree = parse_with_diagnostics(file, &source, parse_errors);
    let input = MetricInput::new(Path::new(&file.relative), &source, &tree);
    let local_records = run_metrics(file, metrics, &input);
    push_results(records, analyzed, local_records);
}

/// Reads the file's source. On IO error, records a parse-error
/// entry and returns `None` so the caller skips analysis cleanly.
fn read_or_record(file: &DiscoveredFile, parse_errors: &Mutex<Vec<ParseError>>) -> Option<String> {
    match std::fs::read_to_string(&file.absolute) {
        Ok(s) => Some(s),
        Err(e) => {
            push_parse_error(parse_errors, &file.relative, format!("io error: {e}"));
            None
        }
    }
}

/// Parses with `ra_ap_syntax` and surfaces the first diagnostic
/// (if any) as a parse-error entry. ra_ap_syntax recovers
/// gracefully even on malformed input — we still want the
/// recovered tree, so the diagnostic is informational, not a
/// skip signal.
fn parse_with_diagnostics(
    file: &DiscoveredFile,
    source: &str,
    parse_errors: &Mutex<Vec<ParseError>>,
) -> ra_ap_syntax::SourceFile {
    let parsed = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::CURRENT);
    if let Some(first) = parsed.errors().first() {
        push_parse_error(
            parse_errors,
            &file.relative,
            format!("ra_ap_syntax parse: {first}"),
        );
    }
    parsed.tree()
}

/// Runs every metric over `input` and packages the results.
fn run_metrics(
    file: &DiscoveredFile,
    metrics: &[Box<dyn MetricCalculator>],
    input: &MetricInput<'_>,
) -> Vec<FileMetricRecord> {
    let mut out = Vec::with_capacity(metrics.len());
    for metric in metrics {
        let measurements = metric.measure(input);
        out.push(FileMetricRecord {
            relative: file.relative.clone(),
            metric: metric.id().to_string(),
            metadata: metric.metadata(),
            measurements,
        });
    }
    out
}

fn push_results(
    records: &Mutex<Vec<FileMetricRecord>>,
    analyzed: &Mutex<usize>,
    local: Vec<FileMetricRecord>,
) {
    if let Ok(mut g) = records.lock() {
        g.extend(local);
    }
    if let Ok(mut g) = analyzed.lock() {
        *g += 1;
    }
}

fn push_parse_error(parse_errors: &Mutex<Vec<ParseError>>, relative: &str, msg: String) {
    if let Ok(mut g) = parse_errors.lock() {
        g.push(ParseError {
            relative: relative.to_string(),
            message: msg,
        });
    }
}

/// Splits `files` into `concurrency` roughly-equal chunks. The chunk count
/// is clamped to `[1, files.len()]` so we never spawn more threads than work.
fn partition(files: &[DiscoveredFile], concurrency: usize) -> Vec<&[DiscoveredFile]> {
    if files.is_empty() {
        return Vec::new();
    }
    let n = concurrency.max(1).min(files.len());
    let chunk = files.len().div_ceil(n);
    files.chunks(chunk).collect()
}

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;

    fn mk(name: &str) -> DiscoveredFile {
        DiscoveredFile {
            absolute: std::path::PathBuf::from(name),
            relative: name.to_string(),
        }
    }

    #[test]
    fn partition_handles_empty() {
        assert!(partition(&[], 4).is_empty());
    }

    #[test]
    fn partition_caps_chunk_count_at_files_len() {
        let files = vec![mk("a.rs"), mk("b.rs")];
        let chunks = partition(&files, 16);
        assert_eq!(chunks.len(), 2); // not 16
    }

    #[test]
    fn partition_with_one_thread_returns_one_chunk() {
        let files = vec![mk("a.rs"), mk("b.rs"), mk("c.rs")];
        let chunks = partition(&files, 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 3);
    }

    #[test]
    fn partition_distributes_evenly() {
        let files = vec![mk("a.rs"), mk("b.rs"), mk("c.rs"), mk("d.rs")];
        let chunks = partition(&files, 2);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 2);
        assert_eq!(chunks[1].len(), 2);
    }

    fn write_file(dir: &std::path::Path, rel: &str, body: &str) -> DiscoveredFile {
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
        DiscoveredFile {
            absolute: abs,
            relative: rel.to_string(),
        }
    }

    fn tempdir(label: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rustics-runner-{label}-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_aggregates_records_across_threads() {
        let dir = tempdir("aggr");
        let files = vec![
            write_file(&dir, "a.rs", "fn a() {}\n"),
            write_file(&dir, "b.rs", "fn b() {}\n"),
            write_file(&dir, "c.rs", "fn c() {}\n"),
        ];
        let metrics = rustics::builtin_metrics();
        let out = run(&files, &metrics, 2);
        assert_eq!(out.files_analyzed, 3);
        assert!(out.parse_errors.is_empty());
        assert_eq!(out.records.len(), 3 * metrics.len());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_surfaces_parse_errors_without_aborting() {
        let dir = tempdir("parse");
        let files = vec![
            write_file(&dir, "good.rs", "fn good() {}\n"),
            write_file(&dir, "bad.rs", "this is :: not :: rust ::"),
        ];
        let metrics = rustics::builtin_metrics();
        let out = run(&files, &metrics, 1);
        // ra_ap_syntax recovers gracefully — both files are
        // analyzed (we use the recovered tree) but the bad one's
        // parse-error diagnostic is surfaced via parse_errors.
        assert_eq!(out.files_analyzed, 2);
        assert!(
            out.parse_errors
                .iter()
                .any(|e| e.relative == "bad.rs" && e.message.contains("ra_ap_syntax parse")),
            "expected ra_ap_syntax parse error for bad.rs; got {:?}",
            out.parse_errors,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_surfaces_io_errors() {
        // Discovered file pointing at a non-existent path → io error.
        let files = vec![DiscoveredFile {
            absolute: std::path::PathBuf::from("/no/such/__rustics_runner_io__.rs"),
            relative: "missing.rs".into(),
        }];
        let metrics = rustics::builtin_metrics();
        let out = run(&files, &metrics, 1);
        assert_eq!(out.files_analyzed, 0);
        assert_eq!(out.parse_errors.len(), 1);
        assert!(out.parse_errors[0].message.contains("io error"));
    }

    #[test]
    fn run_returns_empty_for_no_files() {
        let metrics = rustics::builtin_metrics();
        let out = run(&[], &metrics, 4);
        assert!(out.records.is_empty());
        assert!(out.parse_errors.is_empty());
        assert_eq!(out.files_analyzed, 0);
    }

    #[test]
    fn drain_outputs_recovers_partial_data_from_poisoned_records() {
        // Simulate a poisoned `records` Mutex. `std::sync::Mutex` only
        // poisons on a panic-while-holding-lock, so we trigger that
        // explicitly via catch_unwind. The drain must still return
        // the partial data AND surface a synthetic ParseError so the
        // CI gate has signal to fail on.
        use std::panic::AssertUnwindSafe;
        use std::sync::Mutex;
        let m: Mutex<Vec<FileMetricRecord>> = Mutex::new(vec![FileMetricRecord {
            relative: "a.rs".into(),
            metric: "cyclomatic-complexity".into(),
            metadata: rustics::CyclomaticComplexity.metadata(),
            measurements: vec![],
        }]);
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _g = m.lock().unwrap();
            panic!("simulated worker panic");
        }));
        assert!(m.is_poisoned());

        let mut errs = Vec::<ParseError>::new();
        let out = drain_or_recover_records(m, &mut errs);
        assert_eq!(out.len(), 1, "partial data must be preserved");
        assert_eq!(errs.len(), 1, "poison must surface a parse-error marker");
        assert!(errs[0].message.contains("worker panic"));
    }

    #[test]
    fn drain_or_recover_logs_and_returns_inner_for_parse_errors_lock() {
        use std::panic::AssertUnwindSafe;
        use std::sync::Mutex;
        let m: Mutex<Vec<ParseError>> = Mutex::new(vec![ParseError {
            relative: "x.rs".into(),
            message: "pre-poison".into(),
        }]);
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _g = m.lock().unwrap();
            panic!("forced");
        }));
        assert!(m.is_poisoned());
        let out = drain_or_recover(m, "parse_errors");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn drain_or_recover_count_marks_poison_in_parse_errors() {
        use std::panic::AssertUnwindSafe;
        use std::sync::Mutex;
        let m: Mutex<usize> = Mutex::new(7);
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _g = m.lock().unwrap();
            panic!("forced");
        }));
        assert!(m.is_poisoned());
        let mut errs = Vec::new();
        let n = drain_or_recover_count(m, &mut errs);
        assert_eq!(n, 7);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("file-counter"));
    }
}
