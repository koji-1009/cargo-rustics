//! Parallel metric runner.
//!
//! For each discovered file, parses the source once with `syn::parse_file`
//! (plan §3.3 — AST reparse forbidden) and runs every enabled lens
//! against the parsed AST. The work is sharded across worker threads using
//! `std::thread::scope`. Plan §3.4 picks `std::thread::scope` over `rayon`
//! for M1 — work units are roughly even-sized and we keep the dependency
//! footprint small (plan §1.8).

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

    RunOutput {
        records: records.into_inner().unwrap_or_default(),
        parse_errors: parse_errors.into_inner().unwrap_or_default(),
        files_analyzed: analyzed.into_inner().unwrap_or(0),
    }
}

fn process_one(
    file: &DiscoveredFile,
    metrics: &[Box<dyn MetricCalculator>],
    records: &Mutex<Vec<FileMetricRecord>>,
    parse_errors: &Mutex<Vec<ParseError>>,
    analyzed: &Mutex<usize>,
) {
    let source = match std::fs::read_to_string(&file.absolute) {
        Ok(s) => s,
        Err(e) => {
            push_parse_error(parse_errors, &file.relative, format!("io error: {e}"));
            return;
        }
    };
    let ast = match syn::parse_file(&source) {
        Ok(ast) => ast,
        Err(e) => {
            push_parse_error(parse_errors, &file.relative, format!("syn parse: {e}"));
            return;
        }
    };
    let input = MetricInput::new(Path::new(&file.relative), &source, &ast);
    let mut local_records = Vec::with_capacity(metrics.len());
    for metric in metrics {
        let measurements = metric.measure(&input);
        local_records.push(FileMetricRecord {
            relative: file.relative.clone(),
            metric: metric.id().to_string(),
            metadata: metric.metadata(),
            measurements,
        });
    }
    if let Ok(mut g) = records.lock() {
        g.extend(local_records);
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
        let dir =
            std::env::temp_dir().join(format!("rustics-runner-{label}-{pid}-{n}"));
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
        assert_eq!(out.files_analyzed, 1);
        assert_eq!(out.parse_errors.len(), 1);
        assert_eq!(out.parse_errors[0].relative, "bad.rs");
        assert!(out.parse_errors[0].message.contains("syn parse"));
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
}
