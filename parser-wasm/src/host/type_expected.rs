//! Per-Type checkpoint: borrow the already validated native input, snapshot the
//! remaining fields exactly, and never expose an incomplete dispatch payload.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct TrustedInput<'a>(&'a str);
impl<'a> TrustedInput<'a> {
    pub(super) fn checked(expected: &'a WitExpressionPayload, output: &WitExpressionPayload) -> Option<Self> {
        same_expression_request(output,expected).then_some(Self(&expected.input))
    }
}

pub(super) struct TypeExpected<'a> {
    native_input: TrustedInput<'a>,
    snapshot: WitExpressionPayload,
    #[cfg(test)]
    legacy: bool,
}
pub(super) enum Response {
    Candidates(WitExpressionPayload),
    Empty { restored: WitExpressionPayload, unresolved: Vec<TypeParserUnresolved>, has_outcome: bool },
}

impl<'a> TypeExpected<'a> {
    pub(super) fn capture(payload: &WitExpressionPayload, native_input: TrustedInput<'a>) -> Self {
        // Explicit construction makes a WIT field addition a compile-time audit
        // point. In particular no struct-update payload.clone() may copy input.
        let snapshot = WitExpressionPayload {
            input: String::new(),
            context: payload.context.clone(),
            active_type: payload.active_type.clone(),
            remaining: payload.remaining.clone(),
            span: payload.span.clone(),
            expected_types: payload.expected_types.clone(),
            candidate_ends: payload.candidate_ends.clone(),
            allow_literals: payload.allow_literals,
            allow_expressions: payload.allow_expressions,
            time: payload.time,
            depth: payload.depth,
            type_options: payload.type_options.clone(),
            literal_options: payload.literal_options.clone(),
            type_parser_unresolved: payload.type_parser_unresolved.clone(),
            type_parser_outcome: payload.type_parser_outcome,
            candidates: payload.candidates.clone(),
        };
        Self { native_input,snapshot, #[cfg(test)] legacy:false }
    }

    #[cfg(test)]
    pub(super) fn legacy(payload: &WitExpressionPayload, native_input: TrustedInput<'a>) -> Self {
        Self { native_input,snapshot:payload.clone(),legacy:true }
    }
    fn is_legacy(&self) -> bool {
        #[cfg(test)] { self.legacy }
        #[cfg(not(test))] { false }
    }

    pub(super) fn resolve(mut self, mut output: WitExpressionPayload) -> Result<Response,String> {
        let valid = if self.is_legacy() { same_expression_request(&output,&self.snapshot) }
            else { output.input == self.native_input.0 && same_expression_request_fields(&output,&self.snapshot) };
        if !valid { return Err("Type hook changed immutable Expression request fields".to_owned()); }
        if !output.candidates.is_empty() { return Ok(Response::Candidates(output)); }
        let unresolved = mem::take(&mut output.type_parser_unresolved);
        let has_outcome = output.type_parser_outcome.is_some();
        if !self.is_legacy() {
            // Input reuse is private to this validated path. All other fields,
            // including mutable prestate, come from the exact native checkpoint.
            self.snapshot.input = output.input;
        }
        Ok(Response::Empty { restored:self.snapshot,unresolved,has_outcome })
    }

    pub(super) fn restore_rejected(mut self, output: HookPayload) -> (WitExpressionPayload,bool) {
        if self.is_legacy() { return (self.snapshot,false); }
        // Rejected output remains discard-only. Do not add errors for discarded
        // kind/context mutations, but never trust its input without equality.
        let (input,copied) = match output {
            HookPayload::Expression(output) if output.input == self.native_input.0 => (output.input,false),
            _ => (self.native_input.0.to_owned(),true),
        };
        self.snapshot.input = input;
        (self.snapshot,copied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::nlaocs::skript_parser_addon::types::{ExpressionLeafTiming as WitExpressionLeafTiming,ExpressionLiteralSource,TypeParserOutcome};

    fn catalog() -> Arc<Catalog> {
        static CATALOG: std::sync::OnceLock<Arc<Catalog>> = std::sync::OnceLock::new();
        Arc::clone(CATALOG.get_or_init(||Arc::new(ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")).unwrap().into_catalog())))
    }
    fn payload(input: &str) -> WitExpressionPayload {
        let catalog = catalog();
        let types = all_expression_type_options(Some(&catalog));
        let string = types.iter().find(|t|t.code_name=="string").unwrap().clone();
        let range = WitTextRange { start:0,end:input.len() as u64 };
        let context = parse_context_to_wit(&ExpressionParseContext {
            syntax_context:7,
            event_classes:vec![ClassName("org.bukkit.event.player.PlayerJoinEvent".into())],
            section_stack:vec![skript_parser::SectionScopeFrame {
                scope_id:11,parent_scope_id:Some(4),raw_node_id:ParserRawNodeId::new(5),
                kind:skript_parser::SectionScopeKind::Loop,
                definition_id:"section:fixture:definition".into(),registration_id:"section:fixture:registration:0".into(),
                element_class:Some(ClassName("fixture.SecLoop".into())),pattern_index:1,
                pattern:"loop %objects%".into(),source:"loop all players".into(),
                addon_name:Some("Fixture".into()),addon_version:Some("1.0.0".into()),
                owner_component_id:Some("fixture.component".into()),loop_section:true,effect_section:false,section_expression:false,
                captures:vec![crate::host::tests::native_section_scope_capture()],
                metadata:BTreeMap::from([("fixture.component/unknown-v2".into(),"{\"future\":true}".into())]),
            }],
            values:BTreeMap::from([("fixture.value".into(),"present".into())]),
        });
        WitExpressionPayload {
            input:input.into(),context,active_type:Some(string),remaining:range.clone(),
            span:MappedSpan { virtual_range:range.clone(),origins:vec![WitSourceOrigin {
                original_range:range.clone(),kind:WitOriginKind::Exact,expansion:Some(3),
            }] },
            expected_types:vec![WitExpressionExpectedType { class_name:"java.lang.String".into(),plural:false }],
            candidate_ends:vec![range.end],allow_literals:true,allow_expressions:true,time:0,depth:2,
            type_options:types,literal_options:vec![WitExpressionLiteralOption {
                source_record:None,literal_index:Some(2),code_name:"string".into(),class_name:"java.lang.String".into(),
                type_parse_order:0,range,canonical_value:"fixture".into(),source:ExpressionLiteralSource::Supplier,plural:false,
                addon_name:"Fixture".into(),addon_version:"1.0".into(),parser_class:None,parse_contexts:vec!["default".into()],
                value_class:None,represented_class:None,variable_name:None,debug_text:Some("fixture literal".into()),
                enum_constant:None,alias_all:None,alias_type_count:None,
            }],
            type_parser_unresolved:vec![],type_parser_outcome:None,candidates:vec![],
        }
    }
    fn candidate() -> WitExpressionLeafCandidate {
        WitExpressionLeafCandidate { parser_id:"fixture.type".into(),kind:WitExpressionLeafKind::Literal,
            timing:WitExpressionLeafTiming::AfterRegistered,range:WitTextRange {start:0,end:1},
            return_type:Some("java.lang.String".into()),multiplicity:Some(WitDynamicMultiplicity::Single),
            children:vec![],function:None,public_data:vec![],metadata:vec![] }
    }
    fn checkpoint<'a>(payload:&WitExpressionPayload,native:TrustedInput<'a>,legacy:bool) -> TypeExpected<'a> {
        if legacy { TypeExpected::legacy(payload,native) } else { TypeExpected::capture(payload,native) }
    }

    #[test]
    fn native_input_authority_requires_full_initial_request_validation() {
        let native = payload("\"😀 native\"");
        let mut changed = native.clone(); changed.context.values[0].value.push('!');
        assert!(TrustedInput::checked(&native,&changed).is_none());
        changed = native.clone(); changed.input.push('!');
        assert!(TrustedInput::checked(&native,&changed).is_none());
        assert!(TrustedInput::checked(&native,&native).is_some());
    }

    #[test]
    fn every_immutable_request_category_rejects_untrusted_changes_in_both_modes() {
        let original=payload("\"😀 original\"");
        let native=TrustedInput::checked(&original,&original).unwrap();
        let changes: &[(&str,fn(&mut WitExpressionPayload))] = &[
            ("input",|p|p.input.push('!')),
            ("context identity",|p|p.context.syntax_context+=1),
            ("event classes",|p|p.context.event_classes.clear()),
            ("context values",|p|p.context.values[0].value.push('!')),
            ("scope owner",|p|p.context.section_stack[0].owner_component_id=None),
            ("scope metadata",|p|p.context.section_stack[0].metadata[0].value.push('!')),
            ("capture source",|p|p.context.section_stack[0].captures[0].source.push('!')),
            ("capture public data",|p|p.context.section_stack[0].captures[0].summary.as_mut().unwrap().public_data[0].json.push('!')),
            ("active type",|p|p.active_type.as_mut().unwrap().registration_id.push('!')),
            ("active evidence",|p|p.active_type.as_mut().unwrap().source_record.as_mut().unwrap().source_digest.push('!')),
            ("remaining",|p|p.remaining.start+=1),
            ("virtual span",|p|p.span.virtual_range.end+=1),
            ("origin",|p|p.span.origins[0].expansion=Some(99)),
            ("expected type",|p|p.expected_types[0].class_name.push('!')),
            ("expected cardinality",|p|p.expected_types[0].plural=true),
            ("candidate ends",|p|p.candidate_ends.push(0)),
            ("literal eligibility",|p|p.allow_literals=false),
            ("expression eligibility",|p|p.allow_expressions=false),
            ("time",|p|p.time+=1),("depth",|p|p.depth+=1),
            ("type options",|p|p.type_options[0].class_name.push('!')),
            ("type option provenance",|p|p.type_options[0].source_record.as_mut().unwrap().byte_length+=1),
            ("literal value",|p|p.literal_options[0].canonical_value.push('!')),
            ("literal source",|p|p.literal_options[0].source=ExpressionLiteralSource::Alias),
        ];
        for legacy in [false,true] { for (name,change) in changes {
            let expected=checkpoint(&original,native,legacy); let mut output=original.clone();change(&mut output);
            assert!(expected.resolve(output).is_err(),"{legacy}: {name}");
        }}
    }

    #[test]
    fn empty_outputs_restore_all_prestate_and_keep_any_outcome_distinct_from_absence() {
        let mut original=payload(&"😀 native input\n".repeat(8192));
        original.type_parser_unresolved.push(TypeParserUnresolved {reason:"prior reason".into(),required_provider:Some("prior provider".into())});
        original.type_parser_outcome=Some(TypeParserOutcome::Handled); original.candidates.push(candidate());
        let native=TrustedInput::checked(&original,&original).unwrap();
        for legacy in [false,true] { for outcome in [None,Some(TypeParserOutcome::Handled),Some(TypeParserOutcome::NoMatch)] {
            let pointer=original.input.as_ptr(); let expected=checkpoint(&original,native,legacy);
            assert_eq!(original.input.as_ptr(),pointer);
            if !legacy { assert!(expected.snapshot.input.is_empty()); }
            let mut output=original.clone(); let echo_pointer=output.input.as_ptr();
            output.candidates.clear();output.type_parser_outcome=outcome;
            output.type_parser_unresolved=vec![TypeParserUnresolved {reason:"current unresolved".into(),required_provider:None}];
            let Response::Empty {restored,unresolved,has_outcome}=expected.resolve(output).unwrap() else {panic!("expected empty")};
            assert_eq!(format!("{restored:?}"),format!("{original:?}"));
            assert_eq!(unresolved[0].reason,"current unresolved");assert_eq!(has_outcome,outcome.is_some());
            if !legacy { assert_eq!(restored.input.as_ptr(),echo_pointer); }
        }}
    }

    #[test]
    fn rejected_mutations_are_discarded_and_input_repairs_never_use_foreign_text() {
        let original=payload("\"😀 original\"");let native=TrustedInput::checked(&original,&original).unwrap();
        for legacy in [false,true] { for changed_input in [false,true] {
            let expected=checkpoint(&original,native,legacy);let mut output=original.clone();
            output.context.section_stack.clear();output.active_type=None;output.expected_types.clear();
            output.type_options.clear();output.literal_options.clear();output.time=99;
            output.candidates.push(candidate());output.type_parser_outcome=Some(TypeParserOutcome::NoMatch);
            if changed_input { output.input="foreign input".into(); }
            let echo_pointer=output.input.as_ptr();
            let (restored,copied)=expected.restore_rejected(HookPayload::Expression(output));
            assert_eq!(format!("{restored:?}"),format!("{original:?}"));
            assert_eq!(copied,!legacy && changed_input);
            if !legacy && !changed_input { assert_eq!(restored.input.as_ptr(),echo_pointer); }
        }
        let (restored,copied)=checkpoint(&original,native,legacy).restore_rejected(crate::host::tests::registered_expression());
        assert_eq!(format!("{restored:?}"),format!("{original:?}"));assert_eq!(copied,!legacy);
        }
    }

    #[test]
    fn rejected_and_empty_types_do_not_contaminate_a_later_success() {
        let original=payload("\"native\"");let native=TrustedInput::checked(&original,&original).unwrap();
        for legacy in [false,true] {
            let mut current=original.clone();
            let saved=checkpoint(&current,native,legacy);let mut rejected=current.clone();rejected.input="bad".into();
            current=saved.restore_rejected(HookPayload::Expression(rejected)).0;
            current.active_type.as_mut().unwrap().registration_id.push_str(":second");
            current.type_options.reverse();let second_expected=format!("{current:?}");
            let expected=checkpoint(&current,native,legacy);let mut empty=current.clone();empty.type_parser_outcome=Some(TypeParserOutcome::NoMatch);
            let Response::Empty {restored,has_outcome,..}=expected.resolve(empty).unwrap() else {panic!("expected empty")};
            assert!(has_outcome);assert_eq!(format!("{restored:?}"),second_expected);current=restored;
            let expected=checkpoint(&current,native,legacy);let mut success=current.clone();success.candidates.push(candidate());
            let snapshot=format!("{success:?}");
            let Response::Candidates(output)=expected.resolve(success).unwrap() else {panic!("expected candidates")};
            assert_eq!(format!("{output:?}"),snapshot);
        }
    }

    #[test]
    fn actual_type_interpolation_continuations_preserve_success_and_rejection_recovery() {
        // Interpolation asks the nested parser for ExpressionsOnly; use a
        // registered Expression rather than a bare numeric Type literal.
        for legacy in [false,true] { for (text,success) in [("\"😀 %pi%\"",true),("\"😀 %not a known expression%\"",false)] {
            let catalog=catalog();let mut host=ParserHost::new(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/../artifacts/core-library.wasm")),HostConfig {
                syntax_catalog:Some(catalog),runtime_profile:RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},..Default::default()
            }).unwrap();host.legacy_type_expected=legacy;
            let transaction=host.begin_parse("file:///type-checkpoint","file:///type-checkpoint/public.sk",1).unwrap();
            let mut request=payload(text);request.context.section_stack.clear();request.literal_options.clear();
            request.span.origins[0].expansion=None;request.depth=0;
            let expected_text=format!("{request:?}");let native=TrustedInput::checked(&request,&request).unwrap();
            let expected=checkpoint(&request,native,legacy);
            let active=request.active_type.as_ref().unwrap();
            let target=DispatchTarget::Registration {syntax_kind:SyntaxKind::Type,definition_id:active.definition_id.clone(),registration_id:active.registration_id.clone()};
            let result=host.dispatch_in_parse(&transaction,DispatchRequest {
                context:InvocationContext {invocation_id:1,subscription_id:String::new(),document_id:"file:///type-checkpoint/public.sk".into(),document_revision:1,expansion:None,syntax_context:7},
                target,phase:HookPhase::Expression,payload:HookPayload::Expression(request.clone())
            }).unwrap();
            assert!(result.fault.is_none(),"{:?}",result.fault);
            if success {
                assert!(!matches!(result.decision,HookDecision::Reject(_)),"{:?}",result.decision);
                assert!(!result.available_parse_results.is_empty(),"real nested parse must execute");
                let HookPayload::Expression(output)=result.payload else {panic!("wrong payload")};
                let Response::Candidates(output)=expected.resolve(output).unwrap() else {panic!("missing interpolated candidate")};
                assert!(same_expression_request(&output,&request));assert!(output.candidates.iter().any(|c|c.return_type.as_deref()==Some("java.lang.String")));
            } else {
                assert!(matches!(result.decision,HookDecision::Reject(_)),"{:?}",result.decision);
                assert!(result.calls.iter().all(|call|call.semantic_receipt.is_none()));
                let (restored,_)=expected.restore_rejected(result.payload);assert_eq!(format!("{restored:?}"),expected_text);
            }
            assert_eq!(transaction.state_revision().unwrap(),0);transaction.cancel().unwrap();
        }}
    }
}
