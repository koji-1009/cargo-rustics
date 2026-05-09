//! Stable violation id computation.
//!
//! The contract from:
//!
//! ```text
//! id = sha256("<file>|<scope>|<metric>")[..16]
//! ```
//!
//! Where:
//! * `<file>` is the workspace-relative path with `/` separators,
//! * `<scope>` is the AST-derived path (`module::Type::method`),
//! * `<metric>` is the kebab-case metric id.
//!
//! The id is **content-stable** — it does not depend on line numbers — so an
//! AI agent can detect "same violation persisted across refactor" from the id
//! alone.

use sha2::{Digest, Sha256};

/// Computes the 16-hex-char stable violation id.
///
/// The function is independent of `MetricInput`/`ScopeRef` types so the CLI
/// can compute ids for snapshot diffing (`regression`) without
/// reconstructing AST scopes.
///
/// # Examples
///
/// ```
/// use rustics::violation_id;
///
/// let id = violation_id("src/parser.rs", "parser::Parser::parse", "cyclomatic-complexity");
/// assert_eq!(id.len(), 16);
/// // Determinism — same inputs always produce the same id.
/// assert_eq!(
///     id,
///     violation_id("src/parser.rs", "parser::Parser::parse", "cyclomatic-complexity"),
/// );
/// ```
pub fn violation_id(file: &str, scope: &str, metric: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file.as_bytes());
    hasher.update(b"|");
    hasher.update(scope.as_bytes());
    hasher.update(b"|");
    hasher.update(metric.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        // 8 bytes -> 16 hex chars.
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_16_chars() {
        let id = violation_id("a", "b", "c");
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn id_is_lowercase_hex() {
        let id = violation_id("a", "b", "c");
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn id_is_deterministic() {
        let a = violation_id("src/parser.rs", "parser::Parser::parse", "cc");
        let b = violation_id("src/parser.rs", "parser::Parser::parse", "cc");
        assert_eq!(a, b);
    }

    #[test]
    fn id_is_content_addressed() {
        // Different file -> different id.
        let a = violation_id("src/a.rs", "scope", "cc");
        let b = violation_id("src/b.rs", "scope", "cc");
        assert_ne!(a, b);
    }

    #[test]
    fn separator_avoids_concatenation_collisions() {
        // Without the `|` separator these would hash to the same value.
        let a = violation_id("ab", "c", "d");
        let b = violation_id("a", "bc", "d");
        assert_ne!(a, b);
    }
}
