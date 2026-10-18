//! Protocol-thread generation fences for worker-owned project analysis.
use super::*;

impl Documents {
    pub(super) fn restart_project(
        &mut self,
        worker: &Worker,
        connection: &Connection,
        focused: Option<&Uri>,
    ) -> Result<(), String> {
        let Some(root) = &self.project_root else {
            return Ok(());
        };
        self.project_epoch = self
            .project_epoch
            .checked_add(1)
            .ok_or("project epoch exhausted")?;
        self.semantic_project.clear();
        let snapshot = Arc::new(project::ProjectSnapshot {
            epoch: self.project_epoch,
            root: root.clone(),
            open: self
                .open
                .iter()
                .map(|(uri, document)| project::ProjectSource {
                    uri: uri.clone(),
                    generation: document.generation,
                    source_epoch: document.source_epoch,
                    source: Arc::clone(&document.source),
                })
                .collect(),
        });
        let mut uris = self.open.keys().cloned().collect::<Vec<_>>();
        uris.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        if let Some(focused) = focused {
            if let Some(index) = uris.iter().position(|uri| uri == focused) {
                let focused = uris.remove(index);
                uris.push(focused);
            }
        }
        for uri in uris {
            let revision = self.id()?;
            let document = self.open.get_mut(&uri).unwrap();
            document.cancel_requests(connection, ErrorCode::ContentModified)?;
            document.refresh_completion(connection, &uri, "project-changed")?;
            document.control.cancel();
            document.control = Control::new();
            document.revision = revision;
            document.hover = None;
            document.semantic = None;
            document.inlay = None;
            document.tokens = None;
            document.completion = None;
            document.syntax_completion = None;
            document.prepared_completion = None;
            document.completion_ready = false;
            worker.submit(Job {
                uri,
                generation: document.generation,
                revision,
                version: document.version,
                source: Arc::clone(&document.source),
                control: document.control.clone(),
                queued_at: Instant::now(),
                project: Some(Arc::clone(&snapshot)),
            });
        }
        Ok(())
    }
}
