//! Immutable semantic queries, bound to the currently accepted document generation.
use super::*;
use lsp_types::{GotoDefinitionParams, InlayHintParams, Range, ReferenceParams};

enum Query {
    Definition(Position),
    References(Position, bool),
    Inlay(Range),
}
pub(super) struct Pending {
    id: RequestId,
    query: Query,
}

impl Document {
    fn annotation_response(
        &self,
        id: RequestId,
        query: &Query,
        project: &[Arc<semantic::Document>],
    ) -> Response {
        let documents = project.iter().map(Arc::as_ref).collect::<Vec<_>>();
        if (matches!(query, Query::Definition(position) if self.semantic.as_ref().is_some_and(|document| document.project_definition_unavailable(*position)))
            || matches!(query, Query::References(..)))
            && self
                .semantic
                .as_ref()
                .is_some_and(|document| !document.project_complete)
        {
            return Response::new_err(id, ErrorCode::RequestFailed as i32,
                "Project discovery is incomplete; this project-wide semantic query is unavailable until the workspace can be fully analyzed".into());
        }
        match query {
            Query::Definition(position) => Response::new_ok(
                id,
                self.semantic
                    .as_ref()
                    .map(|document| document.definitions_at(*position, &documents))
                    .unwrap_or_default(),
            ),
            Query::References(position, declarations) => Response::new_ok(
                id,
                self.semantic
                    .as_ref()
                    .map(|document| document.references_at(*position, &documents, *declarations))
                    .unwrap_or_default(),
            ),
            Query::Inlay(range) => Response::new_ok(
                id,
                self.inlay
                    .as_ref()
                    .map(|document| document.at(*range))
                    .unwrap_or_default(),
            ),
        }
    }
    pub(super) fn finish_annotations(
        &mut self,
        connection: &Connection,
        project: &[Arc<semantic::Document>],
    ) -> Result<(), String> {
        for pending in std::mem::take(&mut self.annotation_requests) {
            send(
                connection,
                self.annotation_response(pending.id, &pending.query, project),
            )?;
        }
        Ok(())
    }
    pub(super) fn cancel_annotations(
        &mut self,
        connection: &Connection,
        code: ErrorCode,
    ) -> Result<(), String> {
        for pending in self.annotation_requests.drain(..) {
            send(
                connection,
                Response::new_err(
                    pending.id,
                    code as i32,
                    "document semantics changed before this request completed".into(),
                ),
            )?;
        }
        Ok(())
    }
    pub(super) fn cancel_annotation(
        &mut self,
        connection: &Connection,
        id: &RequestId,
    ) -> Result<bool, String> {
        if let Some(index) = self
            .annotation_requests
            .iter()
            .position(|pending| &pending.id == id)
        {
            let pending = self.annotation_requests.remove(index);
            send(
                connection,
                Response::new_err(
                    pending.id,
                    ErrorCode::RequestCanceled as i32,
                    "semantic request cancelled".into(),
                ),
            )?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn semantic_hover(
        &self,
        position: Position,
        format: &MarkupKind,
    ) -> Option<lsp_types::Hover> {
        let native = self
            .hover
            .as_ref()
            .and_then(|document| document.at(position));
        let Some((range, description)) = self
            .semantic
            .as_ref()
            .and_then(|document| document.hover(position))
        else {
            return native;
        };
        let addition = if *format == MarkupKind::Markdown {
            format!(
                "**Inferred type**\n\n{}",
                hover_text::code_block(&description, "text")
            )
        } else {
            format!("Inferred type\n{description}")
        };
        let (content, range) = match native {
            Some(lsp_types::Hover {
                contents: lsp_types::HoverContents::Markup(content),
                range: native_range,
            }) => (
                format!("{}\n\n{}", content.value, addition),
                native_range.or(Some(range)),
            ),
            _ => (addition, Some(range)),
        };
        Some(lsp_types::Hover {
            contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                kind: format.clone(),
                value: content,
            }),
            range,
        })
    }
}

impl Documents {
    pub(super) fn annotation_request(
        &mut self,
        connection: &Connection,
        id: RequestId,
        method: &str,
        params: Value,
    ) -> Result<(), String> {
        let parsed: Result<(Uri, Query), String> = (|| match method {
            "textDocument/definition" => {
                let params: GotoDefinitionParams =
                    serde_json::from_value(params).map_err(|error| error.to_string())?;
                Ok((
                    params.text_document_position_params.text_document.uri,
                    Query::Definition(params.text_document_position_params.position),
                ))
            }
            "textDocument/references" => {
                let params: ReferenceParams =
                    serde_json::from_value(params).map_err(|error| error.to_string())?;
                Ok((
                    params.text_document_position.text_document.uri,
                    Query::References(
                        params.text_document_position.position,
                        params.context.include_declaration,
                    ),
                ))
            }
            "textDocument/inlayHint" => {
                let params: InlayHintParams =
                    serde_json::from_value(params).map_err(|error| error.to_string())?;
                Ok((params.text_document.uri, Query::Inlay(params.range)))
            }
            _ => Err("unsupported semantic request".into()),
        })();
        let (uri, query) = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                return send(
                    connection,
                    Response::new_err(id, ErrorCode::InvalidParams as i32, error),
                );
            }
        };
        let capacity = self
            .open
            .values()
            .map(|document| document.annotation_requests.len())
            .sum::<usize>()
            < 256;
        let Some(document) = self.open.get_mut(&uri) else {
            return send(connection, Response::new_ok(id, Vec::<Value>::new()));
        };
        let valid = match query {
            Query::Definition(position) | Query::References(position, _) => {
                document.lines.offset(&document.source, position).is_some()
            }
            Query::Inlay(range) => {
                range.start <= range.end
                    && document
                        .lines
                        .offset(&document.source, range.start)
                        .is_some()
                    && document.lines.offset(&document.source, range.end).is_some()
            }
        };
        if !valid {
            return send(
                connection,
                Response::new_err(
                    id,
                    ErrorCode::InvalidParams as i32,
                    "invalid semantic UTF-16 position or range".into(),
                ),
            );
        }
        if document.completion_ready {
            send(
                connection,
                document.annotation_response(id, &query, &self.semantic_project),
            )
        } else if !capacity || document.annotation_requests.len() >= 64 {
            send(
                connection,
                Response::new_err(
                    id,
                    ErrorCode::RequestFailed as i32,
                    "too many pending semantic requests".into(),
                ),
            )
        } else {
            document.annotation_requests.push(Pending { id, query });
            Ok(())
        }
    }
}
