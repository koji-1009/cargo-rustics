//! `rustics-ra` — Layer 2 HIR backend for cargo-rustics.
//!
//! Backs the `unused` detector with rust-analyzer's HIR + name
//! resolution instead of `ra_ap_syntax`'s parse-only walker. Trade:
//! full semantic accuracy (homonym disambiguation across modules,
//! method-call resolution through macro bodies) at the cost of a
//! heavy dep tree whose `0.0.x` API churns roughly monthly. Pulled
//! into the `cargo-rustics` binary via its `layer2` feature; default
//! installs ship Layer 1 only.
//!
//! The public API is intentionally minimal: [`workspace::load`]
//! opens a Cargo workspace via `ra_ap_load-cargo` and returns the
//! `AnalysisHost` + `Vfs` pair; [`unused::detect_at`] does the
//! HIR-backed walk and returns the unused finding set.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

pub mod unused;
pub mod workspace;
