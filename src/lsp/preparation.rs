//! Source and native-attempt fences for provisional completion publications.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Attempt {
    producer_revision: u64,
    native_revision: u64,
}
#[derive(Debug, Clone, Copy)]
pub(super) struct State {
    attempt: Attempt,
    active: bool,
}

impl Documents {
    pub(super) fn prepare_completion(
        &mut self,
        job: &Job,
        preparation: worker::Preparation,
        connection: &Connection,
        snippets: bool,
    ) -> Result<bool, String> {
        if !self.current(job) || job.control.token.is_cancelled() {
            return Ok(false);
        }
        let source = preparation.source;
        let Some(document) = self.open.get_mut(&source.uri) else {
            return Ok(false);
        };
        if document.generation != source.generation
            || document.source_epoch != source.source_epoch
            || document.source != source.source
            || document.completion_ready
        {
            return Ok(false);
        }
        let attempt = Attempt {
            producer_revision: job.revision,
            native_revision: preparation.native_revision,
        };
        if matches!(&preparation.update, worker::PreparationUpdate::Started) {
            // Native revisions are worker-wide monotonic, even when an older queued job
            // runs after a newer producer in the same immutable project batch.
            if document
                .prepared_state
                .is_some_and(|state| state.attempt.native_revision >= attempt.native_revision)
            {
                return Ok(false);
            }
            document.refresh_completion(connection, &source.uri, "analysis-restarted")?;
            document.prepared_completion = None;
            document.prepared_state = Some(State {
                attempt,
                active: true,
            });
            return Ok(true);
        }
        if !document
            .prepared_state
            .is_some_and(|state| state.active && state.attempt == attempt)
        {
            return Ok(false);
        }
        match preparation.update {
            worker::PreparationUpdate::Started => unreachable!(),
            worker::PreparationUpdate::Ready(prepared) => {
                document.prepared_completion = Some(prepared);
                document.prepare_completion_requests(connection, snippets)?;
            }
            worker::PreparationUpdate::Structure(root, prepared) => {
                if let Some(view) = document.prepared_completion.as_mut() {
                    if view.prepare_structure(root, *prepared) {
                        let refresh = document.prepared_ticket.as_ref().is_some_and(|ticket| {
                            view.owner_at(ticket.position) != ticket.owner.as_ref()
                        });
                        if refresh {
                            document.refresh_completion(connection, &source.uri, "prepared")?;
                        }
                        document.prepare_completion_requests(connection, snippets)?;
                    }
                }
            }
            worker::PreparationUpdate::Section(section) => {
                if let Some(view) = document.prepared_completion.as_mut() {
                    let owner = section.owner.clone();
                    if view.prepare_section(section) {
                        let refresh = document
                            .prepared_ticket
                            .as_ref()
                            .is_some_and(|ticket| view.affects(ticket.position, &owner));
                        if refresh {
                            document.refresh_completion(connection, &source.uri, "prepared")?;
                        }
                        document.prepare_completion_requests(connection, snippets)?;
                    }
                }
            }
            worker::PreparationUpdate::Invalidated(owner) => {
                if let Some(view) = document.prepared_completion.as_mut() {
                    if view.revoke(&owner) {
                        let refresh = document
                            .prepared_ticket
                            .as_ref()
                            .is_some_and(|ticket| view.affects(ticket.position, &owner));
                        if refresh {
                            document.refresh_completion(connection, &source.uri, "revoked")?;
                        }
                    }
                }
            }
            worker::PreparationUpdate::Aborted | worker::PreparationUpdate::Unavailable(_) => {
                // Keep a tombstone: a late Ready with this same native revision cannot revive it.
                document.prepared_state = Some(State {
                    attempt,
                    active: false,
                });
                document.prepared_completion = None;
                document.refresh_completion(connection, &source.uri, "revoked")?;
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;
