//! `rustics` — classical and Rust-specific code metrics for the AI coding loop.
//!
//! This crate is the metric calculation core of the `cargo-rustics` tool. The
//! crate is intentionally small: only what is necessary to compute per-scope
//! metric values is exposed publicly. Report assembly, regression diffing,
//! coverage attachment, and dismissal validation are part of the CLI binary
//! and are *not* part of this crate's stable API surface in 0.1.0
//! (see).
//!
//! # Embedding
//!
//! ```no_run
//! use rustics::{CyclomaticComplexity, MetricCalculator, MetricInput};
//!
//! let source = std::fs::read_to_string("src/lib.rs").unwrap();
//! let parsed = ra_ap_syntax::SourceFile::parse(&source, ra_ap_syntax::Edition::CURRENT);
//! let tree = parsed.tree();
//! let input = MetricInput::new(std::path::Path::new("src/lib.rs"), &source, &tree);
//! let cc = CyclomaticComplexity::default();
//! for m in cc.measure(&input) {
//!     println!("{} = {}", m.scope.path, m.value);
//! }
//! ```
//!
//! # 4 段階自己完結性
//!
//! このライブラリは cargo-rustics 自身のメトリクス計算 core であり、CLI
//! 経由で `cargo rustics analyze` が自分自身に対して走る際にも同じコード
//! が走る (self-application)。

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod identifier;
mod input;
mod measurement;
mod metric;
mod metrics;
mod scope;
mod visitor;

pub use identifier::violation_id;
pub use input::MetricInput;
pub use measurement::MetricMeasurement;
pub use metric::{
    MetricCalculator, MetricCategory, MetricMetadata, MetricPolarity, MetricSeverity, Threshold,
};
pub use metrics::abstractness::Abstractness;
pub use metrics::await_depth::AwaitDepth;
pub use metrics::borrow_profile::{BorrowProfileBorrowed, BorrowProfileMut, BorrowProfileOwned};
pub use metrics::boxed_allocation_density::BoxedAllocationDensity;
pub use metrics::clone_density::CloneDensity;
pub use metrics::closure_arity::ClosureArity;
pub use metrics::cognitive_complexity::CognitiveComplexity;
pub use metrics::cyclomatic_complexity::CyclomaticComplexity;
pub use metrics::dyn_density::DynDensity;
pub use metrics::early_return_density::EarlyReturnDensity;
pub use metrics::efferent_coupling::EfferentCoupling;
pub use metrics::format_density::FormatDensity;
pub use metrics::generic_arity::GenericArity;
pub use metrics::halstead_volume::HalsteadVolume;
pub use metrics::impl_length::ImplLength;
pub use metrics::impl_trait_fanout::ImplTraitFanout;
pub use metrics::iterator_chain_length::IteratorChainLength;
pub use metrics::lcom4::Lcom4;
pub use metrics::lifetime_arity::LifetimeArity;
pub use metrics::macro_rules_arm_count::MacroRulesArmCount;
pub use metrics::match_arm_count::MatchArmCount;
pub use metrics::maximum_nesting_level::MaximumNestingLevel;
pub use metrics::npath_complexity::NpathComplexity;
pub use metrics::panic_density::PanicDensity;
pub use metrics::proc_macro_presence::ProcMacroPresence;
pub use metrics::result_chain_depth::ResultChainDepth;
pub use metrics::rfc::Rfc;
pub use metrics::source_lines_of_code::SourceLinesOfCode;
pub use metrics::trait_default_impl_ratio::TraitDefaultImplRatio;
pub use metrics::trait_method_count::TraitMethodCount;
pub use metrics::unsafe_block_scope::UnsafeBlockScope;
pub use metrics::wmc::Wmc;
pub use scope::{ScopeKind, ScopeRef};
pub use visitor::{FunctionFrame, FunctionKind, ImplFrame, TraitFrame};

/// Returns the AI-report contract version (header `# rustics ai-report v1`).
///
/// Bumped on breaking format changes — see
pub fn ai_report_contract_version() -> u32 {
    1
}

/// Factory function shape for one catalogue entry. Each closure
/// constructs one calculator and erases it to a trait object.
type MetricFactory = fn() -> Box<dyn MetricCalculator>;

/// Every built-in metric, in the order `builtin_metrics` returns
/// them. Lifted out of the function body so adding a new lens
/// does not push the body's Halstead vocabulary past the
/// 1500-token threshold (the body's only operands are now
/// `BUILTIN_METRIC_FACTORIES`, `iter`, `map`, `f`, `collect`).
const BUILTIN_METRIC_FACTORIES: &[MetricFactory] = &[
    || Box::new(CyclomaticComplexity),
    || Box::new(SourceLinesOfCode),
    || Box::new(MaximumNestingLevel),
    || Box::new(NpathComplexity),
    || Box::new(LifetimeArity),
    || Box::new(GenericArity),
    || Box::new(CloneDensity),
    || Box::new(UnsafeBlockScope),
    || Box::new(PanicDensity),
    || Box::new(ResultChainDepth),
    || Box::new(AwaitDepth),
    || Box::new(CognitiveComplexity),
    || Box::new(HalsteadVolume),
    || Box::new(ImplTraitFanout),
    || Box::new(DynDensity),
    || Box::new(Wmc),
    || Box::new(Lcom4),
    || Box::new(Rfc),
    || Box::new(ImplLength),
    || Box::new(TraitMethodCount),
    || Box::new(TraitDefaultImplRatio),
    || Box::new(MacroRulesArmCount),
    || Box::new(MatchArmCount),
    || Box::new(EfferentCoupling),
    || Box::new(Abstractness),
    || Box::new(ProcMacroPresence),
    || Box::new(BorrowProfileOwned),
    || Box::new(BorrowProfileBorrowed),
    || Box::new(BorrowProfileMut),
    || Box::new(ClosureArity),
    || Box::new(FormatDensity),
    || Box::new(IteratorChainLength),
    || Box::new(BoxedAllocationDensity),
    || Box::new(EarlyReturnDensity),
];

/// Returns every built-in metric in the catalogue, ordered by id.
///
/// New metrics added by the crate will appear in this list automatically;
/// the CLI uses it to drive `analyze` and `rules` without hard-coding ids.
pub fn builtin_metrics() -> Vec<Box<dyn MetricCalculator>> {
    BUILTIN_METRIC_FACTORIES.iter().map(|f| f()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_report_contract_version_is_one_at_m1() {
        // — `# rustics ai-report v1`. Bumping this header
        // is a contract change that should be visible in code review.
        assert_eq!(ai_report_contract_version(), 1);
    }

    #[test]
    fn builtin_metrics_have_unique_ids() {
        let ms = builtin_metrics();
        let mut ids: Vec<&'static str> = ms.iter().map(|m| m.id()).collect();
        ids.sort_unstable();
        let dedup_count = {
            let mut deduped = ids.clone();
            deduped.dedup();
            deduped.len()
        };
        assert_eq!(
            ids.len(),
            dedup_count,
            "duplicate metric ids in `builtin_metrics()`",
        );
    }

    #[test]
    fn builtin_metrics_metadata_is_well_formed() {
        for m in builtin_metrics() {
            let md = m.metadata();
            assert_eq!(md.id, m.id(), "id mismatch for {}", m.id());
            assert!(
                !md.display_name.is_empty(),
                "{} has empty display_name",
                md.id
            );
            assert!(!md.rationale.is_empty(), "{} has empty rationale", md.id);
            // `references` may be empty for lenses without an
            // academic citation (Rust-specific shape probes); the
            // field exists, the contents are best-effort.
        }
    }
}
