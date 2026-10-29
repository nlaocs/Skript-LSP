//! Opt-in, bounded observations for public effect fixtures, including faults.
use super::*;

pub(super) fn record_fault(error:&wasmtime::Error) {
    use std::sync::atomic::{AtomicUsize,Ordering};
    static SEEN:AtomicUsize=AtomicUsize::new(0);
    if std::env::var_os("SKRIPT_EFFECT_FUEL_PLAN").is_none() || SEEN.fetch_add(1,Ordering::Relaxed)>=8 {return;}
    // Dedicated test output only: bounded Wasm frames, never a payload dump.
    let trace=format!("{error:?}").chars().take(8192).collect::<String>();
    eprintln!("bounded_effect_fuel_backtrace {trace}");
}


pub(super) fn capture(probe:&Probe,key:&str,invocation:&HookInvocation) {
    let HookPayload::Effect(payload)=&invocation.payload else{return;};
    let Some(candidate)=&payload.candidate else{return;};
    let mut state=probe.0.lock().unwrap();
    let sample=state.hooks.entry(key.into()).or_default();
    if sample.effect_inputs.len()>=8{return;}
    sample.effect_inputs.push(serde_json::json!({
        "elementClass":candidate.element_class,"patternIndex":candidate.pattern_index,
        "captures":candidate.parsed_captures.iter().take(32).map(|c|serde_json::json!({
            "capture":c.capture_index,"summary":c.summary.as_ref().map(|s|serde_json::json!({
                "kind":s.kind,"returnType":s.return_type,"possibleReturnTypes":s.possible_return_types,
                "completeness":format!("{:?}",s.possible_return_types_state),
                "multiplicity":format!("{:?}",s.multiplicity),
                "metadataKeys":s.metadata.iter().map(|m|&m.key).collect::<Vec<_>>()
            }))
        })).collect::<Vec<_>>()
    }));
}

#[test]
#[ignore="bounded public effect attribution; requires SKRIPT_EFFECT_FUEL_PLAN"]
fn measure_bounded_effect_fuel() {
    use sha2::{Digest,Sha256};
    let bytes=std::fs::read(std::env::var_os("SKRIPT_EFFECT_FUEL_PLAN").unwrap()).unwrap();
    assert!(bytes.len()<131072);
    let plan:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
    assert_eq!(plan["publicFixtures"],true);
    let hash=|b:&[u8]|format!("{:x}",Sha256::digest(b));
    let core=std::fs::read(plan["component"].as_str().unwrap()).unwrap();
    assert_eq!(hash(&core),plan["coreSha256"]);
    let snapshot=std::path::Path::new(plan["snapshot"].as_str().unwrap());
    let files=plan["snapshotFiles"].as_array().unwrap();assert!(!files.is_empty()&&files.len()<=32);
    let verify=||for f in files {
        let relative=std::path::Path::new(f["name"].as_str().unwrap());
        assert!(relative.is_relative()&&relative.components().all(|c|matches!(c,std::path::Component::Normal(_))));
        assert_eq!(hash(&std::fs::read(snapshot.join(relative)).unwrap()),f["sha256"]);
    };
    verify();
    let catalog=Arc::new(ssg::load(snapshot).unwrap().into_catalog());
    let cases=plan["cases"].as_array().unwrap();assert!(!cases.is_empty()&&cases.len()<=8);
    let mut rows=Vec::new();
    for case in cases {
        let text=case["text"].as_str().unwrap();assert!(text.len()<=1024);
        let mut expected=None;
        for enabled in [false,true] {
            let config=HostConfig {syntax_catalog:Some(Arc::clone(&catalog)),
                runtime_profile:RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()};
            assert_eq!(config.fuel_per_call,20_000_000);
            let mut host=ParserHost::new(&core,config).unwrap();
            let probe=Probe::default();
            if enabled {for entry in &mut host.components {entry.store.data_mut().catalog_measurement=Some(probe.clone());}}
            let start=Instant::now();
            let result=host.parse_document(DocumentParseRequest::new("file:///effect-fuel","file:///effect-fuel/public.sk",1,text),DocumentParserConfig::default()).unwrap();
            let elapsed=start.elapsed().as_secs_f64()*1000.0;
            let observation=format!("{}|{:?}|{:?}|{:?}|{:?}",canonical(&result),result.execution_status,result.fault_regions,result.dependency_unavailable,result.component_failures);
            let digest=hash(observation.as_bytes());
            assert_eq!(expected.get_or_insert_with(||digest.clone()),&digest,"probe must preserve semantic/fault observations");
            let measurement=probe.take_all();
            rows.push(serde_json::json!({"case":case["name"],"probeEnabled":enabled,"parseMs":elapsed,
                "observationSha256":digest,"executionStatus":format!("{:?}",result.execution_status),
                "componentFailures":format!("{:?}",result.component_failures),"syntaxDiagnostics":result.syntax.diagnostics.len(),
                "astNodes":result.ast.nodes.len(),"hooks":measurement.hooks,
                "imports":OPERATIONS.iter().zip(measurement.imports).map(|(name,sample)|serde_json::json!({"operation":name,"sample":sample})).collect::<Vec<_>>()
            }));
        }
    }
    verify();
    let output=std::path::Path::new(plan["output"].as_str().unwrap());
    let data=serde_json::to_vec_pretty(&serde_json::json!({"coreSha256":hash(&core),"fuelPerCall":20_000_000,
        "limits":"<=8 public cases, <=1024 bytes/case, fresh hosts, <=8 effect input shapes/hook, no source text/metadata values captured; timers include callback and ABI costs, not pure guest time",
        "rows":rows})).unwrap();
    assert!(data.len()<=2_000_000);
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(output).unwrap().write_all(&data).unwrap();
}
