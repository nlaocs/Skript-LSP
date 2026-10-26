//! Controlled complete binding-call lifecycles using the unchanged guest.
//! None/echo paths are different semantics, never pure WIT-time subtraction.
use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types::{HookTarget, TypeParserOutcome};
use sha2::{Digest, Sha256};
use std::time::Duration;

fn digest(value: &str) -> String { format!("{:x}", Sha256::digest(value.as_bytes())) }

#[test]
#[ignore = "bounded pinned-Core lifecycle controls; requires SKRIPT_TYPE_LIFECYCLE_PLAN"]
fn measure_type_lifecycle_controls() {
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("SKRIPT_TYPE_LIFECYCLE_PLAN").unwrap()).unwrap()).unwrap();
    assert_eq!(plan["publicFixtures"], true);
    let core = std::fs::read(plan["component"].as_str().unwrap()).unwrap();
    assert_eq!(digest_bytes(&core), plan["coreSha256"]);
    let verify_snapshot = || {
        let files = plan["snapshotFiles"].as_array().unwrap(); assert_eq!(files.len(),21);
        for file in files {
            let name = std::path::Path::new(file["name"].as_str().unwrap());
            assert!(name.is_relative() && name.components().all(|p|matches!(p,std::path::Component::Normal(_))));
            let bytes = std::fs::read(std::path::Path::new(plan["snapshot"].as_str().unwrap()).join(name)).unwrap();
            assert_eq!(digest_bytes(&bytes),file["sha256"]);
        }
    };
    verify_snapshot();
    let catalog = Arc::new(ssg::load(std::path::Path::new(plan["snapshot"].as_str().unwrap())).unwrap().into_catalog());
    let options = all_expression_type_options(Some(&catalog));
    let active = options.iter().find(|t|t.code_name=="number" && t.class_name=="java.lang.Number" && t.addon_name=="Skript").unwrap().clone();
    assert_eq!(active.addon_version,"2.15.4");
    let config = HostConfig::default(); assert_eq!(config.fuel_per_call,20_000_000);
    let sizes = [452_usize,21_540,743_695];
    assert_eq!(plan["inputBytes"],serde_json::json!(sizes));
    let repetitions = plan["repetitions"].as_u64().unwrap() as usize;
    let warmups = plan["warmups"].as_u64().unwrap() as usize;
    assert!((8..=50).contains(&repetitions) && (1..=4).contains(&warmups));
    let cases = sizes.into_iter().flat_map(|bytes|[false,true].into_iter().flat_map(move |all_options|
        ["disabled","miss","hit"].map(move |mode|(bytes,all_options,mode)))).collect::<Vec<_>>();
    let started = Instant::now();
    let mut rows = Vec::new(); let mut expected = BTreeMap::new();
    for block in 0..2 {
        let order = if block==0 {(0..cases.len()).collect::<Vec<_>>()} else {(0..cases.len()).rev().collect()};
        for case_index in order {
        let (bytes,all_options,mode) = cases[case_index];
        for enabled in if block==0 {[false,true]} else {[true,false]} {
            assert!(started.elapsed()<Duration::from_secs(120),"whole control deadline");
            let setup = Instant::now();
            let mut host = ParserHost::new(&core,HostConfig {syntax_catalog:Some(Arc::clone(&catalog)),
                runtime_profile:RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()}).unwrap();
            let setup_ns = setup.elapsed().as_nanos() as u64;
            let transaction = host.begin_parse("file:///type-lifecycle","file:///type-lifecycle/public.sk",1).unwrap();
            let cancellation = DocumentCancellationToken::new();
            let prepare = Instant::now();
            let mut input = String::with_capacity(bytes); input.push(if mode=="hit" {'1'} else {'x'});
            input.extend(std::iter::repeat_n(' ',bytes-1)); assert_eq!(input.len(),bytes);
            let range = WitTextRange {start:0,end:1};
            let payload = WitExpressionPayload {
                input,context:parse_context_to_wit(&ExpressionParseContext::default()),active_type:Some(active.clone()),
                remaining:range.clone(),span:MappedSpan {virtual_range:range.clone(),origins:vec![WitSourceOrigin {
                    original_range:range,kind:WitOriginKind::Exact,expansion:None}]},
                expected_types:vec![WitExpressionExpectedType {class_name:"java.lang.Number".into(),plural:false}],
                candidate_ends:vec![1],allow_literals:mode!="disabled",allow_expressions:true,time:0,depth:0,
                type_options:if all_options {options.clone()} else {Vec::new()},literal_options:Vec::new(),
                type_parser_unresolved:Vec::new(),type_parser_outcome:None,candidates:Vec::new(),
            };
            let option_count = payload.type_options.len();
            let mut invocation = HookInvocation {
                context:InvocationContext {invocation_id:0,subscription_id:"core.type-candidates".into(),
                    document_id:"file:///type-lifecycle/public.sk".into(),document_revision:1,expansion:None,syntax_context:0},
                target:HookTarget::Registration(active.registration_id.clone()),
                phase:HookPhase::Expression,payload:HookPayload::Expression(payload),parse_results:Vec::new(),
            };
            let construction_ns = prepare.elapsed().as_nanos() as u64;
            let entry = &mut host.components[0];
            entry.store.data_mut().cancellation = Some(cancellation.clone());
            let probe = Probe::default();
            let mut wall_ns = Vec::new(); let mut prepare_ns=Vec::new(); let mut verify_ns=Vec::new();
            let mut drop_ns=Vec::new(); let mut fuel=Vec::new(); let mut output_input_bytes=0;
            for repeat in 0..warmups+repetitions {
                assert!(started.elapsed()<Duration::from_secs(120));
                invocation.context.invocation_id=(repeat+1) as u64;
                let preparation = Instant::now();
                entry.store.data_mut().invocation=Some(transaction.begin_invocation(CORE_LIBRARY_COMPONENT_ID).unwrap());
                entry.store.data_mut().catalog_measurement=(enabled && repeat>=warmups).then(||probe.clone());
                prepare_store(&mut entry.store,config.fuel_per_call,config.deadline_ticks(CORE_LIBRARY_COMPONENT_ID),CORE_LIBRARY_COMPONENT_ID,"hook").unwrap();
                let preparation_ns=preparation.elapsed().as_nanos() as u64;
                let before=entry.store.get_fuel().unwrap();
                let hook=(enabled && repeat>=warmups).then(||probe.hook("core.type-candidates",&invocation,Some(before)));
                let call_started=Instant::now();
                let output=entry.bindings.nlaocs_skript_parser_addon_hooks().call_invoke(&mut entry.store,&invocation);
                let elapsed=call_started.elapsed(); let after=entry.store.get_fuel().unwrap();
                if let Some(hook)=hook {hook.finish(Some(after));}
                assert!(elapsed<Duration::from_secs(3),"per-call wall observation exceeded guard");
                let verify=Instant::now(); let output=output.unwrap().unwrap();
                assert_eq!(format!("{:?}",output.effects),format!("{:?}",empty_effects()));
                let signature=match (&output.replacement,mode) {
                    (None,"disabled")=>{assert!(matches!(output.decision,HookDecision::NotApplicable));"not-applicable".to_owned()},
                    (Some(HookPayload::Expression(value)),"miss"|"hit")=>{
                        assert!(matches!(output.decision,HookDecision::ContinueProcessing));
                        let HookPayload::Expression(request)=&invocation.payload else {unreachable!()};
                        assert!(same_expression_request(request,value),"full immutable echo must match before normalization");
                        assert_eq!(value.input,request.input);output_input_bytes=value.input.len();
                        assert!(value.type_parser_unresolved.is_empty());
                        if mode=="miss" {assert!(matches!(value.type_parser_outcome,Some(TypeParserOutcome::NoMatch)));assert!(value.candidates.is_empty());}
                        else {assert!(matches!(value.type_parser_outcome,Some(TypeParserOutcome::Handled)));assert_eq!(value.candidates.len(),1);
                            let candidate=&value.candidates[0];assert_eq!(candidate.parser_id,"core.literal.number");
                            assert!(matches!(candidate.kind,WitExpressionLeafKind::Literal));
                            assert_eq!(candidate.return_type.as_deref(),Some("java.lang.Long"));
                            assert_eq!((candidate.range.start,candidate.range.end),(0,1));}
                        format!("{:?}|{:?}|{:?}|{:?}",output.decision,value.type_parser_outcome,value.candidates,output.effects)
                    },
                    _=>panic!("unexpected control result"),
                };
                let semantic=digest(&signature);
                assert_eq!(expected.entry(mode).or_insert_with(||semantic.clone()),&semantic,"same slice/outcome across geometry and observer controls");
                let state=entry.store.data_mut().invocation.take().unwrap();assert!(state.read_write_set().writes.is_empty());state.rollback();
                let verification_ns=verify.elapsed().as_nanos() as u64;
                let dropping=Instant::now();drop(output);let dropping_ns=dropping.elapsed().as_nanos() as u64;
                if repeat>=warmups {wall_ns.push(elapsed.as_nanos() as u64);prepare_ns.push(preparation_ns);verify_ns.push(verification_ns);drop_ns.push(dropping_ns);fuel.push(before-after);}
            }
            entry.store.data_mut().catalog_measurement=None;
            let measurement=probe.take_all();
            assert_eq!(measurement.hooks.values().map(|h|h.calls).sum::<u64>(),if enabled {repetitions as u64} else {0});
            assert_eq!(transaction.state_revision().unwrap(),0);
            cancellation.cancel();
            assert!(matches!(prepare_store(&mut entry.store,config.fuel_per_call,config.deadline_ticks(CORE_LIBRARY_COMPONENT_ID),CORE_LIBRARY_COMPONENT_ID,"hook"),Err(HostError::Cancelled(_))));
            transaction.cancel().unwrap();
            let row=serde_json::json!({"block":block,"caseIndex":case_index,"mode":mode,"probeEnabled":enabled,
                "inputBytes":bytes,"activeRange":[0,1],"candidateEnds":[1],"typeOptions":option_count,"allOptions":all_options,
                "outputInputBytes":output_input_bytes,"warmups":warmups,"repetitions":repetitions,"setupNs":setup_ns,"requestConstructionNs":construction_ns,
                "bindingCallNs":wall_ns,"hostPrepareNs":prepare_ns,"validationNs":verify_ns,"hostOutputDropNs":drop_ns,"fuel":fuel,
                "semanticSha256":expected[mode],"imports":OPERATIONS.iter().zip(measurement.imports).map(|(name,sample)|serde_json::json!({"operation":name,"sample":sample})).collect::<Vec<_>>()});
            eprintln!("type_lifecycle_observation {row}");rows.push(row);
        }
        }
    }
    verify_snapshot();
    let result=serde_json::json!({"rows":rows,"coreSha256":plan["coreSha256"],"snapshotId":plan["snapshotId"],
        "fuelPerCall":config.fuel_per_call,"wholeElapsedMs":started.elapsed().as_secs_f64()*1000.0,
        "limits":"Synthetic geometry controls, fixed one-byte Number input, sparse context and no parse_results/literal options. Same unchanged guest; fresh hosts, warmups excluded, reverse case/probe order in block2. Boundaries include host lowering, guest lifting/body/lowering, host lifting and guest post-return. Host request construction, validation/rollback and output drop are separate timers. Different outcomes are not semantic optimization before/after pairs or pure WIT subtraction; do not extrapolate to production30s. No world/Bukkit execution, no fuel/API/ABI change."});
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(plan["output"].as_str().unwrap()).unwrap()
        .write_all(&serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}

fn digest_bytes(bytes: &[u8]) -> String { format!("{:x}",Sha256::digest(bytes)) }
