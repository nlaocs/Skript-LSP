use super::*;
use crate::lsp::project::{ProjectSnapshot, ProjectSource};
use crate::lsp::worker::{Preparation, PreparationUpdate};
use std::sync::Arc;

fn document(
    source: Arc<str>,
    generation: u64,
    source_epoch: u64,
    revision: u64,
    version: i32,
) -> Document {
    Document {
        source_epoch,
        generation,
        revision,
        version,
        control: Control::new(),
        hover: None,
        semantic: None,
        inlay: None,
        annotation_requests: Vec::new(),
        tokens: None,
        token_requests: Vec::new(),
        lines: positions::LineIndex::new(&source),
        source,
        completion: None,
        syntax_completion: None,
        prepared_completion: None,
        prepared_state: None,
        prepared_ticket: None,
        next_completion_ticket: 0,
        completion_ready: false,
        completion_requests: Vec::new(),
        signature_requests: Vec::new(),
        signature_capabilities: Default::default(),
    }
}

fn fixture(source: &str) -> (Documents, Job, ProjectSource, Connection, Connection) {
    let target = ProjectSource {
        uri: "file:///target.sk".parse().unwrap(),
        generation: 10,
        source_epoch: 101,
        source: Arc::from(source),
    };
    let producer = ProjectSource {
        uri: "file:///producer.sk".parse().unwrap(),
        generation: 20,
        source_epoch: 202,
        source: Arc::from("producer:\n    hold"),
    };
    let mut documents = Documents::new(Default::default());
    documents.project_epoch = 30;
    documents.open.insert(
        target.uri.clone(),
        document(Arc::clone(&target.source), 10, 101, 120, 7),
    );
    documents.open.insert(
        producer.uri.clone(),
        document(Arc::clone(&producer.source), 20, 202, 220, 2),
    );
    let job = Job {
        uri: producer.uri.clone(),
        project: Some(Arc::new(ProjectSnapshot {
            epoch: 30,
            root: std::env::temp_dir(),
            open: vec![target.clone(), producer.clone()],
        })),
        generation: 20,
        revision: 220,
        version: 2,
        source: producer.source,
        control: documents.open[&producer.uri].control.clone(),
        queued_at: Instant::now(),
    };
    let (server, client) = Connection::memory();
    (documents, job, target, server, client)
}

fn update(
    documents: &mut Documents,
    job: &Job,
    target: &ProjectSource,
    native_revision: u64,
    update: PreparationUpdate,
    connection: &Connection,
) -> bool {
    documents
        .prepare_completion(
            job,
            Preparation {
                source: target.clone(),
                native_revision,
                update,
            },
            connection,
            false,
        )
        .unwrap()
}
fn ready(target: &ProjectSource) -> PreparationUpdate {
    PreparationUpdate::Ready(Box::new(completion::PreparedDocument::test_fixture(
        Arc::clone(&target.source),
    )))
}
fn response(client: &Connection, id: i32) -> Value {
    client
        .receiver
        .try_iter()
        .find_map(|message| match message {
            Message::Response(response) if response.id == id.into() => {
                Some(response.response_result.unwrap())
            }
            _ => None,
        })
        .expect("the queued target completion must receive a response")
}
fn refreshes(client: &Connection) -> Vec<Value> {
    client
        .receiver
        .try_iter()
        .filter_map(|message| match message {
            Message::Notification(notification)
                if notification.method == "skript/completionRefresh" =>
            {
                Some(notification.params)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn side_document_preparation_answers_its_pending_request_before_own_job_runs() {
    let (mut documents, job, target, server, client) = fixture("hold:\n    hold");
    documents
        .open
        .get_mut(&target.uri)
        .unwrap()
        .completion_requests
        .push((40.into(), Position::new(1, 8), Instant::now()));
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        ready(&target),
        &server
    ));
    let list = response(&client, 40);
    assert_eq!(list["items"][0]["label"], "hold pattern");
    assert_eq!(list["items"][0]["data"]["completionPhase"], "prepared");
    assert!(
        list["items"][0]["data"]["completionTicket"]
            .as_str()
            .unwrap()
            .starts_with("10:120:")
    );
    assert!(
        documents.open[&job.uri].prepared_completion.is_none(),
        "the producing job does not borrow its side document's proposals"
    );
    assert!(
        !documents.open[&target.uri].completion_ready,
        "early information must not masquerade as final analysis"
    );
}

#[test]
fn abort_tombstone_rejects_late_ready_and_same_attempt_restart_but_new_attempt_can_prepare() {
    let (mut documents, job, target, server, client) = fixture("hold:\n    hold");
    assert!(
        !update(&mut documents, &job, &target, 10, ready(&target), &server),
        "Ready must follow Started"
    );
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        ready(&target),
        &server
    ));
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Aborted,
        &server
    ));
    assert!(!update(
        &mut documents,
        &job,
        &target,
        10,
        ready(&target),
        &server
    ));
    assert!(!update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(!documents.open[&target.uri].prepared_state.unwrap().active);
    assert!(update(
        &mut documents,
        &job,
        &target,
        11,
        PreparationUpdate::Started,
        &server
    ));
    assert!(update(
        &mut documents,
        &job,
        &target,
        11,
        ready(&target),
        &server
    ));
    assert!(!update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Aborted,
        &server
    ));
    assert!(documents.open[&target.uri].prepared_state.unwrap().active);
    assert!(documents.open[&target.uri].prepared_completion.is_some());
    assert!(
        refreshes(&client).is_empty(),
        "no visible ticket was issued in this case"
    );
}

#[test]
fn native_attempt_order_does_not_confuse_an_older_queued_job_with_older_information() {
    let (mut documents, producer, target, server, client) = fixture("hold:\n    hold");
    assert!(update(
        &mut documents,
        &producer,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(update(
        &mut documents,
        &producer,
        &target,
        10,
        ready(&target),
        &server
    ));
    documents
        .open
        .get_mut(&target.uri)
        .unwrap()
        .try_prepared_completion(&server, &40.into(), Position::new(1, 8), false, 0.0)
        .unwrap();
    let ticket = response(&client, 40)["items"][0]["data"]["completionTicket"].clone();
    let own = Job {
        uri: target.uri.clone(),
        project: producer.project.clone(),
        generation: 10,
        revision: 120,
        version: 7,
        source: Arc::clone(&target.source),
        control: documents.open[&target.uri].control.clone(),
        queued_at: Instant::now(),
    };
    assert!(own.revision < producer.revision);
    assert!(update(
        &mut documents,
        &own,
        &target,
        11,
        PreparationUpdate::Started,
        &server
    ));
    let notices = refreshes(&client);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["ticket"], ticket);
    assert_eq!(notices[0]["uri"], target.uri.as_str());
    assert_eq!(notices[0]["version"], 7);
    assert_eq!(notices[0]["phase"], "analysis-restarted");
    assert!(!update(
        &mut documents,
        &producer,
        &target,
        10,
        ready(&target),
        &server
    ));
    assert!(!update(
        &mut documents,
        &producer,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(update(
        &mut documents,
        &own,
        &target,
        11,
        ready(&target),
        &server
    ));
    let state = documents.open[&target.uri].prepared_state.unwrap();
    assert_eq!(state.attempt.producer_revision, 120);
    assert_eq!(state.attempt.native_revision, 11);
}

#[test]
fn source_generation_epoch_content_producer_and_project_fences_reject_old_notifications() {
    for mutation in 0..8 {
        let (mut documents, mut job, mut target, server, client) = fixture("hold:\n    hold");
        match mutation {
            0 => target.generation += 1,
            1 => target.source_epoch += 1,
            2 => target.source = Arc::from("hold:\n    different"),
            3 => target.uri = "file:///closed.sk".parse().unwrap(),
            4 => job.revision += 1,
            5 => documents.project_epoch += 1,
            6 => job.control.cancel(),
            7 => {
                documents.open.remove(&target.uri);
            }
            _ => unreachable!(),
        }
        assert!(
            !update(
                &mut documents,
                &job,
                &target,
                10,
                PreparationUpdate::Started,
                &server
            ),
            "mutation {mutation}"
        );
        assert!(
            documents
                .open
                .values()
                .all(|document| document.prepared_state.is_none())
        );
        assert!(client.receiver.try_iter().next().is_none());
    }
}

#[test]
fn final_result_is_not_replaced_by_a_provisional_side_document_attempt() {
    let (mut documents, job, target, server, _client) = fixture("hold:\n    hold");
    let document = documents.open.get_mut(&target.uri).unwrap();
    document.completion_ready = true;
    document.completion = Some(completion::Document::test_fixture(Arc::clone(
        &target.source,
    )));
    assert!(!update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(!update(
        &mut documents,
        &job,
        &target,
        10,
        ready(&target),
        &server
    ));
    assert!(documents.open[&target.uri].completion_ready);
    assert!(documents.open[&target.uri].completion.is_some());
    assert!(documents.open[&target.uri].prepared_completion.is_none());
}

#[test]
fn parent_owner_revocation_refreshes_a_ticket_owned_by_a_nested_child_once() {
    let (mut documents, job, target, server, client) = fixture("root:\n    nested:\n        hold");
    let view = completion::PreparedDocument::test_nested_fixture(Arc::clone(&target.source));
    let parent = view.owner_at(Position::new(0, 0)).unwrap().clone();
    let child = view.owner_at(Position::new(2, 12)).unwrap().clone();
    assert_ne!(parent, child);
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Started,
        &server
    ));
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Ready(Box::new(view)),
        &server
    ));
    documents
        .open
        .get_mut(&target.uri)
        .unwrap()
        .try_prepared_completion(&server, &40.into(), Position::new(2, 12), false, 0.0)
        .unwrap();
    let list = response(&client, 40);
    assert_eq!(
        list["items"][0]["data"]["ownerAttempt"]["attemptId"],
        child.attempt_id
    );
    let ticket = list["items"][0]["data"]["completionTicket"].clone();
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Invalidated(parent.clone()),
        &server
    ));
    let notices = refreshes(&client);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["phase"], "revoked");
    assert_eq!(notices[0]["ticket"], ticket);
    assert_eq!(notices[0]["uri"], target.uri.as_str());
    assert!(matches!(
        documents.open[&target.uri]
            .prepared_completion
            .as_ref()
            .unwrap()
            .at(Position::new(2, 12), false)
            .unwrap(),
        completion::PreparedQuery::NeedsAnalysis
    ));
    assert!(update(
        &mut documents,
        &job,
        &target,
        10,
        PreparationUpdate::Invalidated(parent),
        &server
    ));
    assert!(refreshes(&client).is_empty());
}
