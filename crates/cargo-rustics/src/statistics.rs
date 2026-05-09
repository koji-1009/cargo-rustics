//! Lens-pair correlation analysis (`cargo rustics analyze --statistics`).
//!
//! Produces a Pearson `r` for every pair of lenses that have at least
//! `MIN_OBSERVATIONS` measurements in common. The output is sorted by
//! `|r|` descending so the strongest-correlated pairs surface first —
//! pairs with `|r| >= 0.95` are flagged as "redundant", a signal that
//! one of the two carries no independent information beyond the other.
//!
//! Used to gate lens-catalogue rot before M4's planned proliferation:
//! adding a new lens that turns out to be 0.97-correlated with an
//! existing one is padding, not signal.
//!
//! Implementation note: we self-implement Pearson rather than pull in
//! a stats crate. The formula is
//!
//! ```text
//! r = (Σ xy - n·x̄·ȳ) / sqrt((Σx² - n·x̄²)(Σy² - n·ȳ²))
//! ```
//!
//! and we intentionally don't bother with Spearman / partial correlations
//! at this stage — Pearson over the same scope set is enough to catch
//! the "two lenses move together" pattern.

use std::collections::BTreeMap;

use crate::report::MeasurementRecord;

/// Pairs whose `|r| >=` this threshold are flagged as redundant.
const REDUNDANT_THRESHOLD: f64 = 0.95;

/// Minimum scope-pair sample size to compute a meaningful r. With
/// fewer than 6 observations Pearson is too noisy to trust.
const MIN_OBSERVATIONS: usize = 6;

/// One lens-pair correlation row.
pub struct Correlation {
    pub a: String,
    pub b: String,
    pub r: f64,
    pub n: usize,
}

impl Correlation {
    /// True iff the |r| crosses the redundancy threshold.
    pub fn is_redundant(&self) -> bool {
        self.r.abs() >= REDUNDANT_THRESHOLD
    }
}

/// Computes pairwise Pearson correlations from the report's
/// `measurements:` block. Pairs are keyed by the metric ids
/// (alphabetical order) and the joint sample is the set of `(file,
/// scope)` pairs that have BOTH metrics present.
pub fn compute(measurements: &[MeasurementRecord]) -> Vec<Correlation> {
    let by_metric = group_by_metric(measurements);
    let mut metrics: Vec<&str> = by_metric.keys().map(|s| s.as_str()).collect();
    metrics.sort();
    let mut out = Vec::new();
    for (i, a) in metrics.iter().enumerate() {
        for b in &metrics[i + 1..] {
            if let Some(c) = pair_correlation(a, b, &by_metric) {
                out.push(c);
            }
        }
    }
    out.sort_by(|x, y| {
        y.r.abs()
            .partial_cmp(&x.r.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Index `(file, scope) -> value` per metric id.
type ScopeMap = BTreeMap<(String, String), f64>;

fn group_by_metric(records: &[MeasurementRecord]) -> BTreeMap<String, ScopeMap> {
    let mut out: BTreeMap<String, ScopeMap> = BTreeMap::new();
    for m in records {
        out.entry(m.metric.clone())
            .or_default()
            .insert((m.file.clone(), m.scope.clone()), m.value);
    }
    out
}

fn pair_correlation(
    a: &str,
    b: &str,
    by_metric: &BTreeMap<String, ScopeMap>,
) -> Option<Correlation> {
    let xs = by_metric.get(a)?;
    let ys = by_metric.get(b)?;
    let mut joined: Vec<(f64, f64)> = Vec::new();
    for (key, x) in xs {
        if let Some(y) = ys.get(key) {
            joined.push((*x, *y));
        }
    }
    if joined.len() < MIN_OBSERVATIONS {
        return None;
    }
    let r = pearson(&joined)?;
    Some(Correlation {
        a: a.to_string(),
        b: b.to_string(),
        r,
        n: joined.len(),
    })
}

/// Pearson correlation. Returns `None` when either variance is zero
/// (a constant column has no correlation defined; the standard
/// convention is "undefined", not "0", so we drop the pair from
/// output rather than reporting a misleading number).
fn pearson(samples: &[(f64, f64)]) -> Option<f64> {
    let n = samples.len() as f64;
    let (sx, sy) = samples.iter().fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let mx = sx / n;
    let my = sy / n;
    let (mut num, mut dx2, mut dy2) = (0.0, 0.0, 0.0);
    for (x, y) in samples {
        let dx = x - mx;
        let dy = y - my;
        num += dx * dy;
        dx2 += dx * dx;
        dy2 += dy * dy;
    }
    let denom = (dx2 * dy2).sqrt();
    if denom == 0.0 {
        return None;
    }
    Some(num / denom)
}

/// Renders the correlation matrix to stderr in a one-line-per-pair
/// format that's grep-friendly. Highlights redundant pairs (`*` prefix)
/// so the caller can scan for catalogue rot at a glance.
pub fn print_to_stderr(correlations: &[Correlation]) {
    if correlations.is_empty() {
        eprintln!(
            "rustics: --statistics: no metric pair has \\u{{2265}} {n} \
             observations in common (not enough scopes to estimate \
             correlation reliably).",
            n = MIN_OBSERVATIONS
        );
        return;
    }
    eprintln!(
        "rustics: lens-pair Pearson correlations (sorted by |r| desc, \
         `*` flag is |r| >= {REDUNDANT_THRESHOLD}):"
    );
    for c in correlations {
        eprintln!(
            "  {flag}  r={r:>+.3}  n={n}  {a} ↔ {b}",
            flag = if c.is_redundant() { "*" } else { " " },
            r = c.r,
            n = c.n,
            a = c.a,
            b = c.b,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(metric: &str, file: &str, scope: &str, value: f64) -> MeasurementRecord {
        MeasurementRecord {
            file: file.into(),
            scope: scope.into(),
            metric: metric.into(),
            value,
        }
    }

    #[test]
    fn pearson_perfect_positive_is_one() {
        let s: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 2.0 * i as f64 + 3.0)).collect();
        let r = pearson(&s).unwrap();
        assert!((r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pearson_perfect_negative_is_minus_one() {
        let s: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, -i as f64)).collect();
        let r = pearson(&s).unwrap();
        assert!((r + 1.0).abs() < 1e-9);
    }

    #[test]
    fn pearson_returns_none_for_constant_column() {
        let s: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 5.0)).collect();
        assert!(pearson(&s).is_none());
    }

    #[test]
    fn compute_skips_pairs_below_min_observations() {
        let records = vec![
            rec("a", "f.rs", "s1", 1.0),
            rec("b", "f.rs", "s1", 2.0),
        ];
        let out = compute(&records);
        assert!(out.is_empty(), "1-sample pair must be skipped");
    }

    #[test]
    fn compute_finds_perfect_correlation_between_scaled_metrics() {
        // A shape that fires three way: lens `a` is x, lens `b` is 2x.
        let mut records = Vec::new();
        for i in 0..MIN_OBSERVATIONS {
            let scope = format!("s{i}");
            records.push(rec("a", "f.rs", &scope, i as f64));
            records.push(rec("b", "f.rs", &scope, 2.0 * i as f64));
        }
        let out = compute(&records);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.a, "a");
        assert_eq!(c.b, "b");
        assert!(c.is_redundant());
        assert!((c.r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn compute_ignores_disjoint_scopes() {
        // Two metrics, but they don't share any (file, scope) keys.
        let mut records = Vec::new();
        for i in 0..MIN_OBSERVATIONS {
            records.push(rec("a", "x.rs", &format!("s{i}"), i as f64));
            records.push(rec("b", "y.rs", &format!("s{i}"), i as f64));
        }
        let out = compute(&records);
        assert!(out.is_empty());
    }

    #[test]
    fn compute_orders_by_absolute_r_descending() {
        let mut records = Vec::new();
        // a-b: r = +1
        for i in 0..MIN_OBSERVATIONS {
            let scope = format!("s{i}");
            records.push(rec("a", "f.rs", &scope, i as f64));
            records.push(rec("b", "f.rs", &scope, i as f64));
            // a-c: weaker correlation (mostly random)
            records.push(rec("c", "f.rs", &scope, ((i * 7) % 5) as f64));
        }
        let out = compute(&records);
        assert!(out.len() >= 2);
        // First entry must be the |r|=1 pair.
        assert!(out[0].r.abs() > out[1].r.abs());
    }
}
