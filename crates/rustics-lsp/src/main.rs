//! `rustics-lsp` — minimal Language Server Protocol bridge for the
//! rustics lens catalogue.
//!
//! Plan §5.8 / M3 task #53. The server speaks LSP over stdin/stdout
//! and publishes one diagnostic per lens violation in every open
//! document.
//!
//! M3 first-slice surface:
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
    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })?;
    let initialization_params = match connection.initialize(server_capabilities) {
        Ok(p) => p,
        Err(e) if e.channel_is_disconnected() => {
            io_threads.join()?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let _params: InitializeParams = serde_json::from_value(initialization_params)?;
    main_loop(&connection)?;
    io_threads.join()?;
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
    let Ok(ast) = syn::parse_file(source) else {
        return Vec::new();
    };
    let metrics = builtin_metrics();
    let path = std::path::PathBuf::from("__lsp__.rs");
    let input = MetricInput::new(&path, source, &ast);
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
