//! Process isolation for the test-only parser corpus. No CLI output is parsed.

use super::{Case, Fixture, analyze};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const WORKER_DIR: &str = "SKRIPT_CORPUS_WORKER_DIR";
const CASE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Fault {
    Hang,
    Panic,
    Abort,
    Component,
    Setup,
    MissingResponse,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    fixture: Fixture,
    case: Case,
    fault: Option<Fault>,
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
        for name in ["request.json", "response.json", "stderr", "started"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

// Also reap the worker if supervision returns an I/O error or unwinds.
struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run(fixture: &Fixture, case: &Case) -> Result<Value, String> {
    run_with_fault(fixture, case, None, CASE_TIMEOUT)
}

fn run_with_fault(
    fixture: &Fixture,
    case: &Case,
    fault: Option<Fault>,
    timeout: Duration,
) -> Result<Value, String> {
    let execute = || {
        let files = WorkerFiles::create()?;
        let request = serde_json::json!({ "fixture": fixture, "case": case, "fault": fault });
        fs::write(
            files.0.join("request.json"),
            serde_json::to_vec(&request).unwrap(),
        )
        .map_err(|error| format!("worker request: {error}"))?;
        let stderr = File::create(files.0.join("stderr"))
            .map_err(|error| format!("worker stderr: {error}"))?;
        let test_name = format!(
            "{}::case_worker",
            module_path!().split_once("::").unwrap().1
        );
        let started = Instant::now();
        // Let the injected hang start before its short test deadline; ordinary
        // cases include process startup in their 60-second deadline.
        let mut execution_started = (!matches!(fault, Some(Fault::Hang))).then_some(started);
        let mut worker = Worker(
            Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
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
                .map_err(|error| format!("worker spawn: {error}"))?,
        );
        let status = loop {
            if execution_started.is_none() && files.0.join("started").exists() {
                execution_started = Some(Instant::now());
            }
            let budget = if execution_started.is_some() {
                timeout
            } else {
                CASE_TIMEOUT
            };
            let elapsed = execution_started.unwrap_or(started).elapsed();
            match worker
                .0
                .try_wait()
                .map_err(|error| format!("worker wait: {error}"))?
            {
                Some(status) => break status,
                None if elapsed >= budget => {
                    worker
                        .0
                        .kill()
                        .map_err(|error| format!("timeout; worker kill: {error}"))?;
                    let status = worker
                        .0
                        .wait()
                        .map_err(|error| format!("timeout; worker reap: {error}"))?;
                    return Err(format!(
                        "timeout after {} ms; worker {} terminated and reaped ({status}); started={}",
                        budget.as_millis(),
                        worker.0.id(),
                        files.0.join("started").exists(),
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        if !status.success() {
            let mut diagnostic = Vec::new();
            File::open(files.0.join("stderr"))
                .and_then(|file| file.take(16 * 1024).read_to_end(&mut diagnostic))
                .map_err(|error| format!("worker {status}; reading stderr: {error}"))?;
            return Err(format!(
                "worker {} exited with {status}: {}",
                worker.0.id(),
                String::from_utf8_lossy(&diagnostic),
            ));
        }
        let response = fs::read(files.0.join("response.json"))
            .map_err(|error| format!("worker response missing/unreadable: {error}"))?;
        serde_json::from_slice::<Result<Value, String>>(&response)
            .map_err(|error| format!("invalid worker response: {error}"))?
    };
    execute()
}

#[test]
#[ignore = "internal corpus worker; launched with one request by the parent test"]
fn case_worker() {
    let Some(directory) = std::env::var_os(WORKER_DIR) else {
        return;
    };
    let directory = PathBuf::from(directory);
    let request: Request =
        serde_json::from_slice(&fs::read(directory.join("request.json")).expect("worker request"))
            .expect("valid worker request");
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            fs::write(directory.join("started"), b"started").expect("worker start marker");
            let mut fixture = request.fixture;
            match request.fault {
                Some(Fault::Hang) => loop {
                    std::thread::park();
                },
                Some(Fault::Panic) => panic!("injected corpus panic"),
                Some(Fault::Abort) => std::process::abort(),
                Some(Fault::MissingResponse) => std::process::exit(0),
                Some(Fault::Component) => {
                    let mut session = super::SkriptSession::load(super::fixture_path(&fixture))
                        .map_err(|error| error.to_string())?;
                    return session
                        .host
                        .load_addon(b"invalid component")
                        .map(|_| Value::Null)
                        .map_err(|error| format!("component failures: {error}"));
                }
                Some(Fault::Setup) => {
                    fixture.path = directory
                        .join("missing-snapshot")
                        .to_string_lossy()
                        .into_owned()
                }
                None => {}
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
    let directory = PathBuf::from(std::env::var_os(WORKER_DIR).unwrap());
    fs::write(
        directory.join("response.json"),
        serde_json::to_vec(&result).unwrap(),
    )
    .expect("worker response");
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
        (Fault::Panic, "injected corpus panic"),
        (Fault::Abort, "exited with"),
        (Fault::Component, "component failures"),
        (Fault::Setup, "missing-snapshot"),
        (Fault::MissingResponse, "worker response missing"),
    ];
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
            let limit = if matches!(fault, Some(Fault::Hang)) {
                Duration::from_secs(2)
            } else {
                CASE_TIMEOUT
            };
            let started = Instant::now();
            let result = run_with_fault(fixture, case, fault, limit);
            if matches!(fault, Some(Fault::Hang)) {
                let error = result.as_ref().unwrap_err();
                assert!(error.contains("terminated and reaped"), "{error}");
                assert!(
                    error.contains("started=true"),
                    "hang must reach the worker: {error}"
                );
                assert!(started.elapsed() < CASE_TIMEOUT + Duration::from_secs(10));
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
    for (index, (fault, reason)) in faults.iter().enumerate() {
        let error = errors
            .iter()
            .find(|error| error.contains(&format!("case=fault-{index} ")))
            .unwrap();
        assert!(error.contains(reason), "{fault:?}: {error}");
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
        "injected execution/comparison failures aggregated; {} later normal cases completed:\n{}",
        completed.len(),
        errors.join("\n")
    );
}
