//! Manual warm document-pipeline measurement against the archived server export.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

const FIXTURE: &str = "skript-2.16.2-java25";
const CASES: [&str; 3] = ["doc-message-event", "short-teleport", "research-teleport"];

#[test]
#[ignore = "manual pipeline measurement; run alone with --nocapture --test-threads=1"]
fn measure_real_capture_pipeline() {
    // Match the CLI's parser stack without timing thread creation.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(measure)
        .unwrap()
        .join()
        .unwrap();
}

fn measure() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../corpus/cases.json")).unwrap();
    let sample = std::env::var("CAPTURE_PIPELINE_SAMPLE").unwrap_or_else(|_| "0".to_owned());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../parser-wasm/tests/data/type-parser-versions")
        .join(FIXTURE);
    for suffix in CASES {
        let id = format!("{FIXTURE}/{suffix}");
        let case = corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == id)
            .unwrap();
        let input = format!(
            "{}:\n    {}\n",
            case["event"].as_str().unwrap(),
            case["input"].as_str().unwrap()
        );
        let mut session = SkriptSession::from_snapshot(ssg::load(&fixture).unwrap()).unwrap();
        let mut expected = None;
        let mut parse_ns = 0;
        let mut report_ns = 0;
        let mut checksum = 0;
        // One warmup and one measured fresh-revision parse per process and case.
        for iteration in 0..2 {
            let revision = iteration + 1;
            let started = Instant::now();
            let result = session
                .host
                .parse_document(
                    DocumentParseRequest::new(PROJECT_URI, DOCUMENT_URI, revision, input.as_str()),
                    DocumentParserConfig {
                        raw_tree: Some(session.raw_tree_options()),
                        ..DocumentParserConfig::default()
                    },
                )
                .unwrap();
            let parse_elapsed = started.elapsed();
            assert!(
                result.component_failures.is_empty(),
                "{suffix}: component fault"
            );
            let started = Instant::now();
            let report = DocumentAnalysisReport::from_result(
                &input,
                &session.snapshot,
                result,
                session.catalog.as_ref(),
                parse_elapsed,
            );
            let report_elapsed = started.elapsed();
            assert!(report.matched(), "{suffix}: incomplete result");
            // Serialization and equality checking are outside both timed intervals.
            let mut observation: serde_json::Value =
                serde_json::from_str(&report.to_json().unwrap()).unwrap();
            observation
                .as_object_mut()
                .unwrap()
                .remove("parseDurationNs")
                .expect("document report must expose its timing field");
            if let Some(expected) = &expected {
                assert_eq!(&observation, expected, "{suffix}: revision changed output");
            } else {
                println!("pipeline-observation,{suffix},{observation}");
                expected = Some(observation.clone());
            }
            let mut hash = DefaultHasher::new();
            hash.write(observation.to_string().as_bytes());
            checksum = hash.finish();
            if iteration >= 1 {
                parse_ns += parse_elapsed.as_nanos();
                report_ns += report_elapsed.as_nanos();
            }
        }
        println!("capture-pipeline,{suffix},{sample},1,{parse_ns},{report_ns},{checksum}");
    }
}
