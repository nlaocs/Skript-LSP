use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types as wit;

fn payload()->WitStructurePayload {
    WitStructurePayload {input:"fixture".into(),body_tree:wit::RawTree {roots:vec![],nodes:vec![],diagnostics:vec![],indentation:None},
        context:wit::ParseContext {syntax_context:7,event_classes:vec![],section_stack:vec![],values:vec![]},
        timing:WitStructureTiming::EnterBody,type_options:vec![],candidate:wit::StructureCandidate {
            raw_node_id:0,definition_id:"fixture".into(),registration_id:"fixture.0".into(),element_class:None,priority:0,registration_order:0,pattern_index:0,
            pattern:"fixture".into(),span:wit::MappedSpan {virtual_range:wit::TextRange {start:0,end:7},origins:vec![]},
            declared_node_type:WitStructureNodeType::Section,actual_node_type:WitStructureNodeType::Section,
            regex_captures:vec![],tags:vec![],mark:0,marks:vec![],parsed_captures:vec![],body_mode:WitStructureBodyMode::Entries,
            child_node_ids:vec![],entries:vec![],handler:None,declarations:vec![],metadata:vec![WitMetadataEntry {
                key:LAYOUT.into(),owner_component_id:None,value:serde_json::json!({"version":1,"token":"17","structureNode":0,"entries":[]}).to_string()}],
        }}
}
fn plan()->WitMetadataEntry {WitMetadataEntry {key:PLANS.into(),owner_component_id:Some("example.owner".into()),
    value:serde_json::json!({"version":1,"token":"17","structureNode":0,"entries":[{"node":1,"declarationIndex":0,"mode":"trigger","adoptUnknown":false,
        "context":{"eventClasses":["test.Scope"],"values":[]}}]}).to_string()}}

#[test]
fn required_entry_and_context_capabilities_reject_old_or_optional_contracts() {
    let prior=HookPayload::Structure(payload());let mut next=payload();next.candidate.metadata.push(plan());let next=HookPayload::Structure(next);
    let requirement=|id:&str,required:bool|wit::CapabilityRequirement {id:id.into(),minimum_version:1,required};
    for requirements in [vec![],vec![requirement(crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,true)],
        vec![requirement(crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,false),requirement(CAPABILITY_CONTEXT_UPDATES,true)]] {
        assert!(validate_owner_requirements(&prior,&next,"example.owner",&requirements).is_err());
    }
    let requirements=[requirement(crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,true),requirement(CAPABILITY_CONTEXT_UPDATES,true)];
    validate_owner_requirements(&prior,&next,"example.owner",&requirements).unwrap();
    let old=host_capabilities().into_iter().filter(|cap|cap.id!=crate::CAPABILITY_STRUCTURE_ENTRY_PLANS).collect::<Vec<_>>();
    assert!(crate::validate_compatibility(crate::AbiVersion::new(31,0),crate::AbiVersion::new(31,0),
        &[crate::CapabilityRequirement::required(crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,1)],&old).is_err());
    assert!(host_capabilities().iter().any(|cap|cap.id==crate::CAPABILITY_STRUCTURE_ENTRY_PLANS&&cap.version==1));
}

#[test]
fn entry_plan_lease_and_scope_are_immutable_at_exit() {
    let mut prior=payload();prior.candidate.metadata.push(plan());let mut next=prior.clone();
    validate_transition(&prior,&next).unwrap();
    prior.timing=WitStructureTiming::ExitBody;next.timing=WitStructureTiming::ExitBody;
    assert!(!changed(&HookPayload::Structure(prior.clone()),&HookPayload::Structure(next.clone())));
    for case in 0..5 {
        let mut bad=next.clone();match case {
            0=>{bad.candidate.metadata.pop();},
            1=>bad.candidate.metadata.last_mut().unwrap().value=bad.candidate.metadata.last().unwrap().value.replace("test.Scope","other.Scope"),
            2=>bad.candidate.metadata.last_mut().unwrap().value=bad.candidate.metadata.last().unwrap().value.replace("\"17\"","\"16\""),
            3=>bad.candidate.body_mode=WitStructureBodyMode::Raw,
            _=>bad.candidate.actual_node_type=WitStructureNodeType::Simple,
        }
        assert!(validate_transition(&prior,&bad).is_err(),"case {case}");
    }
}

#[test]
fn metadata_normalization_does_not_allow_a_second_component_to_replace_entry_authority() {
    let mut prior=payload();prior.candidate.metadata.push(plan());
    let original=HookPayload::Structure(prior.clone());let mut changed=prior;
    changed.candidate.metadata.last_mut().unwrap().value="{}".into();
    assert!(normalize_hook_metadata(&original,&mut HookPayload::Structure(changed),"other.component").is_err());
}

#[test]
fn unused_layout_does_not_add_a_nonce_to_ordinary_body_metadata_or_hide_invalid_plans() {
    let original=payload().candidate.metadata;
    let mut metadata=original.clone();
    metadata.push(WitMetadataEntry {key:LAYOUT.into(),owner_component_id:Some("component.owner".into()),value:"owned".into()});
    discard_unused_layout(&mut metadata);
    assert_eq!(metadata.len(),1);assert_eq!(metadata[0].owner_component_id.as_deref(),Some("component.owner"));
    for owner in [None,Some("component.owner")] {
        let mut metadata=original.clone();metadata.push(WitMetadataEntry {key:PLANS.into(),owner_component_id:owner.map(str::to_owned),value:"invalid".into()});
        discard_unused_layout(&mut metadata);
        assert!(metadata.iter().any(|entry|entry.key==LAYOUT && entry.owner_component_id.is_none()));
    }
}

#[test]
fn a_new_enter_discards_prior_plan_leases_without_erasing_other_metadata() {
    let mut metadata=payload().candidate.metadata;metadata.push(plan());
    metadata.push(WitMetadataEntry {key:PLANS.into(),owner_component_id:Some("other.owner".into()),value:"old".into()});
    metadata.push(WitMetadataEntry {key:LAYOUT.into(),owner_component_id:Some("component.owner".into()),value:"owned".into()});
    metadata.push(WitMetadataEntry {key:"existing.fact".into(),owner_component_id:Some("component.owner".into()),value:"retained".into()});
    reset_for_enter(&mut metadata);
    assert_eq!(metadata.len(),2);assert!(metadata.iter().all(|entry|entry.owner_component_id.as_deref()==Some("component.owner")));
    assert!(metadata.iter().any(|entry|entry.key=="existing.fact"));
}

mod retry_lifetime {
    use super::*;
    use skript_parser as native;
    use std::collections::BTreeMap;
    use syntax_pattern_parser::syntax::{self, PluralRules};

    const OWNER: &str = "example.entry-retry";
    const FIRST: &str = "entry.first";
    const FALLBACK: &str = "entry.fallback";
    const TOKEN_KEY: &str = "test.entry-lease";

    fn common(order: usize, registration: &str, pattern: &str, rules: &PluralRules) -> syntaxes::CommonSyntax {
        syntaxes::CommonSyntax {
            registration_order: order,
            documentation: syntaxes::Documentation::default(),
            id: None,
            element_class: syntaxes::ClassName("test.EntryRetry".into()),
            super_class: None,
            no_doc: false,
            events: vec![],
            deprecated: None,
            priority_name: None,
            priority: None,
            patterns: vec![syntaxes::Pattern {
                source: pattern.into(),
                parsed: syntax::parse(pattern, rules).unwrap(),
            }],
            addon: syntaxes::Addon { name: "Skript".into(), version: "test".into() },
            definition_id: syntaxes::DefinitionId(format!("definition:{registration}")),
            registration_id: syntaxes::RegistrationId(registration.into()),
            related_property: None,
            supported_events: None,
            supported_events_state: None,
            experimental_syntax: None,
            experimental_syntax_state: None,
            return_handler: None,
            return_handler_state: None,
        }
    }

    fn entry(key: &str, kind: syntaxes::EntryKind, optional: bool) -> syntaxes::EntryData {
        syntaxes::EntryData {
            key: key.into(), default_value: None, optional, multiple: false,
            entry_data_class: syntaxes::ClassName("test.EntryData".into()), kind,
            separator: Some(": ".into()), value_type: None, string_mode: None,
            return_types: vec![], flags: None, nested_validator: None,
        }
    }

    fn catalog(reject_first_body: bool) -> syntaxes::Catalog {
        let rules = PluralRules::from_json(r#"{
            "algorithm":"singular-aware","pluralOverrideSupported":false,
            "rules":[{"ruleOrder":0,"singular":"","plural":"s","completeWord":false,
                "origin":"built-in","addon":{"name":"Skript","version":"test"}}]
        }"#).unwrap();
        let change = entry("change", syntaxes::EntryKind::Unknown, true);
        let mut first_entries = vec![change.clone()];
        if reject_first_body {
            // This rejects the whole first Structure body, rather than merely
            // adding a recoverable statement diagnostic inside a valid body.
            first_entries.push(entry("required", syntaxes::EntryKind::KeyValue, false));
        }
        let structures = [
            (0, FIRST, "root", Some(syntaxes::EntryValidator { entry_data: first_entries })),
            (1, FALLBACK, "root", Some(syntaxes::EntryValidator { entry_data: vec![change] })),
            (2, "entry.after", "after", None),
        ];
        let mut syntaxes = structures.into_iter().map(|(order, id, pattern, entry_validator)| {
            syntaxes::Syntax::Structure(syntaxes::Structure {
                common: common(order, id, pattern, &rules), entry_validator,
                node_type: Some(syntaxes::NodeType::Section),
            })
        }).collect::<Vec<_>>();
        syntaxes.push(syntaxes::Syntax::Effect(syntaxes::Effect {
            common: common(3, "entry.probe", "probe", &rules),
        }));
        syntaxes::Catalog::new(syntaxes::CatalogParts {
            syntaxes, converters: vec![], comparators: vec![], event_values: vec![],
            properties: vec![], operators: vec![], operations: BTreeMap::new(),
            differences: vec![], classes: vec![],
            aliases: syntaxes::AliasRegistry { aliases: BTreeMap::new(), targets: vec![] },
            plural_rules: rules, language: BTreeMap::new(),
        })
    }

    fn token(text: &str) -> String {
        serde_json::from_str::<serde_json::Value>(text).unwrap()["token"]
            .as_str().unwrap().to_owned()
    }

    struct Enter {
        registration: String,
        fresh: String,
        incoming: Option<String>,
    }
    #[derive(Default)]
    struct Environment {
        reject_at_exit: bool,
        enters: Vec<Enter>,
        exits: Vec<String>,
        seen: Vec<(Vec<syntaxes::ClassName>, Option<String>)>,
    }
    impl native::PatternMatchEnvironment for Environment {
        fn resolve_type(&mut self, _: native::TypeExpressionRequest<'_>) -> Result<native::TypeExpressionOutcome, String> {
            Ok(Default::default())
        }
        fn dispatch_hook(&mut self, _: native::PatternHookEvent<'_>) -> Result<native::PatternHookControl, String> {
            Ok(native::PatternHookControl::Continue)
        }
    }
    impl native::ExpressionParseEnvironment for Environment {
        fn state_revision(&self) -> Result<u64, String> { Ok(0) }
        fn parse_expression_leaf(&mut self, _: native::ExpressionLeafRequest<'_>) -> Result<native::ExpressionLeafParse, String> {
            Ok(Default::default())
        }
        fn enter_structure(&mut self, request: native::StructureHookRequest<'_>) -> Result<native::StructureHookDecision, String> {
            if request.candidate.entry_validator.is_none() {
                return Ok(native::StructureHookDecision::Accept {
                    context: request.context.clone(), body_mode: native::StructureBodyMode::Trigger,
                    entry_body_modes: BTreeMap::new(), metadata: request.candidate.metadata.clone(),
                });
            }
            let incoming = request.candidate.metadata.get(&format!("{OWNER}/{PLANS}")).map(|value| token(value));
            // Production projection invokes input_metadata exactly once. In
            // particular, the retry request still owns its first Enter's plan.
            let original = structure_hook_payload(&request, None)?;
            let fresh = token(&original.candidate.metadata.iter()
                .find(|m| m.key == LAYOUT && m.owner_component_id.is_none()).unwrap().value);
            let root = request.tree.get(request.candidate.raw_node_id).unwrap();
            let child = root.children.iter().filter_map(|id| request.tree.get(*id))
                .find(|child| child.text == "change number").unwrap();
            let registration = request.candidate.matched.registration_id.clone();
            let mut next = original.clone();
            next.candidate.body_mode = WitStructureBodyMode::Entries;
            // Simulate a provider replacing its own plan, not manufacturing a
            // duplicate-metadata error before the native transition check.
            next.candidate.metadata.retain(|m| !(m.key == PLANS && m.owner_component_id.as_deref() == Some(OWNER)));
            next.candidate.metadata.push(WitMetadataEntry {
                key: PLANS.into(), owner_component_id: None,
                value: serde_json::json!({"version":1,"token":fresh,"structureNode":root.id.get(),"entries":[{
                    "node":child.id.get(),"declarationIndex":0,"mode":"trigger","adoptUnknown":true,
                    "context":{"eventClasses":[format!("test.{registration}")],
                        "values":[{"key":TOKEN_KEY,"value":fresh}]}
                }]}).to_string(),
            });
            let prior_payload = HookPayload::Structure(original.clone());
            let mut replacement = HookPayload::Structure(next);
            normalize_hook_metadata(&prior_payload, &mut replacement, OWNER)?;
            let requirements = [
                wit::CapabilityRequirement { id: crate::CAPABILITY_STRUCTURE_ENTRY_PLANS.into(), minimum_version: 1, required: true },
                wit::CapabilityRequirement { id: CAPABILITY_CONTEXT_UPDATES.into(), minimum_version: 1, required: true },
            ];
            validate_owner_requirements(&prior_payload, &replacement, OWNER, &requirements)?;
            let HookPayload::Structure(next) = replacement else { unreachable!() };
            // Before reset_for_enter this fails on retry: previous has the old
            // owned plan but input_metadata has already issued a new layout.
            validate_transition(&original, &next)?;
            self.enters.push(Enter { registration, fresh, incoming });
            Ok(native::StructureHookDecision::Accept {
                context: request.context.clone(), body_mode: native::StructureBodyMode::Entries,
                entry_body_modes: BTreeMap::new(), metadata: metadata_entries(next.candidate.metadata)?,
            })
        }
        fn resolve_effect_candidate(&mut self, request: native::EffectSemanticRequest<'_>) -> Result<native::EffectSemanticDecision, String> {
            let current = request.context.values.get(TOKEN_KEY).cloned();
            self.seen.push((request.context.event_classes.clone(), current.clone()));
            Ok(native::EffectSemanticDecision::Accepted {
                context: request.context.clone(), handler: None,
                metadata: current.map(|value| BTreeMap::from([(TOKEN_KEY.into(), value)])).unwrap_or_default(),
            })
        }
        fn exit_structure(&mut self, request: native::StructureHookRequest<'_>) -> Result<native::StructureExitDecision, String> {
            let registration = &request.candidate.matched.registration_id;
            self.exits.push(registration.clone());
            if request.candidate.entry_validator.is_some() {
                let projected = structure_hook_payload(&request, None)?;
                let current = token(&projected.candidate.metadata.iter()
                    .find(|m| m.key == LAYOUT && m.owner_component_id.is_none()).unwrap().value);
                let last = self.enters.iter().rev().find(|e| e.registration == *registration).unwrap();
                assert_eq!(current, last.fresh, "Exit must retain its accepted Enter lease");
                assert_eq!(decode(&projected)?.owner.as_deref(), Some(OWNER));
            }
            if self.reject_at_exit && registration == FIRST {
                return Ok(native::StructureExitDecision::Reject { reason: "reject first after body".into(), diagnostics: vec![] });
            }
            Ok(native::StructureExitDecision::Accept { metadata: request.candidate.metadata.clone() })
        }
    }

    fn run(reject_first_body: bool) {
        let source = native::MappedSource::identity("# Unicode \u{65e5}\u{672c}\u{8a9e} \u{1f600}\nroot:\n    change number:\n        probe\nafter:\n    probe\n");
        let tree = native::parse_raw_tree(&source, native::RawTreeOptions::for_skript_version(2, 15));
        let mut context = native::ExpressionParseContext::default();
        context.syntax_context = 42;
        context.event_classes = vec![syntaxes::ClassName("test.Outer".into())];
        let mut environment = Environment { reject_at_exit: !reject_first_body, ..Default::default() };
        let document = native::parse_structures(&catalog(reject_first_body), native::StructureParseRequest {
            source: &source, tree: &tree, context,
        }, &mut environment, native::StructureParserConfig::default()).expect("fallback must reacquire a current entry plan");
        let candidate = document.roots.iter().find_map(|node| match node {
            native::StructureDocumentNode::Structure(matches) => matches.selected.as_ref()
                .filter(|candidate| candidate.matched.registration_id == FALLBACK),
            _ => None,
        }).expect("the second registration must be adopted after first body/Exit rejection");
        // Overlapping header candidates may retain the normal ambiguity notice;
        // a rejected first candidate must not leave entry/fault diagnostics.
        assert!(document.diagnostics.iter().all(|d| matches!(d.kind, native::StructureDiagnosticKind::MultipleClaims)), "{:?}", document.diagnostics);
        let retries = environment.enters.iter().filter(|e| e.registration == FALLBACK).collect::<Vec<_>>();
        assert_eq!(retries.len(), 2, "fallback must run initial Enter and a real retry Enter");
        assert!(retries[0].incoming.is_none());
        assert_eq!(retries[1].incoming.as_deref(), Some(retries[0].fresh.as_str()), "retry really carried its old native candidate plan");
        assert_ne!(retries[0].fresh, retries[1].fresh, "freshness must not assume adjacent global counter values");
        assert_eq!(token(&candidate.metadata[LAYOUT]), retries[1].fresh);
        let plans = native::validated_structure_entry_plans(candidate, &tree, native::StructureBodyMode::Entries).unwrap();
        assert_eq!(plans.owner.as_deref(), Some(OWNER));
        let native::StructureBody::Entries(entries) = &candidate.body else { panic!("adopted entries") };
        let native::StructureEntryValue::Trigger(body) = &entries[0].value else { panic!("adopted unknown entry body") };
        let native::SectionBodyNode::Effect(effect) = &body[0] else { panic!("adopted probe") };
        assert_eq!(effect.selected.as_ref().unwrap().metadata.get(TOKEN_KEY), Some(&retries[1].fresh));
        assert_eq!(environment.seen.len(), 3, "first, retried fallback, and following ordinary root bodies");
        assert_eq!(environment.seen[0].0, vec![syntaxes::ClassName(format!("test.{FIRST}"))]);
        assert_eq!(environment.seen[1].0, vec![syntaxes::ClassName(format!("test.{FALLBACK}"))]);
        assert_eq!(environment.seen[1].1.as_deref(), Some(retries[1].fresh.as_str()));
        assert_eq!(environment.seen[2], (vec![syntaxes::ClassName("test.Outer".into())], None));
        assert_eq!(environment.exits.iter().any(|id| id == FIRST), !reject_first_body);
    }

    #[test]
    fn native_alternative_reenter_after_exit_rejection_uses_a_new_entry_lease() { run(false); }
    #[test]
    fn native_alternative_reenter_after_entry_body_rejection_uses_a_new_entry_lease() { run(true); }
}


#[test]
fn raw_data_plans_need_a_separate_mandatory_capability() {
    let prior=HookPayload::Structure(payload());let mut next=payload();let mut raw=plan();
    let mut value:serde_json::Value=serde_json::from_str(&raw.value).unwrap();
    value["entries"][0]["mode"]="raw".into();value["entries"][0]["context"]=serde_json::json!({"eventClasses":null,"values":[]});
    raw.value=value.to_string();next.candidate.metadata.push(raw);let next=HookPayload::Structure(next);
    let requirement=|id:&str,required:bool|wit::CapabilityRequirement {id:id.into(),minimum_version:1,required};
    let mut requirements=vec![requirement(crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,true),requirement(CAPABILITY_CONTEXT_UPDATES,true)];
    assert!(validate_owner_requirements(&prior,&next,"example.owner",&requirements).is_err());
    requirements.push(requirement(crate::CAPABILITY_STRUCTURE_ENTRY_DATA,false));
    assert!(validate_owner_requirements(&prior,&next,"example.owner",&requirements).is_err());
    requirements.last_mut().unwrap().required=true;
    validate_owner_requirements(&prior,&next,"example.owner",&requirements).unwrap();
    let old=host_capabilities().into_iter().filter(|cap|cap.id!=crate::CAPABILITY_STRUCTURE_ENTRY_DATA).collect::<Vec<_>>();
    assert!(crate::validate_compatibility(crate::AbiVersion::new(31,0),crate::AbiVersion::new(31,0),
        &[crate::CapabilityRequirement::required(crate::CAPABILITY_STRUCTURE_ENTRY_DATA,1)],&old).is_err());
}


#[test]
fn key_alias_requires_the_explicit_mandatory_host_capability_in_addition_to_data_scope() {
    let prior=HookPayload::Structure(payload());let mut next=payload();let mut record=plan();
    let mut wire:serde_json::Value=serde_json::from_str(&record.value).unwrap();
    wire["entries"][0]["mode"]="raw".into();wire["entries"][0]["adoptUnknown"]=true.into();wire["entries"][0]["keyAlias"]="alternate".into();
    wire["entries"][0]["context"]=serde_json::json!({"eventClasses":null,"values":[]});record.value=wire.to_string();
    next.candidate.metadata.push(record);let next=HookPayload::Structure(next);
    let requirement=|id:&str,required|wit::CapabilityRequirement {id:id.into(),minimum_version:1,required};
    let mut claims=vec![requirement(crate::CAPABILITY_STRUCTURE_ENTRY_PLANS,true),requirement(crate::CAPABILITY_STRUCTURE_ENTRY_DATA,true),requirement(CAPABILITY_CONTEXT_UPDATES,true)];
    assert!(validate_owner_requirements(&prior,&next,"example.owner",&claims).is_err());
    claims.push(requirement(crate::CAPABILITY_STRUCTURE_ENTRY_ALIASES,false));
    assert!(validate_owner_requirements(&prior,&next,"example.owner",&claims).is_err());
    claims.last_mut().unwrap().required=true;validate_owner_requirements(&prior,&next,"example.owner",&claims).unwrap();
    let old=host_capabilities().into_iter().filter(|cap|cap.id!=crate::CAPABILITY_STRUCTURE_ENTRY_ALIASES).collect::<Vec<_>>();
    assert!(crate::validate_compatibility(crate::AbiVersion::new(31,0),crate::AbiVersion::new(31,0),
        &[crate::CapabilityRequirement::required(crate::CAPABILITY_STRUCTURE_ENTRY_ALIASES,1)],&old).is_err());
}
