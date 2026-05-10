//! JSON reporter — machine pipe.
//!
//! Pretty-printed by default so a `> file.json` is greppable. The shape is
//! the `Report` struct itself — see `schemas/rustics-report.schema.json`
//! for the contract.

use std::io::Write;

use anyhow::Result;

use crate::report::Report;

/// Writes `report` as pretty JSON to `out`.
pub fn write(report: &Report, out: &mut dyn Write) -> Result<()> {
    let s = serde_json::to_string_pretty(report)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Summary;

    #[test]
    fn empty_report_round_trips() {
        let r = Report {
            version: 1,
            generated_at: "t".into(),
            summary: Summary {
                files_analyzed: 0,
                violations: 0,
                warnings: 0,
                errors: 0,
                warnings_justified: 0,
                errors_justified: 0,
            },
            violations: vec![],
            truncated: 0,
            measurements: vec![],
            stale_dismissals: vec![],
            unused: vec![],
        };
        let mut buf = Vec::new();
        write(&r, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("\"version\": 1"));
        assert!(s.contains("\"violations\": []"));
    }
}
