//! `cargo rustics rules` — list built-in lenses with their metadata.
//!
//! Used by AI agents to *predict* what `analyze` will see ("if I run rustics,
//! which lenses are even on?") and by humans to learn the catalogue. The
//! output mirrors the metadata each lens already exposes through its
//! `MetricMetadata` — there is no separate doc store.

use std::io::Write;

use anyhow::{bail, Result};

use rustics::{builtin_metrics, MetricCalculator, MetricMetadata, MetricPolarity, Threshold};

use crate::cli::RulesArgs;

/// Runs the `rules` subcommand.
pub fn run(args: RulesArgs) -> Result<u8> {
    let mut out = std::io::stdout().lock();
    let metrics = builtin_metrics();
    if let Some(id) = args.metric.as_deref() {
        let Some(m) = find(&metrics, id) else {
            bail!("no metric with id `{id}`");
        };
        write_one(&mut out, m)?;
        return Ok(0);
    }
    for m in &metrics {
        write_one(&mut out, m.as_ref())?;
        writeln!(out)?;
    }
    Ok(0)
}

fn find<'a>(
    metrics: &'a [Box<dyn MetricCalculator>],
    id: &str,
) -> Option<&'a dyn MetricCalculator> {
    metrics.iter().map(AsRef::as_ref).find(|m| m.id() == id)
}

fn write_one(out: &mut dyn Write, metric: &dyn MetricCalculator) -> Result<()> {
    let md: MetricMetadata = metric.metadata();
    write_header(out, &md)?;
    write_thresholds(out, &md)?;
    write_rationale(out, md.rationale)?;
    write_string_list(out, "refactor hints", md.refactor_hints)?;
    write_string_list(out, "references", md.references)?;
    Ok(())
}

fn write_header(out: &mut dyn Write, md: &MetricMetadata) -> Result<()> {
    writeln!(out, "{} ({})", md.display_name, md.id)?;
    writeln!(out, "  category: {:?}", md.category)?;
    writeln!(out, "  polarity: {}", polarity_word(md.polarity))?;
    Ok(())
}

fn write_thresholds(out: &mut dyn Write, md: &MetricMetadata) -> Result<()> {
    write_threshold_line(out, "default warning", md.default_warning)?;
    write_threshold_line(out, "default error  ", md.default_error)?;
    Ok(())
}

fn write_threshold_line(
    out: &mut dyn Write,
    label: &str,
    threshold: Option<Threshold>,
) -> Result<()> {
    let Some(t) = threshold else {
        return Ok(());
    };
    writeln!(out, "  {label}: {}", t.value)?;
    Ok(())
}

fn write_rationale(out: &mut dyn Write, rationale: &str) -> Result<()> {
    writeln!(out, "  rationale:")?;
    for line in rationale.lines() {
        writeln!(out, "    {line}")?;
    }
    Ok(())
}

fn write_string_list(out: &mut dyn Write, label: &str, items: &[&str]) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(out, "  {label}:")?;
    for item in items {
        writeln!(out, "    - {item}")?;
    }
    Ok(())
}

fn polarity_word(p: MetricPolarity) -> &'static str {
    match p {
        MetricPolarity::LowerIsBetter => "lower-is-better",
        MetricPolarity::HigherIsBetter => "higher-is-better",
        MetricPolarity::Informational => "informational",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_one_includes_id_and_rationale() {
        let metrics = builtin_metrics();
        let m = metrics
            .iter()
            .find(|m| m.id() == "cyclomatic-complexity")
            .expect("cc lens shipped");
        let mut buf = Vec::new();
        write_one(&mut buf, m.as_ref()).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("cyclomatic-complexity"));
        assert!(s.contains("rationale"));
    }
}
