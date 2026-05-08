//! SARIF v2.1.0 reporter.
//!
//! Plan §7.2. SARIF is the static-analysis interchange format that
//! GitHub Code Scanning, Azure DevOps, and other CI dashboards consume.
//! We emit the minimal valid v2.1.0 shape: one `run` with one `tool`
//! and one `result` per violation.
//!
//! We do not pull in a SARIF crate — the schema we emit is small and
//! stable, and the test golden file would catch any drift. Plan §1.8
//! "dependencies are paid for" applies.

use std::io::Write;

use anyhow::Result;

use rustics::MetricSeverity;
use serde_json::{json, Value};

use crate::report::{Report, Violation};

/// Writes the SARIF v2.1.0 form of `report` to `out`.
pub fn write(report: &Report, out: &mut dyn Write) -> Result<()> {
    let value = build_sarif(report);
    let s = serde_json::to_string_pretty(&value)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

fn build_sarif(report: &Report) -> Value {
    json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "cargo-rustics",
                    "informationUri": "https://github.com/kojiwakamiya/cargo-rustics",
                    "rules": collect_rules(&report.violations),
                }
            },
            "results": report.violations.iter().map(violation_to_result).collect::<Vec<_>>(),
        }]
    })
}

fn collect_rules(violations: &[Violation]) -> Vec<Value> {
    let mut seen = std::collections::BTreeSet::new();
    for v in violations {
        seen.insert(v.metric.as_str());
    }
    seen.into_iter()
        .map(|metric| {
            json!({
                "id": metric,
                "name": metric,
                "shortDescription": { "text": metric },
                "helpUri": format!(
                    "https://github.com/kojiwakamiya/cargo-rustics/blob/main/doc/manual.md#{}",
                    metric
                ),
            })
        })
        .collect()
}

fn violation_to_result(v: &Violation) -> Value {
    json!({
        "ruleId": v.metric,
        "level": severity_word(v.severity),
        "message": {
            "text": format!(
                "{metric} = {value} (threshold {threshold}) — scope {scope}",
                metric = v.metric,
                value = v.value,
                threshold = v.threshold,
                scope = v.scope,
            )
        },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": v.file },
                "region": { "startLine": v.line }
            }
        }],
        "fingerprints": { "rusticsId": v.id },
    })
}

fn severity_word(s: MetricSeverity) -> &'static str {
    // SARIF levels: "none" / "note" / "warning" / "error".
    match s {
        MetricSeverity::Error => "error",
        MetricSeverity::Warning => "warning",
        MetricSeverity::Info => "note",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Summary, Violation};
    use rustics::ScopeKind;

    fn fixture() -> Report {
        Report {
            version: 1,
            generated_at: "T".into(),
            summary: Summary {
                files_analyzed: 1,
                violations: 1,
                warnings: 1,
                errors: 0,
            },
            violations: vec![Violation {
                id: "abc".into(),
                file: "src/x.rs".into(),
                line: 42,
                scope: "f".into(),
                scope_kind: ScopeKind::FreeFunction,
                metric: "cyclomatic-complexity".into(),
                value: 14.0,
                threshold: 10.0,
                severity: MetricSeverity::Warning,
                rationale: None,
                refactor_hints: vec![],
                references: vec![],
                rust_context: Default::default(),
            }],
            truncated: 0,
            measurements: vec![],
        }
    }

    #[test]
    fn sarif_has_required_top_level_fields() {
        let v = build_sarif(&fixture());
        assert_eq!(v["version"], "2.1.0");
        assert!(v["$schema"].is_string());
        assert!(v["runs"].is_array());
    }

    #[test]
    fn driver_name_is_cargo_rustics() {
        let v = build_sarif(&fixture());
        assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "cargo-rustics");
    }

    #[test]
    fn each_violation_becomes_a_result() {
        let v = build_sarif(&fixture());
        let results = v["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["ruleId"], "cyclomatic-complexity");
        assert_eq!(results[0]["level"], "warning");
        assert_eq!(results[0]["fingerprints"]["rusticsId"], "abc");
    }

    #[test]
    fn rules_are_deduplicated() {
        let mut report = fixture();
        report.violations.push(report.violations[0].clone());
        let v = build_sarif(&report);
        let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
    }
}
