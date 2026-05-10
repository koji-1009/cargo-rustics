//! `rustics-ra` — Layer 2 spike.
//!
//! Backs cargo-rustics lenses with rust-analyzer's HIR + name
//! resolution instead of `syn`'s parse-only AST. Trade: full
//! semantic accuracy (homonym disambiguation, method-dispatch
//! resolution, macro expansion) at the cost of a heavy dep tree
//! whose `0.0.x` API churns roughly monthly.
//!
//! This module is an experimental spike. The API is intentionally
//! minimal — `analyze_workspace` opens a Cargo workspace via
//! `ra_ap_load-cargo` and returns enough HIR access for the lens
//! implementations to walk it.

#![forbid(unsafe_code)]
#![allow(missing_docs)] // spike crate; doc the API once it stabilises

pub mod cc;
pub mod unused;
pub mod workspace;
