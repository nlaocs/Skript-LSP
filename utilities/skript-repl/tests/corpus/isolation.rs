//! Test-only process isolation with fresh parser state for every request.

use super::{Case, Fixture, analyze};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const WORKER_DIR: &str = "SKRIPT_CORPUS_WORKER_DIR";
const CASE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Fault {
    Hang,
    HangDuringDrop,
    Panic,
    Abort,
    Component,
    Setup,
    MissingResponse,
    WrongSequence,
    InvalidResponse,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    sequence: u64,
    fixture: Fixture,
    case: Case,
    fault: Option<Fault>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Response {
    sequence: u64,
    case_id: String,
    fixture_id: String,
    snapshot_id: String,
    result: Result<Value, String>,
}

struct WorkerFiles(PathBuf);

impl WorkerFiles {
    fn create() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "skript-corpus-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("worker directory: {error}")),
            }
        }
    }
}

impl Drop for WorkerFiles {
    fn drop(&mut self) {
        for name in [
            "request.next",
            "request.json",
            "response.next",
            "response.json",
            "stderr",
            "started",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

struct Worker {
    child: Child,
    files: WorkerFiles,
}

// Kill/reap before field destruction removes the files, including on unwind.
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Worker {
    fn spawn() -> Result<Self, String> {
        let files = WorkerFiles::create()?;
        let stderr = File::create(files.0.join("stderr")).map_err(|error| error.to_string())?;
        let test_name = format!(
            "{}::case_worker",
            module_path!().split_once("::").unwrap().1
        );
        let child = Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
            .args([
                "--exact",
                &test_name,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(WORKER_DIR, &files.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|error| format!("worker spawn: {error}"))?;
        Ok(Self { child, files })
    }

    fn execute(
        &mut self,
        sequence: u64,
        fixture: &Fixture,
        case: &Case,
        fault: Option<Fault>,
        timeout: Duration,
        started: Instant,
    ) -> Result<Value, String> {
        for name in ["response.json", "started"] {
            match fs::remove_file(self.files.0.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("clearing {name}: {error}")),
            }
        }
        let stderr_offset = fs::metadata(self.files.0.join("stderr"))
            .map_err(|error| error.to_string())?
            .len();
        let request = serde_json::json!({ "sequence": sequence, "fixture": fixture, "case": case, "fault": fault });
        publish(
            &self.files.0,
            "request",
            &serde_json::to_vec(&request).unwrap(),
        )?;
        self.wait_response(
            sequence,
            fixture,
            case,
            fault,
            timeout,
            started,
            stderr_offset,
        )
    }

    fn wait_response(
        &mut self,
        sequence: u64,
        fixture: &Fixture,
        case: &Case,
        fault: Option<Fault>,
        timeout: Duration,
        started: Instant,
        stderr_offset: u64,
    ) -> Result<Value, String> {
        // The short injected-hang deadline starts after its marker; normal cases
        // include worker startup and request delivery in their 60-second budget.
        let mut execution_started = (!is_hang(fault)).then_some(started);
        loop {
            if execution_started.is_none() && self.files.0.join("started").exists() {
                execution_started = Some(Instant::now());
            }
            let budget = if execution_started.is_some() {
                timeout
            } else {
                CASE_TIMEOUT
            };
            let deadline_start = execution_started.unwrap_or(started);
            self.enforce_deadline(deadline_start, budget)?;
            let status = self
                .child
                .try_wait()
                .map_err(|error| format!("worker wait: {error}"))?;
            if let Some(status) = status.filter(|status| !status.success()) {
                let mut diagnostic = Vec::new();
                let mut log =
                    File::open(self.files.0.join("stderr")).map_err(|error| error.to_string())?;
                log.seek(SeekFrom::Start(stderr_offset))
                    .map_err(|error| error.to_string())?;
                log.take(16 * 1024)
                    .read_to_end(&mut diagnostic)
                    .map_err(|error| error.to_string())?;
                return Err(format!(
                    "worker {} exited with {status}: {}",
                    self.child.id(),
                    String::from_utf8_lossy(&diagnostic)
                ));
            }
            let response = self.files.0.join("response.json");
            if response.exists() {
                let response: Response =
                    serde_json::from_slice(&fs::read(response).map_err(|error| error.to_string())?)
                        .map_err(|error| format!("invalid worker response: {error}"))?;
                if response.sequence != sequence
                    || response.case_id != case.id
                    || response.fixture_id != fixture.id
                    || response.snapshot_id != fixture.snapshot_id
                {
                    return Err("worker response identity/sequence mismatch".to_owned());
                }
                self.enforce_deadline(deadline_start, budget)?;
                return response.result;
            }
            if status.is_some() {
                return Err("worker response missing after exit".to_owned());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn enforce_deadline(&mut self, started: Instant, budget: Duration) -> Result<(), String> {
        if started.elapsed() < budget {
            return Ok(());
        }
        self.child
            .kill()
            .map_err(|error| format!("timeout; worker kill: {error}"))?;
        let status = self
            .child
            .wait()
            .map_err(|error| format!("timeout; worker reap: {error}"))?;
        Err(format!(
            "timeout after {} ms; worker {} terminated and reaped ({status}); started={}",
            budget.as_millis(),
            self.child.id(),
            self.files.0.join("started").exists()
        ))
    }
}

fn publish(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    fs::write(directory.join(format!("{name}.next")), bytes).map_err(|error| error.to_string())?;
    fs::rename(
        directory.join(format!("{name}.next")),
        directory.join(format!("{name}.json")),
    )
    .map_err(|error| format!("publishing {name}: {error}"))
}

fn is_hang(fault: Option<Fault>) -> bool {
    matches!(fault, Some(Fault::Hang | Fault::HangDuringDrop))
}

#[derive(Default)]
pub(super) struct Runner {
    worker: Option<Worker>,
    sequence: u64,
    generation: u64,
}

impl Runner {
    pub(super) fn run(&mut self, fixture: &Fixture, case: &Case) -> Result<Value, String> {
        self.run_with_fault(fixture, case, None, CASE_TIMEOUT)
    }

    fn run_with_fault(
        &mut self,
        fixture: &Fixture,
        case: &Case,
        fault: Option<Fault>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let started = Instant::now();
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("worker sequence exhausted")?;
        if self.worker.is_none() {
            self.worker = Some(Worker::spawn()?);
            self.generation += 1;
        }
        let result = self.worker.as_mut().unwrap().execute(
            self.sequence,
            fixture,
            case,
            fault,
            timeout,
            started,
        );
        if result.is_err() {
            self.worker.take();
        }
        result
    }
}

struct HangOnDrop(PathBuf);
impl Drop for HangOnDrop {
    fn drop(&mut self) {
        fs::write(&self.0, b"dropping").unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[test]
#[ignore = "internal corpus worker; launched by the parent test"]
fn case_worker() {
    let Some(directory) = std::env::var_os(WORKER_DIR) else {
        return;
    };
    let directory = PathBuf::from(directory);
    loop {
        let bytes = match fs::read(directory.join("request.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => panic!("worker request: {error}"),
        };
        fs::remove_file(directory.join("request.json")).expect("consume worker request");
        let request: Request = serde_json::from_slice(&bytes).expect("valid worker request");
        let mut response = Response {
            sequence: request.sequence,
            case_id: request.case.id.clone(),
            fixture_id: request.fixture.id.clone(),
            snapshot_id: request.fixture.snapshot_id.clone(),
            result: Err("worker did not finish".to_owned()),
        };
        let fault = request.fault;
        let worker_directory = directory.clone();
        response.result = std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let mut fixture = request.fixture;
                let _on_drop = matches!(fault, Some(Fault::HangDuringDrop))
                    .then(|| HangOnDrop(worker_directory.join("started")));
                if !matches!(fault, Some(Fault::HangDuringDrop)) {
                    fs::write(worker_directory.join("started"), b"started").expect("start marker");
                }
                match fault {
                    Some(Fault::Hang) => loop {
                        std::thread::park();
                    },
                    Some(Fault::Panic) => panic!("injected corpus panic"),
                    Some(Fault::Abort) => std::process::abort(),
                    Some(Fault::MissingResponse) => std::process::exit(0),
                    Some(Fault::Component) => {
                        let mut session = super::historical_session(&fixture)?;
                        return session
                            .host
                            .load_addon(b"invalid component")
                            .map(|_| Value::Null)
                            .map_err(|error| format!("component failures: {error}"));
                    }
                    Some(Fault::Setup) => {
                        fixture.path = worker_directory
                            .join("missing-snapshot")
                            .to_string_lossy()
                            .into_owned()
                    }
                    _ => {}
                }
                analyze(&fixture, &request.case)
            })
            .expect("parser thread")
            .join()
            .unwrap_or_else(|panic| {
                let message = panic
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("non-string panic");
                Err(format!("parser worker panicked: {message}"))
            });
        // Thread join includes parser state destruction; only then publish a reply.
        let failed = response.result.is_err();
        if matches!(fault, Some(Fault::WrongSequence)) {
            response.sequence += 1;
        }
        let bytes = if matches!(fault, Some(Fault::InvalidResponse)) {
            b"{".to_vec()
        } else {
            serde_json::to_vec(&response).unwrap()
        };
        publish(&directory, "response", &bytes).expect("worker response");
        if failed {
            return;
        }
    }
}

#[test]
fn failures_are_isolated_reaped_and_followed_by_a_normal_case() {
    let corpus = super::corpus();
    let case = corpus
        .cases
        .iter()
        .find(|case| case.mode == "expression")
        .unwrap();
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.id == case.fixture)
        .unwrap();
    let expected = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, || analyze(fixture, case))
            .unwrap()
            .join()
            .unwrap()
            .unwrap()
    });
    let faults = [
        (Fault::Hang, "timeout"),
        (Fault::HangDuringDrop, "timeout"),
        (Fault::Panic, "injected corpus panic"),
        (Fault::Abort, "exited with"),
        (Fault::Component, "component failures"),
        (Fault::Setup, "missing-snapshot"),
        (Fault::MissingResponse, "worker response missing"),
        (Fault::WrongSequence, "identity/sequence mismatch"),
        (Fault::InvalidResponse, "invalid worker response"),
    ];
    let mut runner = Runner::default();
    assert_eq!(runner.run(fixture, case).unwrap(), expected);
    let first_generation = runner.generation;
    assert_eq!(runner.run(fixture, case).unwrap(), expected);
    assert_eq!(
        runner.generation, first_generation,
        "successful cases reuse compilation"
    );
    let mut cases = Vec::new();
    for (index, (fault, _)) in faults.iter().enumerate() {
        let mut failed = case.clone();
        failed.id = format!("fault-{index}");
        failed.source = serde_json::to_value(fault).unwrap();
        cases.push(failed);
        let mut normal = case.clone();
        normal.id = format!("normal-{index}");
        normal.source = Value::Null;
        cases.push(normal);
    }
    let mut completed = Vec::new();
    let errors = super::collect_case_failures(
        &corpus,
        &cases,
        |case| {
            let fault = if case.source.is_null() {
                None
            } else {
                Some(serde_json::from_value::<Fault>(case.source.clone()).unwrap())
            };
            let generation = runner.generation;
            let result = runner.run_with_fault(
                fixture,
                case,
                fault,
                if is_hang(fault) {
                    Duration::from_secs(2)
                } else {
                    CASE_TIMEOUT
                },
            );
            if fault.is_some() {
                assert!(result.is_err());
                assert!(
                    runner.worker.is_none(),
                    "every failure discards the warm worker"
                );
                assert_eq!(runner.generation, generation);
            } else {
                assert_eq!(
                    runner.generation,
                    generation + 1,
                    "case after a failure starts a fresh process"
                );
            }
            if is_hang(fault) {
                let error = result.as_ref().unwrap_err();
                assert!(
                    error.contains("terminated and reaped") && error.contains("started=true"),
                    "{error}"
                );
            }
            result
        },
        |case, actual| {
            assert_eq!(actual, expected, "fresh normal case {}", case.id);
            completed.push(case.id.clone());
            if case.id == "normal-0" {
                vec!["injected comparison mismatch".to_owned()]
            } else {
                Vec::new()
            }
        },
    );
    assert_eq!(
        completed,
        (0..faults.len())
            .map(|index| format!("normal-{index}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(errors.len(), faults.len() + 1);
    for (index, (_, reason)) in faults.iter().enumerate() {
        let error = errors
            .iter()
            .find(|error| error.contains(&format!("case=fault-{index} ")))
            .unwrap();
        assert!(error.contains(reason), "{error}");
    }
    assert!(
        errors.iter().any(|error| error.contains("case=normal-0 ")
            && error.contains("injected comparison mismatch"))
    );
    for error in &errors {
        for identity in [&case.input, &fixture.id, &fixture.snapshot_id] {
            assert!(error.contains(identity), "missing {identity:?}: {error}");
        }
    }
    eprintln!(
        "{} later normal cases completed; aggregated failures:\n{}",
        completed.len(),
        errors.join("\n")
    );
}

#[test]
fn expired_deadline_rejects_an_already_published_response() {
    let corpus = super::corpus();
    let case = corpus
        .cases
        .iter()
        .find(|case| case.mode == "expression")
        .unwrap();
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.id == case.fixture)
        .unwrap();
    let mut runner = Runner::default();
    runner.run(fixture, case).unwrap();
    let worker = runner.worker.as_mut().unwrap();
    assert!(worker.files.0.join("response.json").exists());
    // Simulate a parent that was descheduled until after this case's deadline.
    // The valid, matching response must not bypass the elapsed-time check.
    let error = worker
        .wait_response(
            runner.sequence,
            fixture,
            case,
            None,
            Duration::from_millis(1),
            Instant::now() - Duration::from_secs(1),
            0,
        )
        .unwrap_err();
    assert!(
        error.contains("timeout") && error.contains("terminated and reaped"),
        "{error}"
    );
    assert!(worker.child.try_wait().unwrap().is_some());
}

#[test]
fn reused_worker_keeps_snapshots_and_event_context_separate() {
    let corpus = super::corpus();
    let a = corpus
        .cases
        .iter()
        .find(|case| case.mode == "expression" && case.event.is_none())
        .unwrap();
    let b = corpus
        .cases
        .iter()
        .find(|case| case.fixture != a.fixture && case.event.is_some())
        .unwrap();
    let mut runner = Runner::default();
    let mut direct = Vec::new();
    for case in [a, b, a] {
        let fixture = corpus
            .fixtures
            .iter()
            .find(|fixture| fixture.id == case.fixture)
            .unwrap();
        let expected = std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || analyze(fixture, case))
                .unwrap()
                .join()
                .unwrap()
                .unwrap()
        });
        let actual = runner.run(fixture, case).unwrap();
        assert_eq!(actual, expected, "{}", case.id);
        direct.push(actual);
    }
    assert_eq!(
        runner.generation, 1,
        "A/B/A share only immutable runtime caches"
    );
    assert_eq!(direct[0], direct[2], "event from B cannot leak into A");
    assert_ne!(direct[1]["context"], direct[2]["context"]);
}
