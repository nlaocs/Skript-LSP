//! Follow-up hierarchy queries may target closed project participants. Items are
//! stateless, bound to the server session and current accepted project generation.
use super::*;
use lsp_types::CallHierarchyIncomingCallsParams;

impl Documents {
    pub(super) fn hierarchy_stamp(&self) -> String {
        format!("{}:{}", self.hierarchy_session, self.next_id)
    }
    pub(super) fn hierarchy_request(
        &self,
        connection: &Connection,
        id: RequestId,
        method: &str,
        params: Value,
    ) -> Result<(), String> {
        let response = (|| -> Result<Value, (ErrorCode, String)> {
            // The two standard follow-up parameter shapes are identical.
            let params: CallHierarchyIncomingCallsParams = serde_json::from_value(params)
                .map_err(|e| (ErrorCode::InvalidParams, e.to_string()))?;
            let stamp = self.hierarchy_stamp();
            if self.open.values().any(|d| !d.completion_ready) {
                return Err((
                    ErrorCode::ContentModified,
                    "Current project analysis is not complete".into(),
                ));
            }
            let mut documents = self
                .semantic_project
                .iter()
                .map(Arc::as_ref)
                .collect::<Vec<_>>();
            for open in self.open.values() {
                if let Some(semantic) = open.semantic.as_deref() {
                    documents.retain(|d| d.uri != semantic.uri);
                    documents.push(semantic);
                }
            }
            if documents.iter().any(|d| !d.project_complete) {
                return Err((
                    ErrorCode::RequestFailed,
                    "Project discovery is incomplete; call hierarchy is unavailable".into(),
                ));
            }
            let document = documents
                .iter()
                .copied()
                .find(|d| d.uri == params.item.uri)
                .ok_or((
                    ErrorCode::ContentModified,
                    "Call hierarchy document is no longer available".into(),
                ))?;
            let result = if method == "callHierarchy/incomingCalls" {
                document
                    .hierarchy_incoming(&params.item, &documents, &stamp)
                    .map(|v| serde_json::to_value(v).expect("serializable incoming calls"))
            } else {
                document
                    .hierarchy_outgoing(&params.item, &documents, &stamp)
                    .map(|v| serde_json::to_value(v).expect("serializable outgoing calls"))
            };
            result.map_err(|e| (ErrorCode::ContentModified, e))
        })();
        send(
            connection,
            match response {
                Ok(value) => Response::new_ok(id, value),
                Err((code, message)) => Response::new_err(id, code as i32, message),
            },
        )
    }
}
