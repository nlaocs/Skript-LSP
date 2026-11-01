use super::*;
const PROJECT: &str = "file:///project";
const OWNER: &str = "example.declarations";
const HOOK: &str = "declarations";
const TEXT: &str = "expression fixture:\n    get:\n        return\n";

fn provider() -> ProviderStamp {
    // Protocol unit authority only; real store/component integration is tested
    // separately through the public native document/Component pipeline.
    ProviderStamp { epoch: Arc::new(AtomicBool::new(true)), catalog: None, source: None,
        owners: vec![(OWNER.into(), "binary".into(), 1, false, false)] }
}
fn source(document: &str) -> DeclarationSource {
    DeclarationSource::new(format!("file:///project/{document}.sk"), document, 1, 2, TEXT)
}
fn input(id: &str) -> DynamicSyntaxInput {
    DynamicSyntaxInput { local_id: id.into(), kind: CatalogSyntaxKind::Expression, patterns: vec!["fixture value".into()],
        priority: 0, before: vec![], after: vec![], return_type: Some("java.lang.String".into()),
        return_multiplicity: Some(DynamicMultiplicity::Single), structure_node_type: None, structure_body_mode: None,
        entry_validator: None, handler: "fixture.expression".into(), metadata: BTreeMap::new() }
}
fn setup(authority: &mut SourceAuthority, provider: ProviderStamp, current: DeclarationSource,
    sources: Vec<DeclarationSource>, exports: Vec<DeclarationExports>, complete: bool)
    -> (Arc<Mutex<Arena>>, tree_view::Lease, Invocation, u64)
{
    let request = DocumentParseRequest::new(PROJECT, current.document_id.clone(), 7, Arc::<str>::from(TEXT));
    let inputs = DeclarationInputs::new(current, sources, complete).with_epoch(10).with_exports(exports);
    let view = authority.prepare(&request, &inputs).unwrap();
    let arena = Arc::new(Mutex::new(Arena::new(PROJECT.into(), view, provider, &inputs,
        BTreeSet::from([(OWNER.into(), HOOK.into())]))));
    let mapped = MappedSource::identity(TEXT);
    let tree = Arc::new(skript_parser::parse_raw_tree(&mapped, skript_parser::RawTreeOptions::for_skript_version(2, 15)));
    let root = tree.roots[0].get();
    let lease = tree_view::Lease::new(tree, 7, None).unwrap();
    let mut payload = HookPayload::Parser(ParseRequest { request_id: 0, parser_id: declaration_prepass::PARSER_ID.into(),
        input: String::new(), expected_types: vec![], options: vec![],
        span: mapped_span_to_wit(mapped.map_range(ParserTextRange::new(0, TEXT.len())).unwrap()) });
    declaration_prepass::bind_tree_reference(&mut payload, lease.reference()).unwrap();
    let invocation = Invocation::prepare(Arc::clone(&arena), authority, OWNER, HOOK, lease.reference(), &mut payload).unwrap();
    (arena, lease, invocation, root)
}
fn export_entry(invocation: &Invocation, root: u64, local: bool, callable: bool) -> WitMetadataEntry {
    let tree = invocation.tree;
    WitMetadataEntry { key: EXPORT.into(), owner_component_id: None, value: json!({"version":1,
        "tree":{"version":1,"epochHigh":tree.epoch_high.to_string(),"epochLow":tree.epoch_low.to_string(),
            "token":tree.token.to_string(),"generation":tree.generation.to_string()},
        "root":root.to_string(),"namespace":"fixture.headers","key":"fixture value",
        "visibility":if local {"document"} else {"project"},"callable":callable}).to_string() }
}
fn finish(arena: &Arc<Mutex<Arena>>, entries: Vec<(&str, NativeSyntaxBinding)>) -> DeclarationExports {
    let frame = lock(arena).unwrap();
    frame.finish(NativeSyntaxBindings { definitions: entries.into_iter().map(|(id, binding)|
        (DynamicSyntaxId::new(OWNER, id), binding)).collect(),
        completeness: BTreeMap::from([((OWNER.into(), HOOK.into()), true)]) }, &frame.provider, true).unwrap()
}
fn publish(authority: &mut SourceAuthority, provider: ProviderStamp, a: DeclarationSource,
    sources: Vec<DeclarationSource>, input: DynamicSyntaxInput) -> DeclarationExports
{
    let (arena, mut lease, invocation, root) = setup(authority, provider, a, sources, vec![], true);
    let binding = invocation.bind(&input, &export_entry(&invocation, root, false, false), &mut lease).unwrap();
    assert!(!binding.callable);
    finish(&arena, vec![(&input.local_id, binding)])
}

#[test]
fn current_roots_export_without_getter_scope_and_local_headers_do_not_leak() {
    for local in [false, true] {
        let mut authority = SourceAuthority::default();
        let a = source("a");
        let (arena, mut lease, invocation, root) = setup(&mut authority, provider(), a.clone(), vec![a], vec![], true);
        let binding = invocation.bind(&input("header"), &export_entry(&invocation, root, local, false), &mut lease).unwrap();
        let stored = Arc::clone(&lock(&arena).unwrap().bindings[&binding.token]);
        assert_eq!(stored.owner, OWNER);
        assert_eq!(stored.origin.root, root);
        assert_eq!(stored.origin.source.stamp.uri, "file:///project/a.sk");
        assert!(!stored.origin.header.origins.is_empty());
        assert!(!stored.input.metadata.contains_key(EXPORT));
        let result = finish(&arena, vec![("header", binding)]);
        assert!(result.complete());
        assert_eq!(result.0.bindings.len(), usize::from(!local));
    }
}

#[test]
fn stale_tree_foreign_owner_and_nonroot_exports_are_not_complete_absence() {
    for fault in ["owner", "tree", "root", "store"] {
        let mut authority = SourceAuthority::default(); let a = source("a");
        let (arena, mut lease, invocation, root) = setup(&mut authority, provider(), a.clone(), vec![a], vec![], true);
        let mut entry = export_entry(&invocation, root, false, true);
        let mut value: Value = serde_json::from_str(&entry.value).unwrap();
        match fault {
            "owner" => entry.owner_component_id = Some("another.publisher".into()),
            "tree" => value["tree"]["token"] = (invocation.tree.token + 1).to_string().into(),
            "root" => value["root"] = "18446744073709551615".into(),
            _ => {
                let mapped = MappedSource::identity(TEXT);
                lease = tree_view::Lease::new(Arc::new(skript_parser::parse_raw_tree(&mapped,
                    skript_parser::RawTreeOptions::for_skript_version(2,15))), 7, None).unwrap();
            },
        }
        entry.value = value.to_string();
        assert!(invocation.bind(&input("header"), &entry, &mut lease).is_err(), "{fault}");
        assert!(!finish(&arena, vec![]).complete(), "a known rejected publication is not complete empty data");
    }
}

fn status_output(invocation: &Invocation, status: Value) -> HookOutput {
    let mut request = invocation.original.clone();
    request.options.push(WitMetadataEntry { key: STATUS.into(), owner_component_id: None, value: status.to_string() });
    HookOutput { decision: HookDecision::ContinueProcessing, replacement: Some(HookPayload::Parser(request)), effects: empty_effects() }
}

#[test]
fn status_echo_preserves_every_actual_sent_field_and_does_not_certify_missing_reports() {
    let mut authority = SourceAuthority::default(); let a = source("a");
    let (_, _, invocation, _) = setup(&mut authority, provider(), a.clone(), vec![a], vec![], true);
    assert_eq!(invocation.status(&status_output(&invocation, json!({"version":1,"complete":true}))).unwrap(), Some(true));
    for status in [json!({"version":2,"complete":true}),json!({"version":1,"complete":"true"}),
        json!({"version":1,"complete":true,"ignore":true})] {
        assert!(invocation.status(&status_output(&invocation,status)).is_err());
    }
    for fault in ["input","parser","span","tree","imports","owner","duplicate"] {
        let mut output = status_output(&invocation, json!({"version":1,"complete":true}));
        let Some(HookPayload::Parser(reply)) = output.replacement.as_mut() else { unreachable!() };
        match fault {
            "input" => reply.input = "changed".into(), "parser" => reply.parser_id = "foreign".into(),
            "span" => reply.span.virtual_range.end += 1, "tree" => reply.options[0].value = "{}".into(),
            "imports" => reply.options[1].value = "{}".into(),
            "owner" => reply.options.last_mut().unwrap().owner_component_id = Some(OWNER.into()),
            _ => reply.options.push(reply.options.last().unwrap().clone()),
        }
        assert!(invocation.status(&output).is_err(), "{fault}");
    }
    let mut missing = status_output(&invocation, json!({"version":1,"complete":false}));
    assert_eq!(invocation.status(&missing).unwrap(), Some(false));
    missing.replacement = None;
    assert_eq!(invocation.status(&missing).unwrap(), None);
    missing = status_output(&invocation, json!({"version":1,"complete":true}));
    missing.decision = HookDecision::NotApplicable;
    assert_eq!(invocation.status(&missing).unwrap(), None);
}

#[test]
fn imports_keep_original_origin_and_signature_and_never_reexport() {
    let mut authority = SourceAuthority::default(); let stamp = provider();
    let a = source("a"); let b = source("b"); let sources = vec![a.clone(), b.clone()];
    let published = publish(&mut authority, stamp.clone(), a, sources.clone(), input("header"));
    let (_, mut lease, invocation, _) = setup(&mut authority, stamp, b, sources, vec![published.clone()], true);
    assert_eq!(invocation.imports.len(), 1);
    let (&reference, original) = invocation.imports.iter().next().unwrap();
    let imported = original.imported_input();
    assert!(imported.local_id.starts_with("project-"));
    assert_ne!(imported.local_id, original.input.local_id);
    let entry = WitMetadataEntry {key:IMPORT.into(),owner_component_id:None,
        value:json!({"version":1,"reference":reference.to_string()}).to_string()};
    let bound = invocation.bind(&imported, &entry, &mut lease).unwrap();
    let stored = Arc::clone(&lock(&invocation.arena).unwrap().bindings[&bound.token]);
    assert!(stored.imported);
    assert_eq!(stored.origin.source.stamp.uri, "file:///project/a.sk");
    let result = finish(&invocation.arena, vec![(&imported.local_id, bound)]);
    assert!(result.0.bindings.is_empty());
    assert!(result.complete());
    let mut tampered = imported; tampered.return_type = Some("java.lang.Boolean".into());
    assert!(invocation.bind(&tampered, &entry, &mut lease).is_err());
    assert!(!finish(&invocation.arena, vec![]).complete());
}

#[test]
fn references_expire_on_new_invocations_and_partial_or_other_host_views() {
    let mut authority = SourceAuthority::default(); let stamp = provider();
    let a = source("a"); let b = source("b"); let sources = vec![a.clone(), b.clone()];
    let published = publish(&mut authority, stamp.clone(), a, sources.clone(), input("header"));
    let (_, _, first, _) = setup(&mut authority, stamp.clone(), b.clone(), sources.clone(), vec![published.clone()], true);
    let (&old_reference, prototype) = first.imports.iter().next().unwrap();
    let imported = prototype.imported_input();
    let (_, mut lease, current, _) = setup(&mut authority, stamp.clone(), b.clone(), sources.clone(), vec![published.clone()], true);
    let entry = WitMetadataEntry {key:IMPORT.into(),owner_component_id:None,
        value:json!({"version":1,"reference":old_reference.to_string()}).to_string()};
    assert!(current.bind(&imported,&entry,&mut lease).is_err());
    let (_, _, partial, _) = setup(&mut authority, stamp.clone(), b.clone(), vec![b.clone()], vec![published.clone()], false);
    assert!(partial.imports.is_empty());
    let (_, _, after_partial, _) = setup(&mut authority, stamp.clone(), b.clone(), sources.clone(), vec![published.clone()], true);
    assert!(after_partial.imports.is_empty(), "a stored old full view is not restored by matching source bytes");
    let mut other_host = SourceAuthority::default();
    let (_, _, other, _) = setup(&mut other_host, stamp, b, sources, vec![published], true);
    assert!(other.imports.is_empty(), "a foreign source fence is not a native membership witness");
}

#[test]
fn import_byte_overflow_withdraws_the_namespace_without_truncation() {
    let mut authority = SourceAuthority::default(); let stamp = provider();
    let a = source("a"); let b = source("b"); let sources = vec![a.clone(), b.clone()];
    let (arena, mut lease, invocation, root) = setup(&mut authority, stamp.clone(), a, sources.clone(), vec![], true);
    let mut first = input("first"); first.metadata.insert("long".into(), "x".repeat(40 * 1024));
    let mut second = input("second"); second.metadata.insert("long".into(), "y".repeat(40 * 1024));
    let one = invocation.bind(&first,&export_entry(&invocation,root,false,false),&mut lease).unwrap();
    let two = invocation.bind(&second,&export_entry(&invocation,root,false,false),&mut lease).unwrap();
    let published = finish(&arena,vec![("first",one),("second",two)]);
    assert!(published.complete());
    let (_,_,consumer,_) = setup(&mut authority,stamp,b,sources,vec![published],true);
    assert!(consumer.imports.is_empty());
    let value:Value = serde_json::from_str(&consumer.original.options[1].value).unwrap();
    assert_eq!(value["complete"],false);
    assert!(value["declarations"].as_array().unwrap().is_empty());
    assert!(consumer.original.options[1].value.len() <= MAX_OPTION_BYTES);
}

#[test]
fn excessive_or_repeated_publications_are_bounded_and_not_complete() {
    let mut authority = SourceAuthority::default(); let a = source("a");
    let (arena,mut lease,invocation,root) = setup(&mut authority,provider(),a.clone(),vec![a],vec![],true);
    let mut huge = input("huge"); huge.metadata.insert("huge".into(),"x".repeat(MAX_ARENA_BYTES+1));
    assert!(invocation.bind(&huge,&export_entry(&invocation,root,false,false),&mut lease).is_err());
    assert!(lock(&arena).unwrap().bindings.is_empty());
    assert!(!finish(&arena,vec![]).complete());
    let mut count = 0;
    loop {
        if invocation.bind(&input("repeated"),&export_entry(&invocation,root,false,false),&mut lease).is_err() { break; }
        count += 1; assert!(count <= MAX_ALLOCATIONS);
    }
    assert!(count > 0);
    assert!(!finish(&arena,vec![]).complete());
}

#[test]
fn editing_locations_keep_native_headers_and_expire_with_consumer_or_source_view() {
    for local in [false,true] {
        let mut authority=SourceAuthority::default();let a=source("a");
        let (arena,mut lease,invocation,root)=setup(&mut authority,provider(),a.clone(),vec![a],vec![],true);
        let binding=invocation.bind(&input("header"),&export_entry(&invocation,root,local,true),&mut lease).unwrap();
        let exports=finish(&arena,vec![("header",binding)]);
        let consumer=Arc::new(editor_completion::typed::Fence {project:PROJECT.into(),document:"consumer".into(),revision:1,
            current:AtomicBool::new(true),providers:CompletionTypeCertificate(Arc::new(AtomicBool::new(true)))});
        let locations=exports.locations(Arc::clone(&consumer));let id=DynamicSyntaxId::new(OWNER,"header").qualified();
        let (source,span)=locations.site(&id).unwrap();
        assert_eq!(source.uri(),"file:///project/a.sk");assert_eq!(span.origins.len(),1);
        assert_eq!(exports.0.bindings.len(),usize::from(!local),"local editing sites do not become global exports");
        assert!(locations.site("dynamic:foreign/header").is_none());
        consumer.current.store(false,Ordering::Release);
        assert!(locations.site(&id).is_none(),"old consumer revisions never serve navigation");
        let fresh=Arc::new(editor_completion::typed::Fence {project:PROJECT.into(),document:"consumer".into(),revision:2,
            current:AtomicBool::new(true),providers:CompletionTypeCertificate(Arc::new(AtomicBool::new(true)))});
        let locations=exports.locations(fresh);
        authority.release(PROJECT,"a");assert!(!locations.is_current());assert!(locations.site(&id).is_none());
    }
}


fn with_conflicts(arena:&Arc<Mutex<Arena>>,authority:&SourceAuthority,lease:&tree_view::Lease,prior:&Invocation)->Invocation {
    let mut request=prior.original.clone();request.options.truncate(1);let mut payload=HookPayload::Parser(request);
    Invocation::prepare_with_conflicts(Arc::clone(arena),authority,OWNER,HOOK,lease.reference(),true,&mut payload).unwrap()
}
fn conflict_entry(invocation:&Invocation,root:u64,key:&str)->WitMetadataEntry {
    let mut entry=export_entry(invocation,root,true,false);let mut value:Value=serde_json::from_str(&entry.value).unwrap();
    value["projectConflictKey"]=key.into();entry.value=value.to_string();entry
}
fn private_conflict_record(authority:&mut SourceAuthority,stamp:ProviderStamp,a:DeclarationSource,sources:Vec<DeclarationSource>,key:&str)->DeclarationExports {
    let (arena,mut lease,prior,root)=setup(authority,stamp,a,sources,vec![],true);
    let invocation=with_conflicts(&arena,authority,&lease,&prior);
    let first=invocation.bind(&input("private-first"),&conflict_entry(&invocation,root,key),&mut lease).unwrap();
    let mut second_input=input("private-second");second_input.patterns=vec!["second fixture value".into()];
    let second=invocation.bind(&second_input,&conflict_entry(&invocation,root,key),&mut lease).unwrap();
    finish(&arena,vec![("private-first",first),("private-second",second)])
}

#[test]
fn private_noncallable_aliases_export_one_conflict_identity_without_a_callable_or_import_token() {
    let mut authority=SourceAuthority::default();let stamp=provider();let a=source("a");let b=source("b");
    let record=private_conflict_record(&mut authority,stamp.clone(),a.clone(),vec![a.clone(),b.clone()],"opaque identifier");
    assert!(record.complete());assert!(record.0.bindings.is_empty());assert!(record.0.selected.is_empty());
    assert_eq!(record.0.conflicts.len(),1,"only conflict facts deduplicate aliases at the source root");
    let (arena,mut lease,prior,_)=setup(&mut authority,stamp,b.clone(),vec![a,b],vec![record],true);
    let invocation=with_conflicts(&arena,&authority,&lease,&prior);
    let sidecar:Value=serde_json::from_str(&invocation.original.options[2].value).unwrap();
    assert_eq!(invocation.original.options[2].key,CONFLICTS);
    assert_eq!(sidecar["complete"],true);assert_eq!(sidecar["conflicts"].as_array().unwrap().len(),1);
    let key=&sidecar["conflicts"][0];assert_eq!(key["uri"],"file:///project/a.sk");assert_eq!(key["key"],"opaque identifier");
    assert_eq!(key.as_object().unwrap().len(),4,"only source URI/root/namespace/key travel");
    assert!(invocation.imports.is_empty());
    let headers:Value=serde_json::from_str(&invocation.original.options[1].value).unwrap();assert_eq!(headers["declarations"],json!([]));
    let entry=WitMetadataEntry {key:IMPORT.into(),owner_component_id:None,value:json!({"version":1,"reference":key["root"]}).to_string()};
    assert!(invocation.bind(&input("foreign-private"),&entry,&mut lease).is_err(),"a conflict root is not an import capability");
}

#[test]
fn private_conflict_changes_participate_in_source_and_body_fingerprints() {
    let mut fingerprints=Vec::new();let mut body_keys=Vec::new();
    for key in ["first identity","second identity"] {
        let mut authority=SourceAuthority::default();let stamp=provider();let a=source("a");let b=source("b");
        let record=private_conflict_record(&mut authority,stamp.clone(),a.clone(),vec![a.clone(),b.clone()],key);
        fingerprints.push(record.fingerprint().to_owned());
        let (arena,lease,prior,_)=setup(&mut authority,stamp,b.clone(),vec![a,b],vec![record],true);
        let _current=with_conflicts(&arena,&authority,&lease,&prior);body_keys.push(lock(&arena).unwrap().body_key.clone());
    }
    assert_ne!(fingerprints[0],fingerprints[1]);assert_ne!(body_keys[0],body_keys[1]);
}

#[test]
fn conflict_keys_require_opt_in_and_cannot_claim_complete_empty_after_rejection() {
    let mut authority=SourceAuthority::default();let a=source("a");
    let (arena,mut lease,invocation,root)=setup(&mut authority,provider(),a.clone(),vec![a],vec![],true);
    assert!(invocation.bind(&input("old-contract"),&conflict_entry(&invocation,root,"private id"),&mut lease).is_err());
    assert!(!finish(&arena,vec![]).complete());
    for key in [String::new(),"bad\nkey".into(),"x".repeat(1025)] {
        let mut authority=SourceAuthority::default();let a=source("a");
        let (arena,mut lease,prior,root)=setup(&mut authority,provider(),a.clone(),vec![a],vec![],true);
        let invocation=with_conflicts(&arena,&authority,&lease,&prior);
        assert!(invocation.bind(&input("invalid-key"),&conflict_entry(&invocation,root,&key),&mut lease).is_err());
        assert!(!finish(&arena,vec![]).complete());
    }
}

#[test]
fn incomplete_private_conflict_view_is_unknown_and_legacy_inputs_stay_unchanged() {
    let mut authority=SourceAuthority::default();let a=source("a");let b=source("b");
    let (arena,lease,prior,_)=setup(&mut authority,provider(),b.clone(),vec![a,b],vec![],true);
    assert_eq!(prior.original.options.len(),2);
    let current=with_conflicts(&arena,&authority,&lease,&prior);
    let sidecar:Value=serde_json::from_str(&current.original.options[2].value).unwrap();
    assert_eq!(sidecar["complete"],false);assert_eq!(sidecar["conflicts"],json!([]));assert!(!lock(&arena).unwrap().imports_complete);
    let reply=status_output(&current,json!({"version":1,"complete":true}));assert_eq!(current.status(&reply).unwrap(),Some(true));
    let mut changed=reply;if let Some(HookPayload::Parser(reply))=&mut changed.replacement {reply.options[2].value.push(' ');}
    assert!(current.status(&changed).is_err(),"the conflict sidecar is an immutable input, even for semantically equivalent JSON");
}
