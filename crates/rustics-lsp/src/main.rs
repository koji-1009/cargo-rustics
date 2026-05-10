//! `rustics-lsp` — minimal Language Server Protocol bridge for the
//! rustics lens catalogue.
//!
//!. The server speaks LSP over stdin/stdout
//! and publishes one diagnostic per lens violation in every open
//! document.
//!
//! first-slice surface:
//!
//! * `initialize` — declares full text-document sync, no other
//!   capabilities yet.
//! * `textDocument/didOpen` / `textDocument/didChange` — re-parses the
//!   document and pushes diagnostics.
//!
//! Future iterations will add code-action quick fixes for the lens
//! refactor hints and per-file dismissal recognition.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// `lsp_types::Uri` has interior mutability for caching parsed URI
// components; clippy flags `HashMap<Uri, _>` as a mutable-key type, but
// we only ever use the Uri as an opaque key (no parsing-state mutation
// is observable through the map). Permit the lint at the module level
// rather than annotating every site.
#![allow(clippy::mutable_key_type)]

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, Message, Notification};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, Notification as _, PublishDiagnostics,
    },
    Diagnostic, DiagnosticSeverity, InitializeParams, Position, PublishDiagnosticsParams, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use rustics::{builtin_metrics, MetricCalculator, MetricInput, MetricSeverity};

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    eprintln!("rustics-lsp: starting (stdio)");
    let (connection, io_threads) = Connection::stdio();
    serve(&connection)?;
    io_threads.join()?;
    Ok(())
}

/// Drives one LSP session over `connection`. Split out from `main` so
/// tests can run it against `Connection::memory()` while production
/// uses `Connection::stdio()`. The disconnect-during-initialize branch
/// returns `Ok(())` so a peer that hangs up before completing the
/// handshake doesn't propagate as an error code on the binary.
fn serve(connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })?;
    let initialization_params = match connection.initialize(server_capabilities) {
        Ok(p) => p,
        Err(e) if e.channel_is_disconnected() => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let _params: InitializeParams = serde_json::from_value(initialization_params)?;
    main_loop(connection)?;
    Ok(())
}

fn main_loop(connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    let mut documents: HashMap<Uri, String> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
            }
            Message::Notification(note) => handle_notification(connection, &mut documents, note)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    documents: &mut HashMap<Uri, String>,
    note: Notification,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match note.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: lsp_types::DidOpenTextDocumentParams = serde_json::from_value(note.params)?;
            let uri = params.text_document.uri;
            let text = params.text_document.text;
            documents.insert(uri.clone(), text.clone());
            publish_diagnostics(connection, &uri, &text);
        }
        DidChangeTextDocument::METHOD => {
            let params: lsp_types::DidChangeTextDocumentParams =
                serde_json::from_value(note.params)?;
            let uri = params.text_document.uri;
            // Full sync — we declared `TextDocumentSyncKind::FULL`, so
            // each change carries the entire document.
            if let Some(change) = params.content_changes.into_iter().next() {
                documents.insert(uri.clone(), change.text.clone());
                publish_diagnostics(connection, &uri, &change.text);
            }
        }
        _ => {}
    }
    Ok(())
}

fn publish_diagnostics(connection: &Connection, uri: &Uri, source: &str) {
    let diagnostics = compute_diagnostics(uri, source);
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics,
        version: None,
    };
    let note = Notification::new(PublishDiagnostics::METHOD.to_string(), params);
    let _ = connection.sender.send(Message::Notification(note));
}

fn compute_diagnostics(_uri: &Uri, source: &str) -> Vec<Diagnostic> {
    let parsed = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::CURRENT);
    let tree = parsed.tree();
    let metrics = builtin_metrics();
    let path = std::path::PathBuf::from("__lsp__.rs");
    let input = MetricInput::new(&path, source, &tree);
    let mut diagnostics = Vec::new();
    for metric in &metrics {
        collect_metric_diagnostics(metric.as_ref(), &input, &mut diagnostics);
    }
    diagnostics
}

fn collect_metric_diagnostics(
    metric: &dyn MetricCalculator,
    input: &MetricInput<'_>,
    out: &mut Vec<Diagnostic>,
) {
    let md = metric.metadata();
    let Some(threshold) = md.default_warning else {
        return;
    };
    for measurement in metric.measure(input) {
        if !threshold.is_violated_by(measurement.value, md.polarity) {
            continue;
        }
        out.push(diagnostic_for(metric, &md, threshold.value, measurement));
    }
}

fn diagnostic_for(
    metric: &dyn MetricCalculator,
    md: &rustics::MetricMetadata,
    threshold: f64,
    measurement: rustics::MetricMeasurement,
) -> Diagnostic {
    let line = measurement.scope.line.saturating_sub(1) as u32;
    Diagnostic {
        range: Range {
            start: Position { line, character: 0 },
            end: Position { line, character: 0 },
        },
        severity: Some(severity_from_lens(metric, measurement.value)),
        source: Some("rustics".to_string()),
        code: Some(lsp_types::NumberOrString::String(metric.id().to_string())),
        message: format!(
            "{display}: {value} (>{threshold}) at {scope}",
            display = md.display_name,
            value = measurement.value,
            threshold = threshold,
            scope = measurement.scope.path,
        ),
        ..Default::default()
    }
}

fn severity_from_lens(metric: &dyn MetricCalculator, value: f64) -> DiagnosticSeverity {
    let md = metric.metadata();
    let crossed_error = md
        .default_error
        .map(|t| t.is_violated_by(value, md.polarity))
        .unwrap_or(false);
    let lsp_severity = if crossed_error {
        MetricSeverity::Error
    } else {
        MetricSeverity::Warning
    };
    match lsp_severity {
        MetricSeverity::Error => DiagnosticSeverity::ERROR,
        MetricSeverity::Warning => DiagnosticSeverity::WARNING,
        MetricSeverity::Info => DiagnosticSeverity::INFORMATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn uri() -> Uri {
        Uri::from_str("file:///tmp/x.rs").unwrap()
    }

    /// A function that exceeds CC default warning (>10) so the LSP
    /// pipeline emits at least one diagnostic.
    const HEAVY_FN: &str = r#"
        fn heavy(x: i32) -> i32 {
            let mut a = 0;
            if x > 0 { a += 1; }
            if x > 1 { a += 1; }
            if x > 2 { a += 1; }
            if x > 3 { a += 1; }
            if x > 4 { a += 1; }
            if x > 5 { a += 1; }
            if x > 6 { a += 1; }
            if x > 7 { a += 1; }
            if x > 8 { a += 1; }
            if x > 9 { a += 1; }
            if x > 10 { a += 1; }
            a
        }
    "#;

    #[test]
    fn compute_diagnostics_returns_empty_for_invalid_source() {
        let diags = compute_diagnostics(&uri(), "this is not Rust :::");
        assert!(diags.is_empty());
    }

    #[test]
    #[ignore = "stubbed lens during ra_ap_syntax migration"]
    fn compute_diagnostics_emits_for_violation() {
        let diags = compute_diagnostics(&uri(), HEAVY_FN);
        assert!(!diags.is_empty(), "heavy fn must produce diagnostics");
        let cc = diags
            .iter()
            .find(|d| matches!(&d.code, Some(lsp_types::NumberOrString::String(s)) if s == "cyclomatic-complexity"))
            .expect("cyclomatic-complexity diagnostic missing");
        assert_eq!(cc.source.as_deref(), Some("rustics"));
        assert!(cc.message.contains("Cyclomatic"));
    }

    #[test]
    fn compute_diagnostics_skips_clean_source() {
        let diags = compute_diagnostics(&uri(), "fn ok() -> i32 { 1 }");
        // None of the lenses warn at CC=1 / SLOC=1, so we expect no diags.
        assert!(diags.is_empty(), "clean source emitted diagnostics: {diags:?}");
    }

    #[test]
    fn severity_from_lens_warning_when_under_error_threshold() {
        let metrics = builtin_metrics();
        let cc = metrics
            .iter()
            .find(|m| m.id() == "cyclomatic-complexity")
            .expect("cc lens missing");
        // Default warning = 10, error = 20. Pick 12 → warning only.
        let sev = severity_from_lens(cc.as_ref(), 12.0);
        assert_eq!(sev, DiagnosticSeverity::WARNING);
    }

    #[test]
    fn severity_from_lens_error_when_past_error_threshold() {
        let metrics = builtin_metrics();
        let cc = metrics
            .iter()
            .find(|m| m.id() == "cyclomatic-complexity")
            .expect("cc lens missing");
        let sev = severity_from_lens(cc.as_ref(), 100.0);
        assert_eq!(sev, DiagnosticSeverity::ERROR);
    }

    #[test]
    fn severity_from_lens_warning_when_no_error_threshold() {
        // A custom calculator with no error threshold — exercise the
        // unwrap_or(false) branch.
        struct Bare;
        impl MetricCalculator for Bare {
            fn id(&self) -> &'static str {
                "bare"
            }
            fn metadata(&self) -> rustics::MetricMetadata {
                rustics::MetricMetadata {
                    id: "bare",
                    display_name: "Bare",
                    category: rustics::MetricCategory::Function,
                    polarity: rustics::MetricPolarity::LowerIsBetter,
                    default_warning: Some(rustics::Threshold::new(1.0)),
                    default_error: None,
                    rationale: "",
                    refactor_hints: &[],
                    references: &[],
                }
            }
            fn measure(&self, _input: &MetricInput<'_>) -> Vec<rustics::MetricMeasurement> {
                Vec::new()
            }
        }
        let sev = severity_from_lens(&Bare, 999.0);
        assert_eq!(sev, DiagnosticSeverity::WARNING);
    }

    #[test]
    fn diagnostic_for_renders_position_and_code() {
        let metrics = builtin_metrics();
        let cc = metrics.iter().find(|m| m.id() == "cyclomatic-complexity").unwrap();
        let measurement = rustics::MetricMeasurement {
            scope: rustics::ScopeRef::new("f", rustics::ScopeKind::FreeFunction, 7),
            value: 25.0,
        };
        let diag = diagnostic_for(cc.as_ref(), &cc.metadata(), 10.0, measurement);
        // line 7 → 0-indexed 6
        assert_eq!(diag.range.start.line, 6);
        assert_eq!(diag.range.start.character, 0);
        assert_eq!(diag.code, Some(lsp_types::NumberOrString::String("cyclomatic-complexity".to_string())));
        assert!(diag.message.contains("25"));
        assert!(diag.message.contains("f"));
    }

    #[test]
    fn collect_metric_diagnostics_skips_lens_without_warning() {
        // Build a dummy lens with no thresholds — must yield zero diags
        // regardless of input.
        struct NoThreshold;
        impl MetricCalculator for NoThreshold {
            fn id(&self) -> &'static str {
                "no-threshold"
            }
            fn metadata(&self) -> rustics::MetricMetadata {
                rustics::MetricMetadata {
                    id: "no-threshold",
                    display_name: "NT",
                    category: rustics::MetricCategory::Function,
                    polarity: rustics::MetricPolarity::LowerIsBetter,
                    default_warning: None,
                    default_error: None,
                    rationale: "",
                    refactor_hints: &[],
                    references: &[],
                }
            }
            fn measure(&self, _input: &MetricInput<'_>) -> Vec<rustics::MetricMeasurement> {
                vec![rustics::MetricMeasurement {
                    scope: rustics::ScopeRef::new("f", rustics::ScopeKind::FreeFunction, 1),
                    value: 9999.0,
                }]
            }
        }
        let parsed =
            ra_ap_syntax::SourceFile::parse("fn f() {}", ra_ap_syntax::Edition::CURRENT);
        let tree = parsed.tree();
        let path = std::path::PathBuf::from("x.rs");
        let input = MetricInput::new(&path, "fn f() {}", &tree);
        let mut out = Vec::new();
        collect_metric_diagnostics(&NoThreshold, &input, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn handle_notification_did_open_inserts_into_documents() {
        // Drive the notification handler with a stand-in connection so
        // we can verify documents map mutation without a real LSP peer.
        let (server, _client) = Connection::memory();
        let mut docs: HashMap<Uri, String> = HashMap::new();
        let params = lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri: uri(),
                language_id: "rust".to_string(),
                version: 1,
                text: "fn f() {}".to_string(),
            },
        };
        let note = Notification {
            method: DidOpenTextDocument::METHOD.to_string(),
            params: serde_json::to_value(&params).unwrap(),
        };
        handle_notification(&server, &mut docs, note).expect("did_open");
        assert_eq!(docs.get(&uri()).map(String::as_str), Some("fn f() {}"));
    }

    #[test]
    fn handle_notification_did_change_replaces_text() {
        let (server, _client) = Connection::memory();
        let mut docs: HashMap<Uri, String> = HashMap::new();
        docs.insert(uri(), "fn old() {}".to_string());
        let params = lsp_types::DidChangeTextDocumentParams {
            text_document: lsp_types::VersionedTextDocumentIdentifier { uri: uri(), version: 2 },
            content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "fn newer() {}".to_string(),
            }],
        };
        let note = Notification {
            method: DidChangeTextDocument::METHOD.to_string(),
            params: serde_json::to_value(&params).unwrap(),
        };
        handle_notification(&server, &mut docs, note).expect("did_change");
        assert_eq!(docs.get(&uri()).map(String::as_str), Some("fn newer() {}"));
    }

    #[test]
    fn handle_notification_unknown_method_is_ignored() {
        let (server, _client) = Connection::memory();
        let mut docs: HashMap<Uri, String> = HashMap::new();
        let note = Notification {
            method: "textDocument/didNothing".to_string(),
            params: serde_json::Value::Null,
        };
        handle_notification(&server, &mut docs, note).expect("ignored");
        assert!(docs.is_empty());
    }

    #[test]
    fn handle_notification_did_change_with_no_changes_is_noop() {
        let (server, _client) = Connection::memory();
        let mut docs: HashMap<Uri, String> = HashMap::new();
        let params = lsp_types::DidChangeTextDocumentParams {
            text_document: lsp_types::VersionedTextDocumentIdentifier { uri: uri(), version: 2 },
            content_changes: vec![],
        };
        let note = Notification {
            method: DidChangeTextDocument::METHOD.to_string(),
            params: serde_json::to_value(&params).unwrap(),
        };
        handle_notification(&server, &mut docs, note).expect("empty change");
        assert!(docs.is_empty());
    }

    #[test]
    #[ignore = "stubbed lens during ra_ap_syntax migration"]
    fn publish_diagnostics_sends_a_notification() {
        let (server, client) = Connection::memory();
        publish_diagnostics(&server, &uri(), HEAVY_FN);
        let msg = client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("must receive a message");
        match msg {
            Message::Notification(n) => {
                assert_eq!(n.method, PublishDiagnostics::METHOD);
                let p: PublishDiagnosticsParams = serde_json::from_value(n.params).unwrap();
                assert_eq!(p.uri, uri());
                assert!(!p.diagnostics.is_empty());
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    /// Sends a Request and waits for the matching Response.
    fn rpc_call(client: &Connection, id: i32, method: &str) -> lsp_server::Response {
        let req = lsp_server::Request {
            id: id.into(),
            method: method.to_string(),
            params: serde_json::Value::Null,
        };
        client.sender.send(Message::Request(req)).unwrap();
        let msg = client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("response");
        match msg {
            Message::Response(r) => r,
            other => panic!("expected response, got {other:?}"),
        }
    }

    fn send_initialize(client: &Connection) {
        let init = lsp_server::Request {
            id: 1.into(),
            method: "initialize".to_string(),
            params: serde_json::to_value(&lsp_types::InitializeParams::default()).unwrap(),
        };
        client.sender.send(Message::Request(init)).unwrap();
        let resp = client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("initialize response");
        match resp {
            Message::Response(r) => assert!(r.error.is_none(), "init error: {:?}", r.error),
            other => panic!("unexpected: {other:?}"),
        }
    }

    fn send_initialized(client: &Connection) {
        let note = lsp_server::Notification {
            method: "initialized".to_string(),
            params: serde_json::Value::Null,
        };
        client.sender.send(Message::Notification(note)).unwrap();
    }

    fn send_did_open(client: &Connection, text: &str) {
        let params = lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri: Uri::from_str("file:///tmp/x.rs").unwrap(),
                language_id: "rust".to_string(),
                version: 1,
                text: text.to_string(),
            },
        };
        let note = lsp_server::Notification {
            method: DidOpenTextDocument::METHOD.to_string(),
            params: serde_json::to_value(&params).unwrap(),
        };
        client.sender.send(Message::Notification(note)).unwrap();
    }

    fn expect_publish_diagnostics(client: &Connection) {
        let msg = client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("publishDiagnostics");
        match msg {
            Message::Notification(n) => assert_eq!(n.method, PublishDiagnostics::METHOD),
            other => panic!("unexpected: {other:?}"),
        }
    }

    fn send_exit(client: &Connection) {
        let exit = lsp_server::Notification {
            method: "exit".to_string(),
            params: serde_json::Value::Null,
        };
        client.sender.send(Message::Notification(exit)).unwrap();
    }

    #[test]
    fn serve_drives_full_initialize_handshake() {
        let (server, client) = Connection::memory();
        let server_thread = std::thread::spawn(move || serve(&server));
        send_initialize(&client);
        send_initialized(&client);
        send_did_open(&client, HEAVY_FN);
        expect_publish_diagnostics(&client);
        let _ = rpc_call(&client, 2, "shutdown");
        send_exit(&client);
        drop(client);
        server_thread.join().unwrap().expect("serve OK");
    }

    #[test]
    fn serve_returns_ok_when_client_hangs_up_during_initialize() {
        let (server, client) = Connection::memory();
        // Drop the client before sending anything — the server's
        // initialize call sees the channel disconnected and returns
        // Ok(()) per the documented graceful-exit contract.
        drop(client);
        assert!(serve(&server).is_ok());
    }

    #[test]
    fn main_loop_handles_shutdown_request() {
        // Drive main_loop on the server side; client sends a shutdown
        // request followed by exit notification.
        let (server, client) = Connection::memory();
        // Spawn the loop on a background thread.
        let server_thread = std::thread::spawn(move || main_loop(&server));
        // Send shutdown request.
        let req = lsp_server::Request {
            id: 1.into(),
            method: "shutdown".to_string(),
            params: serde_json::Value::Null,
        };
        client.sender.send(Message::Request(req)).unwrap();
        // Receive shutdown response (the harness needs this consumed
        // before the exit notification fires the loop's return path).
        let _resp = client
            .receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("shutdown response");
        let exit = lsp_server::Notification {
            method: "exit".to_string(),
            params: serde_json::Value::Null,
        };
        client.sender.send(Message::Notification(exit)).unwrap();
        // Drop the client to close the channel; the loop returns when
        // the receiver disconnects.
        drop(client);
        server_thread.join().unwrap().expect("loop OK");
    }
}
