//! `rustics` — classical and Rust-specific code metrics for the AI coding loop.
//!
//! This crate is the metric calculation core of the `cargo-rustics` tool. The
//! crate is intentionally small: only what is necessary to compute per-scope
//! metric values is exposed publicly. Report assembly, regression diffing,
//! coverage attachment, and dismissal validation are part of the CLI binary
//! and are *not* part of this crate's stable API surface in 0.1.0
//! (see plan §11.6).
//!
//! # Embedding
//!
//! ```no_run
//! use rustics::{CyclomaticComplexity, MetricCalculator, MetricInput};
//!
//! let source = std::fs::read_to_string("src/lib.rs").unwrap();
//! let ast = syn::parse_file(&source).unwrap();
//! let input = MetricInput::new(std::path::Path::new("src/lib.rs"), &source, &ast);
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
pub use metrics::await_depth::AwaitDepth;
pub use metrics::clone_density::CloneDensity;
pub use metrics::cognitive_complexity::CognitiveComplexity;
pub use metrics::cyclomatic_complexity::CyclomaticComplexity;
pub use metrics::dyn_density::DynDensity;
pub use metrics::generic_arity::GenericArity;
pub use metrics::halstead_volume::HalsteadVolume;
pub use metrics::impl_length::ImplLength;
pub use metrics::impl_method_count::ImplMethodCount;
pub use metrics::impl_trait_fanout::ImplTraitFanout;
pub use metrics::lifetime_arity::LifetimeArity;
pub use metrics::maximum_nesting_level::MaximumNestingLevel;
pub use metrics::method_length::MethodLength;
pub use metrics::number_of_parameters::NumberOfParameters;
pub use metrics::panic_density::PanicDensity;
pub use metrics::result_chain_depth::ResultChainDepth;
pub use metrics::source_lines_of_code::SourceLinesOfCode;
pub use metrics::trait_default_impl_ratio::TraitDefaultImplRatio;
pub use metrics::trait_method_count::TraitMethodCount;
pub use metrics::unsafe_block_scope::UnsafeBlockScope;
pub use scope::{ScopeKind, ScopeRef};
pub use visitor::{
    walk_functions, walk_impls, walk_traits, FunctionFrame, FunctionKind, ImplFrame, TraitFrame,
};

/// Returns the version of the `rustics` library.
///
/// Useful for cross-checking that an embedding host pinned the same crate
/// version that the CLI was built against. Wired through `Cargo.toml`'s
/// `CARGO_PKG_VERSION` at compile time.
pub fn rustics_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Returns the AI-report contract version (header `# rustics ai-report v1`).
///
/// Bumped on breaking format changes — see plan §4.1.
pub fn ai_report_contract_version() -> u32 {
    1
}

/// Returns every built-in metric in the M1 catalogue, ordered by id.
///
/// New metrics added by the crate will appear in this list automatically;
/// the CLI uses it to drive `analyze` and `rules` without hard-coding ids.
pub fn builtin_metrics() -> Vec<Box<dyn MetricCalculator>> {
    vec![
        Box::new(CyclomaticComplexity),
        Box::new(SourceLinesOfCode),
        Box::new(MethodLength),
        Box::new(NumberOfParameters),
        Box::new(MaximumNestingLevel),
        Box::new(LifetimeArity),
        Box::new(GenericArity),
        Box::new(CloneDensity),
        Box::new(UnsafeBlockScope),
        Box::new(PanicDensity),
        Box::new(ResultChainDepth),
        Box::new(AwaitDepth),
        Box::new(CognitiveComplexity),
        Box::new(HalsteadVolume),
        Box::new(ImplTraitFanout),
        Box::new(DynDensity),
        Box::new(ImplMethodCount),
        Box::new(ImplLength),
        Box::new(TraitMethodCount),
        Box::new(TraitDefaultImplRatio),
    ]
}
