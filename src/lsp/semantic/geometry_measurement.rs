//! Opt-in public-fixture attribution. No production logging or timing assertions.
use super::*;
use std::time::{Duration, Instant};

#[test]
#[ignore = "bounded public-fixture measurement; requires SKRIPT_GEOMETRY_MEASUREMENT_PLAN"]
fn measure_public_document_geometry() {
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("SKRIPT_GEOMETRY_MEASUREMENT_PLAN").unwrap(),
    ).unwrap()).unwrap();
    assert_eq!(plan["publicFixtures"], true);
    assert_eq!(format!("{:x}",Sha256::digest(crate::core_library_component())),plan["coreSha256"]);
    let cases=plan["cases"].as_array().unwrap();assert!(cases.len()<=3);
    let snapshot_files=plan["snapshotFiles"].as_array().unwrap();
    assert_eq!(snapshot_files.len(),21);
    let verify_snapshot=|| {
        for file in snapshot_files {
            let relative=std::path::Path::new(file["name"].as_str().unwrap());
            assert!(relative.is_relative() && relative.components().all(|part|matches!(part,std::path::Component::Normal(_))));
            let bytes=std::fs::read(std::path::Path::new(plan["snapshot"].as_str().unwrap()).join(relative)).unwrap();
            assert_eq!(format!("{:x}",Sha256::digest(bytes)),file["sha256"]);
        }
    };
    verify_snapshot();
    let catalog=Arc::new(ssg::load(plan["snapshot"].as_str().unwrap()).unwrap().into_catalog());
    let mut observations=Vec::new();
    for case in cases {
        let source:Arc<str>=Arc::from(std::fs::read_to_string(case["path"].as_str().unwrap()).unwrap());
        assert!(source.len()<=150_000);
        assert_eq!(format!("{:x}",Sha256::digest(source.as_bytes())),case["sha256"]);
        let setup=Instant::now();
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog:Some(Arc::clone(&catalog)), ..Default::default()
        }).unwrap();
        let setup_ms=setup.elapsed().as_secs_f64()*1000.0;
        let mut config=parser_wasm::DocumentParserConfig::default();
        let cancel=parser_wasm::DocumentCancellationToken::new();
        config.cancellation=cancel.clone();
        let (stop,deadline)=std::sync::mpsc::channel();
        let timer=std::thread::spawn(move||{
            if deadline.recv_timeout(Duration::from_secs(180)).is_err(){cancel.cancel();}
        });
        let parse=Instant::now();let mut headers=Vec::new();
        let parsed=host.parse_document_with_preparation(
            parser_wasm::DocumentParseRequest::new("file:///geometry","file:///geometry/public.sk",1,Arc::clone(&source)),
            config,&mut |event|{
                if let skript_parser::StructurePreparationEvent::Headers {source,headers:items,..}=event {
                    assert!(headers.len()<16);
                    headers.push(serde_json::json!({"ms":parse.elapsed().as_secs_f64()*1000.0,
                        "headers":items.len(),"arenaBytes":source.virtual_source().len(),
                        "mapSegments":source.source_map().segments().len()}));
                }
            },
        );
        let parse_ms=parse.elapsed().as_secs_f64()*1000.0;
        let _=stop.send(());timer.join().unwrap();
        let result=parsed.unwrap();
        assert_eq!(result.execution_status,parser_wasm::DocumentExecutionStatus::Complete);
        assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
        let uri:Uri="file:///geometry/public.sk".parse().unwrap();
        let project=Instant::now();
        let semantic=Document::from_result(&result,Arc::clone(&source),uri,Arc::clone(&catalog)).unwrap();
        let semantic_ms=project.elapsed().as_secs_f64()*1000.0;
        let ranges=result.ast.nodes.iter().step_by((result.ast.nodes.len()/64).max(1)).take(64)
            .map(|node|TextRange::new(node.span.virtual_range.start as usize,node.span.virtual_range.end as usize))
            .collect::<Vec<_>>();
        let replay=Instant::now();let mut replayed=0;
        for range in &ranges {
            std::hint::black_box(result.source.map_range(*range).unwrap());replayed+=1;
            if replay.elapsed()>Duration::from_secs(2){break;}
        }
        let replay_ms=replay.elapsed().as_secs_f64()*1000.0;
        let identity=skript_parser::MappedSource::identity(result.source.virtual_source());
        let replay=Instant::now();
        for range in ranges.iter().take(replayed){std::hint::black_box(identity.map_range(*range).unwrap());}
        let identity_ms=replay.elapsed().as_secs_f64()*1000.0;
        observations.push(serde_json::json!({
            "name":case["name"],"sourceSha256":case["sha256"],"setupMs":setup_ms,"nativeParseMs":parse_ms,
            "headerObservations":headers,"semanticProjectionMs":semantic_ms,"arenaBytes":result.source.virtual_source().len(),
            "originalBytes":source.len(),"mapSegments":result.source.source_map().segments().len(),
            "astNodes":result.ast.nodes.len(),"rawNodes":result.raw_tree.nodes.len(),
            "treeExpansions":result.tree_macro_calls.iter().filter(|call|call.expansion.is_some()).count(),
            "symbols":semantic.symbols.len(),"mappingSamples":replayed,"mappingReplayMs":replay_ms,
            "identityReplayMs":identity_ms,"extrapolatedTotalCost":false,
            "limits":"Headers include preprocessing and all root header hooks; remaining parse combines bodies, AST stages and proof. Sample replay is outside the parse timer."
        }));
    }
    verify_snapshot();
    let output=serde_json::json!({
        "publicFixtures":true,"snapshotId":plan["snapshotId"],"snapshotFiles":plan["snapshotFiles"],
        "snapshotFence":"All21 exact files checked before catalog loading and after measurements; ssg::load performs internal snapshot validation",
        "bounds":"Native parse cancellation180s; replay2s is a soft threshold between calls. External owned-process deadline bounds the complete run.",
        "observations":observations
    });
    use std::io::Write;
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(plan["output"].as_str().unwrap()).unwrap();
    file.write_all(&serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    eprintln!("geometry observations: {output}");
}
