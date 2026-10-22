//! Versioned source edits only. Preparing never reserves a stale symbol; the
//! actual rename repeats binding, coverage, collision and source checks.
use super::*;
use lsp_types::{RenameParams, TextDocumentPositionParams};
use std::collections::BTreeMap;

impl Documents {
    pub(super) fn rename_request(
        &mut self,
        worker: &Worker,
        connection: &Connection,
        id: RequestId,
        method: &str,
        params: Value,
        supported: bool,
    ) -> Result<(), String> {
        let mut refresh = None;
        let response = (|| -> Result<Value, (ErrorCode, String)> {
            if !supported {
                return Err((
                    ErrorCode::RequestFailed,
                    "Rename requires client support for versioned documentChanges".into(),
                ));
            }
            let (target, new_name) = if method == "textDocument/rename" {
                let params: RenameParams = serde_json::from_value(params)
                    .map_err(|e| (ErrorCode::InvalidParams, e.to_string()))?;
                (params.text_document_position, Some(params.new_name))
            } else {
                (
                    serde_json::from_value::<TextDocumentPositionParams>(params)
                        .map_err(|e| (ErrorCode::InvalidParams, e.to_string()))?,
                    None,
                )
            };
            let document = self.open.get(&target.text_document.uri).ok_or((
                ErrorCode::RequestFailed,
                "Rename requires an open document".into(),
            ))?;
            if document
                .lines
                .offset(&document.source, target.position)
                .is_none()
            {
                return Err((
                    ErrorCode::InvalidParams,
                    "Invalid rename UTF-16 position".into(),
                ));
            }
            let semantic = document
                .semantic
                .as_ref()
                .filter(|_| document.completion_ready)
                .ok_or((
                    ErrorCode::ContentModified,
                    "Current document analysis is not complete".into(),
                ))?;
            let project = self
                .semantic_project
                .iter()
                .map(Arc::as_ref)
                .collect::<Vec<_>>();
            let snapshot = self.project_snapshot().ok_or((
                ErrorCode::RequestFailed,
                "Rename requires an analyzed local project".into(),
            ))?;
            let mut expected = BTreeMap::new();
            for participant in std::iter::once(semantic.as_ref())
                .chain(project.iter().copied().filter(|d| d.uri != semantic.uri))
            {
                if let Some(open) = self.open.get(&participant.uri) {
                    if !open.completion_ready || !participant.source_matches(&open.source) {
                        return Err((
                            ErrorCode::ContentModified,
                            "A project participant is still being analyzed".into(),
                        ));
                    }
                }
                expected.insert(
                    participant.uri.as_str().to_owned(),
                    participant.source_witness.clone(),
                );
            }
            // Repeat bounded discovery as well as guarded source reads. Watching
            // is optional; a newly added caller must invalidate the edit set.
            if let Err(error) = project::verify_source_set(&snapshot, &expected) {
                refresh = Some(target.text_document.uri.clone());
                return Err((
                    ErrorCode::ContentModified,
                    format!("Cannot verify current project: {error}"),
                ));
            }
            let Some(plan) = semantic
                .rename_plan(target.position, &project, new_name.as_deref())
                .map_err(|e| (ErrorCode::RequestFailed, e))?
            else {
                return if new_name.is_none() {
                    Ok(Value::Null)
                } else {
                    Err((
                        ErrorCode::RequestFailed,
                        "No safely editable symbol is available at this position".into(),
                    ))
                };
            };
            // Validate target availability even for prepareRename. No versionless
            // fallback is offered when a closed file would need modification.
            let mut changes = BTreeMap::<String, Vec<Value>>::new();
            for location in plan.edits {
                let open = self.open.get(&location.uri).ok_or((ErrorCode::RequestFailed, "Open every affected document before renaming so each edit has a checked version".into()))?;
                if !open.completion_ready {
                    return Err((
                        ErrorCode::ContentModified,
                        "An edit target is being analyzed".into(),
                    ));
                }
                changes.entry(location.uri.as_str().to_owned()).or_default().push(json!({"range":location.range,"newText":new_name.as_deref().unwrap_or(&plan.placeholder)}));
            }
            if new_name.is_none() {
                return Ok(json!({"range":plan.range,"placeholder":plan.placeholder}));
            }
            let changes = changes.into_iter().map(|(uri, edits)| {
                let key: Uri = uri.parse().expect("source URI already validated");
                json!({"textDocument":{"uri":uri,"version":self.open[&key].version},"edits":edits})
            }).collect::<Vec<_>>();
            Ok(json!({"documentChanges":changes}))
        })();
        send(
            connection,
            match response {
                Ok(result) => Response::new_ok(id, result),
                Err((code, message)) => Response::new_err(id, code as i32, message),
            },
        )?;
        if let Some(focused) = refresh {
            self.restart_project(worker, connection, Some(&focused))?;
        }
        Ok(())
    }
}
