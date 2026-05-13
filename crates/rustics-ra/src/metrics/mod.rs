//! HIR-backed metric implementations.
//!
//! Each submodule mirrors a metric in the `rustics` lib crate but
//! computes it with name resolution. The `cargo-rustics` binary
//! invokes these in place of the AST equivalents for the lenses
//! where HIR demonstrably gains accuracy (see
//! `tmp/ra-ap-spike-notes.md`'s triage table).

pub mod coupling_graph;
pub mod efferent_coupling;
pub mod function_complexity;
