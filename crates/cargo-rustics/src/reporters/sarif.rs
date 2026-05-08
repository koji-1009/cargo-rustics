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
    let mut result = json!({
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
    });
    if let Some(j) = &v.complexity_justified {
        let basis = match j.by {
            crate::report::JustificationBasis::Line => "line",
            crate::report::JustificationBasis::Branch => "branch",
        };
        // SARIF `properties` is the canonical place for non-standard
        // tool-specific attributes — a Code Scanning reader can use
        // this to suppress / down-rank well-tested complex code.
        result["properties"] = json!({
            "complexityJustified": {
                "by": basis,
                "threshold": j.threshold,
                "actual": j.actual,
            }
        });
    }
    result
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
                complexity_justified: None,
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

    #[test]
    fn severity_word_maps_each_variant() {
        assert_eq!(severity_word(MetricSeverity::Error), "error");
        assert_eq!(severity_word(MetricSeverity::Warning), "warning");
        assert_eq!(severity_word(MetricSeverity::Info), "note");
    }

    #[test]
    fn severity_levels_render_in_results() {
        for (sev, word) in [
            (MetricSeverity::Error, "error"),
            (MetricSeverity::Warning, "warning"),
            (MetricSeverity::Info, "note"),
        ] {
            let mut r = fixture();
            r.violations[0].severity = sev;
            let v = build_sarif(&r);
            assert_eq!(v["runs"][0]["results"][0]["level"], word);
        }
    }

    #[test]
    fn write_emits_pretty_json_with_trailing_newline() {
        let mut buf = Vec::new();
        write(&fixture(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.ends_with("\n"), "expected trailing newline");
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["version"], "2.1.0");
    }

    #[test]
    fn collect_rules_orders_alphabetically() {
        let mut r = fixture();
        // Add a "z-late" lens and an "a-early" — collect_rules sorts via
        // BTreeSet so they should come out a, c, z (cyclomatic).
        r.violations.push(Violation {
            id: "z".into(),
            file: "x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "z-late".into(),
            value: 1.0,
            threshold: 0.5,
            severity: MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        });
        r.violations.push(Violation {
            id: "a".into(),
            file: "x.rs".into(),
            line: 1,
            scope: "f".into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: "a-early".into(),
            value: 1.0,
            threshold: 0.5,
            severity: MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        });
        let v = build_sarif(&r);
        let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        let ids: Vec<_> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["a-early", "cyclomatic-complexity", "z-late"]);
    }

    #[test]
    fn empty_report_emits_no_results_or_rules() {
        let mut r = fixture();
        r.violations.clear();
        let v = build_sarif(&r);
        assert!(v["runs"][0]["results"].as_array().unwrap().is_empty());
        assert!(v["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn violation_location_carries_file_and_line() {
        let v = build_sarif(&fixture());
        let loc = &v["runs"][0]["results"][0]["locations"][0]["physicalLocation"];
        assert_eq!(loc["artifactLocation"]["uri"], "src/x.rs");
        assert_eq!(loc["region"]["startLine"], 42);
    }
}
