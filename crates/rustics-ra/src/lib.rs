//! `rustics-ra` — rust-analyzer-as-library backend for cargo-rustics.
//!
//! Backs the `unused` detector and the strong-gain metric lenses
//! (per `tmp/ra-ap-spike-notes.md`) with rust-analyzer's HIR + name
//! resolution. Trade: full semantic accuracy (homonym disambiguation
//! across modules, method-call resolution through macro bodies,
//! re-export-aware coupling) at the cost of a heavy dep tree whose
//! `0.0.x` API churns roughly monthly.
//!
//! Pulled into the `cargo-rustics` binary as a regular dependency
//! — no feature gate. The unused detector + the HIR-aware metric
//! lenses are the bridge from "fast but token-only" to "slower
//! but name-resolution-aware" that the AI-loop use case warrants.
//!
//! Public surface:
//!
//! * [`workspace::load`] opens a Cargo workspace via
//!   `ra_ap_load_cargo` and returns the `AnalysisHost` + `Vfs` pair.
//! * [`unused::detect_at`] does the HIR-backed unused-public-API
//!   walk.
//! * [`metrics`] hosts per-lens HIR walkers (one submodule per
//!   migrated lens).

#![forbid(unsafe_code)]
#![allow(missing_docs)]

pub mod metrics;
pub mod unused;
pub mod workspace;
